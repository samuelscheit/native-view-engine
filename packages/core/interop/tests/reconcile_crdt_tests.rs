//! Integration & convergence tests for `nve_interop::reconcile::CrdtReconcileAdapter`.

use nve_interop::crdt::{CrdtWireError, HlcTimestamp, PeerId};
use nve_interop::reconcile::{ConflictPolicy, CrdtReconcileAdapter, DivergenceKind};

#[test]
fn offline_optimistic_edits_converge_across_peers_regardless_of_order() {
    for reverse_order in [false, true] {
        let mut peer_a = CrdtReconcileAdapter::new(PeerId::new(1), 7);
        let mut peer_b = CrdtReconcileAdapter::new(PeerId::new(2), 7);

        let seed_ts = HlcTimestamp::new(1_000, 0, 0);
        for p in [&mut peer_a, &mut peer_b] {
            p.seed_synced("doc/title", b"Initial".to_vec(), seed_ts);
            p.seed_synced("doc/body", b"Body v0".to_vec(), seed_ts);
            p.seed_synced("doc/author", b"Alice".to_vec(), seed_ts);
        }

        peer_a.write_optimistic("doc/title", b"Title A".to_vec(), 2_000);
        peer_a.write_optimistic("doc/body", b"Body A".to_vec(), 2_010);
        peer_a.write_optimistic("doc/author", b"Bob".to_vec(), 2_020);

        peer_b.write_optimistic("doc/title", b"Title B".to_vec(), 2_500);
        peer_b.write_optimistic("doc/author", b"Bob".to_vec(), 2_510);

        let pkt_a = peer_a.export_nvc1_delta().expect("encode A").expect("dirty A");
        let pkt_b = peer_b.export_nvc1_delta().expect("encode B").expect("dirty B");

        let (batch_a, batch_b) = if !reverse_order {
            let ba = peer_a
                .apply_nvc1_packet(&pkt_b, 3_000, ConflictPolicy::LastWriteWins)
                .expect("A applies B");
            let bb = peer_b
                .apply_nvc1_packet(&pkt_a, 3_000, ConflictPolicy::LastWriteWins)
                .expect("B applies A");
            (ba, bb)
        } else {
            let bb = peer_b
                .apply_nvc1_packet(&pkt_a, 3_000, ConflictPolicy::LastWriteWins)
                .expect("B applies A");
            let ba = peer_a
                .apply_nvc1_packet(&pkt_b, 3_000, ConflictPolicy::LastWriteWins)
                .expect("A applies B");
            (ba, bb)
        };

        assert_eq!(batch_a.conflict_count(), 1);
        assert_eq!(batch_b.conflict_count(), 1);

        assert_eq!(peer_a.value("doc/title"), Some(b"Title B".as_slice()));
        assert_eq!(peer_b.value("doc/title"), Some(b"Title B".as_slice()));
        assert_eq!(peer_a.value("doc/body"), Some(b"Body A".as_slice()));
        assert_eq!(peer_b.value("doc/body"), Some(b"Body A".as_slice()));
        assert_eq!(peer_a.value("doc/author"), Some(b"Bob".as_slice()));
        assert_eq!(peer_b.value("doc/author"), Some(b"Bob".as_slice()));

        // Replaying the same packet is strictly idempotent.
        let replay = peer_a
            .apply_nvc1_packet(&pkt_b, 3_100, ConflictPolicy::LastWriteWins)
            .expect("idempotent replay");
        assert!(replay.is_empty());
    }
}

#[test]
fn same_timestamp_peer_id_tiebreak_and_ack_race() {
    let mut low_peer = CrdtReconcileAdapter::new(PeerId::new(10), 1);
    let mut high_peer = CrdtReconcileAdapter::new(PeerId::new(20), 1);

    let seed = HlcTimestamp::new(500, 0, 0);
    low_peer.seed_synced("k", b"v0".to_vec(), seed);
    high_peer.seed_synced("k", b"v0".to_vec(), seed);

    low_peer.write_optimistic("k", b"low".to_vec(), 1_000);
    high_peer.write_optimistic("k", b"high".to_vec(), 1_000);

    let pkt_low = low_peer.export_nvc1_delta().unwrap().unwrap();
    let pkt_high = high_peer.export_nvc1_delta().unwrap().unwrap();

    low_peer
        .apply_nvc1_packet(&pkt_high, 1_100, ConflictPolicy::LastWriteWins)
        .unwrap();
    high_peer
        .apply_nvc1_packet(&pkt_low, 1_100, ConflictPolicy::LastWriteWins)
        .unwrap();

    assert_eq!(low_peer.value("k"), Some(b"high".as_slice()));
    assert_eq!(high_peer.value("k"), Some(b"high".as_slice()));

    // Ack race: user writes v2 before v1 ack returns.
    low_peer.write_optimistic("k", b"v1".to_vec(), 1_200);
    low_peer.write_optimistic("k", b"v2".to_vec(), 1_250);
    assert!(!low_peer.ack_synced("k", b"v1"));
    assert_eq!(low_peer.dirty_count(), 1);
    assert_eq!(low_peer.value("k"), Some(b"v2".as_slice()));

    // Acking v2 clears dirty completely.
    assert!(low_peer.ack_synced("k", b"v2"));
    assert_eq!(low_peer.dirty_count(), 0);
    assert!(low_peer.export_nvc1_delta().unwrap().is_none());
}

#[test]
fn namespace_isolation_and_corrupt_nvc1_rejection() {
    let mut peer_ns1 = CrdtReconcileAdapter::new(PeerId::new(1), 1);
    let mut peer_ns2 = CrdtReconcileAdapter::new(PeerId::new(2), 2);
    peer_ns1.register_key("shared");
    peer_ns2.write_optimistic("shared", b"from_ns2".to_vec(), 100);

    let pkt_ns2 = peer_ns2.export_nvc1_delta().unwrap().unwrap();
    let batch = peer_ns1
        .apply_nvc1_packet(&pkt_ns2, 200, ConflictPolicy::PreferIncoming)
        .unwrap();
    assert!(batch.is_empty());
    assert_eq!(peer_ns1.value("shared"), None);

    // PreferLocal keeps local edit even when remote has newer timestamp.
    let mut a = CrdtReconcileAdapter::new(PeerId::new(1), 3);
    let mut b = CrdtReconcileAdapter::new(PeerId::new(2), 3);
    a.seed_synced("x", b"0".to_vec(), HlcTimestamp::ZERO);
    b.seed_synced("x", b"0".to_vec(), HlcTimestamp::ZERO);
    a.write_optimistic("x", b"a".to_vec(), 10);
    b.write_optimistic("x", b"b".to_vec(), 9_999);
    let pkt_b = b.export_nvc1_delta().unwrap().unwrap();
    let res = a.apply_nvc1_packet(&pkt_b, 10_000, ConflictPolicy::PreferLocal).unwrap();
    assert_eq!(res.decisions[0].kind, DivergenceKind::Conflict);
    assert_eq!(a.value("x"), Some(b"a".as_slice()));

    let err = peer_ns1.apply_nvc1_packet(b"BAD!", 300, ConflictPolicy::LastWriteWins);
    assert!(matches!(err, Err(CrdtWireError::Truncated { .. })));
}
