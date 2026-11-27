//! Shared-fetch sibling unmount (P1 #5), row-unmount optimistic survival & pre-poll provider
//! unmount rollback (P1 #6), zero-alloc `flush_dirty_slots`, and clean wake-callback teardown.

use core::task::Waker;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_query_dioxus::DioxusQueryBridge;
use nve_rpc::Tag64;
use nve_ui::prelude::*;

use crate::{
    count_allocs, find_by_test_id, new_web_backend, pump_settled, render_once, GetItem, Item,
    NoopFlagWaker, RegTransport, RenameItem, RenameItemInput,
};

#[test]
fn sibling_unmount_mid_fetch_never_strands_survivor_and_optimistic_survives_row_unmount() {
    let transport = RegTransport::new("Initial");
    transport.inner.lock().unwrap().defer_queries = true;
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    type FlagsSlot = Rc<RefCell<Option<(Signal<bool>, Signal<bool>, Signal<bool>)>>>;
    let flags_slot: FlagsSlot = Rc::new(RefCell::new(None));

    #[component]
    fn SiblingA() -> Element {
        let q = use_query_with::<GetItem>(1, QueryOptions::default().stale_time_ms(60_000));
        let txt = q.data().map(|d| d.name.clone()).unwrap_or_else(|| "LOAD_A".into());
        rsx! { text { "A:{txt}" } }
    }

    #[component]
    fn SiblingB() -> Element {
        let q = use_query_with::<GetItem>(1, QueryOptions::default().stale_time_ms(60_000));
        let txt = q.data().map(|d| d.name.clone()).unwrap_or_else(|| "LOAD_B".into());
        rsx! { text { "B:{txt}" } }
    }

    #[component]
    fn RowAction(mut show_row: Signal<bool>) -> Element {
        let act = use_action::<RenameItem>();
        rsx! {
            Button {
                test_id: "row-opt-btn".to_owned(),
                on_press: move |_| {
                    act.run_optimistic(
                        RenameItemInput {
                            id: 1,
                            name: "RenamedOnServer".into(),
                        },
                        |tx| {
                            let _ = tx.update_query::<GetItem>(&1, |old| Item {
                                id: old.id,
                                name: "OptimisticName".into(),
                            });
                        },
                    );
                    show_row.set(false);
                },
                "RenameAndUnmount"
            }
        }
    }

    #[component]
    fn SharedRoot() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let slot = dioxus_core::consume_context::<FlagsSlot>();
        let show_a = use_signal(|| true);
        let show_row = use_signal(|| true);
        let show_provider = use_signal(|| true);
        use_hook(move || {
            *slot.borrow_mut() = Some((show_a, show_row, show_provider));
        });
        rsx! {
            ThemeProvider {
                theme: UiTheme::light(),
                if show_provider() {
                    QueryClientProvider {
                        client: client,
                        Col {
                            if show_a() {
                                SiblingA {}
                            }
                            SiblingB {}
                            if show_row() {
                                RowAction { show_row: show_row }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, SharedRoot)
        .with_context(client.clone())
        .with_context(Rc::clone(&flags_slot));
    let mut web = new_web_backend();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html(surface.store()).contains("A:LOAD_A"));
    assert!(web.render_html(surface.store()).contains("B:LOAD_B"));

    // Unmount SiblingA (starter of shared fetch) while request is pending.
    let (mut show_a, _, mut show_provider) = flags_slot.borrow().unwrap();
    surface.in_root_scope(|| show_a.set(false));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    // Resolve server query -> SiblingB MUST receive "Initial" (provider-scoped fetch survives!).
    transport.release_all();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(
        web.render_html(surface.store()).contains("B:Initial"),
        "surviving sibling must not be stranded when starter unmounts: {}",
        web.render_html(surface.store())
    );

    // Defer action, click RowAction (which applies optimistic patch AND unmounts itself!).
    transport.inner.lock().unwrap().defer_actions = true;
    let btn = find_by_test_id(surface.store(), "row-opt-btn");
    assert!(surface.dispatch_events(&[nve_abi::NativeEvent {
        surface_id: SurfaceId::ROOT,
        node_id: btn,
        timestamp_us: 0,
        kind: nve_abi::NativeEventKind::Press,
        payload: nve_abi::EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(
        web.render_html(surface.store()).contains("B:OptimisticName"),
        "optimistic patch must remain visible after triggering row unmounts: {}",
        web.render_html(surface.store())
    );

    // Release server action -> completes and settles server value even though RowAction unmounted!
    transport.release_all();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().action_calls, 1);
    assert!(
        web.render_html(surface.store()).contains("B:RenamedOnServer"),
        "action must complete after triggering row unmounts: {}",
        web.render_html(surface.store())
    );

    // Unmount provider cleanly -> active slots drop to 0 and detached wake callback never panics.
    surface.in_root_scope(|| show_provider.set(false));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    assert_eq!(client.active_slot_count(), 0);
    let _ = client.invalidate_all();
}

#[test]
fn provider_unmount_before_first_poll_rolls_back_optimistic_and_flush_is_zero_alloc() {
    let transport = RegTransport::new("Base");
    let client = erased_query_client_with_clock(
        transport,
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    client
        .set_query_data::<GetItem>(
            &1,
            Item {
                id: 1,
                name: "Base".into(),
            },
        )
        .unwrap();

    type BridgeSlot = Rc<RefCell<Option<(DioxusQueryBridge, Signal<bool>, Signal<u32>)>>>;
    let slot: BridgeSlot = Rc::new(RefCell::new(None));

    #[component]
    fn Inner(mut show_provider: Signal<bool>) -> Element {
        let q = use_query_with::<GetItem>(1, QueryOptions::default().stale_time_ms(60_000));
        let act = use_action::<RenameItem>();
        let raw_sig = use_signal(|| 0u32);
        let bridge = dioxus_core::consume_context::<DioxusQueryBridge>();
        let slot = dioxus_core::consume_context::<BridgeSlot>();
        use_hook(move || {
            *slot.borrow_mut() = Some((bridge, show_provider, raw_sig));
        });
        let name = q.data().map(|d| d.name.clone()).unwrap_or_default();
        let raw_v = *raw_sig.read();
        rsx! {
            Col {
                text { "VAL:{name}:{raw_v}" }
                Button {
                    test_id: "unmount-provider-btn".to_owned(),
                    on_press: move |_| {
                        act.run_optimistic(
                            RenameItemInput {
                                id: 1,
                                name: "Ghost".into(),
                            },
                            |tx| {
                                let _ = tx.update_query::<GetItem>(&1, |old| Item {
                                    id: old.id,
                                    name: "GhostOpt".into(),
                                });
                            },
                        );
                        show_provider.set(false);
                    },
                    "KillProvider"
                }
            }
        }
    }

    #[component]
    fn WrapRoot() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let show_provider = use_signal(|| true);
        rsx! {
            ThemeProvider {
                theme: UiTheme::light(),
                if show_provider() {
                    QueryClientProvider {
                        client: client,
                        Inner { show_provider: show_provider }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, WrapRoot)
        .with_context(client.clone())
        .with_context(Rc::clone(&slot));
    let mut web = new_web_backend();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html(surface.store()).contains("VAL:Base"));

    let (bridge, _, mut raw_sig) = slot.borrow().clone().unwrap();
    assert_eq!(bridge.live_signal_count(), 1);

    // Warm both `flush_dirty_slots` and the same-subscriber `raw_sig` once, then verify:
    // 1. Quiet/no-op `flush_dirty_slots` performs 0 total heap allocations.
    // 2. Dirty `flush_dirty_slots` adds EXACTLY 0 bridge allocations on top of raw `Signal::set`.
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 1)]);
    surface.in_root_scope(|| bridge.flush_dirty_slots());
    surface.in_root_scope(|| raw_sig.set(1));
    let (_, quiet_allocs) = count_allocs(|| {
        surface.in_root_scope(|| bridge.flush_dirty_slots());
    });
    assert_eq!(quiet_allocs, 0, "quiet flush_dirty_slots must allocate 0 times");

    let (_, raw_sig_allocs) = count_allocs(|| {
        surface.in_root_scope(|| raw_sig.set(2));
    });
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 1)]);
    let (_, flush_allocs) = count_allocs(|| {
        surface.in_root_scope(|| bridge.flush_dirty_slots());
    });
    assert_eq!(
        flush_allocs, raw_sig_allocs,
        "flush_dirty_slots must add exactly 0 bridge allocations over raw Signal::set"
    );

    // Click KillProvider: applies optimistic patch, spawns task, and IMMEDIATELY unmounts provider
    // before the spawned future's first poll -> pre-spawn OptimisticDropGuard MUST roll back!
    let btn = find_by_test_id(surface.store(), "unmount-provider-btn");
    assert!(surface.dispatch_events(&[nve_abi::NativeEvent {
        surface_id: SurfaceId::ROOT,
        node_id: btn,
        timestamp_us: 0,
        kind: nve_abi::NativeEventKind::Press,
        payload: nve_abi::EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    assert_eq!(bridge.live_signal_count(), 0);

    // Re-subscribe and verify the optimistic patch was rolled back cleanly to "Base"!
    let h = client.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    assert_eq!(client.read::<GetItem>(h).data.unwrap().name, "Base");
}

#[test]
fn use_action_run_and_action_form_survive_same_frame_unmount_without_panic() {
    let transport = RegTransport::new("Start");
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    #[component]
    fn RunRow(mut show_row: Signal<bool>) -> Element {
        let act = use_action::<RenameItem>();
        rsx! {
            Button {
                test_id: "run-and-unmount-btn".to_owned(),
                on_press: move |_| {
                    act.run(RenameItemInput {
                        id: 1,
                        name: "FromRun".into(),
                    });
                    show_row.set(false);
                },
                "Run"
            }
        }
    }

    #[component]
    fn FormRow(mut show_form: Signal<bool>) -> Element {
        let mut ctrl = use_signal(FormController::new);
        use_hook(move || {
            ctrl.write().set_text("name", "FromActionForm");
        });
        rsx! {
            ActionForm::<RenameItem> {
                action: "/items/rename".to_owned(),
                test_id: "unmount-action-form".to_owned(),
                controller: Some(ctrl),
                FormField { name: "name".to_owned(), test_id: "uf-name".to_owned() }
                FormSubmitButton { test_id: "uf-submit".to_owned(), "Submit" }
            }
            Button {
                test_id: "hide-form-btn".to_owned(),
                on_press: move |_| show_form.set(false),
                "Hide"
            }
        }
    }

    #[component]
    fn Root() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let show_row = use_signal(|| true);
        let show_form = use_signal(|| true);
        rsx! {
            ThemeProvider {
                theme: UiTheme::light(),
                QueryClientProvider {
                    client: client,
                    Col {
                        if show_row() {
                            RunRow { show_row: show_row }
                        }
                        if show_form() {
                            FormRow { show_form: show_form }
                        }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, Root).with_context(client);
    let mut web = new_web_backend();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    let _ = render_once(&mut surface, &mut web, &mut frame);

    // 1. Trigger UseAction::run() and immediately unmount RunRow before first task poll.
    let run_btn = find_by_test_id(surface.store(), "run-and-unmount-btn");
    assert!(surface.dispatch_events(&[nve_abi::NativeEvent {
        surface_id: SurfaceId::ROOT,
        node_id: run_btn,
        timestamp_us: 0,
        kind: nve_abi::NativeEventKind::Press,
        payload: nve_abi::EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().action_calls, 1);
    assert_eq!(transport.inner.lock().unwrap().name, "FromRun");

    // 2. Submit ActionForm and unmount FormRow in the SAME event batch before first task poll.
    let form_id = find_by_test_id(surface.store(), "unmount-action-form");
    let hide_btn = find_by_test_id(surface.store(), "hide-form-btn");
    assert!(surface.dispatch_events(&[
        nve_abi::NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: form_id,
            timestamp_us: 0,
            kind: nve_abi::NativeEventKind::Submit,
            payload: nve_abi::EventPayload::Text(String::new()),
        },
        nve_abi::NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: hide_btn,
            timestamp_us: 0,
            kind: nve_abi::NativeEventKind::Press,
            payload: nve_abi::EventPayload::Point { x: 1.0, y: 1.0 },
        },
    ]));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().action_calls, 2);
    assert_eq!(transport.inner.lock().unwrap().name, "FromActionForm");
}

#[test]
fn spawn_slot_fetch_supersede_cancels_sleeping_retry_and_siblings_dedupe_failed_query() {
    use std::sync::Mutex;

    // Part A: Two mounted siblings sharing a failing query fire ONLY 1 request (not 2!).
    let fail_transport = RegTransport::new("Err");
    fail_transport.inner.lock().unwrap().fail_queries = true;
    let fail_client = erased_query_client_with_clock(
        fail_transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    #[component]
    fn FailSibling() -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 0;
        let q = use_query_with::<GetItem>(1, opts);
        let has_err = q.error().is_some();
        rsx! { text { "ERR:{has_err}" } }
    }

    #[component]
    fn FailRoot() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                Col {
                    FailSibling {}
                    FailSibling {}
                }
            }
        }
    }

    let mut s1 = DioxusSurface::new(SurfaceId::ROOT, FailRoot).with_context(fail_client);
    let mut w1 = WebBackend::new();
    let mut f1 = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let _ = render_once(&mut s1, &mut w1, &mut f1);
    let _ = pump_settled(&mut s1, &mut w1, &waker, &mut f1);
    assert_eq!(
        fail_transport.inner.lock().unwrap().query_calls,
        1,
        "two siblings sharing a failing query must not each fire a duplicate fetch"
    );

    // Part B: Superseding a sleeping retry task cancels the old task (no ghost fetch after unmount).
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
                    core::task::Poll::Ready(())
                } else {
                    g.wakers.push(cx.waker().clone());
                    core::task::Poll::Pending
                }
            })
            .await;
        })
    });

    let retry_transport = RegTransport::new("Retry");
    retry_transport.inner.lock().unwrap().transient_failures_left = 1;
    let retry_client = erased_query_client_with_clock(
        retry_transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    type CtrlSlot = Rc<RefCell<Option<(Signal<bool>, UseQuery<GetItem>)>>>;
    let ctrl_slot: CtrlSlot = Rc::new(RefCell::new(None));

    #[component]
    fn RetryView(show: Signal<bool>) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 2;
        let q = use_query_with::<GetItem>(2, opts);
        let slot = dioxus_core::consume_context::<CtrlSlot>();
        use_hook(move || {
            *slot.borrow_mut() = Some((show, q));
        });
        let name = q.data().map(|d| d.name.clone()).unwrap_or_default();
        rsx! { text { "R:{name}" } }
    }

    #[component]
    fn RetryRoot() -> Element {
        let (client, sleep) = dioxus_core::consume_context::<(ErasedQueryClient, SleepHandle)>();
        let show = use_signal(|| true);
        rsx! {
            QueryClientProvider {
                client: client,
                sleep: Some(sleep),
                if show() {
                    RetryView { show: show }
                }
            }
        }
    }

    let mut s2 = DioxusSurface::new(SurfaceId::ROOT, RetryRoot)
        .with_context((retry_client, sleep))
        .with_context(Rc::clone(&ctrl_slot));
    let mut w2 = WebBackend::new();
    let mut f2 = 0;
    let _ = render_once(&mut s2, &mut w2, &mut f2);
    let _ = pump_settled(&mut s2, &mut w2, &waker, &mut f2);
    // Task 1 failed attempt 1 and is now parked in sleep_gate.
    assert_eq!(retry_transport.inner.lock().unwrap().query_calls, 1);

    // Supersede Task 1 with an explicit refetch (Task 2 succeeds), then unmount RetryView!
    let (mut show_sig, q_handle) = ctrl_slot.borrow().unwrap();
    s2.in_root_scope(|| q_handle.refetch());
    let _ = pump_settled(&mut s2, &mut w2, &waker, &mut f2);
    assert_eq!(retry_transport.inner.lock().unwrap().query_calls, 2);
    s2.in_root_scope(|| show_sig.set(false));
    let _ = render_once(&mut s2, &mut w2, &mut f2);

    // Releasing the old sleep gate MUST NOT trigger any ghost retry from cancelled Task 1!
    let wakers = {
        let mut g = sleep_gate.lock().unwrap();
        g.released = true;
        core::mem::take(&mut g.wakers)
    };
    for w in wakers {
        w.wake();
    }
    let _ = pump_settled(&mut s2, &mut w2, &waker, &mut f2);
    assert_eq!(
        retry_transport.inner.lock().unwrap().query_calls,
        2,
        "superseded retry task must be cancelled and never fire ghost fetches after unmount"
    );
}
