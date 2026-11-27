//! Discriminating regression guards for external-fetch (`prefetch`/`fetch_handle`) interaction
//! across cold, time-stale, `stale_time_ms == 0`, and invalidated slots, plus mid-flight sibling
//! invalidation watermark latching.

use core::cell::Cell;
use core::task::{Poll, Waker};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_rpc::Tag64;
use nve_ui::prelude::*;

use crate::{pump_settled, render_once, GetItem, NoopFlagWaker, RegTransport};

#[test]
fn fresh_mount_during_external_fetch_recovers_on_cancel_across_cold_stale_and_invalidated_slots() {
    let clock = Shared::new(ManualClock::new(1_000));
    let transport = RegTransport::new("Seed");
    let client = erased_query_client_with_clock(
        transport.clone(),
        clock.clone(),
        QueryClientConfig::default(),
    );
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    // Helper that mounts a fresh `use_query_with(id, opts)` component while an external fetch is
    // in flight, then cancels or completes that external fetch.
    #[component]
    fn Probe(id: u64, stale_ms: u32) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(stale_ms);
        opts.max_retries = 0;
        let q = use_query_with::<GetItem>(id, opts);
        let label = q
            .data()
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "EMPTY".into());
        rsx! { text { "VAL:{id}:{label}" } }
    }

    #[component]
    fn MountRoot() -> Element {
        let (client, id, stale_ms) =
            dioxus_core::consume_context::<(ErasedQueryClient, u64, u32)>();
        rsx! {
            QueryClientProvider {
                client: client,
                Probe { id: id, stale_ms: stale_ms }
            }
        }
    }

    // 1. Cold slot (no data, not invalidated): external `prefetch` cancelled -> fresh hook recovers!
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.name = "ColdRecovered".into();
    }
    let mut ext_cold = Box::pin(client.prefetch::<GetItem>(&20));
    let mut cx = core::task::Context::from_waker(&waker);
    assert!(matches!(core::pin::Pin::as_mut(&mut ext_cold).poll(&mut cx), Poll::Pending));

    let mut s = DioxusSurface::new(1, MountRoot).with_context((client.clone(), 20u64, 60_000u32));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let _ = render_once(&mut s, &mut web, &mut frame);
    let _ = pump_settled(&mut s, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("VAL:20:EMPTY"));

    drop(ext_cold);
    transport.release_all();
    let _ = pump_settled(&mut s, &mut web, &waker, &mut frame);
    assert!(
        web.render_html().contains("VAL:20:ColdRecovered"),
        "cold-slot fresh mount must recover after cancelled external prefetch: {}",
        web.render_html()
    );

    // 2. Time-stale slot (has data, is_stale, !is_invalidated): external fetch cancelled -> recovers!
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = false;
        g.name = "StaleV1".into();
    }
    let h_stale = client
        .subscribe::<GetItem>(&21, QueryOptions::default().stale_time_ms(5_000))
        .unwrap();
    {
        let mut init_fut = Box::pin(client.fetch_handle::<GetItem>(h_stale));
        assert!(matches!(
            core::pin::Pin::as_mut(&mut init_fut).poll(&mut cx),
            Poll::Ready(Ok(Some(_)))
        ));
    }
    clock.advance(10_000);
    assert!(client.read::<GetItem>(h_stale).is_stale);
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.name = "StaleV2".into();
    }
    let mut ext_stale = Box::pin(client.fetch_handle::<GetItem>(h_stale));
    assert!(matches!(core::pin::Pin::as_mut(&mut ext_stale).poll(&mut cx), Poll::Pending));

    let mut s2 = DioxusSurface::new(1, MountRoot).with_context((client.clone(), 21u64, 5_000u32));
    let mut web2 = WebBackend::new();
    let mut frame2 = 0;
    let _ = render_once(&mut s2, &mut web2, &mut frame2);
    let _ = pump_settled(&mut s2, &mut web2, &waker, &mut frame2);
    assert!(web2.render_html().contains("VAL:21:StaleV1"));

    drop(ext_stale);
    transport.release_all();
    let _ = pump_settled(&mut s2, &mut web2, &waker, &mut frame2);
    assert!(
        web2.render_html().contains("VAL:21:StaleV2"),
        "stale-slot fresh mount must recover after cancelled external fetch: {}",
        web2.render_html()
    );
    client.unsubscribe(h_stale);

    // 3. Invalidated slot (has data, is_invalidated): external fetch cancelled -> fresh hook recovers!
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = false;
        g.name = "InvV1".into();
    }
    let h_inv = client
        .subscribe::<GetItem>(&22, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    {
        let mut init_fut = Box::pin(client.fetch_handle::<GetItem>(h_inv));
        assert!(matches!(
            core::pin::Pin::as_mut(&mut init_fut).poll(&mut cx),
            Poll::Ready(Ok(Some(_)))
        ));
    }
    client.invalidate_handles(&[h_inv]);
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.name = "InvV2".into();
    }
    let mut ext_inv = Box::pin(client.fetch_handle::<GetItem>(h_inv));
    assert!(matches!(core::pin::Pin::as_mut(&mut ext_inv).poll(&mut cx), Poll::Pending));

    let mut s3 = DioxusSurface::new(1, MountRoot).with_context((client.clone(), 22u64, 60_000u32));
    let mut web3 = WebBackend::new();
    let mut frame3 = 0;
    let _ = render_once(&mut s3, &mut web3, &mut frame3);
    let _ = pump_settled(&mut s3, &mut web3, &waker, &mut frame3);
    assert!(web3.render_html().contains("VAL:22:InvV1"));

    drop(ext_inv);
    transport.release_all();
    let _ = pump_settled(&mut s3, &mut web3, &waker, &mut frame3);
    assert!(
        web3.render_html().contains("VAL:22:InvV2"),
        "invalidated-slot fresh mount must recover after cancelled external fetch: {}",
        web3.render_html()
    );
    client.unsubscribe(h_inv);

    // 4. Cold slot with `stale_time_ms == 0` where external `prefetch` SUCCEEDS -> ZERO duplicate fetch!
    let calls_before = transport.inner.lock().unwrap().query_calls;
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.name = "ZeroStaleOk".into();
    }
    let mut ext_ok = Box::pin(client.prefetch::<GetItem>(&23));
    assert!(matches!(core::pin::Pin::as_mut(&mut ext_ok).poll(&mut cx), Poll::Pending));

    let mut s4 = DioxusSurface::new(1, MountRoot).with_context((client.clone(), 23u64, 0u32));
    let mut web4 = WebBackend::new();
    let mut frame4 = 0;
    let _ = render_once(&mut s4, &mut web4, &mut frame4);
    let _ = pump_settled(&mut s4, &mut web4, &waker, &mut frame4);

    transport.release_all();
    assert!(matches!(
        core::pin::Pin::as_mut(&mut ext_ok).poll(&mut cx),
        Poll::Ready(Ok(_))
    ));
    let _ = pump_settled(&mut s4, &mut web4, &waker, &mut frame4);
    assert!(web4.render_html().contains("VAL:23:ZeroStaleOk"));
    assert_eq!(
        transport.inner.lock().unwrap().query_calls - calls_before,
        1,
        "completed external prefetch must not trigger a duplicate fetch even when stale_time_ms == 0"
    );
}

#[test]
fn invalidated_hook_refetches_after_external_fetch_is_cancelled() {
    let transport = RegTransport::new("V1");
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    #[component]
    fn View() -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 0;
        let q = use_query_with::<GetItem>(11, opts);
        let label = q.data().map(|d| d.name.clone()).unwrap_or_default();
        rsx! { text { "A:{label}" } }
    }

    #[component]
    fn Root() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                View {}
            }
        }
    }

    let mut surface = DioxusSurface::new(1, Root).with_context(client.clone());
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    // 1. Initial load succeeds ("V1").
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("A:V1"));

    // 2. Invalidate the slot and immediately start an EXTERNAL (non-bridge-owned) fetch before
    //    the hook re-renders. Hook re-renders while `is_fetching == true` and `active_for_ver == false`.
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.name = "V2".into();
    }
    let invalidated = client.invalidate_tags(&[Tag64::keyed("item", 11)]);
    assert_eq!(invalidated.len(), 1);
    let h = invalidated[0];

    let mut ext_fut = Box::pin(client.fetch_handle::<GetItem>(h));
    let mut cx = core::task::Context::from_waker(&waker);
    assert!(matches!(core::pin::Pin::as_mut(&mut ext_fut).poll(&mut cx), Poll::Pending));
    assert!(client.read::<GetItem>(h).is_fetching);

    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    // 3. Cancel external fetch -> already-mounted hook MUST refetch and render "V2".
    drop(ext_fut);
    transport.release_all();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(
        web.render_html().contains("A:V2"),
        "mounted hook must refetch after an external fetch on an invalidated slot is cancelled: {}",
        web.render_html()
    );
}

#[test]
fn invalidated_sibling_observing_in_flight_bridge_fetch_does_not_double_fire_on_failure() {
    let transport = RegTransport::new("V1");
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    type PokeSlot = Rc<RefCell<Option<Signal<u32>>>>;
    let poke_slot: PokeSlot = Rc::new(RefCell::new(None));
    let sibling_b_saw_fetching = Rc::new(Cell::new(false));

    #[component]
    fn SiblingA() -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 0;
        let poke = use_signal(|| 0u32);
        let slot = dioxus_core::consume_context::<PokeSlot>();
        use_hook(move || {
            *slot.borrow_mut() = Some(poke);
        });
        let p = poke();
        let q = use_query_with::<GetItem>(12, opts);
        let err = q.error().is_some();
        rsx! { text { "A:{p}:{err}" } }
    }

    #[component]
    fn SiblingB() -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 0;
        let saw_fetching = dioxus_core::consume_context::<Rc<Cell<bool>>>();
        let q = use_query_with::<GetItem>(12, opts);
        let snap = q.snapshot();
        if snap.is_invalidated && snap.is_fetching {
            saw_fetching.set(true);
        }
        let err = snap.error.is_some();
        rsx! { text { "B:{err}" } }
    }

    #[component]
    fn Root() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                Col {
                    SiblingA {}
                    SiblingB {}
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(1, Root)
        .with_context(client.clone())
        .with_context(Rc::clone(&poke_slot))
        .with_context(Rc::clone(&sibling_b_saw_fetching));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    // 1. Both siblings mount and settle with "V1" (call 1).
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 1);

    // 2. Invalidate slot while failing+deferred, drain the pre-poke dirty entry so `SiblingA`'s
    //    spawned bridge fetch enters `is_fetching == true` BEFORE `flush_dirty_slots` wakes
    //    `SiblingB` for the new `inv_ver`!
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.fail_queries = true;
    }
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 12)]);
    let mut discard = Vec::new();
    client.drain_dirty_slots(&mut discard);

    let mut poke_a = poke_slot.borrow().unwrap();
    surface.in_root_scope(|| poke_a.set(1));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    assert!(
        sibling_b_saw_fetching.get(),
        "SiblingB must genuinely observe `is_invalidated && is_fetching` before transport release"
    );

    // 3. Release failing transport -> bridge fetch fails -> SiblingB MUST NOT fire a duplicate call!
    transport.release_all();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(
        transport.inner.lock().unwrap().query_calls,
        2,
        "sibling observing in-flight bridge refetch after invalidation must not double-fire on failure"
    );
}

#[test]
fn invalidation_after_external_prefetch_settlement_triggers_refetch() {
    let transport = RegTransport::new("Pre");
    transport.inner.lock().unwrap().defer_queries = true;
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let mut cx = core::task::Context::from_waker(&waker);

    #[component]
    fn View(id: u64) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 0;
        let q = use_query_with::<GetItem>(id, opts);
        let label = q
            .data()
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "EMPTY".into());
        rsx! { text { "R:{id}:{label}" } }
    }

    #[component]
    fn Root() -> Element {
        let (client, id) = dioxus_core::consume_context::<(ErasedQueryClient, u64)>();
        rsx! {
            QueryClientProvider {
                client: client,
                View { id: id }
            }
        }
    }

    // Case 1: External prefetch settles ("Pre"), then `invalidate_tags` lands BEFORE the hook's
    // next render -> hook must NOT swallow the invalidation and MUST refetch "Post".
    let mut ext1 = Box::pin(client.prefetch::<GetItem>(&30));
    assert!(matches!(core::pin::Pin::as_mut(&mut ext1).poll(&mut cx), Poll::Pending));

    let mut s1 = DioxusSurface::new(1, Root).with_context((client.clone(), 30u64));
    let mut web1 = WebBackend::new();
    let mut frame1 = 0;
    let _ = render_once(&mut s1, &mut web1, &mut frame1);
    let _ = pump_settled(&mut s1, &mut web1, &waker, &mut frame1);

    transport.release_all();
    assert!(matches!(
        core::pin::Pin::as_mut(&mut ext1).poll(&mut cx),
        Poll::Ready(Ok(_))
    ));
    transport.inner.lock().unwrap().name = "Post".into();
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 30)]);
    let _ = pump_settled(&mut s1, &mut web1, &waker, &mut frame1);
    assert!(
        web1.render_html().contains("R:30:Post"),
        "invalidation right after external prefetch completion must trigger refetch: {}",
        web1.render_html()
    );

    // Case 2: External prefetch settles ("Pre2"), `invalidate_tags` bumps inv_ver, a SECOND
    // external fetch starts on the invalidated slot and is CANCELLED -> hook MUST still refetch "Post2".
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.name = "Pre2".into();
    }
    let mut ext2 = Box::pin(client.prefetch::<GetItem>(&31));
    assert!(matches!(core::pin::Pin::as_mut(&mut ext2).poll(&mut cx), Poll::Pending));

    let mut s2 = DioxusSurface::new(1, Root).with_context((client.clone(), 31u64));
    let mut web2 = WebBackend::new();
    let mut frame2 = 0;
    let _ = render_once(&mut s2, &mut web2, &mut frame2);
    let _ = pump_settled(&mut s2, &mut web2, &waker, &mut frame2);

    transport.release_all();
    assert!(matches!(
        core::pin::Pin::as_mut(&mut ext2).poll(&mut cx),
        Poll::Ready(Ok(_))
    ));
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = true;
        g.name = "Post2".into();
    }
    let invalidated = client.invalidate_tags(&[Tag64::keyed("item", 31)]);
    let mut ext2_second = Box::pin(client.fetch_handle::<GetItem>(invalidated[0]));
    assert!(matches!(
        core::pin::Pin::as_mut(&mut ext2_second).poll(&mut cx),
        Poll::Pending
    ));
    let _ = pump_settled(&mut s2, &mut web2, &waker, &mut frame2);

    drop(ext2_second);
    transport.release_all();
    let _ = pump_settled(&mut s2, &mut web2, &waker, &mut frame2);
    assert!(
        web2.render_html().contains("R:31:Post2"),
        "hook must still recover when invalidation + cancelled 2nd external fetch follow prefetch: {}",
        web2.render_html()
    );
}

#[test]
fn failed_external_refetch_after_invalidation_does_not_duplicate_network_call() {
    let transport = RegTransport::new("V1");
    transport.inner.lock().unwrap().defer_queries = true;
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let mut cx = core::task::Context::from_waker(&waker);

    #[component]
    fn View(id: u64) -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 0;
        let q = use_query_with::<GetItem>(id, opts);
        let label = q
            .data()
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "EMPTY".into());
        let has_err = q.error().is_some();
        rsx! { text { "F:{id}:{label}:{has_err}" } }
    }

    #[component]
    fn Root() -> Element {
        let (client, id) = dioxus_core::consume_context::<(ErasedQueryClient, u64)>();
        rsx! {
            QueryClientProvider {
                client: client,
                View { id: id }
            }
        }
    }

    // Case 1: Fresh hook mounts waiting on an external prefetch -> `invalidate_tags` discards it ->
    // external `fetch_handle` on the invalidated slot FAILS -> hook renders without a duplicate
    // 3rd request, yet a subsequent `invalidate_tags` DOES trigger refetch ("Recovered1").
    let mut ext_pre = Box::pin(client.prefetch::<GetItem>(&40));
    assert!(matches!(core::pin::Pin::as_mut(&mut ext_pre).poll(&mut cx), Poll::Pending));
    let mut s1 = DioxusSurface::new(1, Root).with_context((client.clone(), 40u64));
    let mut web1 = WebBackend::new();
    let mut frame1 = 0;
    let _ = render_once(&mut s1, &mut web1, &mut frame1);
    let _ = pump_settled(&mut s1, &mut web1, &waker, &mut frame1);

    let inv1 = client.invalidate_tags(&[Tag64::keyed("item", 40)]);
    assert_eq!(inv1.len(), 1);
    transport.release_all();
    // Stale prefetch settles after invalidation and is discarded cleanly (call 1).
    assert!(matches!(
        core::pin::Pin::as_mut(&mut ext_pre).poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    transport.inner.lock().unwrap().fail_queries = true;
    // External fetch on the invalidated slot fails with an error (call 2).
    let mut ext_fail1 = Box::pin(client.fetch_handle::<GetItem>(inv1[0]));
    assert!(matches!(
        core::pin::Pin::as_mut(&mut ext_fail1).poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    assert_eq!(transport.inner.lock().unwrap().query_calls, 2);

    let _ = pump_settled(&mut s1, &mut web1, &waker, &mut frame1);
    assert!(web1.render_html().contains("F:40:EMPTY:true"));
    assert_eq!(
        transport.inner.lock().unwrap().query_calls,
        2,
        "hook must not re-fetch an invalidation already answered by a failed external fetch"
    );

    {
        let mut g = transport.inner.lock().unwrap();
        g.fail_queries = false;
        g.name = "Recovered1".into();
    }
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 40)]);
    let _ = pump_settled(&mut s1, &mut web1, &waker, &mut frame1);
    assert!(web1.render_html().contains("F:40:Recovered1:false"));
    assert_eq!(transport.inner.lock().unwrap().query_calls, 3);

    // Case 2: Already-mounted hook settles ("V1") -> `invalidate_tags` -> external `fetch_handle`
    // FAILS -> hook renders without a duplicate refetch, and a later `invalidate_tags` recovers.
    let calls_base = transport.inner.lock().unwrap().query_calls;
    {
        let mut g = transport.inner.lock().unwrap();
        g.defer_queries = false;
        g.fail_queries = false;
        g.name = "MountedV1".into();
    }
    let mut s2 = DioxusSurface::new(1, Root).with_context((client.clone(), 41u64));
    let mut web2 = WebBackend::new();
    let mut frame2 = 0;
    let _ = render_once(&mut s2, &mut web2, &mut frame2);
    let _ = pump_settled(&mut s2, &mut web2, &waker, &mut frame2);
    assert!(web2.render_html().contains("F:41:MountedV1:false"));
    assert_eq!(transport.inner.lock().unwrap().query_calls - calls_base, 1);

    let inv2 = client.invalidate_tags(&[Tag64::keyed("item", 41)]);
    assert_eq!(inv2.len(), 1);
    transport.inner.lock().unwrap().fail_queries = true;
    let mut ext_fail2 = Box::pin(client.fetch_handle::<GetItem>(inv2[0]));
    assert!(matches!(
        core::pin::Pin::as_mut(&mut ext_fail2).poll(&mut cx),
        Poll::Ready(Err(_))
    ));
    let _ = pump_settled(&mut s2, &mut web2, &waker, &mut frame2);
    assert!(web2.render_html().contains("F:41:MountedV1:true"));
    assert_eq!(
        transport.inner.lock().unwrap().query_calls - calls_base,
        2,
        "already-mounted hook must not re-fetch an invalidation already answered by a failed external fetch"
    );

    {
        let mut g = transport.inner.lock().unwrap();
        g.fail_queries = false;
        g.name = "Recovered2".into();
    }
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 41)]);
    let _ = pump_settled(&mut s2, &mut web2, &waker, &mut frame2);
    assert!(web2.render_html().contains("F:41:Recovered2:false"));
    assert_eq!(transport.inner.lock().unwrap().query_calls - calls_base, 3);
}
