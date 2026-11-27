//! Contract, concurrency/interleaving property, optimistic rebase/rollback, hydration roundtrip,
//! bounded GC, retry backoff, and `CountingAlloc` zero-allocation tests for `nve-query`.

use core::future::Future;
use core::task::{Context, Poll, Waker};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use nve_query::{
    ManualClock, QueryClient, QueryClientConfig, QueryHandle, QueryMode, QueryOptions, Shared,
};
use nve_rpc::{
    procedures, BoxFuture, CallReply, CborCodec, DomainStatus, Procedure, RawCall, RawReply,
    ReplyKind, RpcCodec, RpcError, RpcStatus, Tag64, Transport, TransportError,
};

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().saturating_add(1)));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let out = f();
    let after = THREAD_ALLOCS.with(Cell::get);
    (out, after.saturating_sub(before))
}

fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("sans-I/O query test future unexpectedly yielded Poll::Pending"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct Item {
    #[n(0)]
    id: u64,
    #[n(1)]
    title: String,
    #[n(2)]
    closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct RenameInput {
    #[n(0)]
    id: u64,
    #[n(1)]
    title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
enum DemoErr {
    #[n(0)]
    Rejected,
}

impl DomainStatus for DemoErr {}

procedures! {
    query ListItems = "demo.v1.Items/List" (u64) -> Vec<Item>, DemoErr,
        tags |_page, sink| {
            sink.push(Tag64::of("items.list"));
        };
    query GetItem = "demo.v1.Items/Get" (u64) -> Item, DemoErr,
        tags |id, sink| {
            sink.push(Tag64::keyed("item", *id));
        };
    action RenameItem = "demo.v1.Items/Rename" (RenameInput) -> Item, DemoErr,
        invalidates |input, sink| {
            sink.push(Tag64::of("items.list"));
            sink.push(Tag64::keyed("item", input.id));
        };
}

#[derive(Clone, Default)]
struct ScriptedTransport {
    calls: Arc<AtomicUsize>,
    fail_first_n: Arc<AtomicUsize>,
    action_fail: Arc<AtomicUsize>,
}

impl Transport for ScriptedTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        let fail_n = self.fail_first_n.load(Ordering::Relaxed);
        let action_fail = self.action_fail.load(Ordering::Relaxed);
        Box::pin(async move {
            if req.name == ListItems::NAME {
                if n < fail_n {
                    return Err(TransportError::network("transient disconnect"));
                }
                let items = vec![
                    Item {
                        id: 1,
                        title: format!("Alpha v{n}"),
                        closed: false,
                    },
                    Item {
                        id: 2,
                        title: "Beta".into(),
                        closed: false,
                    },
                ];
                let mut body = Vec::new();
                CborCodec::default().encode_into(&items, &mut body).unwrap();
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            if req.name == RenameItem::NAME {
                if action_fail == 1 {
                    let mut body = Vec::new();
                    CborCodec::default()
                        .encode_into(&DemoErr::Rejected, &mut body)
                        .unwrap();
                    return Ok(RawReply {
                        http_status: 422,
                        kind: ReplyKind::Domain,
                        invalidated_tags: Vec::new(),
                        invalidate_all: false,
                        body: Bytes::from(body),
                    });
                }
                if action_fail == 2 {
                    return Err(TransportError::timeout("gateway timeout"));
                }
                let input: RenameInput = CborCodec::default().decode_bounded(&req.body).unwrap();
                let updated = Item {
                    id: input.id,
                    title: input.title,
                    closed: false,
                };
                let mut body = Vec::new();
                CborCodec::default().encode_into(&updated, &mut body).unwrap();
                return Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: vec![
                        Tag64::of("items.list"),
                        Tag64::keyed("item", input.id),
                    ],
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            Err(TransportError::config("unexpected procedure"))
        })
    }
}

#[test]
fn deduplication_stale_response_race_guard_and_tag_wildcard_invalidation_work() {
    let clock = Arc::new(ManualClock::new(1_000));
    let client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig::default(),
    );
    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_cb = Arc::clone(&wakes);
    client.set_wake_callback(Arc::new(move || {
        wakes_cb.fetch_add(1, Ordering::Relaxed);
    }));

    // Two subscribers share one slot and one in-flight fetch (dedup).
    let opts = QueryOptions::default().stale_time_ms(5_000);
    let h1 = client.subscribe::<ListItems>(&0, opts).unwrap();
    let h2 = client.subscribe::<ListItems>(&0, opts).unwrap();
    assert_eq!(h1, h2);
    assert_eq!(client.active_slot_count(), 1);

    let (t1, _) = client.begin_fetch(h1).expect("first fetch starts");
    assert!(
        client.begin_fetch(h2).is_none(),
        "second subscriber must deduplicate into in-flight fetch"
    );

    // Invalidation bumps fetch_gen BEFORE slow pre-mutation reply arrives!
    let invalidated = client.invalidate_tags(&[Tag64::of("items.list")]);
    assert_eq!(invalidated, vec![h1]);
    let (t2, _) = client
        .begin_fetch(h1)
        .expect("invalidated slot starts fresh fetch_gen");
    assert_ne!(t1.fetch_gen, t2.fetch_gen);

    // Late arrival of stale ticket `t1` MUST be discarded!
    let stale_accepted = client.complete_fetch::<ListItems>(
        t1,
        Ok(CallReply {
            value: vec![Item {
                id: 1,
                title: "STALE".into(),
                closed: false,
            }],
            invalidated_tags: Vec::new(),
            invalidate_all: false,
            raw_body: Bytes::from_static(b"stale"),
        }),
    );
    assert!(!stale_accepted, "stale response MUST be discarded");
    assert!(client.read::<ListItems>(h1).data.is_none());

    // Current ticket `t2` commits cleanly.
    let fresh_val = vec![Item {
        id: 1,
        title: "FRESH".into(),
        closed: false,
    }];
    let mut fresh_bytes = Vec::new();
    CborCodec::default()
        .encode_into(&fresh_val, &mut fresh_bytes)
        .unwrap();
    assert!(client.complete_fetch::<ListItems>(
        t2,
        Ok(CallReply {
            value: fresh_val.clone(),
            invalidated_tags: Vec::new(),
            invalidate_all: false,
            raw_body: Bytes::from(fresh_bytes),
        }),
    ));
    let snap = client.read::<ListItems>(h1);
    assert_eq!(snap.data.as_deref(), Some(&fresh_val));
    assert!(!snap.is_stale);

    // O(1) wildcard invalidation marks entry stale immediately even within stale_time_ms.
    let wild = client.invalidate_all();
    assert_eq!(wild, vec![h1]);
    assert!(client.read::<ListItems>(h1).is_stale);
    assert!(wakes.load(Ordering::Relaxed) > 0);

    client.unsubscribe(h1);
    assert_eq!(client.active_slot_count(), 1);
    client.unsubscribe(h2);
    assert_eq!(client.active_slot_count(), 0);
}

#[test]
fn overlapping_optimistic_layers_rebase_without_clobbering_or_flicker() {
    let clock = Arc::new(ManualClock::new(1_000));
    let client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        clock,
        QueryClientConfig::default(),
    );

    let h = client
        .subscribe::<ListItems>(&0, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    client
        .set_query_data::<ListItems>(
            &0,
            vec![
                Item {
                    id: 1,
                    title: "A".into(),
                    closed: false,
                },
                Item {
                    id: 2,
                    title: "B".into(),
                    closed: false,
                },
            ],
        )
        .unwrap();

    // Mutation 1 renames #1 A -> A1; Mutation 2 concurrently closes #2.
    let m1 = client.apply_optimistic(|tx| {
        tx.update_each::<ListItems>(|list| {
            let mut next = list.clone();
            next[0].title = "A1".into();
            next
        });
    });
    let m2 = client.apply_optimistic(|tx| {
        tx.update_queries_by_tag::<ListItems>(Tag64::of("items.list"), |list| {
            let mut next = list.clone();
            next[1].closed = true;
            next
        });
    });

    let both = client.read::<ListItems>(h).data.unwrap();
    assert_eq!(both[0].title, "A1");
    assert!(both[1].closed);

    // M1 fails & rolls back: M2 MUST stay intact (unlike naive TanStack snapshot restore!).
    client.rollback_optimistic(&m1);
    let after_m1_fail = client.read::<ListItems>(h).data.unwrap();
    assert_eq!(after_m1_fail[0].title, "A");
    assert!(
        after_m1_fail[1].closed,
        "rolling back M1 must NEVER erase sibling mutation M2"
    );

    // M2 succeeds -> settle arms layer until refetch commits (zero flicker while refetch runs!).
    let to_refetch = client.settle_optimistic(&m2);
    assert_eq!(to_refetch, vec![h]);
    let during_refetch = client.read::<ListItems>(h).data.unwrap();
    assert!(
        during_refetch[1].closed,
        "optimistic view must not flicker off before post-action refetch lands"
    );

    // Refetch commits server truth -> M2 layer retires cleanly.
    let (ticket, _) = client.begin_fetch(h).unwrap();
    let server_truth = vec![
        Item {
            id: 1,
            title: "A".into(),
            closed: false,
        },
        Item {
            id: 2,
            title: "B (server)".into(),
            closed: true,
        },
    ];
    let mut raw = Vec::new();
    CborCodec::default()
        .encode_into(&server_truth, &mut raw)
        .unwrap();
    assert!(client.complete_fetch::<ListItems>(
        ticket,
        Ok(CallReply {
            value: server_truth.clone(),
            invalidated_tags: Vec::new(),
            invalidate_all: false,
            raw_body: Bytes::from(raw),
        }),
    ));
    let final_data = client.read::<ListItems>(h).data.unwrap();
    assert_eq!(*final_data, server_truth);
}

#[test]
fn retry_backoff_action_transport_invalidation_hydration_and_gc_work() {
    let clock = Arc::new(ManualClock::new(10_000));
    let transport = ScriptedTransport::default();
    transport.fail_first_n.store(2, Ordering::Relaxed);
    let client = QueryClient::with_clock(
        transport.clone(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig {
            max_entries: 4,
            ..QueryClientConfig::default()
        },
    );

    // 1. Query retries transient Network failures with exponential backoff and succeeds on attempt 3.
    let h = client
        .subscribe::<ListItems>(
            &0,
            QueryOptions::default()
                .stale_time_ms(60_000)
                .max_retries(3)
                .retry_base_delay_ms(100),
        )
        .unwrap();
    let mut delays = Vec::new();
    let val = block_on(client.fetch_handle_with_retry::<ListItems, _, _>(h, |ms| {
        delays.push(ms);
        async {}
    }))
    .unwrap()
    .unwrap();
    assert_eq!(delays, vec![100, 200]);
    assert_eq!(val[0].title, "Alpha v2");

    // 2. Action domain error rolls back without invalidating; transport error rolls back AND
    //    invalidates static tags (ambiguous commit state).
    transport.action_fail.store(1, Ordering::Relaxed);
    let dom_err = block_on(client.execute_action::<RenameItem>(&RenameInput {
        id: 1,
        title: "Bad".into(),
    }))
    .unwrap_err();
    assert!(matches!(
        dom_err,
        RpcError::Domain {
            status: RpcStatus::Unprocessable,
            error: DemoErr::Rejected
        }
    ));
    assert!(!client.read::<ListItems>(h).is_stale);

    transport.action_fail.store(2, Ordering::Relaxed);
    let net_err = block_on(client.execute_action::<RenameItem>(&RenameInput {
        id: 1,
        title: "Timeout".into(),
    }))
    .unwrap_err();
    assert!(matches!(net_err, RpcError::Transport(_)));
    assert!(
        client.read::<ListItems>(h).is_stale,
        "ambiguous transport failure on Action must invalidate static tags"
    );

    // 3. Dehydration -> Hydration roundtrip (lazy decode + pre-read tag index + SSR read-only mode).
    let blob = client.dehydrate_bytes();
    let ssr_client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        Arc::new(ManualClock::new(10_500)),
        QueryClientConfig {
            mode: QueryMode::SsrReadOnly,
            ..QueryClientConfig::default()
        },
    );
    assert_eq!(ssr_client.hydrate_bytes(&blob).unwrap(), 1);
    let hh = ssr_client
        .subscribe::<ListItems>(&0, QueryOptions::default())
        .unwrap();
    assert!(!ssr_client.needs_fetch(hh), "SsrReadOnly must never trigger fetch");
    let hydrated = ssr_client.read::<ListItems>(hh).data.unwrap();
    assert_eq!(hydrated[0].title, "Alpha v2");

    // 4. Bounded GC never evicts active subscribers, and evicts expired inactive slots cleanly
    //    with zero tag-edge leaks.
    client.unsubscribe(h);
    clock.advance(400_000);
    assert_eq!(client.gc_sweep(64), 1);
    assert_eq!(client.live_slot_count(), 0);
    assert_eq!(client.tag_edge_count(), 0);
}

#[test]
fn random_interleaving_property_invariants_and_zero_alloc_hot_paths() {
    let clock = Arc::new(ManualClock::new(1_000));
    let client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig {
            max_entries: 16,
            ..QueryClientConfig::default()
        },
    );

    // 2,000-step deterministic PRNG interleaving subscribe, fetch, invalidate, wildcard,
    // optimistic apply/rollback, unsubscribe, and GC sweep.
    let mut rng: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let mut next_u64 = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        rng
    };

    let mut active_handles: Vec<QueryHandle> = Vec::new();
    let mut pending_tickets = Vec::new();
    let mut pending_opts = Vec::new();

    for step in 0..2_000u64 {
        let op = next_u64() % 8;
        match op {
            0 => {
                let id = next_u64() % 12;
                let h = client
                    .subscribe::<GetItem>(&id, QueryOptions::default().gc_time_ms(500))
                    .unwrap();
                active_handles.push(h);
            }
            1 if !active_handles.is_empty() => {
                let idx = (next_u64() as usize) % active_handles.len();
                let h = active_handles.swap_remove(idx);
                client.unsubscribe(h);
            }
            2 if !active_handles.is_empty() => {
                let h = active_handles[(next_u64() as usize) % active_handles.len()];
                if let Some((ticket, _)) = client.begin_fetch(h) {
                    pending_tickets.push((ticket, step));
                }
            }
            3 if !pending_tickets.is_empty() => {
                let idx = (next_u64() as usize) % pending_tickets.len();
                let (ticket, ver) = pending_tickets.swap_remove(idx);
                let item = Item {
                    id: ver,
                    title: format!("v{ver}"),
                    closed: false,
                };
                let mut raw = Vec::new();
                CborCodec::default().encode_into(&item, &mut raw).unwrap();
                let _ = client.complete_fetch::<GetItem>(
                    ticket,
                    Ok(CallReply {
                        value: item,
                        invalidated_tags: Vec::new(),
                        invalidate_all: false,
                        raw_body: Bytes::from(raw),
                    }),
                );
            }
            4 => {
                let id = next_u64() % 12;
                let _ = client.invalidate_tags(&[Tag64::keyed("item", id)]);
            }
            5 => {
                let _ = client.invalidate_all();
            }
            6 => {
                let opt = client.apply_optimistic(|tx| {
                    tx.update_each::<GetItem>(|it| Item {
                        id: it.id,
                        title: format!("{}*", it.title),
                        closed: !it.closed,
                    });
                });
                pending_opts.push(opt);
                if pending_opts.len() > 4 {
                    let old = pending_opts.remove(0);
                    client.rollback_optimistic(&old);
                }
            }
            _ => {
                clock.advance(250);
                let _ = client.gc_sweep(8);
            }
        }
        // Invariant: no subscribed handle is ever evicted!
        for &h in &active_handles {
            let _ = client.read::<GetItem>(h);
        }
    }

    for opt in pending_opts {
        client.rollback_optimistic(&opt);
    }
    for h in active_handles {
        client.unsubscribe(h);
    }
    clock.advance(10_000);
    client.gc_sweep(1024);
    assert_eq!(client.live_slot_count(), 0);
    assert_eq!(client.active_slot_count(), 0);
    assert_eq!(client.tag_edge_count(), 0);

    // CountingAlloc zero-allocation steady-state hot-path guards:
    let h = client
        .subscribe::<GetItem>(&7, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    client
        .set_query_data::<GetItem>(
            &7,
            Item {
                id: 7,
                title: "Zero alloc".into(),
                closed: false,
            },
        )
        .unwrap();

    let (snap, read_allocs) = count_allocs(|| client.read::<GetItem>(h));
    assert_eq!(snap.data.as_ref().unwrap().id, 7);
    assert_eq!(read_allocs, 0, "cache-hit read::<P> must allocate 0 times");

    let tags = [Tag64::keyed("item", 7)];
    let mut active_scratch = Vec::with_capacity(8);
    let (_, inv_allocs) = count_allocs(|| client.invalidate_tags_into(&tags, &mut active_scratch));
    assert_eq!(active_scratch, vec![h]);
    assert_eq!(
        inv_allocs, 0,
        "invalidate_tags_into must allocate 0 times on hot path"
    );

    let mut dirty_scratch = Vec::with_capacity(8);
    let (_, drain_allocs) = count_allocs(|| client.drain_dirty_slots(&mut dirty_scratch));
    assert!(!dirty_scratch.is_empty());
    assert_eq!(
        drain_allocs, 0,
        "drain_dirty_slots must allocate 0 times with reused buffer"
    );

    let _ = Shared::new(0u8);
}
