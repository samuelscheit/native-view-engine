//! Deep integration tests for live multi-peer binary `NVC1` CRDT sync over:
//! 1. Real OS loopback TCP sockets (`LoopbackTcpCrdtTransport` on `127.0.0.1:0` via `nve_interop::sync_transport`)
//! 2. Corrupt & truncated `NVC1`/`NVCD` frame rejection over live loopback TCP
//! 3. Deterministic `InMemoryMeshTransport` with packet drop, reverse reordering, and partition/heal
//! 4. Interactive Studio UI sync telemetry panel (`crdt-sync-now` & `crdt-toggle-partition`) + `nve-ui` chrome

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use nve_abi::{EventBatchBuf, FrameId, SurfaceId, TimestampUs};
use nve_backend_headless::HeadlessBackend;
use nve_components::tables::DebugId;
use nve_engine::{StoreBackend, Viewport};
use nve_store::NodeId;
use nve_interop::crdt::{
    CrdtRecord, CrdtValue, HlcTimestamp, NveCrdtDelta, OpId, PeerId, VersionVector,
};
use nve_studio_workspace::{
    create_surface_for_viewport, ChannelKind, CrdtPeerNode, CrdtPeerTransport, CrdtWireFrame,
    DocBlockKind, InMemoryMeshHub, LoopbackTcpCrdtCluster, LoopbackTcpCrdtTransport,
    MeshFaultConfig, Priority, StudioCrdtDoc, TaskStatus, WorkspaceSnapshot, WorkspaceState,
};

#[test]
fn loopback_tcp_three_peer_live_sync_partition_reconnect_anti_entropy_convergence() {
    let base = WorkspaceSnapshot::default();
    let seed_task_1 = base.tasks[0].id;
    let seed_task_2 = base.tasks[1].id;
    let seed_eng_msg = base.eng_messages[0].id;
    let seed_eng_reactions = base.eng_messages[0].reactions;
    let seed_doc_id = base.docs[0].id;
    let seed_block_0 = base.docs[0].blocks[0].id;
    let checklist_block = base.docs[0]
        .blocks
        .iter()
        .find(|b| b.kind.is_checklist())
        .expect("seeded checklist block")
        .id;
    let initial_checked = base.docs[0]
        .blocks
        .iter()
        .find(|b| b.id == checklist_block)
        .unwrap()
        .kind
        .is_checked();

    let docs = vec![
        StudioCrdtDoc::from_snapshot(PeerId(1), 50_000, &base),
        StudioCrdtDoc::from_snapshot(PeerId(2), 50_000, &base),
        StudioCrdtDoc::from_snapshot(PeerId(3), 50_000, &base),
    ];

    let mut cluster =
        LoopbackTcpCrdtCluster::spawn_full_mesh(docs, "studio-crdt-loopback-secret")
            .expect("spawn 3-peer loopback TCP CRDT cluster");

    // Verify real distinct 127.0.0.1 ephemeral sockets and full-mesh connectivity.
    let mut ports = BTreeSet::new();
    for node in cluster.nodes() {
        let addr = node.transport().local_addr();
        assert!(addr.ip().is_loopback(), "expected loopback addr, got {addr}");
        assert!(ports.insert(addr.port()), "duplicate TCP port {addr}");
        assert_eq!(node.transport().connected_peer_count(), 2);
        assert!(!node.transport().is_partitioned());
    }

    // Phase 1: Live concurrent edits across all 3 connected TCP peers.
    let t_p1 = cluster.node_mut(0).doc_mut().add_task(
        50_100,
        "TCP-101",
        "Live TCP NVC1 delta exchange",
        "@maya",
        "Net",
        Priority::P0,
        TaskStatus::InProgress,
    );
    let m_p2 = cluster.node_mut(1).doc_mut().post_chat_message(
        50_110,
        ChannelKind::Engineering,
        "liam",
        "#0c4a6e",
        "#7dd3fc",
        "11:01",
        "Synced over 127.0.0.1 NVT1/NVC1!",
    );
    let b_p3 = cluster
        .node_mut(2)
        .doc_mut()
        .insert_doc_block_after(
            50_120,
            seed_doc_id,
            Some(seed_block_0),
            DocBlockKind::Callout,
            "Phase 1 live loopback block from Peer 3",
        )
        .expect("p3 block insert");

    let frames_p1 = cluster
        .sync_active_until_converged(Duration::from_secs(3))
        .expect("phase 1 3-peer TCP convergence");
    assert!(frames_p1 > 0, "expected real TCP frames exchanged");

    let snaps_p1: Vec<WorkspaceSnapshot> = cluster
        .nodes()
        .iter()
        .map(|n| n.doc().project_snapshot(&base))
        .collect();
    assert_eq!(snaps_p1[0], snaps_p1[1]);
    assert_eq!(snaps_p1[1], snaps_p1[2]);
    assert_eq!(
        cluster.node(0).doc().version_vector(),
        cluster.node(2).doc().version_vector()
    );
    assert!(snaps_p1[0].tasks.iter().any(|t| t.id == t_p1));
    assert!(snaps_p1[0].eng_messages.iter().any(|m| m.id == m_p2));
    assert!(snaps_p1[0].docs[0].blocks.iter().any(|b| b.id == b_p3));

    // Phase 2: Disconnect / partition Peer 3 over real TCP sockets!
    cluster.partition_peer(2);
    assert!(cluster.node(2).transport().is_partitioned());
    assert_eq!(cluster.node(2).transport().connected_peer_count(), 0);
    assert_eq!(cluster.node(0).transport().connected_peer_count(), 1);
    assert_eq!(cluster.node(1).transport().connected_peer_count(), 1);

    // Divergent edits on online partition {Peer 1, Peer 2}:
    assert!(cluster
        .node_mut(0)
        .doc_mut()
        .edit_task_title(51_000, seed_task_1, "Partition-AB earlier title"));
    assert!(cluster
        .node_mut(0)
        .doc_mut()
        .edit_task_status(51_010, seed_task_1, TaskStatus::Done));
    assert!(cluster.node_mut(0).doc_mut().react_chat_message(
        51_020,
        ChannelKind::Engineering,
        seed_eng_msg,
        2
    ));
    let blk_p1_split = cluster
        .node_mut(0)
        .doc_mut()
        .insert_doc_block_after(
            51_030,
            seed_doc_id,
            Some(seed_block_0),
            DocBlockKind::Paragraph,
            "Inserted by Peer 1 during partition",
        )
        .expect("p1 split block");

    assert!(cluster
        .node_mut(1)
        .doc_mut()
        .delete_task(51_040, seed_task_2));
    assert!(cluster.node_mut(1).doc_mut().edit_doc_block_text(
        51_050,
        seed_doc_id,
        checklist_block,
        "Checklist edited by Peer 2 while split"
    ));
    assert!(cluster.node_mut(1).doc_mut().react_chat_message(
        51_060,
        ChannelKind::Engineering,
        seed_eng_msg,
        3
    ));

    // Peers 1 and 2 converge with each other while Peer 3 stays isolated.
    cluster
        .sync_active_until_converged(Duration::from_secs(3))
        .expect("online peers 1 & 2 converge during partition");

    // Divergent edits on isolated Peer 3 while offline:
    assert!(cluster.node_mut(2).doc_mut().edit_task_title(
        52_000,
        seed_task_1,
        "Peer 3 later winning title after reconnect"
    ));
    assert!(cluster
        .node_mut(2)
        .doc_mut()
        .edit_task_priority(52_010, seed_task_1, Priority::P0));
    let toggled = cluster
        .node_mut(2)
        .doc_mut()
        .toggle_doc_block_checklist(52_020, seed_doc_id, checklist_block)
        .expect("p3 toggle checklist");
    assert_eq!(toggled, !initial_checked);
    let blk_p3_split = cluster
        .node_mut(2)
        .doc_mut()
        .insert_doc_block_after(
            52_030,
            seed_doc_id,
            Some(seed_block_0),
            DocBlockKind::Code,
            "let offline_peer = 3;",
        )
        .expect("p3 split block");
    let doc_p3 = cluster
        .node_mut(2)
        .doc_mut()
        .add_doc(52_040, "offline-rfc", "Offline Partition RFC", "11:05");
    let doc_p3_blk = cluster
        .node_mut(2)
        .doc_mut()
        .add_doc_block(
            52_050,
            doc_p3,
            DocBlockKind::Heading1,
            "Anti-Entropy Vector-Clock Catch-Up",
        )
        .expect("p3 doc block");
    assert!(cluster.node_mut(2).doc_mut().react_chat_message(
        52_060,
        ChannelKind::Engineering,
        seed_eng_msg,
        4
    ));

    // Confirm Peer 3 diverged from Peers 1 & 2 prior to heal.
    assert_eq!(
        cluster.node(0).doc().project_snapshot(&base),
        cluster.node(1).doc().project_snapshot(&base)
    );
    assert_ne!(
        cluster.node(0).doc().project_snapshot(&base),
        cluster.node(2).doc().project_snapshot(&base)
    );

    // Phase 3: Reconnect Peer 3 over real TCP -> automatic NVCD vector-clock anti-entropy catch-up!
    cluster
        .heal_peer(2)
        .expect("heal Peer 3 TCP sockets and broadcast digests");
    cluster
        .sync_active_until_converged(Duration::from_secs(4))
        .expect("post-heal 3-peer bit-for-bit convergence");

    let final_snaps: Vec<WorkspaceSnapshot> = cluster
        .nodes()
        .iter()
        .map(|n| n.doc().project_snapshot(&base))
        .collect();
    assert_eq!(final_snaps[0], final_snaps[1]);
    assert_eq!(final_snaps[1], final_snaps[2]);
    assert_eq!(
        cluster.node(0).doc().version_vector(),
        cluster.node(1).doc().version_vector()
    );
    assert_eq!(
        cluster.node(1).doc().version_vector(),
        cluster.node(2).doc().version_vector()
    );

    // Verify exact merged semantics across all CRDT types:
    let snap = &final_snaps[0];
    let t1 = snap.tasks.iter().find(|t| t.id == seed_task_1).unwrap();
    assert_eq!(t1.title, "Peer 3 later winning title after reconnect");
    assert_eq!(t1.status, TaskStatus::Done);
    assert_eq!(t1.priority, Priority::P0);
    assert!(snap.tasks.iter().all(|t| t.id != seed_task_2));

    let eng0 = snap
        .eng_messages
        .iter()
        .find(|m| m.id == seed_eng_msg)
        .unwrap();
    assert_eq!(eng0.reactions, seed_eng_reactions + 2 + 3 + 4);

    let doc0 = snap.docs.iter().find(|d| d.id == seed_doc_id).unwrap();
    assert!(doc0.blocks.iter().any(|b| b.id == blk_p1_split));
    assert!(doc0.blocks.iter().any(|b| b.id == blk_p3_split));
    let chk = doc0.blocks.iter().find(|b| b.id == checklist_block).unwrap();
    assert_eq!(chk.text, "Checklist edited by Peer 2 while split");
    assert_eq!(chk.kind.is_checked(), !initial_checked);

    let new_doc = snap.docs.iter().find(|d| d.id == doc_p3).unwrap();
    assert_eq!(new_doc.title, "Offline Partition RFC");
    assert_eq!(new_doc.blocks.len(), 1);
    assert_eq!(new_doc.blocks[0].id, doc_p3_blk);

    // Verify real NveWireServerMetrics and CRDT sync metrics recorded non-zero QUIC wire traffic.
    for node in cluster.nodes() {
        let cm = node.metrics();
        assert_eq!(cm.connected_peers, 2);
        assert_eq!(cm.corrupt_frames_rejected, 0);
        assert!(cm.deltas_sent > 0);
        assert!(cm.deltas_received > 0);
        assert!(cm.bytes_sent > 0);
        assert!(cm.bytes_received > 0);
        let pm = node.transport().server_wire_metrics();
        assert!(pm.wire_bytes_tx > 0);
        assert!(pm.frames_sent > 0);
    }
}

#[test]
fn loopback_tcp_rejects_corrupt_and_truncated_nvc1_frames_without_corrupting_state() {
    let base = WorkspaceSnapshot::default();
    let token = "studio-corrupt-wire-test";

    let mut sender_t = LoopbackTcpCrdtTransport::bind(PeerId(1), token).expect("bind sender");
    let mut recv_t = LoopbackTcpCrdtTransport::bind(PeerId(2), token).expect("bind receiver");

    recv_t
        .connect_peer_pinned(PeerId(1), sender_t.local_addr(), sender_t.cert_sha256())
        .expect("connect receiver -> sender");
    sender_t
        .connect_peer_pinned(PeerId(2), recv_t.local_addr(), recv_t.cert_sha256())
        .expect("connect sender -> receiver");
    assert!(sender_t.wait_for_inbound_peers(1, Duration::from_secs(2)));
    assert!(recv_t.wait_for_inbound_peers(1, Duration::from_secs(2)));

    let mut sender = CrdtPeerNode::new(
        StudioCrdtDoc::from_snapshot(PeerId(1), 10_000, &base),
        sender_t,
    );
    let mut receiver = CrdtPeerNode::new(
        StudioCrdtDoc::from_snapshot(PeerId(2), 10_000, &base),
        recv_t,
    );

    let snapshot_before = receiver.doc().project_snapshot(&base);
    let vv_before = receiver.doc().version_vector().clone();

    // Prepare one valid delta to derive realistic corrupt/truncated mutations.
    let valid_task_id = sender.doc_mut().add_task(
        10_100,
        "SAFE-1",
        "Survives corrupt wire bombardment",
        "@sara",
        "Sec",
        Priority::P1,
        TaskStatus::Todo,
    );
    let valid_delta = sender.doc().delta_since(receiver.doc().version_vector());

    // 1. Truncated NVC1 header
    let truncated = valid_delta[..12].to_vec();
    // 2. Bit-flipped NVC1 checksum / body
    let mut flipped = valid_delta.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x55;
    // 3. Invalid 8-byte magic
    let mut bad_magic = valid_delta.clone();
    bad_magic[0..8].copy_from_slice(b"BADMAGIC");
    // 4. Corrupt digest checksum
    let mut bad_digest =
        CrdtWireFrame::encode_digest(PeerId(1), sender.doc().version_vector()).unwrap();
    let last = bad_digest.len() - 1;
    bad_digest[last] ^= 0xFF;
    // 5. Valid NveCrdtDelta wire checksum, invalid Studio Priority discriminant (99)
    let bad_domain = NveCrdtDelta::new(
        PeerId(1),
        VersionVector::new(),
        vec![
            CrdtRecord::OrSetAddU64 {
                namespace: 1,
                key: 9999,
                op_id: OpId::new(PeerId(1), 500),
            },
            CrdtRecord::LwwSet {
                namespace: 2,
                entity_id: 9999,
                field_id: 4,
                op_id: OpId::new(PeerId(1), 501),
                timestamp: HlcTimestamp::new(15_000, 0, 1),
                value: CrdtValue::U32(99),
            },
        ],
    )
    .encode()
    .unwrap();
    // 6. Plain JSON payload must be strictly rejected on the binary wire.
    let json_junk = br#"{"kind":"delta","tasks":[]}"#.to_vec();

    let corrupt_frames = [
        truncated,
        flipped,
        bad_magic,
        bad_digest,
        bad_domain,
        json_junk,
    ];
    for frame in &corrupt_frames {
        let sent = sender
            .transport_mut()
            .send_raw_wire_frame(frame)
            .expect("send corrupt frame over TCP");
        assert_eq!(sent, 1);
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    while receiver.metrics().corrupt_frames_rejected < corrupt_frames.len() as u64
        && Instant::now() < deadline
    {
        let _ = receiver.pump_incoming();
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(
        receiver.metrics().corrupt_frames_rejected,
        corrupt_frames.len() as u64,
        "every corrupt/truncated/JSON wire frame must be rejected cleanly"
    );
    assert_eq!(receiver.metrics().deltas_received, 0);
    assert_eq!(receiver.doc().project_snapshot(&base), snapshot_before);
    assert_eq!(receiver.doc().version_vector(), &vv_before);

    // Subsequent valid NVC1 delta over the EXACT same TCP connection still succeeds!
    sender
        .transport_mut()
        .send_raw_wire_frame(&valid_delta)
        .expect("send valid NVC1 delta after corrupt burst");

    let deadline = Instant::now() + Duration::from_secs(3);
    while receiver.metrics().deltas_received == 0 && Instant::now() < deadline {
        let _ = receiver.pump_incoming();
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(receiver.metrics().deltas_received, 1);
    let after = receiver.doc().project_snapshot(&base);
    assert!(
        after.tasks.iter().any(|t| t.id == valid_task_id),
        "valid NVC1 delta must still merge cleanly after corrupt wire frames"
    );
}

#[test]
fn in_memory_mesh_handles_packet_drops_reordering_and_partition_heal() {
    let base = WorkspaceSnapshot::default();
    let hub = InMemoryMeshHub::new();

    let mut nodes: Vec<CrdtPeerNode<_>> = (1..=3)
        .map(|id| {
            let doc = StudioCrdtDoc::from_snapshot(PeerId(id), 30_000, &base);
            let tr = hub.register_peer(PeerId(id));
            CrdtPeerNode::new(doc, tr)
        })
        .collect();

    // Enable deterministic packet drops (every 2nd frame) + reverse-order delivery + split partition.
    hub.set_fault_config(MeshFaultConfig {
        drop_every_n: Some(2),
        reorder_reverse: true,
    });
    hub.set_partition_groups(&[PeerId(1)], &[PeerId(2), PeerId(3)]);

    let t1 = nodes[0].doc_mut().add_task(
        30_100,
        "MESH-1",
        "Dropped & reordered task",
        "@maya",
        "Mesh",
        Priority::P0,
        TaskStatus::Todo,
    );
    let m2 = nodes[1].doc_mut().post_chat_message(
        30_120,
        ChannelKind::Design,
        "aria",
        "#451a03",
        "#fcd34d",
        "12:00",
        "Survives packet drop & LIFO reorder",
    );
    let d3 = nodes[2]
        .doc_mut()
        .add_doc(30_130, "mesh-spec", "Mesh Fault Spec", "12:01");
    nodes[2]
        .doc_mut()
        .add_doc_block(30_140, d3, DocBlockKind::Paragraph, "Deterministic convergence.");

    // Broadcast during partition + loss -> frames get dropped across partition boundary.
    for node in &mut nodes {
        let _ = node.broadcast_delta();
    }
    for node in &mut nodes {
        let _ = node.pump_incoming();
    }
    assert!(hub.dropped_frames() > 0, "expected dropped frames under fault config");

    // Inject corrupt frame into Peer 2 inbox to verify atomic rejection on mesh transport too.
    hub.inject_raw_frame(PeerId(1), PeerId(2), b"NVC1_CORRUPT".to_vec());
    let corrupt_rep = nodes[1].pump_incoming();
    assert_eq!(corrupt_rep.corrupt_frames_rejected, 1);

    // Heal partition, keep reverse reordering ON, disable drop so anti-entropy completes cleanly.
    hub.heal_all();
    hub.set_fault_config(MeshFaultConfig {
        drop_every_n: None,
        reorder_reverse: true,
    });

    for _ in 0..4 {
        for node in &mut nodes {
            let _ = node.broadcast_digest();
        }
        for node in &mut nodes {
            let _ = node.pump_incoming();
        }
    }

    let s0 = nodes[0].doc().project_snapshot(&base);
    let s1 = nodes[1].doc().project_snapshot(&base);
    let s2 = nodes[2].doc().project_snapshot(&base);
    assert_eq!(s0, s1);
    assert_eq!(s1, s2);
    assert!(s0.tasks.iter().any(|t| t.id == t1));
    assert!(s0.design_messages.iter().any(|m| m.id == m2));
    assert!(s0.docs.iter().any(|d| d.id == d3));
}

#[test]
fn studio_ui_crdt_sync_panel_interactive_sync_partition_heal_and_nws1_roundtrip() {
    let mut surface = create_surface_for_viewport(SurfaceId::ROOT, 960.0, 640.0);
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 960.0,
            height: 640.0,
            scale_factor: 1.0,
        },
    );

    let mut fid = 1u64;
    let render = |surface: &mut nve_dioxus::DioxusSurface,
                  headless: &mut HeadlessBackend,
                  fid: &mut u64| {
        surface
            .commit_to(FrameId(*fid), headless)
            .expect("commit studio frame");
        *fid += 1;
    };
    let press = |surface: &mut nve_dioxus::DioxusSurface,
                 headless: &mut HeadlessBackend,
                 fid: &mut u64,
                 tid: &str| {
        let store = surface.store();
        let mut stack = vec![NodeId::ROOT];
        let mut found = None;
        while let Some(id) = stack.pop() {
            if id != NodeId::ROOT
                && let Some(dbg) = store.try_row::<DebugId>(id)
                && dbg.test_id() == tid
            {
                found = Some(id);
                break;
            }
            stack.extend_from_slice(store.children(id));
        }
        let nid = found.unwrap_or_else(|| {
            panic!("missing testID='{tid}'\n{}", headless.render_ascii_tree(store))
        });
        let rect = headless.layout_of(nid).unwrap();
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        let hit = headless
            .hit_test(store, cx, cy)
            .unwrap_or_else(|| panic!("hit_test missed '{tid}' at ({cx}, {cy}) rect={rect:?}"));
        assert_eq!(hit, nid, "hit_test hit wrong node for '{tid}'");
        let mut batch = EventBatchBuf::new();
        batch.push_press(SurfaceId::ROOT, hit, TimestampUs(*fid * 16_000), cx, cy);
        assert!(surface.dispatch_events(batch.as_validated()));
        surface
            .commit_to(FrameId(*fid), headless)
            .expect("commit after press");
        *fid += 1;
    };

    render(&mut surface, &mut headless, &mut fid);
    let init_tree = headless.render_ascii_tree(surface.store());
    assert!(init_tree.contains("NVC1 0/0d | 0B"));
    assert!(init_tree.contains("MESH"));
    assert!(init_tree.contains("Sync Now"));
    assert!(init_tree.contains("Partition"));

    // 1. Trigger live binary NVC1 Sync Now directly from Studio panel!
    press(&mut surface, &mut headless, &mut fid, "crdt-sync-now");
    let after_sync = headless.render_ascii_tree(surface.store());
    assert!(
        after_sync.contains("NVC1 1/1d |") && !after_sync.contains("| 0B"),
        "expected non-zero NVC1 delta & byte metrics after Sync Now:\n{after_sync}"
    );

    // 2. Simulate network partition -> peers drop to 0, badge flips to SPLIT, button flips to Heal Mesh.
    press(&mut surface, &mut headless, &mut fid, "crdt-toggle-partition");
    let split_tree = headless.render_ascii_tree(surface.store());
    assert!(split_tree.contains("SPLIT"));
    assert!(split_tree.contains("0 peers"));
    assert!(split_tree.contains("Heal Mesh"));

    // Pressing Sync Now while partitioned delivers 0 new remote deltas.
    press(&mut surface, &mut headless, &mut fid, "crdt-sync-now");
    assert!(headless.render_ascii_tree(surface.store()).contains("NVC1 1/1d |"));

    // 3. Heal Mesh -> reconnects peers, runs real NVC1 anti-entropy merge, updates counters & reactions!
    press(&mut surface, &mut headless, &mut fid, "crdt-toggle-partition");
    let healed_tree = headless.render_ascii_tree(surface.store());
    assert!(healed_tree.contains("MESH"));
    assert!(healed_tree.contains("2 peers"));
    assert!(healed_tree.contains("NVC1 2/2d |"));
    assert!(healed_tree.contains("CRDT Sync ("));

    // 4. Verify NWS1 v2 binary codec preserves CRDT sync telemetry fields identically.
    let mut state = WorkspaceState::default();
    state.sync_crdt_now();
    state.toggle_crdt_partition();
    let encoded = state.to_bytes();
    let decoded = WorkspaceState::from_bytes(&encoded).expect("decode NWS1 v2 with CRDT telemetry");
    assert_eq!(decoded, state);
    assert!(decoded.sync_telemetry.crdt_partitioned);
    assert_eq!(decoded.sync_telemetry.crdt_deltas_sent, 1);
    assert!(decoded.sync_telemetry.crdt_bytes_synced > 0);
}
