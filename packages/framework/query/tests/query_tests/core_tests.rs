//! Core cache contract tests: deduplication, stale-response guards, overlapping optimistic
//! rebase/rollback/settle, retries, hydration roundtrip, and bounded GC.

use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use nve_query::{
    ManualClock, QueryClient, QueryClientConfig, QueryMode, QueryOptions, Shared,
};
use nve_rpc::{CallReply, CborCodec, RpcCodec, RpcError, RpcStatus, Tag64};

use crate::{block_on, DemoErr, Item, ListItems, RenameInput, RenameItem, ScriptedTransport};

#[test]
fn deduplication_stale_response_race_guard_and_tag_wildcard_invalidation_work() {
    let clock = Shared::new(ManualClock::new(1_000));
    let client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        clock.clone(),
        QueryClientConfig::default(),
    );
    let wakes = Shared::new(AtomicUsize::new(0));
    let wakes_cb = Shared::clone(&wakes);
    client.set_wake_callback(Shared::new(move || {
        wakes_cb.fetch_add(1, Ordering::Relaxed);
    }));

    let opts = QueryOptions::default().stale_time_ms(5_000);
    let h1 = client.subscribe::<ListItems>(&0, opts).unwrap();
    let h2 = client.subscribe::<ListItems>(&0, opts).unwrap();
    assert_eq!(h1, h2);
    assert_eq!(client.active_slot_count(), 1);

    let (t1, _) = client.begin_fetch(h1).expect("first fetch starts");
    assert!(client.begin_fetch(h2).is_none());

    let invalidated = client.invalidate_tags(&[Tag64::of("items.list")]);
    assert_eq!(invalidated, vec![h1]);
    let (t2, _) = client
        .begin_fetch(h1)
        .expect("invalidated slot starts fresh fetch_gen");
    assert_ne!(t1.fetch_gen, t2.fetch_gen);

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

    let wild = client.invalidate_all();
    assert_eq!(wild, vec![h1]);
    assert!(client.read::<ListItems>(h1).is_stale);
    assert!(client.read::<ListItems>(h1).is_invalidated);
    assert!(wakes.load(Ordering::Relaxed) > 0);

    client.unsubscribe(h1);
    assert_eq!(client.active_slot_count(), 1);
    client.unsubscribe(h2);
    assert_eq!(client.active_slot_count(), 0);
}

#[test]
fn overlapping_optimistic_layers_rebase_without_clobbering_or_flicker() {
    let clock = Shared::new(ManualClock::new(1_000));
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

    client.rollback_optimistic(&m1);
    let after_m1_fail = client.read::<ListItems>(h).data.unwrap();
    assert_eq!(after_m1_fail[0].title, "A");
    assert!(after_m1_fail[1].closed);

    let to_refetch = client.settle_optimistic(&m2);
    assert_eq!(to_refetch, vec![h]);
    let during_refetch = client.read::<ListItems>(h).data.unwrap();
    assert!(during_refetch[1].closed);

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
    let clock = Shared::new(ManualClock::new(10_000));
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
    assert!(client.read::<ListItems>(h).is_stale);

    let blob = client.dehydrate_bytes().unwrap();
    let ssr_clock = Shared::new(ManualClock::new(10_500));
    let ssr_client = QueryClient::with_clock(
        ScriptedTransport::default(),
        CborCodec::default(),
        ssr_clock.clone(),
        QueryClientConfig {
            mode: QueryMode::Ssr,
            ..QueryClientConfig::default()
        },
    );
    assert_eq!(ssr_client.hydrate_bytes(&blob).unwrap(), 1);
    let hh = ssr_client
        .subscribe::<ListItems>(&0, QueryOptions::default())
        .unwrap();
    assert!(!ssr_client.needs_fetch(hh));
    let hydrated = ssr_client.read::<ListItems>(hh).data.unwrap();
    assert_eq!(hydrated[0].title, "Alpha v2");

    client.unsubscribe(h);
    clock.advance(400_000);
    assert_eq!(client.gc_sweep(64), 1);
    assert_eq!(client.live_slot_count(), 0);
    assert_eq!(client.tag_edge_count(), 0);
}
