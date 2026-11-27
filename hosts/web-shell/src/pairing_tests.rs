//! End-to-end tests for `PairClientMachine` and `PairedWebSession` over real `nve-wire`
//! `NveWireServer` (WebSocket fallback + `NveWireFrame` v2 `CommandDelta` + `EventBatch`).

use std::time::Duration;

use nve_abi::{
    decode_events, EventPayload, NativeEvent, NativeEventKind, NodeId, NVE_KEY_MOD_META,
};
use nve_backend_web::WebBackend;
use nve_engine::Viewport;
use nve_wire::{
    NveWireFrame, NveWireKind, NveWireRejectReason, NveWireServer, NveWireServerConfig,
    NVE_WIRE_MAGIC,
};
use tungstenite::Message;

use super::*;

fn find_counter_increment_button(store: &nve_store::NodeStore) -> NodeId {
    store
        .live_nodes()
        .find_map(|id| {
            store
                .try_row::<nve_components::tables::DebugId>(id)
                .filter(|d| d.test_id() == "ks-counter-inc")
                .map(|_| id.raw())
        })
        .expect("ks-counter-inc button in remote mirror")
}

#[test]
fn web_pair_client_full_wire_v2_ws_handshake_delta_dom_render_and_event_batch() {
    let token = "nve_web_pair_e2e_token";
    let server = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .expect("bind NveWireServer");

    let mut kitchen_sink = nve_example_kitchen_sink::create_surface(1);
    let frame1 = kitchen_sink.render_nve2_slice(1).to_vec();
    server
        .broadcast_scene_keyframe(1, &frame1)
        .expect("seed initial keyframe");

    let viewport = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.0,
    };
    let mut web_session = PairedWebSession::new(1, WebBackend::new(), viewport, token);

    let ws_url = format!("ws://127.0.0.1:{}/nve-wire", server.wss_addr().port());
    let (mut ws, _) = tungstenite::connect(&ws_url).expect("connect WS");

    let hello = web_session.build_hello();
    assert_eq!(&hello[0..2], &NVE_WIRE_MAGIC);
    ws.send(Message::Binary(hello.into())).unwrap();

    while web_session.frames_received() == 0 {
        let Message::Binary(msg) = ws.read().expect("read ws binary") else {
            continue;
        };
        let reply = web_session
            .on_pair_binary_message(&msg)
            .expect("valid NveWireFrame server message");
        if !reply.is_empty() {
            ws.send(Message::Binary(reply.into())).unwrap();
        }
        while let Some(extra) = web_session.take_outbound_message() {
            ws.send(Message::Binary(extra.into())).unwrap();
        }
    }

    assert!(web_session.is_connected());
    assert_ne!(web_session.session_id(), 0);
    assert_eq!(web_session.frames_received(), 1);
    assert!(web_session.render_html().contains("Count: 0"));
    #[cfg(feature = "scene-view")]
    assert!(web_session.scene().view().node_count > 0);

    // Server receives initial Layout EventBatch automatically on handshake.
    let layout_msg = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server received initial Layout EventBatch");
    assert_eq!(layout_msg.frame.kind, NveWireKind::EventBatch);
    let layout_evs = decode_events(&layout_msg.frame.payload).expect("valid Layout EventRing");
    assert_eq!(layout_evs[0].kind, NativeEventKind::Layout);

    // Local DOM click -> EventBatch -> server mutates KitchenSinkApp -> CommandDelta -> Count: 1.
    let btn_id = find_counter_increment_button(web_session.store());
    web_session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: btn_id,
        timestamp_us: 12_345,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 24.0, y: 18.0 },
    });
    let click_frame = web_session
        .drain_dom_events_as_wire_frame()
        .expect("encode click EventBatch")
        .expect("non-empty click frame");
    assert_eq!(&click_frame[0..2], &NVE_WIRE_MAGIC);
    assert_eq!(click_frame[4], NveWireKind::EventBatch as u8);
    ws.send(Message::Binary(click_frame.into())).unwrap();

    let click_in = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server received click EventBatch");
    let click_evs = decode_events(&click_in.frame.payload).expect("valid click EventRing");
    assert!(kitchen_sink.dispatch_events(&click_evs));
    let frame2 = kitchen_sink.render_nve2_slice(2).to_vec();
    assert_eq!(server.broadcast_command_delta(2, &frame2).unwrap(), 1);

    while web_session.frames_received() < 2 {
        let Message::Binary(msg) = ws.read().expect("read frame2") else {
            continue;
        };
        let reply = web_session
            .on_pair_binary_message(&msg)
            .expect("valid delta message");
        if !reply.is_empty() {
            ws.send(Message::Binary(reply.into())).unwrap();
        }
    }
    assert!(web_session.render_html().contains("Count: 1"));

    // Synthetic keydown -> EventBatch -> decoded by server.
    let kd_frame = web_session
        .encode_keydown("k", true, false)
        .expect("encode keydown")
        .expect("non-empty keydown packet");
    ws.send(Message::Binary(kd_frame.into())).unwrap();
    let kd_in = server
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server received keydown");
    let kd_evs = decode_events(&kd_in.frame.payload).expect("decode keydown EventRing");
    assert_eq!(kd_evs[0].kind, NativeEventKind::KeyDown);
    match &kd_evs[0].payload {
        EventPayload::Key { key, modifiers, .. } => {
            assert_eq!(key, "k");
            assert_ne!(*modifiers & NVE_KEY_MOD_META, 0);
        }
        other => panic!("expected Key payload, got {other:?}"),
    }

    if let Some(bye) = web_session.build_bye() {
        let _ = ws.send(Message::Binary(bye.into()));
    }
    assert!(!web_session.is_connected());
    server.shutdown();
}

#[test]
fn pair_client_machine_rejects_bad_frames_and_unauthorized_reject() {
    let mut client = PairClientMachine::new("secret-token");
    assert!(!client.is_connected());
    assert!(client
        .encode_input_events(&[NativeEvent {
            surface_id: 1,
            node_id: 1,
            timestamp_us: 0,
            kind: NativeEventKind::Focus,
            payload: EventPayload::Empty,
        }])
        .is_err());

    // Truncated / bad magic / bad version frames are rejected.
    assert!(client.on_binary_message(b"NW").is_err());
    assert!(client.on_binary_message(&[0u8; 16]).is_err());

    let _ = client.build_hello();
    let reject = NveWireFrame::new(
        NveWireKind::Reject,
        1,
        NveWireRejectReason::Unauthorized.encode("invalid bearer token"),
    )
    .encode()
    .unwrap();
    assert!(client.on_binary_message(&reject).is_err());
    assert!(matches!(client.state(), PairClientState::Rejected(_)));
}
