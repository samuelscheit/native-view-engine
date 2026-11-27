//! `StreamPending` lifecycle guards: `invalidate_all()` and action `Nve-Invalidate: *` settlement
//! release inactive stream-pending slots,
//! late `seq > 0` chunks never land after a wildcard invalidation, `fetch_handle` followers of a
//! stream-pending slot settled by a hydrated error fire zero RPCs, and stale data wins over a
//! coexisting refetch error on the wire.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use nve_query::{DehydrateCursor, QueryClient, QueryOptions};
use nve_rpc::{
    BoxFuture, CborCodec, Procedure, RawCall, RawReply, ReplyKind, RpcCodec, RpcError, RpcStatus,
    Transport, TransportError,
};

use crate::ssr_stream_tests::{ErrorMatrixTransport, WakeCount};
use crate::{block_on, DemoErr, GetItem, Item, RenameInput, RenameItem};

/// [`ErrorMatrixTransport`] that counts calls and can be switched to fail with a network error.
#[derive(Clone, Default)]
struct CountingMatrix {
    calls: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
}

impl Transport for CountingMatrix {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if req.name == RenameItem::NAME {
            // Action replies carry `Nve-Invalidate: *`.
            let input: RenameInput = CborCodec::default().decode_bounded(&req.body).expect("test precondition holds");
            let mut body = Vec::new();
            let item = Item {
                id: input.id,
                title: input.title,
                closed: false,
            };
            CborCodec::default().encode_into(&item, &mut body).expect("test precondition holds");
            return Box::pin(async move {
                Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: true,
                    body: Bytes::from(body),
                })
            });
        }
        if self.fail.load(Ordering::SeqCst) {
            return Box::pin(async { Err(TransportError::network("refetch failed")) });
        }
        ErrorMatrixTransport.call_raw(req)
    }
}

fn waker() -> (Arc<WakeCount>, Waker) {
    let count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&count));
    (count, waker)
}

fn poll_once<F: Future>(fut: &mut Pin<Box<F>>, waker: &Waker) -> Poll<F::Output> {
    fut.as_mut().poll(&mut Context::from_waker(waker))
}

/// Server-side `seq=0` blob marking keys `ids` stream-pending.
fn pending_blob(ids: &[u64]) -> Vec<u8> {
    let ssr = QueryClient::ssr(ErrorMatrixTransport);
    let handles: Vec<_> = ids
        .iter()
        .map(|id| ssr.subscribe::<GetItem>(id, QueryOptions::default()).expect("test precondition holds"))
        .collect();
    ssr.dehydrate_initial_stream_chunk(&handles, &mut DehydrateCursor::new())
        .expect("test precondition holds")
}

/// Server-side `seq > 0` delta chunk carrying the settled result of key `id`.
fn settled_chunk(id: u64) -> Vec<u8> {
    let ssr = QueryClient::ssr(ErrorMatrixTransport);
    let h = ssr.subscribe::<GetItem>(&id, QueryOptions::default()).expect("test precondition holds");
    let _ = block_on(ssr.fetch_handle::<GetItem>(h));
    ssr.dehydrate_handles_delta(&[h], &mut DehydrateCursor::new())
        .expect("test precondition holds")
}

#[test]
fn invalidate_all_releases_inactive_stream_pending_slots_and_wakes_waiters() {
    let cli = QueryClient::new(CountingMatrix::default());
    assert_eq!(cli.hydrate_bytes(&pending_blob(&[1, 2])).expect("test precondition holds"), 2);

    let c1 = cli.subscribe::<GetItem>(&1, QueryOptions::default()).expect("test precondition holds");
    assert!(cli.read::<GetItem>(c1).is_stream_pending);
    let (wakes, w) = waker();
    let mut wait = Box::pin(cli.wait_for_settled(c1));
    assert!(poll_once(&mut wait, &w).is_pending());

    // Unmount: the slot stays live but inactive, so `invalidate_slot` never visits it.
    cli.unsubscribe(c1);
    assert_eq!(cli.active_slot_count(), 0);
    let active = cli.invalidate_all();
    assert!(active.is_empty());

    assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "waiter woken by invalidate_all");
    assert!(poll_once(&mut wait, &w).is_ready());
    let snap = cli.read::<GetItem>(c1);
    assert!(!snap.is_stream_pending && !snap.is_fetching);
    assert_eq!(cli.finish_ssr_stream(), 0, "nothing left stream-pending");

    let remount = cli.subscribe::<GetItem>(&2, QueryOptions::default()).expect("test precondition holds");
    assert!(cli.needs_fetch(remount));
}

#[test]
fn action_wildcard_invalidation_releases_inactive_stream_pending_slots_and_refuses_late_chunks() {
    let transport = CountingMatrix::default();
    let cli = QueryClient::new(transport.clone());
    assert_eq!(cli.hydrate_bytes(&pending_blob(&[1])).expect("test precondition holds"), 1);

    let c1 = cli.subscribe::<GetItem>(&1, QueryOptions::default()).expect("test precondition holds");
    let (wakes, w) = waker();
    let mut wait = Box::pin(cli.wait_for_settled(c1));
    assert!(poll_once(&mut wait, &w).is_pending());
    cli.unsubscribe(c1);
    assert_eq!(cli.active_slot_count(), 0, "slot is live but inactive");

    let input = RenameInput {
        id: 9,
        title: "renamed".into(),
    };
    let (reply, refetch) = block_on(cli.execute_action::<RenameItem>(&input)).expect("test precondition holds");
    assert!(reply.invalidate_all, "server replied `Nve-Invalidate: *`");
    assert!(refetch.is_empty(), "no active slot to refetch");

    assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "waiter woken by the action settlement");
    assert!(poll_once(&mut wait, &w).is_ready());
    let snap = cli.read::<GetItem>(c1);
    assert!(!snap.is_stream_pending && !snap.is_fetching);
    assert_eq!(cli.finish_ssr_stream(), 0, "nothing left stream-pending");

    // The late stream chunk for the released slot is refused after the wildcard bump.
    assert_eq!(cli.hydrate_stream_chunk_bytes(&settled_chunk(1)).expect("test precondition holds"), 0);
    let snap = cli.read::<GetItem>(c1);
    assert!(snap.data.is_none() && snap.completed_seq == 0);
    assert!(cli.needs_fetch(c1));
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1, "only the action RPC fired");
}

#[test]
fn late_stream_chunk_after_invalidate_all_never_overwrites_a_fresh_slot() {
    let transport = CountingMatrix::default();
    let cli = QueryClient::new(transport.clone());
    let _ = cli.invalidate_all();

    // Fresh slot created after the wildcard bump: not invalidated, not fetched, no data.
    let c1 = cli.subscribe::<GetItem>(&1, QueryOptions::default()).expect("test precondition holds");
    let chunk = settled_chunk(1);
    assert_eq!(cli.hydrate_stream_chunk_bytes(&chunk).expect("test precondition holds"), 0);
    let snap = cli.read::<GetItem>(c1);
    assert!(snap.data.is_none() && snap.completed_seq == 0);
    assert!(cli.needs_fetch(c1));

    // Absent slots are refused as well.
    assert_eq!(cli.hydrate_stream_chunk_bytes(&settled_chunk(2)).expect("test precondition holds"), 0);
    assert_eq!(cli.live_slot_count(), 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn fetch_follower_of_stream_pending_slot_settled_by_hydrated_error_fires_zero_rpcs() {
    for (id, is_domain) in [(2u64, true), (3u64, false)] {
        let transport = CountingMatrix::default();
        let cli = QueryClient::new(transport.clone());
        assert_eq!(cli.hydrate_bytes(&pending_blob(&[id])).expect("test precondition holds"), 1);
        let h = cli.subscribe::<GetItem>(&id, QueryOptions::default()).expect("test precondition holds");

        let (_wakes, w) = waker();
        let mut fetch = Box::pin(cli.fetch_handle::<GetItem>(h));
        assert!(poll_once(&mut fetch, &w).is_pending(), "follows the stream");

        assert_eq!(cli.hydrate_stream_chunk_bytes(&settled_chunk(id)).expect("test precondition holds"), 1);
        let Poll::Ready(res) = poll_once(&mut fetch, &w) else {
            panic!("fetch must settle on the hydrated error (key {id})");
        };
        let err = res.expect_err("hydrated error surfaces");
        if is_domain {
            assert!(matches!(
                err.as_ref(),
                RpcError::Domain {
                    status: RpcStatus::PermissionDenied,
                    error: DemoErr::Rejected
                }
            ));
        } else {
            assert!(matches!(
                err.as_ref(),
                RpcError::Protocol {
                    status: RpcStatus::Internal,
                    ..
                }
            ));
        }
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            0,
            "zero duplicate RPCs behind a hydrated error (key {id})"
        );
    }
}

#[test]
fn stale_data_wins_over_coexisting_refetch_error_on_the_wire() {
    let transport = CountingMatrix::default();
    let ssr = QueryClient::ssr(transport.clone());
    let h = ssr.subscribe::<GetItem>(&1, QueryOptions::default()).expect("test precondition holds");
    assert!(block_on(ssr.fetch_handle::<GetItem>(h)).is_ok());
    ssr.invalidate_handles(&[h]);
    transport.fail.store(true, Ordering::SeqCst);
    assert!(block_on(ssr.fetch_handle::<GetItem>(h)).is_err());
    let snap = ssr.read::<GetItem>(h);
    assert!(snap.data.is_some() && snap.error.is_some(), "data + error coexist");

    let cli = QueryClient::new(ErrorMatrixTransport);
    assert_eq!(cli.hydrate_bytes(&ssr.dehydrate_bytes().expect("test precondition holds")).expect("test precondition holds"), 1);
    let c = cli.subscribe::<GetItem>(&1, QueryOptions::default()).expect("test precondition holds");
    let snap = cli.read::<GetItem>(c);
    assert_eq!(
        snap.data.as_deref(),
        Some(&Item {
            id: 1,
            title: "Ok Item".into(),
            closed: false,
        })
    );
    assert!(snap.error.is_none());

    // The same priority applies to per-boundary deltas.
    let delta = ssr
        .dehydrate_handles_delta(&[h], &mut DehydrateCursor::new())
        .expect("test precondition holds");
    let cli2 = QueryClient::new(ErrorMatrixTransport);
    assert_eq!(cli2.hydrate_bytes(&delta).expect("test precondition holds"), 1);
    let c2 = cli2.subscribe::<GetItem>(&1, QueryOptions::default()).expect("test precondition holds");
    assert!(cli2.read::<GetItem>(c2).data.is_some());
}
