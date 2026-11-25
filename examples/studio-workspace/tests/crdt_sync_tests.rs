//! Multi-peer offline partition & heal convergence tests, out-of-order/duplicate delta
//! delivery tests, and corrupt `NVC1` rejection tests for [`StudioCrdtDoc`].

use nve_interop::crdt::{
    CrdtRecord, CrdtValue, CrdtWireError, HlcTimestamp, NVE_CRDT_DELTA_MAGIC,
    NVE_CRDT_DELTA_VERSION, NveCrdtDelta, OpId, PeerId, VersionVector, crdt_xxh64,
};
use nve_studio_workspace::state::{
    ActiveView, BoardFilter, ChannelKind, DocBlockKind, Priority, StudioCrdtDoc, TaskStatus,
    WorkspaceSnapshot,
};

fn sync_pair(a: &mut StudioCrdtDoc, b: &mut StudioCrdtDoc) {
    let delta_ab = a.delta_since(b.version_vector());
    let delta_ba = b.delta_since(a.version_vector());
    b.merge_delta(&delta_ab).expect("merge a->b");
    a.merge_delta(&delta_ba).expect("merge b->a");
}

fn full_mesh_sync(peers: &mut [StudioCrdtDoc]) {
    let n = peers.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let (left, right) = peers.split_at_mut(j);
            sync_pair(&mut left[i], &mut right[0]);
        }
    }
    // Second pass propagates transitive knowledge to all peers.
    for i in (0..n).rev() {
        for j in 0..i {
            let (left, right) = peers.split_at_mut(i);
            sync_pair(&mut left[j], &mut right[0]);
        }
    }
}

#[test]
fn five_peer_offline_partition_and_heal_converges_deterministically() {
    let base = WorkspaceSnapshot::default();
    let seed_task_1 = base.tasks[0].id;
    let seed_task_2 = base.tasks[1].id;
    let seed_eng_msg = base.eng_messages[0].id;
    let seed_eng_initial_reactions = base.eng_messages[0].reactions;
    let seed_design_msg = base.design_messages[0].id;
    let seed_design_initial_reactions = base.design_messages[0].reactions;
    let seed_doc_id = base.docs[0].id;
    let seed_block_0 = base.docs[0].blocks[0].id;
    let seed_block_1 = base.docs[0].blocks[1].id;
    // Find a checklist block in the seed doc if present, otherwise use block 1.
    let checklist_block = base.docs[0]
        .blocks
        .iter()
        .find(|b| b.kind.is_checklist())
        .map_or(seed_block_1, |b| b.id);
    let initial_checked = base.docs[0]
        .blocks
        .iter()
        .find(|b| b.id == checklist_block)
        .expect("checklist block exists")
        .kind
        .is_checked();

    let mut peers: Vec<StudioCrdtDoc> = (1..=5)
        .map(|id| StudioCrdtDoc::from_snapshot(PeerId(id), 10_000, &base))
        .collect();

    // Partition A: peers 0, 1 ({PeerId(1), PeerId(2)})
    // Partition B: peers 2, 3, 4 ({PeerId(3), PeerId(4), PeerId(5)})

    // Peer 1 (Partition A): edits task 1 title, reacts +2 to eng msg, inserts doc block after block 0, adds a new task.
    let p1_task = peers[0].add_task(
        10_100,
        "NVE-901",
        "Zero-copy CRDT delta transport",
        "Alice",
        "CRDT",
        Priority::P0,
        TaskStatus::InProgress,
    );
    assert!(peers[0].edit_task_title(10_110, seed_task_1, "CRDT title from Peer 1"));
    assert!(peers[0].react_chat_message(10_120, ChannelKind::Engineering, seed_eng_msg, 2));
    let p1_blk = peers[0]
        .insert_doc_block_after(
            10_130,
            seed_doc_id,
            Some(seed_block_0),
            DocBlockKind::Callout,
            "Inserted by Peer 1 after header",
        )
        .expect("p1 block insert");
    assert!(peers[0].edit_doc_block_text(
        10_140,
        seed_doc_id,
        checklist_block,
        "Checklist text edited by Peer 1"
    ));

    // Peer 2 (Partition A): edits task 1 status (different field!), deletes seed task 2, posts design chat msg, reacts +1 design.
    assert!(peers[1].edit_task_status(10_115, seed_task_1, TaskStatus::Done));
    assert!(peers[1].delete_task(10_125, seed_task_2));
    let p2_msg = peers[1].post_chat_message(
        10_135,
        ChannelKind::Design,
        "Maya",
        "#1E293B",
        "#38BDF8",
        "09:15",
        "Offline design critique from Peer 2",
    );
    assert!(peers[1].react_chat_message(10_145, ChannelKind::Design, seed_design_msg, 1));
    assert!(peers[1].react_chat_message(10_150, ChannelKind::Engineering, seed_eng_msg, 3));

    // Sync inside Partition A before healing.
    {
        let (a, b) = peers.split_at_mut(1);
        sync_pair(&mut a[0], &mut b[0]);
    }

    // Peer 3 (Partition B): edits task 1 priority (third independent field!), concurrently inserts block at SAME anchor seed_block_0,
    // and concurrently toggles checklist_block (different field from Peer 1's text edit!).
    assert!(peers[2].edit_task_priority(10_120, seed_task_1, Priority::P0));
    let p3_blk = peers[2]
        .insert_doc_block_after(
            10_132,
            seed_doc_id,
            Some(seed_block_0),
            DocBlockKind::Paragraph,
            "Inserted by Peer 3 at same anchor",
        )
        .expect("p3 block insert");
    let toggled = peers[2]
        .toggle_doc_block_checklist(10_142, seed_doc_id, checklist_block)
        .expect("toggle checklist");
    assert_eq!(toggled, !initial_checked);
    assert!(peers[2].react_chat_message(10_155, ChannelKind::Engineering, seed_eng_msg, 1));

    // Peer 4 (Partition B): concurrently edits task 1 title at LATER timestamp (must beat Peer 1's title!),
    // posts eng msg, adds a new doc with a block, reacts -1 on eng msg (PnCounter decrement!).
    assert!(peers[3].edit_task_title(10_500, seed_task_1, "Winning later title from Peer 4"));
    let p4_msg = peers[3].post_chat_message(
        10_510,
        ChannelKind::Engineering,
        "Devon",
        "#1E1B4B",
        "#A5B4FC",
        "09:20",
        "Shipped NVC1 wire format",
    );
    assert!(peers[3].react_chat_message(10_520, ChannelKind::Engineering, seed_eng_msg, -1));
    let p4_doc = peers[3].add_doc(10_530, "crdt-rfc", "CRDT Sync RFC", "Just now");
    let p4_doc_blk = peers[3]
        .add_doc_block(
            10_540,
            p4_doc,
            DocBlockKind::Heading1,
            "Binary CRDT Wire Protocol",
        )
        .expect("p4 doc block");

    // Peer 5 (Partition B): reorders seed_block_1 to head of doc 1, reacts +4 to design msg, deletes nothing.
    assert!(peers[4].reorder_doc_block(10_200, seed_doc_id, seed_block_1, None));
    assert!(peers[4].react_chat_message(10_210, ChannelKind::Design, seed_design_msg, 4));

    // Intra-partition B sync (ring order).
    {
        let (left, right) = peers.split_at_mut(3);
        sync_pair(&mut left[2], &mut right[0]);
    }
    {
        let (left, right) = peers.split_at_mut(4);
        sync_pair(&mut left[3], &mut right[0]);
    }

    // Heal partitions via full mesh sync!
    full_mesh_sync(&mut peers);

    // Every single one of the 5 peers must project the EXACT SAME WorkspaceSnapshot!
    let projected: Vec<WorkspaceSnapshot> =
        peers.iter().map(|p| p.project_snapshot(&base)).collect();
    for i in 1..projected.len() {
        assert_eq!(
            projected[0], projected[i],
            "peer 1 and peer {} diverged after partition heal",
            i + 1
        );
    }

    let final_snap = &projected[0];

    // 1. Task assertions:
    // - seed_task_1 has Peer 4's later title, Peer 2's status, Peer 3's priority!
    let t1 = final_snap
        .tasks
        .iter()
        .find(|t| t.id == seed_task_1)
        .expect("seed_task_1 present");
    assert_eq!(t1.title, "Winning later title from Peer 4");
    assert_eq!(t1.status, TaskStatus::Done);
    assert_eq!(t1.priority, Priority::P0);
    // - seed_task_2 was deleted by Peer 2 and stayed deleted across all peers!
    assert!(final_snap.tasks.iter().all(|t| t.id != seed_task_2));
    // - p1_task exists on all peers.
    assert!(final_snap.tasks.iter().any(|t| t.id == p1_task));

    // 2. Chat & PnCounter reaction assertions:
    // eng msg reactions: seed + 2 (P1) + 3 (P2) + 1 (P3) - 1 (P4) = seed + 5 (no double-counting seed!)
    let eng0 = final_snap
        .eng_messages
        .iter()
        .find(|m| m.id == seed_eng_msg)
        .expect("eng msg 0");
    assert_eq!(eng0.reactions, seed_eng_initial_reactions + 5);
    assert!(final_snap.eng_messages.iter().any(|m| m.id == p4_msg));

    // design msg reactions: seed + 1 (P2) + 4 (P5) = seed + 5
    let des0 = final_snap
        .design_messages
        .iter()
        .find(|m| m.id == seed_design_msg)
        .expect("design msg 0");
    assert_eq!(des0.reactions, seed_design_initial_reactions + 5);
    assert!(final_snap.design_messages.iter().any(|m| m.id == p2_msg));

    // 3. Docs & RGA block ordering assertions:
    let doc0 = final_snap
        .docs
        .iter()
        .find(|d| d.id == seed_doc_id)
        .expect("seed doc");
    // No duplicate block IDs after reorder + concurrent inserts.
    let mut unique_ids = std::collections::BTreeSet::new();
    for blk in &doc0.blocks {
        assert!(unique_ids.insert(blk.id), "duplicate block id {}", blk.id);
    }
    // seed_block_1 was reordered to the head (index 0) by Peer 5.
    assert_eq!(doc0.blocks[0].id, seed_block_1);
    // Both p1_blk and p3_blk exist right after seed_block_0 in deterministic order.
    let idx_b0 = doc0.blocks.iter().position(|b| b.id == seed_block_0).unwrap();
    let idx_p1 = doc0.blocks.iter().position(|b| b.id == p1_blk).unwrap();
    let idx_p3 = doc0.blocks.iter().position(|b| b.id == p3_blk).unwrap();
    assert!(idx_p1 > idx_b0 && idx_p3 > idx_b0);
    // Checklist block kept BOTH Peer 1's text edit AND Peer 3's checked toggle!
    let chk = doc0
        .blocks
        .iter()
        .find(|b| b.id == checklist_block)
        .expect("checklist block");
    assert_eq!(chk.text, "Checklist text edited by Peer 1");
    assert_eq!(chk.kind.is_checked(), !initial_checked);

    // Peer 4's new doc and block converged everywhere.
    let new_doc = final_snap
        .docs
        .iter()
        .find(|d| d.id == p4_doc)
        .expect("p4 doc");
    assert_eq!(new_doc.title, "CRDT Sync RFC");
    assert_eq!(new_doc.blocks.len(), 1);
    assert_eq!(new_doc.blocks[0].id, p4_doc_blk);

    // Local UI/viewport state is preserved independently per peer on project_snapshot.
    let mut custom_ui = base.clone();
    custom_ui.active_view = ActiveView::Docs;
    custom_ui.board_filter = BoardFilter::Done;
    custom_ui.viewport_width = 1440;
    let projected_ui = peers[0].project_snapshot(&custom_ui);
    assert_eq!(projected_ui.active_view, ActiveView::Docs);
    assert_eq!(projected_ui.board_filter, BoardFilter::Done);
    assert_eq!(projected_ui.viewport_width, 1440);
    assert_eq!(projected_ui.tasks, final_snap.tasks);
    assert_eq!(projected_ui.docs, final_snap.docs);
}

#[test]
fn out_of_order_duplicate_deltas_and_empty_peer_catchup() {
    let base = WorkspaceSnapshot::default();
    let mut p1 = StudioCrdtDoc::from_snapshot(PeerId(1), 5_000, &base);
    let mut p2 = StudioCrdtDoc::from_snapshot(PeerId(2), 5_000, &base);
    let mut p3 = StudioCrdtDoc::from_snapshot(PeerId(3), 5_000, &base);

    let mut deltas = Vec::new();

    // Generate a sequence of incremental deltas from p1 and p2 without syncing yet.
    let vv0 = p3.version_vector().clone();
    let t_a = p1.add_task(5_010, "A-1", "Alpha", "Ana", "Sync", Priority::P1, TaskStatus::Todo);
    deltas.push(p1.delta_since(&vv0));

    let vv1 = p1.version_vector().clone();
    p1.edit_task_title(5_020, t_a, "Alpha v2");
    deltas.push(p1.delta_since(&vv1));

    let vv2 = p1.version_vector().clone();
    let doc_id = base.docs[0].id;
    let b1 = p1
        .add_doc_block(5_030, doc_id, DocBlockKind::Code, "fn sync() {}")
        .unwrap();
    let b2 = p1
        .insert_doc_block_after(5_040, doc_id, Some(b1), DocBlockKind::Paragraph, "After code")
        .unwrap();
    deltas.push(p1.delta_since(&vv2));

    let vv3 = p1.version_vector().clone();
    p1.delete_doc_block(5_050, doc_id, b1);
    deltas.push(p1.delta_since(&vv3));

    p2.react_chat_message(5_025, ChannelKind::Engineering, base.eng_messages[0].id, 3);
    p2.post_chat_message(
        5_035,
        ChannelKind::Design,
        "Kai",
        "#0F172A",
        "#F8FAFC",
        "10:00",
        "Out of order test",
    );
    deltas.push(p2.delta_since(&vv0));

    // Deliver deltas to p3 in REVERSE order, AND deliver every delta 3 times!
    for round in 0..3 {
        for delta in deltas.iter().rev() {
            let summary = p3.merge_delta(delta).expect("merge ooo delta");
            if round > 0 {
                assert_eq!(
                    summary.records_applied, 0,
                    "duplicate delta delivery must be strictly idempotent"
                );
            }
        }
    }

    // Also sync p1 and p2 normally and compare with p3.
    sync_pair(&mut p1, &mut p2);
    sync_pair(&mut p1, &mut p3);
    sync_pair(&mut p2, &mut p3);

    let s1 = p1.project_snapshot(&base);
    let s2 = p2.project_snapshot(&base);
    let s3 = p3.project_snapshot(&base);
    assert_eq!(s1, s2);
    assert_eq!(s2, s3);

    // Verify b1 is deleted and b2 survived despite delete arriving before insert!
    let blocks = &s3.docs[0].blocks;
    assert!(blocks.iter().all(|b| b.id != b1));
    assert!(blocks.iter().any(|b| b.id == b2));

    // Brand-new empty peer catches up completely from p3 via delta_since(&VersionVector::new()).
    let mut fresh = StudioCrdtDoc::new(PeerId(99), 6_000);
    let catchup = p3.delta_since(fresh.version_vector());
    let summary = fresh.merge_delta(&catchup).expect("fresh peer catchup");
    assert!(summary.records_applied > 0);
    assert_eq!(fresh.project_snapshot(&base), s3);
}

#[test]
fn corrupt_nvc1_deltas_are_rejected_without_mutating_state() {
    let base = WorkspaceSnapshot::default();
    let mut p1 = StudioCrdtDoc::from_snapshot(PeerId(1), 1_000, &base);
    let mut p2 = StudioCrdtDoc::from_snapshot(PeerId(2), 1_000, &base);

    p1.add_task(1_100, "T-1", "Valid task", "Sam", "Core", Priority::P0, TaskStatus::Todo);
    let valid_delta = p1.delta_since(p2.version_vector());
    let before = p2.project_snapshot(&base);
    let vv_before = p2.version_vector().clone();

    // 1. Truncated bytes
    assert!(matches!(
        p2.merge_delta(&valid_delta[..12]),
        Err(CrdtWireError::Truncated { .. })
    ));

    // 2. Corrupted checksum / bit flip
    let mut flipped = valid_delta.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x40;
    assert!(p2.merge_delta(&flipped).is_err());

    // 3. Bad magic
    let mut bad_magic = valid_delta.clone();
    bad_magic[0..8].copy_from_slice(b"BADMAGIC");
    assert!(matches!(
        p2.merge_delta(&bad_magic),
        Err(CrdtWireError::InvalidMagic(_))
    ));

    // 4. Valid NveCrdtDelta wire checksum, but invalid Studio domain payload (Priority = 99) -> atomic rejection!
    let bogus_domain = NveCrdtDelta::new(
        PeerId(1),
        VersionVector::new(),
        vec![
            CrdtRecord::OrSetAddU64 {
                namespace: 1,
                key: 7777,
                op_id: OpId::new(PeerId(1), 99),
            },
            CrdtRecord::LwwSet {
                namespace: 2,
                entity_id: 7777,
                field_id: 4, // priority
                op_id: OpId::new(PeerId(1), 100),
                timestamp: HlcTimestamp::new(2000, 0, 1),
                value: CrdtValue::U32(99), // invalid Priority discriminant
            },
        ],
    )
    .encode()
    .unwrap();
    assert!(matches!(
        p2.merge_delta(&bogus_domain),
        Err(CrdtWireError::InvalidPayload(_))
    ));

    // 5. Raw header with unsupported version + recomputed checksum.
    let mut bad_ver = Vec::new();
    bad_ver.extend_from_slice(&NVE_CRDT_DELTA_MAGIC);
    bad_ver.extend_from_slice(&(NVE_CRDT_DELTA_VERSION + 7).to_le_bytes());
    bad_ver.extend_from_slice(&1u64.to_le_bytes());
    bad_ver.extend_from_slice(&0u32.to_le_bytes());
    bad_ver.extend_from_slice(&0u32.to_le_bytes());
    let ck = crdt_xxh64(&bad_ver);
    bad_ver.extend_from_slice(&ck.to_le_bytes());
    assert!(matches!(
        p2.merge_delta(&bad_ver),
        Err(CrdtWireError::UnsupportedVersion(_))
    ));

    // Peer 2 state and VV must remain completely unchanged after all rejected merges.
    assert_eq!(p2.project_snapshot(&base), before);
    assert_eq!(p2.version_vector(), &vv_before);
}
