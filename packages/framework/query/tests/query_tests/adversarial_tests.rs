//! Adversarial regression guards, cancellation safety, lock-free re-entrancy, hydration error
//! bounds, concurrent `prefetch` dedup, reference-model property tests, and `CountingAlloc` guards.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use nve_query::{
    DehydrateError, ManualClock, QueryClient, QueryClientConfig, QueryHandle, QueryOptions, Shared,
};
use nve_rpc::{
    BoxFuture, CallReply, CborCodec, RawCall, RawReply, ReplyKind, RpcCodec, Tag64, Transport,
    TransportError,
};

use crate::{
    block_on, GetItem, Item, ListItems, ManyTagQuery, RenameInput, RenameItem, ScriptedTransport,
};

/// Transport whose calls suspend until explicitly released, testing cancellation & dedup waiting.
#[derive(Clone, Default)]
struct ManualGateTransport {
    started: Shared<AtomicUsize>,
    released: Shared<std::sync::atomic::AtomicBool>,
}

impl Transport for ManualGateTransport {
    fn call_raw<'a>(&'a self, _req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        self.started.fetch_add(1, Ordering::Relaxed);
        let released = Shared::clone(&self.released);
        Box::pin(core::future::poll_fn(move |_cx| {
            if released.load(Ordering::Acquire) {
                let item = Item {
                    id: 9,
                    title: "Gated".into(),
                    closed: false,
                };
                let mut body = Vec::new();
                CborCodec::default().encode_into(&item, &mut body).expect("encode gated item");
                Poll::Ready(Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                }))
            } else {
                Poll::Pending
            }
        }))
    }
}

#[test]
fn cancellation_safety_dedup_wait_atomic_settle_and_reentrant_optimistic_work() {
    let clock = Shared::new(ManualClock::new(1_000));
    let gate = ManualGateTransport::default();
    let client = QueryClient::with_clock(
        gate.clone(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig::default(),
    );

    // 1. Dropping `fetch_handle` mid-await clears FLAG_IS_FETCHING (never wedges!).
    let h = client
        .subscribe::<GetItem>(&9, QueryOptions::default())
        .unwrap();
    {
        let mut fut = std::pin::pin!(client.fetch_handle::<GetItem>(h));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        assert!(client.read::<GetItem>(h).is_fetching);
    }
    assert!(
        !client.read::<GetItem>(h).is_fetching,
        "dropped fetch_handle must clear IS_FETCHING"
    );

    // 2. Dropping `prefetch` mid-await unsubscribes cleanly (zero subscriber_count leak!).
    {
        let mut fut = std::pin::pin!(client.prefetch::<GetItem>(&99));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        assert_eq!(client.active_slot_count(), 2);
    }
    assert_eq!(
        client.active_slot_count(),
        1,
        "dropped prefetch must release its subscription"
    );

    // 3. Concurrent `prefetch` dedup waits for the single in-flight call and BOTH succeed.
    let mut p1 = std::pin::pin!(client.prefetch::<GetItem>(&9));
    let mut p2 = std::pin::pin!(client.prefetch::<GetItem>(&9));
    let mut cx = Context::from_waker(Waker::noop());
    let calls_before = gate.started.load(Ordering::Relaxed);
    assert!(p1.as_mut().poll(&mut cx).is_pending());
    assert!(p2.as_mut().poll(&mut cx).is_pending());
    assert_eq!(
        gate.started.load(Ordering::Relaxed) - calls_before,
        1,
        "concurrent prefetches must share ONE network call"
    );
    gate.released.store(true, Ordering::Release);
    let r1 = match p1.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(v)) => v,
        other => panic!("expected Ready(Ok), got {other:?}"),
    };
    let r2 = match p2.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(v)) => v,
        other => panic!("expected Ready(Ok), got {other:?}"),
    };
    assert_eq!(r1.title, "Gated");
    assert_eq!(r2.title, "Gated");

    // 4. Dropping `execute_action_optimistic` mid-await rolls back optimistic layer cleanly!
    gate.released.store(false, Ordering::Release);
    {
        let transient_input = RenameInput {
            id: 9,
            title: "Transient".into(),
        };
        let mut act = std::pin::pin!(client.execute_action_optimistic::<RenameItem, _>(
            &transient_input,
            |tx| {
                tx.update_query::<GetItem>(&9, |it| Item {
                    id: it.id,
                    title: "Optimistic!".into(),
                    closed: it.closed,
                })
                .unwrap();
            },
        ));
        assert!(act.as_mut().poll(&mut cx).is_pending());
        assert_eq!(client.read::<GetItem>(h).data.unwrap().title, "Optimistic!");
    }
    assert_eq!(
        client.read::<GetItem>(h).data.unwrap().title,
        "Gated",
        "dropped execute_action_optimistic must roll back its layer"
    );

    // 5. Re-entrant callback inside optimistic transform MUST NOT deadlock (native) or RefCell panic!
    let client_for_cb = client.clone();
    let reentrant = client.apply_optimistic(|tx| {
        tx.update_each::<GetItem>(move |it| {
            let _ = client_for_cb.read::<GetItem>(h);
            Item {
                id: it.id,
                title: format!("{}-reentrant", it.title),
                closed: it.closed,
            }
        });
    });
    assert_eq!(
        client.read::<GetItem>(h).data.unwrap().title,
        "Gated-reentrant"
    );
    client.rollback_optimistic(&reentrant);

    // 6. Single-lock atomic `execute_action_optimistic` wakes ONCE on completion (no double-bump!).
    let wakes = Shared::new(AtomicUsize::new(0));
    let scripted = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig::default(),
    );
    let sh = scripted
        .subscribe::<GetItem>(&1, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    scripted
        .set_query_data::<GetItem>(
            &1,
            Item {
                id: 1,
                title: "Initial".into(),
                closed: false,
            },
        )
        .unwrap();
    let w_cb = Shared::clone(&wakes);
    scripted.set_wake_callback(Shared::new(move || {
        w_cb.fetch_add(1, Ordering::Relaxed);
    }));
    let (_reply, refetch) = block_on(scripted.execute_action_optimistic::<RenameItem, _>(
        &RenameInput {
            id: 1,
            title: "Renamed".into(),
        },
        |tx| {
            tx.update_query::<GetItem>(&1, |it| Item {
                id: it.id,
                title: "Renamed".into(),
                closed: it.closed,
            })
            .unwrap();
        },
    ))
    .unwrap();
    assert_eq!(refetch, vec![sh]);
    // 1 wake for optimistic apply + 1 single atomic wake for settle+invalidation!
    assert_eq!(wakes.load(Ordering::Relaxed), 2);
}

#[test]
fn wildcard_inactive_race_hydration_expiry_overflow_tags_and_dehydrate_errors_work() {
    let clock = Shared::new(ManualClock::new(1_000));
    let client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig {
            max_entries: 4,
            hydration_grace_ms: 5_000,
            ..QueryClientConfig::default()
        },
    );

    // 1. Wildcard invalidation while an inactive slot has an in-flight ticket MUST reject it!
    let h = client
        .subscribe::<GetItem>(&1, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    let (ticket, _) = client.begin_fetch(h).unwrap();
    client.unsubscribe(h);
    let _ = client.invalidate_all();
    let mut raw = Vec::new();
    let old_item = Item {
        id: 1,
        title: "Old".into(),
        closed: false,
    };
    CborCodec::default().encode_into(&old_item, &mut raw).unwrap();
    assert!(
        !client.complete_fetch::<GetItem>(
            ticket,
            Ok(CallReply {
                value: old_item,
                invalidated_tags: Vec::new(),
                invalidate_all: false,
                raw_body: Bytes::from(raw),
            }),
        ),
        "ticket started before invalidate_all() must be rejected even on inactive slot"
    );

    // 2. Inactive FLAG_SUBSCRIBED_ALL (>128 tags) slot MUST be invalidated by `invalidate_tags`!
    let mh = client
        .subscribe::<ManyTagQuery>(&1, QueryOptions::default().stale_time_ms(60_000))
        .unwrap();
    client
        .set_query_data::<ManyTagQuery>(
            &1,
            Item {
                id: 1,
                title: "Many".into(),
                closed: false,
            },
        )
        .unwrap();
    client.unsubscribe(mh);
    assert!(!client.read::<ManyTagQuery>(mh).is_stale);
    let _ = client.invalidate_tags(&[Tag64::of("unrelated.tag")]);
    assert!(
        client.read::<ManyTagQuery>(mh).is_stale,
        "inactive overflow-tag slot must invalidate on any tag invalidation"
    );

    // 3. Hydrated entry with default stale_time_ms=0 is fresh during grace window, expires after!
    let blob = client.dehydrate_bytes().unwrap();
    let c2_clock = Shared::new(ManualClock::new(10_000));
    let c2 = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        c2_clock.clone(),
        QueryClientConfig {
            max_entries: 4,
            hydration_grace_ms: 5_000,
            ..QueryClientConfig::default()
        },
    );
    assert_eq!(c2.hydrate_bytes(&blob).unwrap(), 1);
    let hh = c2
        .subscribe::<ManyTagQuery>(&1, QueryOptions::default())
        .unwrap();
    assert!(!c2.needs_fetch(hh), "fresh within 5s hydration grace window");
    // Reading with wrong Procedure type MUST NOT corrupt hydrated raw_response!
    assert!(c2.read::<ListItems>(hh).data.is_none());
    assert_eq!(c2.read::<ManyTagQuery>(hh).data.unwrap().title, "Many");
    c2_clock.advance(5_001);
    assert!(
        c2.needs_fetch(hh),
        "hydrated entry MUST expire after hydration_grace_ms"
    );
    assert_eq!(c2.on_window_focus(), vec![hh]);
    assert_eq!(c2.on_network_online(), vec![hh]);

    // 4. `max_entries` is enforced by `set_query_data` as well as `subscribe`.
    for i in 0..20u64 {
        client
            .set_query_data::<GetItem>(
                &i,
                Item {
                    id: i,
                    title: format!("cap-{i}"),
                    closed: false,
                },
            )
            .unwrap();
    }
    assert!(
        client.live_slot_count() <= 4,
        "max_entries=4 must be enforced (got {})",
        client.live_slot_count()
    );

    // 5. `DehydrateError` bounds: bad magic, unsupported version, truncated, trailing bytes.
    assert_eq!(c2.hydrate_bytes(b"NOPE12345"), Err(DehydrateError::InvalidMagic));
    assert_eq!(
        c2.hydrate_bytes(b"NVEQ\x99\x00\x00\x00\x00"),
        Err(DehydrateError::UnsupportedVersion(0x99))
    );
    assert_eq!(c2.hydrate_bytes(b"NVEQ"), Err(DehydrateError::Truncated));
    assert_eq!(
        c2.hydrate_bytes(b"NVEQ\x01\x00\x00\x00\x00EXTRA"),
        Err(DehydrateError::TrailingBytes(5))
    );
}

#[test]
fn random_interleaving_property_invariants_and_zero_alloc_hot_paths() {
    let clock = Shared::new(ManualClock::new(1_000));
    let client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig {
            max_entries: 16,
            ..QueryClientConfig::default()
        },
    );

    let mut rng: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let mut next_u64 = || {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        rng
    };

    let mut active_handles: Vec<QueryHandle> = Vec::new();
    let mut ref_subs: HashMap<QueryHandle, usize> = HashMap::new();
    let mut pending_tickets = Vec::new();
    let mut pending_opts = Vec::new();
    let mut dirty_scratch = Vec::with_capacity(32);

    for step in 0..2_000u64 {
        let op = next_u64() % 9;
        match op {
            0 => {
                let id = next_u64() % 12;
                let h = client
                    .subscribe::<GetItem>(&id, QueryOptions::default().gc_time_ms(500))
                    .unwrap();
                active_handles.push(h);
                *ref_subs.entry(h).or_insert(0) += 1;
            }
            1 if !active_handles.is_empty() => {
                let idx = (next_u64() as usize) % active_handles.len();
                let h = active_handles.swap_remove(idx);
                client.unsubscribe(h);
                if let Some(cnt) = ref_subs.get_mut(&h) {
                    *cnt -= 1;
                    if *cnt == 0 {
                        ref_subs.remove(&h);
                    }
                }
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
                if ver % 5 == 0 {
                    let _ = client.complete_fetch::<GetItem>(
                        ticket,
                        Err(nve_rpc::RpcError::Transport(TransportError::network(
                            "flaky",
                        ))),
                    );
                } else {
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
                    if step % 2 == 0 {
                        let _ = client.settle_optimistic(&old);
                    } else {
                        client.rollback_optimistic(&old);
                    }
                }
            }
            7 => {
                let id = next_u64() % 24;
                let _ = client.set_query_data::<GetItem>(
                    &id,
                    Item {
                        id,
                        title: format!("seed-{step}"),
                        closed: false,
                    },
                );
            }
            _ => {
                clock.advance(250);
                let _ = client.gc_sweep(8);
            }
        }

        // Strict per-step invariants verified against reference model:
        assert_eq!(client.active_slot_count(), ref_subs.len());
        assert!(
            client.live_slot_count() <= 16.max(ref_subs.len()),
            "live slots exceeded max_entries"
        );
        for &h in ref_subs.keys() {
            let snap = client.read::<GetItem>(h);
            assert!(
                snap.data_version > 0,
                "subscribed handle {h:?} was evicted at step {step}"
            );
        }
    }

    // Drain dirty slots: verify `dirty_slots` never grew unbounded across evict+reuse cycles!
    client.drain_dirty_slots(&mut dirty_scratch);
    assert!(dirty_scratch.len() <= 32);

    for opt in pending_opts {
        client.rollback_optimistic(&opt);
    }
    for h in active_handles {
        client.unsubscribe(h);
    }
    clock.advance(400_000);
    client.gc_sweep(1024);
    assert_eq!(client.live_slot_count(), 0);
    assert_eq!(client.active_slot_count(), 0);
    assert_eq!(client.tag_edge_count(), 0);

    #[cfg(not(target_arch = "wasm32"))]
    {
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

        let (snap, read_allocs) = crate::count_allocs(|| client.read::<GetItem>(h));
        assert_eq!(snap.data.as_ref().unwrap().id, 7);
        assert_eq!(read_allocs, 0, "cache-hit read::<P> must allocate 0 times");

        let tags = [Tag64::keyed("item", 7)];
        let mut active_scratch = Vec::with_capacity(8);
        let (_, inv_allocs) =
            crate::count_allocs(|| client.invalidate_tags_into(&tags, &mut active_scratch));
        assert_eq!(active_scratch, vec![h]);
        assert_eq!(inv_allocs, 0);

        let mut dirty_buf = Vec::with_capacity(8);
        let (_, drain_allocs) =
            crate::count_allocs(|| client.drain_dirty_slots(&mut dirty_buf));
        assert!(!dirty_buf.is_empty());
        assert_eq!(drain_allocs, 0);
    }

    let _ = Pin::new(&0u8);
}
