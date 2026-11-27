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
        t.state.lock().unwrap().defer = defer;
        t
    }

    fn release(&self) {
        let wakers = {
            let mut g = self.state.lock().unwrap();
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
                let mut g = state.lock().unwrap();
                if g.defer {
                    g.wakers.push(cx.waker().clone());
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            let n = {
                let mut g = state.lock().unwrap();
                g.calls += 1;
                g.calls
            };
            if req.name == GetItem::NAME {
                let id: u64 = CborCodec::default().decode_bounded(&req.body).unwrap();
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
                    .unwrap();
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            if req.name == RenameItem::NAME {
                let inp: RenameInput = CborCodec::default().decode_bounded(&req.body).unwrap();
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
                    .unwrap();
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
fn pre_boot_hydration_age_above_grace_marks_slot_invalidated_and_stale() {
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
        snap.is_invalidated && snap.is_stale,
        "pre-boot hydrated entry exceeding grace window must be marked invalidated & stale"
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
