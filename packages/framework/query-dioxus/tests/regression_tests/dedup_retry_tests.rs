//! Discriminating regression guards for sibling query dedup, post-failure late-mount recovery,
//! stale revalidation after external `prefetch`, and `refetch()` retry-backoff preservation.

use core::task::{Poll, Waker};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_ui::prelude::*;

use crate::{pump_settled, render_once, GetItem, NoopFlagWaker, RegTransport};

#[test]
fn mid_flight_sibling_mount_does_not_refire_when_in_flight_fetch_fails() {
    let transport = RegTransport::new("Err");
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.fail_queries = true;
    }
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    type ToggleSlot = Rc<RefCell<Option<Signal<bool>>>>;
    let toggle_slot: ToggleSlot = Rc::new(RefCell::new(None));

    #[component]
    fn Sibling(tag: &'static str) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 0;
        let q = use_query_with::<GetItem>(1, opts);
        let has_err = q.error().is_some();
        rsx! { text { "{tag}:ERR:{has_err}" } }
    }

    #[component]
    fn Root() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let slot = dioxus_core::consume_context::<ToggleSlot>();
        let show_b = use_signal(|| false);
        use_hook(move || {
            *slot.borrow_mut() = Some(show_b);
        });
        rsx! {
            QueryClientProvider {
                client: client,
                Col {
                    Sibling { tag: "A" }
                    if show_b() {
                        Sibling { tag: "B" }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, Root)
        .with_context(client)
        .with_context(Rc::clone(&toggle_slot));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    // Frame 1: Sibling A starts fetch and parks in-flight on RegTransport.
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 0);

    // Mount Sibling B WHILE Sibling A's fetch is in-flight!
    let mut show_b = toggle_slot.borrow().unwrap();
    surface.in_root_scope(|| show_b.set(true));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    // Release transport so Sibling A's in-flight fetch fails -> Sibling B must NOT fire a 2nd call!
    transport.release_all();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(
        transport.inner.lock().unwrap().query_calls,
        1,
        "sibling mounted while fetch is in-flight must not refire when that fetch fails"
    );
    let html = web.render_html(surface.store());
    assert!(html.contains("A:ERR:true"));
    assert!(html.contains("B:ERR:true"));
}

#[test]
fn late_component_mount_after_failure_retries_and_recovers() {
    let transport = RegTransport::new("Recovered");
    transport.inner.lock().unwrap().fail_queries = true;
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    type ToggleSlot = Rc<RefCell<Option<Signal<bool>>>>;
    let toggle_slot: ToggleSlot = Rc::new(RefCell::new(None));

    #[component]
    fn Consumer(tag: &'static str) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 0;
        let q = use_query_with::<GetItem>(7, opts);
        let label = q
            .data()
            .map(|d| d.name.clone())
            .unwrap_or_else(|| if q.error().is_some() { "ERR".into() } else { "LOAD".into() });
        rsx! { text { "{tag}:{label}" } }
    }

    #[component]
    fn Root() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let slot = dioxus_core::consume_context::<ToggleSlot>();
        let show_b = use_signal(|| false);
        use_hook(move || {
            *slot.borrow_mut() = Some(show_b);
        });
        rsx! {
            QueryClientProvider {
                client: client,
                Col {
                    Consumer { tag: "A" }
                    if show_b() {
                        Consumer { tag: "B" }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, Root)
        .with_context(client)
        .with_context(Rc::clone(&toggle_slot));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    // Component A mounts while transport is failing -> settles in error state (1 call).
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 1);
    assert!(web.render_html(surface.store()).contains("A:ERR"));

    // Transport recovers; Component B mounts later while A stays mounted -> MUST retry & recover!
    transport.inner.lock().unwrap().fail_queries = false;
    let mut show_b = toggle_slot.borrow().unwrap();
    surface.in_root_scope(|| show_b.set(true));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    assert_eq!(
        transport.inner.lock().unwrap().query_calls,
        2,
        "new component mounting after a completed failure must retry the slot"
    );
    let html = web.render_html(surface.store());
    assert!(html.contains("A:Recovered"), "existing subscriber must see recovered data: {html}");
    assert!(html.contains("B:Recovered"), "new subscriber must see recovered data: {html}");
}

#[test]
fn external_prefetch_after_failed_mount_and_stale_remount_revalidate_cleanly() {
    let clock = Shared::new(ManualClock::new(1_000));
    let transport = RegTransport::new("V1");
    transport.inner.lock().unwrap().fail_queries = true;
    let client = erased_query_client_with_clock(
        transport.clone(),
        clock.clone(),
        QueryClientConfig::default(),
    );

    type ToggleSlot = Rc<RefCell<Option<Signal<bool>>>>;
    let toggle_slot: ToggleSlot = Rc::new(RefCell::new(None));

    #[component]
    fn StaleView(tag: &'static str) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(5_000);
        opts.max_retries = 0;
        let q = use_query_with::<GetItem>(1, opts);
        let label = q
            .data()
            .map(|d| d.name.clone())
            .unwrap_or_else(|| if q.error().is_some() { "ERR".into() } else { "LOAD".into() });
        rsx! { text { "{tag}:{label}" } }
    }

    #[component]
    fn Root() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let slot = dioxus_core::consume_context::<ToggleSlot>();
        let show_b = use_signal(|| false);
        use_hook(move || {
            *slot.borrow_mut() = Some(show_b);
        });
        rsx! {
            QueryClientProvider {
                client: client,
                Col {
                    StaleView { tag: "A" }
                    if show_b() {
                        StaleView { tag: "B" }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, Root)
        .with_context(client.clone())
        .with_context(Rc::clone(&toggle_slot));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    // 1. Component A mounts while failing -> settles in error state (call 1).
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 1);
    assert!(web.render_html(surface.store()).contains("A:ERR"));

    // 2. Transport recovers; external `client.prefetch()` populates the SAME slot at the SAME
    //    `inv_ver` ("V1", call 2) while Component A stays mounted.
    transport.inner.lock().unwrap().fail_queries = false;
    {
        let mut fut = Box::pin(client.prefetch::<GetItem>(&1));
        let mut cx = core::task::Context::from_waker(&waker);
        let res = match core::pin::Pin::as_mut(&mut fut).poll(&mut cx) {
            Poll::Ready(r) => r.unwrap(),
            Poll::Pending => panic!("immediate RegTransport must resolve prefetch"),
        };
        assert_eq!(res.name, "V1");
    }
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 2);
    assert!(web.render_html(surface.store()).contains("A:V1"));

    // 3. Advance past stale_time_ms(5_000) and mount B -> MUST revalidate to "V2" (call 3)!
    //    (Under sticky `last_attempted`, the earlier failure token stalled B indefinitely.)
    clock.advance(6_000);
    transport.inner.lock().unwrap().name = "V2".into();
    let mut show_b = toggle_slot.borrow().unwrap();
    surface.in_root_scope(|| show_b.set(true));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(
        transport.inner.lock().unwrap().query_calls,
        3,
        "stale slot populated via external prefetch after earlier failure must revalidate on mount"
    );
    assert!(web.render_html(surface.store()).contains("A:V2"));
    assert!(web.render_html(surface.store()).contains("B:V2"));
}

#[test]
fn sibling_mount_during_failing_or_retry_sleeping_refetch_preserves_dedup_and_backoff() {
    #[derive(Default)]
    struct SleepGate {
        released: bool,
        wakers: Vec<Waker>,
    }

    let sleep_gate = Arc::new(Mutex::new(SleepGate::default()));
    let gate_for_sleep = Arc::clone(&sleep_gate);
    let sleep = SleepHandle::new(move |_ms| {
        let gate = Arc::clone(&gate_for_sleep);
        Box::pin(async move {
            core::future::poll_fn(|cx| {
                let mut g = gate.lock().unwrap();
                if g.released {
                    Poll::Ready(())
                } else {
                    g.wakers.push(cx.waker().clone());
                    Poll::Pending
                }
            })
            .await;
        })
    });

    let clock = Shared::new(ManualClock::new(1_000));
    let transport = RegTransport::new("V1");
    let client = erased_query_client_with_clock(
        transport.clone(),
        clock.clone(),
        QueryClientConfig::default(),
    );

    type CtrlSlot = Rc<RefCell<Option<(Signal<bool>, Signal<bool>, UseQuery<GetItem>)>>>;
    let ctrl_slot: CtrlSlot = Rc::new(RefCell::new(None));

    #[component]
    fn Primary(show_b: Signal<bool>, show_c: Signal<bool>) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 2;
        let q = use_query_with::<GetItem>(9, opts);
        let slot = dioxus_core::consume_context::<CtrlSlot>();
        use_hook(move || {
            *slot.borrow_mut() = Some((show_b, show_c, q));
        });
        let label = q.data().map(|d| d.name.clone()).unwrap_or_default();
        rsx! { text { "P:{label}" } }
    }

    #[component]
    fn LateSibling(tag: &'static str) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 2;
        let q = use_query_with::<GetItem>(9, opts);
        let label = q.data().map(|d| d.name.clone()).unwrap_or_default();
        rsx! { text { "{tag}:{label}" } }
    }

    #[component]
    fn Root() -> Element {
        let (client, sleep) = dioxus_core::consume_context::<(ErasedQueryClient, SleepHandle)>();
        let show_b = use_signal(|| false);
        let show_c = use_signal(|| false);
        rsx! {
            QueryClientProvider {
                client: client,
                sleep: Some(sleep),
                Col {
                    Primary { show_b: show_b, show_c: show_c }
                    if show_b() {
                        LateSibling { tag: "B" }
                    }
                    if show_c() {
                        LateSibling { tag: "C" }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, Root)
        .with_context((client, sleep))
        .with_context(Rc::clone(&ctrl_slot));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    // Initial load succeeds -> 1 call ("V1").
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 1);

    let (mut show_b, mut show_c, q_handle) = ctrl_slot.borrow().unwrap();

    // Case A: slot becomes stale (`needs_fetch == true`), refetch() fails attempt 1 and sleeps in
    // retry backoff -> mounting LateSibling B MUST NOT cancel the sleeping backoff task or trigger
    // an immediate duplicate fetch!
    clock.advance(61_000);
    {
        let mut g = transport.inner.lock().unwrap();
        g.transient_failures_left = 1;
        g.name = "V2".into();
    }
    surface.in_root_scope(|| q_handle.refetch());
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 2);

    surface.in_root_scope(|| show_b.set(true));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(
        transport.inner.lock().unwrap().query_calls,
        2,
        "mounting sibling during active refetch retry-backoff must not cancel backoff or refire"
    );

    // Release backoff gate -> original retry task finishes attempt 2 cleanly ("V2").
    let wakers = {
        let mut g = sleep_gate.lock().unwrap();
        g.released = true;
        core::mem::take(&mut g.wakers)
    };
    for w in wakers {
        w.wake();
    }
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 3);
    assert!(web.render_html(surface.store()).contains("P:V2"));
    assert!(web.render_html(surface.store()).contains("B:V2"));

    // Case B: slot becomes stale (`needs_fetch == true`), refetch() is in-flight -> LateSibling C
    // mounts mid-flight -> refetch() exhausts retries -> C must NOT fire a second refetch cycle!
    clock.advance(61_000);
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.fail_queries = true;
    }
    surface.in_root_scope(|| q_handle.refetch());
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    surface.in_root_scope(|| show_c.set(true));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    transport.release_all();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    // Non-retryable `fail_queries` (Wait: RegTransport returns `TransportError::network` which IS
    // retryable, so with max_retries=2 and sleep_gate already released, that single refetch runs
    // 1 initial + 2 retries = 3 calls total: 3 + 3 = 6; if C had refired, it would be 9!).
    assert_eq!(
        transport.inner.lock().unwrap().query_calls,
        6,
        "sibling mounted during failing refetch must not trigger a second refetch cycle"
    );
}
