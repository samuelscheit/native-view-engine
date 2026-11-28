//! Corrupt-wire resync and inbound overflow recovery tests for embedded `nve-wire` pairing.

use std::time::{Duration, Instant};

use nve_engine::Viewport;
use nve_runtime::RuntimeBuilder;
use nve_wire::{NveWireFrame, NveWireKind, NveWireServer, NveWireServerConfig};

use super::{encode_scene, read_text_slab, wait_until, StatefulSceneServer};

/// Builds a corrupt mid-frame `NVE2` packet whose 24-byte header and op 1 (`CreateNodes`) are
/// valid (mutating the open `Transaction`), while op 2 is truncated/malformed so `apply()` fails
/// mid-transaction.
fn build_mid_frame_corrupt_nve2(frame_num: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let push_u32 = |buf: &mut Vec<u8>, v: u32| buf.extend_from_slice(&v.to_le_bytes());
    // 24-byte NVE2 header placeholder (6 words).
    for _ in 0..6 {
        push_u32(&mut out, 0);
    }
    // Op 1: CreateNodes (op=3, table=0, count=1, payload_words=1) -> creates node 99.
    push_u32(&mut out, 3u32 | (1u32 << 16));
    push_u32(&mut out, 1);
    push_u32(&mut out, 99);
    // Op 2: Unknown/corrupt opcode 0xFE with payload_words=1 -> fails mid-frame!
    push_u32(&mut out, 0xFEu32 | (1u32 << 16));
    push_u32(&mut out, 1);
    push_u32(&mut out, 0xDEAD_BEEF);

    let total_words = (out.len() / 4) as u32;
    out[0..4].copy_from_slice(&nve_store::wire::MAGIC.to_le_bytes()); // "NVE2"
    out[4..8].copy_from_slice(&u32::from(nve_store::wire::VERSION).to_le_bytes());
    out[8..12].copy_from_slice(&frame_num.to_le_bytes());
    out[12..16].copy_from_slice(&0u32.to_le_bytes());
    out[16..20].copy_from_slice(&2u32.to_le_bytes()); // 2 ops
    out[20..24].copy_from_slice(&total_words.to_le_bytes());
    out
}

#[test]
fn corrupt_nve2_mid_frame_and_multi_frame_batch_recovers_with_rate_limited_keyframe_request() {
    let token = "corrupt-recovery-tok";
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .expect("bind server");
    let (mut scene_srv, frame1) = StatefulSceneServer::new("HEALTHY_V1");
    let _ = server.broadcast_scene_keyframe(1, &frame1);

    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .build();
    let uri = format!("ws://{}/nve-wire", server.wss_addr());
    rt.pair_connect(&uri, token).expect("pair_connect");
    assert!(wait_until(Duration::from_secs(3), || rt.pair_is_connected()));
    assert!(wait_until(Duration::from_secs(3), || rt.tick(0.016).unwrap()));
    {
        let g = rt.acquire_scene().unwrap();
        assert!(read_text_slab(g.view()).contains("HEALTHY_V1"));
    }

    // 1. Mid-frame corrupt delta: `tick()` MUST NOT error, resets cleanly, and sends ONE KeyframeRequest.
    let bad_f2 = build_mid_frame_corrupt_nve2(2);
    assert_eq!(server.broadcast_command_delta(2, &bad_f2).unwrap(), 1);
    assert!(wait_until(Duration::from_secs(2), || {
        rt.tick(0.016).expect("tick must not abort on corrupt remote wire")
    }));
    let req1 = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server receives single KeyframeRequest");
    assert_eq!(req1.frame.kind, NveWireKind::KeyframeRequest);
    // Because `broadcast_command_delta` invalidated the server's stale Frame-1 keyframe, the
    // producer explicitly replies to `req1` with a fresh targeted snapshot (no stale rollback!).
    assert!(server
        .send_scene_keyframe_to_peer(req1.session_id, 3, &frame1)
        .unwrap());
    assert!(wait_until(Duration::from_secs(2), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("HEALTHY_V1")
    }));
    rt.pair_disconnect().unwrap();
    server.shutdown();

    // 2. Server WITHOUT a cached SceneKeyframe: verifies rate-limited single KeyframeRequest,
    // dropping deltas while `waiting_for_keyframe`, AND multi-frame batch (valid F2 + corrupt F3)
    // in one `tick()`!
    let server_nocache = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .expect("bind server_nocache");
    let uri2 = format!("ws://{}/nve-wire", server_nocache.wss_addr());
    rt.pair_connect(&uri2, token).expect("pair_connect 2");
    assert!(wait_until(Duration::from_secs(3), || rt.pair_is_connected()));

    // Send Frame 1 as CommandDelta (does NOT populate server `latest_keyframe`).
    let (mut scene_srv2, f1) = StatefulSceneServer::new("BATCH_V1");
    assert_eq!(server_nocache.broadcast_command_delta(1, &f1).unwrap(), 1);
    assert!(wait_until(Duration::from_secs(2), || rt.tick(0.016).unwrap()));
    {
        let g = rt.acquire_scene().unwrap();
        assert!(read_text_slab(g.view()).contains("BATCH_V1"));
    }

    // Queue BOTH valid Frame 2 AND mid-frame corrupt Frame 3 BEFORE a single `tick()`!
    let rx_before_batch = rt.metrics().unwrap().wire_bytes_rx;
    let f2_valid = scene_srv2.update_label_delta("BATCH_V2_BEFORE_CORRUPT");
    let f3_corrupt = build_mid_frame_corrupt_nve2(3);
    assert_eq!(server_nocache.broadcast_command_delta(2, &f2_valid).unwrap(), 1);
    assert_eq!(server_nocache.broadcast_command_delta(3, &f3_corrupt).unwrap(), 1);
    // Wait deterministically until live transport metrics reflect BOTH frames already enqueued.
    let expected_batch_bytes = (f2_valid.len() + f3_corrupt.len()) as u64;
    assert!(wait_until(Duration::from_secs(2), || {
        rt.metrics().unwrap().wire_bytes_rx >= rx_before_batch + expected_batch_bytes
    }));
    assert!(rt.tick(0.016).expect("multi-frame batch tick must recover, not error"));
    {
        let g = rt.acquire_scene().unwrap();
        assert!(
            g.view().node_capacity <= 1 && read_text_slab(g.view()).is_empty(),
            "corrupt batch must reset scene cleanly (cap={}, text={:?})",
            g.view().node_capacity,
            read_text_slab(g.view()),
        );
        assert_eq!(rt.metrics().unwrap().live_nodes, 0);
    }

    let req_batch = server_nocache
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server receives KeyframeRequest after corrupt batch");
    assert_eq!(req_batch.frame.kind, NveWireKind::KeyframeRequest);

    // While `waiting_for_keyframe`: stale delta AND even a corrupt keyframe MUST NOT emit another
    // KeyframeRequest (strictly rate-limited per session/epoch).
    let rx_before_stale = rt.metrics().unwrap().wire_bytes_rx;
    let f4_stale = scene_srv2.update_label_delta("STALE_DELTA_IGNORED");
    assert_eq!(server_nocache.broadcast_command_delta(4, &f4_stale).unwrap(), 1);
    assert_eq!(server_nocache.broadcast_scene_keyframe(5, b"NOT_NVE2").unwrap(), 1);
    let expected_stale_bytes = (f4_stale.len() + b"NOT_NVE2".len()) as u64;
    assert!(wait_until(Duration::from_secs(2), || {
        rt.metrics().unwrap().wire_bytes_rx >= rx_before_stale + expected_stale_bytes
    }));
    let _ = rt.tick(0.016).expect("still no abort on broken keyframe");
    assert!(
        server_nocache
            .recv_inbound_timeout(Duration::from_millis(150))
            .is_none(),
        "must not spam duplicate KeyframeRequests in same resync epoch"
    );

    // Send genuine recovery SceneKeyframe -> scene recovers cleanly!
    let recovery = encode_scene("RECOVERED_AFTER_BATCH");
    assert_eq!(server_nocache.broadcast_scene_keyframe(6, &recovery).unwrap(), 1);
    assert!(wait_until(Duration::from_secs(2), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("RECOVERED_AFTER_BATCH")
    }));
    // Keep `scene_srv` used cleanly.
    let _ = scene_srv.update_label_delta("unused");

    rt.pair_disconnect().unwrap();
    server_nocache.shutdown();
}

#[test]
fn pair_inbound_overflow_recovers_with_single_keyframe_request() {
    let token = "runtime-overflow-tok";
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .expect("bind server");
    let (mut scene_srv, f1) = StatefulSceneServer::new("OVERFLOW_V1");
    let _ = server.broadcast_scene_keyframe(1, &f1);

    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        })
        .build();
    let uri = format!("ws://{}/nve-wire", server.wss_addr());
    rt.pair_connect(&uri, token).expect("pair_connect");
    assert!(wait_until(Duration::from_secs(3), || rt.pair_is_connected()));
    assert!(wait_until(Duration::from_secs(3), || rt.tick(0.016).unwrap()));

    // Flood 320 CommandDeltas without calling tick() -> triggers WireInboundItem::Overflowed!
    let rx_before = rt.metrics().unwrap().wire_bytes_rx;
    let mut total_delta_bytes = 0u64;
    for seq in 2..=321u32 {
        let delta = scene_srv.update_label_delta(&format!("OVERFLOW_DELTA_{seq}"));
        total_delta_bytes += delta.len() as u64;
        assert_eq!(server.broadcast_command_delta(seq, &delta).unwrap(), 1);
    }
    assert!(wait_until(Duration::from_secs(4), || {
        rt.metrics().unwrap().wire_bytes_rx >= rx_before + total_delta_bytes
    }));

    // Drain until the Overflowed sentinel is processed -> tick() stays Ok and emits ONE KeyframeRequest.
    let req_deadline = Instant::now() + Duration::from_secs(4);
    let req = loop {
        let _ = rt.tick(0.016).expect("overflow tick must not error");
        if let Some(m) = server.recv_inbound_timeout(Duration::from_millis(50)) {
            break m;
        }
        assert!(
            Instant::now() < req_deadline,
            "timed out waiting for overflow recovery KeyframeRequest"
        );
    };
    assert_eq!(req.frame.kind, NveWireKind::KeyframeRequest);
    assert!(
        server
            .recv_inbound_timeout(Duration::from_millis(150))
            .is_none(),
        "overflow recovery must emit at most ONE KeyframeRequest"
    );

    // Send targeted recovery keyframe -> scene recovers cleanly!
    let recovery = encode_scene("OVERFLOW_RECOVERED");
    assert!(server
        .send_scene_keyframe_to_peer(req.session_id, 400, &recovery)
        .unwrap());
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("OVERFLOW_RECOVERED")
    }));

    rt.pair_disconnect().unwrap();
    server.shutdown();
}

#[test]
fn pair_non_delta_overflow_recovers_from_single_keyframe_request_when_server_cache_present() {
    let token = "runtime-cached-overflow-tok";
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .expect("bind server");
    let kf_bytes = encode_scene("CACHED_KF_OVERFLOW_V1");
    let _ = server.broadcast_scene_keyframe(1, &kf_bytes);

    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        })
        .build();
    let uri = format!("ws://{}/nve-wire", server.wss_addr());
    rt.pair_connect(&uri, token).expect("pair_connect");
    assert!(wait_until(Duration::from_secs(3), || rt.pair_is_connected()));
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("CACHED_KF_OVERFLOW_V1")
    }));

    // Broadcast a second SceneKeyframe via `broadcast_frame` while ALREADY connected so the peer
    // received it via broadcast (previously setting `last_sent_kf_seq = seq`), then flood 320
    // non-CommandDelta `Presence` frames (keeping server `latest_keyframe == Some`) to overflow!
    let kf2_bytes = encode_scene("CACHED_KF_OVERFLOW_V2");
    assert_eq!(server.broadcast_scene_keyframe(2, &kf2_bytes).unwrap(), 1);
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("CACHED_KF_OVERFLOW_V2")
    }));

    let rx_before = rt.metrics().unwrap().wire_bytes_rx;
    for seq in 3..=330u32 {
        let presence = NveWireFrame::new(NveWireKind::Presence, seq, vec![1, 2, 3, 4]);
        assert_eq!(server.broadcast_frame(&presence).unwrap(), 1);
    }
    assert!(wait_until(Duration::from_secs(4), || {
        rt.metrics().unwrap().wire_bytes_rx >= rx_before + (328 * 20)
    }));

    // Tick processes `Overflowed`, resets replica, and sends ONE explicit `KeyframeRequest`.
    // Server MUST reply immediately from `latest_keyframe` (NEVER swallowing it!) with zero
    // producer involvement, restoring `CACHED_KF_OVERFLOW_V2`!
    let mut saw_reset = false;
    let mut saw_recovered = false;
    let deadline = Instant::now() + Duration::from_secs(4);
    while !saw_recovered && Instant::now() < deadline {
        let _ = rt.tick(0.016).expect("overflow tick must stay Ok");
        let g = rt.acquire_scene().unwrap();
        let text = read_text_slab(g.view()).to_owned();
        if text.is_empty() {
            saw_reset = true;
        } else if saw_reset && text.contains("CACHED_KF_OVERFLOW_V2") {
            saw_recovered = true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(saw_reset, "overflow must reset replica before recovery");
    assert!(
        saw_recovered,
        "server must answer single explicit recovery KeyframeRequest from cached latest_keyframe"
    );
    assert!(
        server
            .recv_inbound_timeout(Duration::from_millis(100))
            .is_none(),
        "cached server reply must not forward KeyframeRequest to producer"
    );

    rt.pair_disconnect().unwrap();
    server.shutdown();
}
