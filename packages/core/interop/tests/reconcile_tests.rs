//! Deep unit, edge-case, and randomized property tests for `nve_interop::reconcile`.

use nve_interop::reconcile::{
    reconcile_three_way, ConflictPolicy, DivergenceKind, LogicalClock, StateReconciler,
    ThreeWayEntry,
};

struct XorShift64(u64);
impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[test]
fn three_way_all_five_divergence_kinds_and_policies() {
    // 1. Clean
    let d = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "v0",
            incoming: "v0",
            local_clock: None,
            incoming_clock: None,
        },
        ConflictPolicy::PreferLocal,
    );
    assert_eq!(d.kind, DivergenceKind::Clean);
    assert_eq!(d.resolved, "v0");
    assert!(!d.emit_upstream);
    assert!(!d.apply_local);

    // 2. LocalOnlyDirty
    let d = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "v1_local",
            incoming: "v0",
            local_clock: Some(LogicalClock::at(1, 1)),
            incoming_clock: None,
        },
        ConflictPolicy::PreferIncoming,
    );
    assert_eq!(d.kind, DivergenceKind::LocalOnlyDirty);
    assert_eq!(d.resolved, "v1_local");
    assert!(d.emit_upstream);
    assert!(!d.apply_local);

    // 3. IncomingOnly
    let d = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "v0",
            incoming: "v1_remote",
            local_clock: None,
            incoming_clock: Some(LogicalClock::at(2, 2)),
        },
        ConflictPolicy::PreferLocal,
    );
    assert_eq!(d.kind, DivergenceKind::IncomingOnly);
    assert_eq!(d.resolved, "v1_remote");
    assert!(!d.emit_upstream);
    assert!(d.apply_local);

    // 4. Converged
    let d = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "same",
            incoming: "same",
            local_clock: Some(LogicalClock::at(1, 1)),
            incoming_clock: Some(LogicalClock::at(9, 2)),
        },
        ConflictPolicy::PreferIncoming,
    );
    assert_eq!(d.kind, DivergenceKind::Converged);
    assert_eq!(d.resolved, "same");
    assert!(!d.emit_upstream);
    assert!(!d.apply_local);

    // 5a. Conflict + PreferLocal
    let d = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "local",
            incoming: "remote",
            local_clock: Some(LogicalClock::at(1, 1)),
            incoming_clock: Some(LogicalClock::at(100, 2)),
        },
        ConflictPolicy::PreferLocal,
    );
    assert_eq!(d.kind, DivergenceKind::Conflict);
    assert_eq!(d.resolved, "local");
    assert!(d.emit_upstream);
    assert!(!d.apply_local);

    // 5b. Conflict + PreferIncoming
    let d = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "local",
            incoming: "remote",
            local_clock: Some(LogicalClock::at(100, 9)),
            incoming_clock: Some(LogicalClock::at(1, 1)),
        },
        ConflictPolicy::PreferIncoming,
    );
    assert_eq!(d.kind, DivergenceKind::Conflict);
    assert_eq!(d.resolved, "remote");
    assert!(!d.emit_upstream);
    assert!(d.apply_local);

    // 5c. Conflict + LastWriteWins (counter wins, then peer_id tie-breaks)
    let d_inc_counter = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "local",
            incoming: "remote",
            local_clock: Some(LogicalClock::at(5, 99)),
            incoming_clock: Some(LogicalClock::at(6, 1)),
        },
        ConflictPolicy::LastWriteWins,
    );
    assert_eq!(d_inc_counter.resolved, "remote");
    assert!(d_inc_counter.apply_local && !d_inc_counter.emit_upstream);

    let d_tie_inc = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "local",
            incoming: "remote",
            local_clock: Some(LogicalClock::at(5, 10)),
            incoming_clock: Some(LogicalClock::at(5, 20)),
        },
        ConflictPolicy::LastWriteWins,
    );
    assert_eq!(d_tie_inc.resolved, "remote");
    assert!(d_tie_inc.apply_local);

    let d_tie_local = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "local",
            incoming: "remote",
            local_clock: Some(LogicalClock::at(5, 20)),
            incoming_clock: Some(LogicalClock::at(5, 10)),
        },
        ConflictPolicy::LastWriteWins,
    );
    assert_eq!(d_tie_local.resolved, "local");
    assert!(d_tie_local.emit_upstream);

    // Missing clock combinations in LWW are deterministic.
    let d_only_inc_clk = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "local",
            incoming: "remote",
            local_clock: None,
            incoming_clock: Some(LogicalClock::at(1, 1)),
        },
        ConflictPolicy::LastWriteWins,
    );
    assert_eq!(d_only_inc_clk.resolved, "remote");
    assert!(d_only_inc_clk.apply_local);

    let d_only_local_clk = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "local",
            incoming: "remote",
            local_clock: Some(LogicalClock::at(1, 1)),
            incoming_clock: None,
        },
        ConflictPolicy::LastWriteWins,
    );
    assert_eq!(d_only_local_clk.resolved, "local");
    assert!(d_only_local_clk.emit_upstream);

    let d_no_clks = reconcile_three_way(
        ThreeWayEntry {
            key: "k",
            base: "v0",
            local: "local",
            incoming: "remote",
            local_clock: None,
            incoming_clock: None,
        },
        ConflictPolicy::LastWriteWins,
    );
    assert_eq!(d_no_clks.resolved, "local");
    assert!(d_no_clks.emit_upstream);
}

#[test]
fn logical_clock_monotonicity_merge_idempotence_and_saturation() {
    let mut c = LogicalClock::new(7);
    assert_eq!(c.tick(), LogicalClock::at(1, 7));
    assert_eq!(c.tick(), LogicalClock::at(2, 7));

    let remote = LogicalClock::at(42, 99);
    assert_eq!(c.merge(remote), LogicalClock::at(42, 7));
    assert_eq!(c.merge(remote), LogicalClock::at(42, 7));
    assert_eq!(c.merge(LogicalClock::at(10, 1)), LogicalClock::at(42, 7));
    assert_eq!(c.tick(), LogicalClock::at(43, 7));

    let mut sat = LogicalClock::at(u64::MAX, 3);
    assert_eq!(sat.tick(), LogicalClock::at(u64::MAX, 3));
}

#[test]
fn state_reconciler_ack_synced_in_flight_edit_race() {
    let mut rec = StateReconciler::<String, String>::new(1);
    rec.seed_baseline("email".into(), "init@example.com".into());
    assert_eq!(rec.dirty_count(), 0);

    let c1 = rec.record_local("email".into(), "draft1@example.com".into());
    assert_eq!(c1, LogicalClock::at(1, 1));
    assert_eq!(rec.dirty_count(), 1);

    let c2 = rec.record_local("email".into(), "draft2@example.com".into());
    assert_eq!(c2, LogicalClock::at(2, 1));
    assert_eq!(rec.dirty_count(), 1);

    // Ack for sync #1 arrives while newer local edit exists: MUST NOT clear dirty!
    assert!(!rec.ack_synced(&"email".into(), &"draft1@example.com".into()));
    assert_eq!(rec.dirty_count(), 1);
    assert_eq!(rec.baseline(&"email".into()).map(String::as_str), Some("draft1@example.com"));
    assert_eq!(rec.local_value(&"email".into()).map(String::as_str), Some("draft2@example.com"));

    // Ack for sync #2 clears dirty flag cleanly.
    assert!(rec.ack_synced(&"email".into(), &"draft2@example.com".into()));
    assert_eq!(rec.dirty_count(), 0);

    // Reverting a local edit back to baseline clears dirty flag immediately.
    rec.record_local("email".into(), "temp".into());
    assert_eq!(rec.dirty_count(), 1);
    rec.record_local("email".into(), "draft2@example.com".into());
    assert_eq!(rec.dirty_count(), 0);
}

#[test]
fn state_reconciler_batch_handles_all_cases_and_concurrent_creations() {
    let mut rec = StateReconciler::<String, String>::new(1);
    rec.seed_baseline("clean".into(), "a".into());
    rec.seed_baseline("local_dirty".into(), "b".into());
    rec.seed_baseline("inc_only".into(), "c".into());
    rec.seed_baseline("converged".into(), "d".into());
    rec.seed_baseline("conflict".into(), "e".into());

    rec.record_local("local_dirty".into(), "b_user".into());
    rec.record_local("converged".into(), "d_both".into());
    rec.record_local("conflict".into(), "e_local".into());
    rec.record_local("new_same".into(), "same".into());
    rec.record_local("new_conflict".into(), "mine".into());

    let batch = rec.reconcile_incoming_batch(
        vec![
            ("clean".into(), "a".into(), None),
            ("local_dirty".into(), "b".into(), None),
            ("inc_only".into(), "c_remote".into(), Some(LogicalClock::at(10, 2))),
            ("converged".into(), "d_both".into(), None),
            ("conflict".into(), "e_remote".into(), Some(LogicalClock::at(20, 2))),
            ("brand_new".into(), "z".into(), None),
            ("new_same".into(), "same".into(), None),
            ("new_conflict".into(), "theirs".into(), Some(LogicalClock::at(30, 2))),
        ],
        ConflictPolicy::LastWriteWins,
    );

    assert_eq!(batch.decisions.len(), 8);
    assert_eq!(batch.conflict_count(), 2);
    assert_eq!(rec.clock().counter, 30);
    assert_eq!(batch.to_emit_upstream.len(), 1);
    assert_eq!(batch.to_emit_upstream[0].0, "local_dirty");
    assert_eq!(batch.to_apply_local.len(), 4);
    assert_eq!(rec.local_value(&"inc_only".into()).map(String::as_str), Some("c_remote"));
    assert_eq!(rec.local_value(&"conflict".into()).map(String::as_str), Some("e_remote"));
    assert_eq!(rec.local_value(&"brand_new".into()).map(String::as_str), Some("z"));
    assert_eq!(rec.local_value(&"new_conflict".into()).map(String::as_str), Some("theirs"));
    assert_eq!(rec.dirty_count(), 1);
    assert_eq!(rec.dirty_keys().collect::<Vec<_>>(), vec![&"local_dirty".to_string()]);
}

#[test]
fn randomized_two_peer_lww_property_always_converges_and_preserves_invariants() {
    let mut rng = XorShift64(0x9E37_79B9_7F4A_7C15);
    let mut peer_a = StateReconciler::<u8, u32>::new(1);
    let mut peer_b = StateReconciler::<u8, u32>::new(2);

    for k in 0u8..8 {
        peer_a.seed_baseline(k, 0);
        peer_b.seed_baseline(k, 0);
    }

    for _round in 0..200 {
        let mut outbox_a = Vec::new();
        let mut outbox_b = Vec::new();

        for _ in 0..3 {
            let k = (rng.next() % 8) as u8;
            let v = (rng.next() % 50) as u32;
            let clk = peer_a.record_local(k, v);
            outbox_a.push((k, v, Some(clk)));
        }
        for _ in 0..3 {
            let k = (rng.next() % 8) as u8;
            let v = (rng.next() % 50) as u32;
            let clk = peer_b.record_local(k, v);
            outbox_b.push((k, v, Some(clk)));
        }

        // Keep only the latest mutation per key in each peer's current snapshot.
        let snap_a: Vec<_> = peer_a
            .dirty_entries()
            .map(|(&k, &v, c)| (k, v, Some(c)))
            .collect();
        let snap_b: Vec<_> = peer_b
            .dirty_entries()
            .map(|(&k, &v, c)| (k, v, Some(c)))
            .collect();
        let _ = (outbox_a, outbox_b);

        if rng.next().is_multiple_of(2) {
            peer_a.reconcile_incoming_batch(snap_b, ConflictPolicy::LastWriteWins);
            peer_b.reconcile_incoming_batch(snap_a, ConflictPolicy::LastWriteWins);
        } else {
            peer_b.reconcile_incoming_batch(snap_a, ConflictPolicy::LastWriteWins);
            peer_a.reconcile_incoming_batch(snap_b, ConflictPolicy::LastWriteWins);
        }

        // Flush any surviving winning local edits to the losing peer and ack synced values.
        let win_a: Vec<_> = peer_a
            .dirty_entries()
            .map(|(&k, &v, c)| (k, v, Some(c)))
            .collect();
        let win_b: Vec<_> = peer_b
            .dirty_entries()
            .map(|(&k, &v, c)| (k, v, Some(c)))
            .collect();
        peer_b.reconcile_incoming_batch(win_a.clone(), ConflictPolicy::LastWriteWins);
        peer_a.reconcile_incoming_batch(win_b.clone(), ConflictPolicy::LastWriteWins);
        for (k, v, _) in win_a {
            peer_a.ack_synced(&k, &v);
        }
        for (k, v, _) in win_b {
            peer_b.ack_synced(&k, &v);
        }

        assert_eq!(peer_a.dirty_count(), peer_a.dirty_keys().count());
        assert_eq!(peer_b.dirty_count(), peer_b.dirty_keys().count());
        for k in 0u8..8 {
            assert_eq!(peer_a.local_value(&k), peer_b.local_value(&k), "diverged on key {k}");
        }
    }
}
