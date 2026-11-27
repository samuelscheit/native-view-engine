//! Targeted regression tests promoted from 4-lens dual review of `nve-query`.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use nve_query::{ManualClock, QueryClient, QueryClientConfig, QueryOptions, Shared};
use nve_rpc::{
    BoxFuture, CallReply, CborCodec, Procedure, RawCall, RawReply, ReplyKind, RpcCodec, Tag64,
    Transport, TransportError,
};

use crate::{GetItem, Item, ListItems, RenameInput, RenameItem, ScriptedTransport};

#[derive(Clone, Default)]
struct DeferredTransport {
    state: Arc<Mutex<DeferredState>>,
}

#[derive(Default)]
struct DeferredState {
    defer: bool,
    calls: usize,
    wakers: Vec<Waker>,
}

impl DeferredTransport {
    fn new(defer: bool) -> Self {
        let t = Self::default();
        t.state.lock().unwrap_or_else(|e| e.into_inner()).defer = defer;
        t
    }

    fn release(&self) {
        let wakers = {
            let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
            g.defer = false;
            core::mem::take(&mut g.wakers)
        };
        for w in wakers {
            w.wake();
        }
    }
}

impl Transport for DeferredTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            core::future::poll_fn(|cx| {
                let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
                if g.defer {
                    g.wakers.push(cx.waker().clone());
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            let n = {
                let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
                g.calls += 1;
                g.calls
            };
            if req.name == GetItem::NAME {
                let id: u64 = CborCodec::default().decode_bounded(&req.body).expect("decode id");
                let mut body = Vec::new();
                CborCodec::default()
                    .encode_into(
                        &Item {
                            id,
                            title: format!("Deferred #{id} v{n}"),
                            closed: false,
                        },
                        &mut body,
                    )
                    .expect("encode deferred item");
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            if req.name == RenameItem::NAME {
                let inp: RenameInput = CborCodec::default().decode_bounded(&req.body).expect("decode rename");
                let mut body = Vec::new();
                CborCodec::default()
                    .encode_into(
                        &Item {
                            id: inp.id,
                            title: inp.title,
                            closed: false,
                        },
                        &mut body,
                    )
                    .expect("encode renamed item");
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: vec![Tag64::keyed("item", inp.id)],
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            Err(TransportError::config("unknown proc"))
        })
    }
}

struct WakeCounter(AtomicUsize);
impl std::task::Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn pre_boot_hydration_age_above_grace_marks_slot_stale_without_clamping() {
    let src_clock = Shared::new(ManualClock::new(1_000));
    let src = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        src_clock.clone(),
        QueryClientConfig::default(),
    );
    src.set_query_data::<GetItem>(
        &1,
        Item {
            id: 1,
            title: "From SSR".into(),
            closed: false,
        },
    )
    .unwrap();
    // Advance server clock 50_000 ms so dehydrated wire entry carries `age_ms = 50_000`.
    src_clock.advance(50_000);
    let wire = src.dehydrate_bytes().unwrap();

    // Target browser clock is 5_000 ms (< age_ms 50_000 ms) with 30_000 ms grace -> already expired!
    let dst_clock = Shared::new(ManualClock::new(5_000));
    let dst = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        dst_clock,
        QueryClientConfig {
            hydration_grace_ms: 30_000,
            ..QueryClientConfig::default()
        },
    );
    let rep = dst.hydrate_bytes(&wire).unwrap();
    assert_eq!(rep, 1);

    let h = dst.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    let snap = dst.read::<GetItem>(h);
    assert!(snap.data.is_some());
    assert!(
        snap.is_stale && !snap.is_invalidated,
        "pre-boot hydrated entry exceeding grace window must be stale via exact signed updated_at_ms without permanently invalidating"
    );
    assert!(dst.needs_fetch(h));

    // Contrast: `age_ms = 10_000` (> now_ms 5_000, but < 30_000 grace) stays fresh within grace!
    let fresh_src_clock = Shared::new(ManualClock::new(1_000));
    let fresh_src = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        fresh_src_clock.clone(),
        QueryClientConfig::default(),
    );
    fresh_src
        .set_query_data::<GetItem>(
            &2,
            Item {
                id: 2,
                title: "Recent SSR".into(),
                closed: false,
            },
        )
        .unwrap();
    fresh_src_clock.advance(10_000);
    let fresh_wire = fresh_src.dehydrate_bytes().unwrap();
    dst.hydrate_bytes(&fresh_wire).unwrap();
    let h2 = dst.subscribe::<GetItem>(&2, QueryOptions::default()).unwrap();
    let snap2 = dst.read::<GetItem>(h2);
    assert!(!snap2.is_invalidated && !snap2.is_stale);
    assert!(!dst.needs_fetch(h2));
}

#[test]
fn stale_wildcard_reject_marks_dirty_and_wakes_reattached_subscriber() {
    let clock = Shared::new(ManualClock::new(1_000));
    let client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        clock,
        QueryClientConfig::default(),
    );

    let h1 = client
        .subscribe::<GetItem>(&1, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    let (ticket, _call) = client.begin_fetch_for::<GetItem>(h1).unwrap();
    client.unsubscribe(h1);
    let _ = client.invalidate_all();

    let h2 = client
        .subscribe::<GetItem>(&1, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    let mut dirty = Vec::new();
    client.drain_dirty_slots(&mut dirty);

    let woke = Shared::new(AtomicBool::new(false));
    let woke_cb = Shared::clone(&woke);
    let token = client.set_wake_callback(Shared::new(move || {
        woke_cb.store(true, Ordering::SeqCst);
    }));

    let mut raw = Vec::new();
    let item = Item {
        id: 1,
        title: "Stale".into(),
        closed: false,
    };
    CborCodec::default().encode_into(&item, &mut raw).unwrap();
    let accepted = client.complete_fetch::<GetItem>(
        ticket,
        Ok(CallReply {
            value: item,
            invalidated_tags: Vec::new(),
            invalidate_all: false,
            raw_body: Bytes::from(raw),
        }),
    );
    assert!(!accepted);
    assert!(!client.read::<GetItem>(h2).is_fetching);
    assert!(
        woke.load(Ordering::SeqCst),
        "epoch-rejected completion clearing IS_FETCHING must wake re-attached observers"
    );
    client.drain_dirty_slots(&mut dirty);
    assert!(dirty.iter().any(|&(slot, _)| slot == h2.slot));

    client.clear_wake_callback(token);
    woke.store(false, Ordering::SeqCst);
    let _ = client.invalidate_all();
    assert!(
        !woke.load(Ordering::SeqCst),
        "clear_wake_callback must detach the old callback"
    );
}

#[test]
fn begin_fetch_for_rejects_mismatched_procedure_without_mutating_slot() {
    let client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    let h = client.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    assert!(
        client.begin_fetch_for::<ListItems>(h).is_none(),
        "begin_fetch_for::<WrongProc> must reject mismatched proc_id"
    );
    assert!(!client.read::<GetItem>(h).is_fetching);
    assert!(client.begin_fetch_for::<GetItem>(h).is_some());
}

#[test]
fn cancelled_leader_wakes_deduped_follower_for_clean_takeover() {
    let transport = DeferredTransport::new(true);
    let client = QueryClient::with_clock(
        transport.clone(),
        CborCodec::default(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    let h = client.subscribe::<GetItem>(&42, QueryOptions::default()).unwrap();

    let leader_waker = Waker::from(Arc::new(WakeCounter(AtomicUsize::new(0))));
    let follower_counter = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let follower_waker = Waker::from(Arc::clone(&follower_counter));

    let mut leader_fut = Box::pin(client.fetch_handle::<GetItem>(h));
    let mut follower_fut = Box::pin(client.fetch_handle::<GetItem>(h));

    // Poll leader -> starts network call and yields Pending.
    assert!(matches!(
        Pin::as_mut(&mut leader_fut).poll(&mut Context::from_waker(&leader_waker)),
        Poll::Pending
    ));
    // Poll follower twice with same waker -> deduplicated wait, yields Pending.
    assert!(matches!(
        Pin::as_mut(&mut follower_fut).poll(&mut Context::from_waker(&follower_waker)),
        Poll::Pending
    ));
    assert!(matches!(
        Pin::as_mut(&mut follower_fut).poll(&mut Context::from_waker(&follower_waker)),
        Poll::Pending
    ));

    // Drop leader mid-flight -> FetchCancelGuard clears IS_FETCHING and wakes follower!
    drop(leader_fut);
    assert_eq!(
        follower_counter.0.load(Ordering::SeqCst),
        1,
        "deduped follower waker must fire once when leader cancels"
    );

    // Release transport and poll follower -> follower takes over the fetch and succeeds!
    transport.release();
    let res = match Pin::as_mut(&mut follower_fut).poll(&mut Context::from_waker(&follower_waker)) {
        Poll::Ready(res) => res.expect("takeover fetch must succeed"),
        Poll::Pending => panic!("follower should complete after transport released"),
    };
    assert_eq!(res.expect("item must resolve").id, 42);
    assert_eq!(transport.state.lock().unwrap().calls, 1);
}

#[test]
fn optimistic_action_dropped_mid_flight_rolls_back_and_invalidates_static_tags() {
    let transport = DeferredTransport::new(true);
    let client = QueryClient::with_clock(
        transport,
        CborCodec::default(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    let h = client
        .subscribe::<GetItem>(&7, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    client
        .set_query_data::<GetItem>(
            &7,
            Item {
                id: 7,
                title: "Committed".into(),
                closed: false,
            },
        )
        .unwrap();
    assert!(!client.read::<GetItem>(h).is_invalidated);

    {
        let input = RenameInput {
            id: 7,
            title: "Dropped".into(),
        };
        let mut fut = Box::pin(client.execute_action_optimistic::<RenameItem, _>(&input, |tx| {
            let _ = tx.update_query::<GetItem>(&7, |old| Item {
                id: old.id,
                title: "Optimistic".into(),
                closed: old.closed,
            });
        }));
        let waker = Waker::noop();
        assert!(matches!(
            Pin::as_mut(&mut fut).poll(&mut Context::from_waker(waker)),
            Poll::Pending
        ));
        assert_eq!(client.read::<GetItem>(h).data.unwrap().title, "Optimistic");
        // Drop mid-network-call!
    }

    let snap = client.read::<GetItem>(h);
    assert_eq!(
        snap.data.unwrap().title,
        "Committed",
        "dropping optimistic action mid-flight must roll back optimistic layer"
    );
    assert!(
        snap.is_invalidated,
        "dropping action mid-network-call has ambiguous server state and must invalidate static tags"
    );
}

#[test]
fn hydrated_entry_preserves_exact_pre_boot_age_and_subsequent_stale_time() {
    let src_clock = Shared::new(ManualClock::new(100_000));
    let src = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        src_clock.clone(),
        QueryClientConfig::default(),
    );
    src.set_query_data::<GetItem>(
        &1,
        Item {
            id: 1,
            title: "HydratedExact".into(),
            closed: false,
        },
    )
    .unwrap();
    // 20 s elapsed before dehydration.
    src_clock.advance(20_000);
    let blob = src.dehydrate_bytes().unwrap();

    // Case 1: age (20_000) > now_ms (5_000), grace = 30_000 -> remaining freshness MUST be 10_000 ms
    // (stale at now_ms = 15_000, NOT clamped to updated_at = 0 / stale at 30_000).
    let dst_clock = Shared::new(ManualClock::new(5_000));
    let dst = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        dst_clock.clone(),
        QueryClientConfig {
            hydration_grace_ms: 30_000,
            ..QueryClientConfig::default()
        },
    );
    assert_eq!(dst.hydrate_bytes(&blob).unwrap(), 1);
    let h = dst.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    assert!(!dst.read::<GetItem>(h).is_stale);
    dst_clock.advance(9_999);
    assert!(!dst.read::<GetItem>(h).is_stale);
    dst_clock.advance(1);
    assert!(
        dst.read::<GetItem>(h).is_stale,
        "entry with age=20s hydrated at now=5s with 30s grace must go stale at now=15s"
    );

    // Case 2: age (20_000) > now_ms (50) and > grace (5_000), hydrated BEFORE component subscribes
    // with stale_time_ms(60_000) -> MUST NOT be permanently invalidated; stays fresh for 40_000 ms!
    let dst2_clock = Shared::new(ManualClock::new(50));
    let dst2 = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        dst2_clock.clone(),
        QueryClientConfig {
            hydration_grace_ms: 5_000,
            ..QueryClientConfig::default()
        },
    );
    assert_eq!(dst2.hydrate_bytes(&blob).unwrap(), 1);
    let h2 = dst2
        .subscribe::<GetItem>(&1, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    let snap = dst2.read::<GetItem>(h2);
    assert!(!snap.is_invalidated);
    assert!(
        !snap.is_stale,
        "20s-old hydrated entry must respect caller's 60s stale_time_ms even when now_ms=50"
    );
    dst2_clock.advance(39_999);
    assert!(!dst2.read::<GetItem>(h2).is_stale);
    dst2_clock.advance(1);
    assert!(dst2.read::<GetItem>(h2).is_stale);
}

#[test]
fn cancelled_leader_takeover_works_on_stale_data_and_prior_error() {
    let clock = Shared::new(ManualClock::new(1_000));
    let transport = DeferredTransport::new(false);
    let client = QueryClient::with_clock(
        transport.clone(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig::default(),
    );

    // Case 1: Stale data revalidation (has data, !is_invalidated, is_stale == true).
    let h_stale = client
        .subscribe::<GetItem>(&10, QueryOptions::default().stale_time_ms(5_000))
        .unwrap();
    client
        .set_query_data::<GetItem>(
            &10,
            Item {
                id: 10,
                title: "StaleV1".into(),
                closed: false,
            },
        )
        .unwrap();
    clock.advance(10_000);
    assert!(client.read::<GetItem>(h_stale).is_stale);
    assert!(!client.read::<GetItem>(h_stale).is_invalidated);

    transport.state.lock().unwrap().defer = true;
    let waker = Waker::from(Arc::new(WakeCounter(AtomicUsize::new(0))));
    let mut leader = Box::pin(client.fetch_handle::<GetItem>(h_stale));
    let mut follower = Box::pin(client.fetch_handle::<GetItem>(h_stale));
    assert!(matches!(
        Pin::as_mut(&mut leader).poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    assert!(matches!(
        Pin::as_mut(&mut follower).poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    drop(leader);
    transport.release();
    let res = match Pin::as_mut(&mut follower).poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(r) => r.expect("follower must take over stale-data revalidation"),
        Poll::Pending => panic!("follower should resolve after takeover"),
    };
    assert_eq!(
        res.expect("fresh data expected").title,
        "Deferred #10 v1",
        "cancelled leader must not cause follower to return stale V1 data"
    );

    // Case 2: Prior error on slot (committed_error.is_some()), leader drops mid-retry -> follower
    // MUST take over and resolve Ok instead of returning the old error!
    let h_err = client
        .subscribe::<GetItem>(&11, QueryOptions::default())
        .unwrap();
    let (bad_ticket, _) = client.begin_fetch_for::<GetItem>(h_err).unwrap();
    assert!(client.complete_fetch::<GetItem>(
        bad_ticket,
        Err(nve_rpc::RpcError::Transport(TransportError::network("old failure"))),
    ));
    assert!(client.read::<GetItem>(h_err).error.is_some());

    transport.state.lock().unwrap().defer = true;
    let mut err_leader = Box::pin(client.fetch_handle::<GetItem>(h_err));
    let mut err_follower = Box::pin(client.fetch_handle::<GetItem>(h_err));
    assert!(matches!(
        Pin::as_mut(&mut err_leader).poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    assert!(matches!(
        Pin::as_mut(&mut err_follower).poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    drop(err_leader);
    transport.release();
    let res2 = match Pin::as_mut(&mut err_follower).poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(r) => r.expect("follower must take over retry after prior error"),
        Poll::Pending => panic!("follower should resolve after takeover"),
    };
    assert_eq!(res2.expect("fresh item expected").title, "Deferred #11 v2");
}

#[test]
fn concurrent_multithreaded_leader_cancel_never_causes_follower_to_return_none() {
    use std::sync::Barrier;

    struct NotifyThread(std::thread::Thread);
    impl std::task::Wake for NotifyThread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let transport = DeferredTransport::new(false);
    let client = QueryClient::new(transport.clone());

    for round in 0..24u64 {
        let id = 100 + round;
        let h = client
            .subscribe::<GetItem>(&id, QueryOptions::default())
            .unwrap();
        transport.state.lock().unwrap().defer = true;

        let barrier = Arc::new(Barrier::new(2));
        let b_follower = Arc::clone(&barrier);
        let c_follower = client.clone();

        // Start leader on main thread so slot is in-flight, then race cancel vs follower thread!
        let mut leader = Box::pin(client.fetch_handle::<GetItem>(h));
        let main_waker = Waker::from(Arc::new(WakeCounter(AtomicUsize::new(0))));
        assert!(matches!(
            Pin::as_mut(&mut leader).poll(&mut Context::from_waker(&main_waker)),
            Poll::Pending
        ));

        let follower_thread = std::thread::spawn(move || {
            let waker = Waker::from(Arc::new(NotifyThread(std::thread::current())));
            let mut cx = Context::from_waker(&waker);
            let mut fut = Box::pin(c_follower.fetch_handle::<GetItem>(h));
            b_follower.wait();
            loop {
                match Pin::as_mut(&mut fut).poll(&mut cx) {
                    Poll::Ready(res) => return res,
                    Poll::Pending => std::thread::park_timeout(std::time::Duration::from_millis(5)),
                }
            }
        });

        barrier.wait();
        drop(leader);
        transport.release();

        let res = follower_thread
            .join()
            .unwrap()
            .expect("follower must succeed after leader cancel race");
        assert!(
            res.is_some(),
            "round {round}: atomic begin_fetch_or_wait_snapshot must never let cancelled-leader race return Ok(None)"
        );
    }
}
