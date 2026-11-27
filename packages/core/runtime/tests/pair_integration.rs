//! End-to-end live loopback integration tests for `SurfaceRuntime` embedded `nve-wire[client]`
//! pairing over both the Rust API and the C ABI (`pair_connect`, `pair_is_connected`, `tick`,
//! `scene_acquire`, `dispatch_events`, automatic reconnect + `KeyframeRequest` + `SceneKeyframe`
//! recovery, and `pair_disconnect`).

#![cfg(all(feature = "wire", not(target_arch = "wasm32")))]

use std::ffi::c_void;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nve_abi::{
    decode_events, EventMask, NativeEventKind, NveBufferSlice, NveDirectEvent, NveEventBatch,
    NveEventBatchRef, NveRuntimeApi, NveRuntimeConfig, NveRuntimeMetrics, NveRuntimeSceneView,
    NveRuntimeStatus, NveStrSlice, NVE_RUNTIME_ABI_VERSION,
};
use nve_components::tables::{Interaction, Layout, TextContent, Visual};
use nve_engine::Viewport;
use nve_runtime::{
    AppFrame, RuntimeApp, RuntimeBuilder, RuntimeError, nve_runtime_get_interface,
};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_wire::{NveWireKind, NveWireServer, NveWireServerConfig};

fn get_api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable returned for matching ABI version.
    unsafe { *ptr }
}

struct StatefulSceneServer {
    store: NodeStore,
    enc: WireEncoder,
    btn: NodeId,
}

impl StatefulSceneServer {
    fn new(initial_label: &str) -> (Self, Vec<u8>) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let btn = {
            let mut tx = store.tx();
            let btn = tx.create();
            tx.edit::<Layout>(btn)
                .width(Length::points(180.0))
                .height(Length::points(48.0));
            tx.edit::<Visual>(btn).background(Rgba::rgb(37, 99, 235));
            tx.edit::<TextContent>(btn).text(initial_label);
            tx.edit::<Interaction>(btn)
                .event_mask(EventMask::PRESS as u32);
            tx.set_children(NodeId::ROOT, &[btn]);
            tx.commit_into(&mut changes);
            btn
        };
        let mut enc = WireEncoder::default();
        let mut out = Vec::new();
        enc.encode(&store, &changes, &mut out);
        (Self { store, enc, btn }, out)
    }

    fn update_label_delta(&mut self, next_label: &str) -> Vec<u8> {
        let mut changes = ChangeSet::default();
        {
            let mut tx = self.store.tx();
            tx.edit::<TextContent>(self.btn).text(next_label);
            tx.commit_into(&mut changes);
        }
        let mut out = Vec::new();
        self.enc.encode(&self.store, &changes, &mut out);
        out
    }
}

fn encode_scene(label: &str) -> Vec<u8> {
    let (srv, _) = StatefulSceneServer::new(label);
    let mut enc = WireEncoder::default();
    let mut out = Vec::new();
    enc.encode_full(&srv.store, &mut out);
    out
}

fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    pred()
}

fn read_text_slab(view: &NveRuntimeSceneView) -> String {
    if view.text_slab.is_null() || view.text_slab_len == 0 {
        return String::new();
    }
    // SAFETY: Caller holds an active `scene_acquire` guard over `view`.
    let bytes = unsafe { std::slice::from_raw_parts(view.text_slab, view.text_slab_len as usize) };
    std::str::from_utf8(bytes).expect("UTF-8 text slab").to_owned()
}

struct DummyLocalApp;

impl RuntimeApp for DummyLocalApp {
    fn render_frame(&mut self, _frame_id: u64) -> AppFrame<'_> {
        AppFrame::Clean
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }
}

#[test]
fn c_abi_embedded_pair_live_frames_events_reconnect_keyframe_and_disconnect() {
    let api = get_api();
    let token = "rt-pair-live-token";
    let server1 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().expect("udp addr"),
        "127.0.0.1:0".parse().expect("tcp addr"),
        token,
    ))
    .expect("bind server1");
    let wss_addr: SocketAddr = server1.wss_addr();
    let (mut scene_srv, frame1) = StatefulSceneServer::new("INITIAL_REMOTE_V1");
    let _ = server1.broadcast_scene_keyframe(1, &frame1);

    let cfg = NveRuntimeConfig {
        surface_id: 1,
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Valid stack config.
    let h = unsafe { (api.create)(&cfg) };
    assert!(!h.is_null());

    static PAIR_WAKES: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn on_pair_wake(ctx: *mut c_void) {
        let counter = ctx.cast::<AtomicUsize>();
        // SAFETY: `ctx` points to static `PAIR_WAKES`.
        unsafe { (*counter).fetch_add(1, Ordering::Release) };
    }
    PAIR_WAKES.store(0, Ordering::Release);
    // SAFETY: Valid handle and static callback/context.
    unsafe {
        assert_eq!(
            (api.set_wake_callback)(
                h,
                Some(on_pair_wake),
                (&raw const PAIR_WAKES).cast_mut().cast::<c_void>(),
            ),
            NveRuntimeStatus::Ok.as_i32()
        );
    }

    // Invalid UTF-8 & empty URI/token are rejected cleanly with InvalidArg.
    let bad_utf8 = [0xFFu8, 0xFEu8];
    let bad_slice = NveStrSlice {
        ptr: bad_utf8.as_ptr(),
        len: bad_utf8.len(),
    };
    // SAFETY: Valid handle and slices.
    unsafe {
        assert_eq!(
            (api.pair_connect)(h, bad_slice, NveStrSlice::EMPTY),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.pair_connect)(h, NveStrSlice::from_str("ws://127.0.0.1:9999/nve-wire"), NveStrSlice::EMPTY),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
    }

    // Connect via C ABI `pair_connect` using `ws://` URI + separate `token` slice.
    let uri = format!("ws://{wss_addr}/nve-wire");
    // SAFETY: Valid handle and stack UTF-8 slices.
    let rc = unsafe {
        (api.pair_connect)(h, NveStrSlice::from_str(&uri), NveStrSlice::from_str(token))
    };
    assert_eq!(rc, NveRuntimeStatus::Ok.as_i32());

    // Wait for background worker handshake and verify cross-thread `pair_is_connected`.
    assert!(
        wait_until(Duration::from_secs(3), || {
            // SAFETY: Valid live handle.
            unsafe { (api.pair_is_connected)(h) == 1 }
        }),
        "runtime pairing client did not connect to server1"
    );
    let h_addr = h as usize;
    let bg_connected = std::thread::spawn(move || {
        // SAFETY: Handle stays alive until thread joins.
        unsafe { (api.pair_is_connected)(h_addr as *mut c_void) }
    })
    .join()
    .expect("bg thread");
    assert_eq!(bg_connected, 1);

    // Wait for initial frame wake + tick -> published zero-copy view has `INITIAL_REMOTE_V1`.
    let mut changed = 0u32;
    assert!(
        wait_until(Duration::from_secs(3), || {
            // SAFETY: Owner-thread tick on live handle.
            let tick_rc = unsafe { (api.tick)(h, 0.016, &mut changed) };
            assert_eq!(tick_rc, NveRuntimeStatus::Ok.as_i32());
            changed == 1
        }),
        "initial paired frame never arrived"
    );
    assert!(PAIR_WAKES.load(Ordering::Acquire) >= 1);

    // Inspect zero-copy view, verify `submit_nve2` rejected while paired (preserving view!),
    // and check `get_metrics`.
    let mut view = NveRuntimeSceneView::default();
    let mut metrics = NveRuntimeMetrics::default();
    // SAFETY: Valid handle and stack outputs.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&frame1)),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.scene_acquire)(h, &mut view),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(read_text_slab(&view).contains("INITIAL_REMOTE_V1"));
        assert!(view.node_capacity >= 3);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());

        assert_eq!(
            (api.get_metrics)(h, &mut metrics),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(metrics.pair_connected, 1);
        assert!(metrics.wire_bytes_rx > 0);
    }

    // Dispatch upstream input event over the live paired link; local scene has not changed yet
    // (`ev_changed == 0`), while the server receives the EventBatch and pushes a delta back.
    let ev = [NveDirectEvent::press(1, 2, 123, 15.0, 15.0)];
    let mut ev_changed = 1u32;
    // SAFETY: Valid handle and stack event slice.
    let drc = unsafe { (api.dispatch_events)(h, NveEventBatch::from_slice(&ev), &mut ev_changed) };
    assert_eq!(drc, NveRuntimeStatus::Ok.as_i32());
    assert_eq!(ev_changed, 0);

    let inbound = server1
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server1 must receive upstream EventBatch");
    assert_eq!(inbound.frame.kind, NveWireKind::EventBatch);
    let decoded = decode_events(&inbound.frame.payload).expect("decode upstream events");
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].kind, NativeEventKind::Press);
    assert_eq!(decoded[0].node_id, 2);

    // Server replies with a true incremental `CommandDelta` (frame 2) mutating only TextContent.
    let frame2 = scene_srv.update_label_delta("CLICKED_REMOTE_V1");
    let _ = server1.broadcast_command_delta(2, &frame2);
    assert!(
        wait_until(Duration::from_secs(3), || {
            let mut ch = 0u32;
            // SAFETY: Owner-thread tick.
            let tick_rc = unsafe { (api.tick)(h, 0.016, &mut ch) };
            assert_eq!(tick_rc, NveRuntimeStatus::Ok.as_i32());
            ch == 1
        }),
        "updated paired frame never arrived"
    );
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!(
            (api.scene_acquire)(h, &mut view),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(read_text_slab(&view).contains("CLICKED_REMOTE_V1"));
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // Kill server1 -> client detects disconnect -> restart server2 on SAME TCP port!
    server1.shutdown();
    drop(server1);
    assert!(
        wait_until(Duration::from_secs(3), || {
            // SAFETY: Live handle.
            unsafe { (api.pair_is_connected)(h) == 0 }
        }),
        "client did not observe disconnect after server1 shutdown"
    );

    let rebind_deadline = Instant::now() + Duration::from_secs(3);
    let server2 = loop {
        match NveWireServer::bind_sync(NveWireServerConfig::new(
            "127.0.0.1:0".parse().expect("udp addr"),
            wss_addr,
            token,
        )) {
            Ok(s) => break s,
            Err(_) if Instant::now() < rebind_deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("bind server2 on same TCP port {wss_addr} failed: {e}"),
        }
    };

    // ReconnectingWireClient must reconnect automatically and send KeyframeRequest!
    let kf_req = server2
        .recv_inbound_timeout(Duration::from_secs(4))
        .expect("server2 must receive KeyframeRequest after reconnect");
    assert_eq!(kf_req.frame.kind, NveWireKind::KeyframeRequest);

    // Server2 responds with a full `SceneKeyframe` carrying a fresh tree.
    let keyframe = encode_scene("RECOVERED_KEYFRAME_V2");
    let _ = server2.broadcast_scene_keyframe(1, &keyframe);
    assert!(
        wait_until(Duration::from_secs(3), || {
            let mut ch = 0u32;
            // SAFETY: Owner-thread tick.
            let tick_rc = unsafe { (api.tick)(h, 0.016, &mut ch) };
            assert_eq!(tick_rc, NveRuntimeStatus::Ok.as_i32());
            ch == 1
        }),
        "reconnected SceneKeyframe never arrived"
    );

    // SAFETY: Valid handle.
    unsafe {
        assert_eq!((api.pair_is_connected)(h), 1);
        assert_eq!(
            (api.scene_acquire)(h, &mut view),
            NveRuntimeStatus::Ok.as_i32()
        );
        let text = read_text_slab(&view);
        assert!(
            text.contains("RECOVERED_KEYFRAME_V2"),
            "expected recovered keyframe text, got `{text}`"
        );
        assert!(
            !text.contains("CLICKED_REMOTE_V1"),
            "SceneKeyframe must reset prior replica text slab"
        );
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());

        // `pair_disconnect` cleanly stops worker, clears view, and is idempotent.
        assert_eq!((api.pair_disconnect)(h), NveRuntimeStatus::Ok.as_i32());
        assert_eq!((api.pair_is_connected)(h), 0);
        assert_eq!((api.pair_disconnect)(h), NveRuntimeStatus::Ok.as_i32());
        assert_eq!(
            (api.scene_acquire)(h, &mut view),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(view.rows.is_null() && view.node_capacity == 0);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    server2.shutdown();
    // SAFETY: Destroy once on owner thread.
    unsafe { (api.destroy)(h) };
}

/// A local app publishing one `LOCAL_APP` keyframe, with state, counting its drops.
struct LocalSceneApp {
    frame: Vec<u8>,
    rendered: bool,
    drops: &'static AtomicUsize,
}

impl RuntimeApp for LocalSceneApp {
    fn render_frame(&mut self, _frame_id: u64) -> AppFrame<'_> {
        if std::mem::replace(&mut self.rendered, true) {
            AppFrame::Clean
        } else {
            AppFrame::Wire(&self.frame)
        }
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }

    fn snapshot_state(&mut self) -> Result<&[u8], nve_runtime::app::CapabilityUnsupported> {
        Ok(b"local-state")
    }
}

impl Drop for LocalSceneApp {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

#[test]
fn pair_connect_swaps_a_local_app_atomically_and_failed_connects_keep_it() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        })
        .app(Box::new(LocalSceneApp {
            frame: encode_scene("LOCAL_APP"),
            rendered: false,
            drops: &DROPS,
        }))
        .build();
    let _ = rt.tick(0.016).unwrap();
    let scene_text = |rt: &nve_runtime::SurfaceRuntime| read_text_slab(rt.acquire_scene().unwrap().view());
    assert!(scene_text(&rt).contains("LOCAL_APP"));

    // Rejected connects never touch the running app, its scene, or its state.
    for bad in ["", "not-a-url", "wss://127.0.0.1:8092/nve-wire", "ws://127.0.0.1:0/nve-wire"] {
        assert!(matches!(rt.pair_connect(bad, "tok"), Err(RuntimeError::InvalidArg(_))), "{bad}");
        assert_eq!(DROPS.load(Ordering::Acquire), 0, "{bad}");
        assert_eq!(rt.snapshot_state().unwrap(), b"local-state", "{bad}");
        assert!(!rt.pair_is_connected());
        assert!(scene_text(&rt).contains("LOCAL_APP"), "{bad}");
    }

    // A valid connect replaces the app in one write: dropped, replica reset, pairing installed.
    let token = "app-to-pair-tok";
    let srv = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let _ = srv.broadcast_scene_keyframe(1, &encode_scene("REMOTE_SCENE"));
    rt.pair_connect(&format!("ws://{}/nve-wire", srv.wss_addr()), token)
        .expect("pairing replaces the local app");
    assert_eq!(DROPS.load(Ordering::Acquire), 1, "the local app is dropped by the switch");
    assert!(matches!(rt.snapshot_state(), Err(RuntimeError::Unsupported(_))), "no app remains");
    assert!(!scene_text(&rt).contains("LOCAL_APP"), "the app's scene is reset at the switch");
    assert!(wait_until(Duration::from_secs(3), || rt.pair_is_connected()));
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        scene_text(&rt).contains("REMOTE_SCENE")
    }));
    // And back: attaching an app replaces the pairing.
    assert!(rt.attach_app(Box::new(DummyLocalApp)).is_ok());
    assert!(!rt.pair_is_connected());
    srv.shutdown();
}

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
    out[0..4].copy_from_slice(&0x3245_564Eu32.to_le_bytes()); // "NVE2"
    out[4..8].copy_from_slice(&1u32.to_le_bytes()); // version 1
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
fn pair_disconnect_then_pair_connect_preserves_wakes_and_validates_uri_synchronously() {
    use std::sync::Arc;

    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        })
        .build();

    // Synchronous URI / host-port / TLS cert-pin validation BEFORE spawning any worker.
    for bad in [
        "",
        "not-a-url",
        "http://127.0.0.1:8092",
        "ws://bad host:8092/nve-wire",
        "ws://127.0.0.1:0/nve-wire",
        "ws://http::8092/nve-wire",
        "wss://127.0.0.1:8092/nve-wire",
        "quic://127.0.0.1:8091",
        "https://127.0.0.1:8091/nve-wire",
        "127.0.0.1:8092",
    ] {
        let err = rt
            .pair_connect(bad, "tok")
            .expect_err(&format!("URI `{bad}` must be rejected synchronously"));
        assert!(matches!(err, RuntimeError::InvalidArg(_)), "bad={bad} err={err:?}");
    }

    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_cb = Arc::clone(&wakes);
    rt.set_wake_callback(Arc::new(move || {
        wakes_cb.fetch_add(1, Ordering::Release);
    }));

    // Session 1: receive a wake WITHOUT calling `tick()`, leaving WakeBridge.pending == true.
    let token = "wake-preserve-tok";
    let srv1 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let _ = srv1.broadcast_scene_keyframe(1, &encode_scene("SESSION_ONE"));
    rt.pair_connect(&format!("ws://{}/nve-wire", srv1.wss_addr()), token)
        .unwrap();
    assert!(wait_until(Duration::from_secs(3), || {
        wakes.load(Ordering::Acquire) >= 1
    }));

    // Disconnect WITHOUT ever calling `tick()`, then connect to Session 2.
    rt.pair_disconnect().unwrap();
    let wakes_after_disconnect = wakes.load(Ordering::Acquire);

    let srv2 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let _ = srv2.broadcast_scene_keyframe(1, &encode_scene("SESSION_TWO"));
    rt.pair_connect(&format!("ws://{}/nve-wire", srv2.wss_addr()), token)
        .unwrap();

    // Session 2 MUST fire a fresh wake (never silenced by Session 1's unconsumed wake latch!).
    assert!(
        wait_until(Duration::from_secs(3), || {
            wakes.load(Ordering::Acquire) > wakes_after_disconnect
        }),
        "stale pending wake from disconnected pair session silenced new pair session"
    );
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("SESSION_TWO")
    }));

    // Calling pair_connect with an invalid URI while ALREADY paired MUST fail synchronously
    // WITHOUT disconnecting or disturbing the live paired session!
    assert!(matches!(
        rt.pair_connect("not-a-url", "tok"),
        Err(RuntimeError::InvalidArg(_))
    ));
    assert!(rt.pair_is_connected());
    assert_eq!(
        srv2.broadcast_scene_keyframe(2, &encode_scene("STILL_LIVE_AFTER_BAD_URI"))
            .unwrap(),
        1
    );
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        let g = rt.acquire_scene().unwrap();
        read_text_slab(g.view()).contains("STILL_LIVE_AFTER_BAD_URI")
    }));

    rt.pair_disconnect().unwrap();
    srv1.shutdown();
    srv2.shutdown();
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
    use nve_wire::NveWireFrame;

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

#[test]
fn pair_lifecycle_edge_cases_busy_rebind_detach_null_non_owner_and_destroy() {
    let api = get_api();
    let token = "lifecycle-edge-tok";
    let srv1 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let (mut scene1, f1) = StatefulSceneServer::new("EDGE_SRV1_V1");
    let _ = srv1.broadcast_scene_keyframe(1, &f1);

    let srv2 = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let _ = srv2.broadcast_scene_keyframe(1, &encode_scene("EDGE_SRV2_V1"));

    let cfg = NveRuntimeConfig {
        surface_id: 1,
        width: 320.0,
        height: 240.0,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Valid config.
    let h = unsafe { (api.create)(&cfg) };
    assert!(!h.is_null());

    // 1. Non-owner thread `pair_connect` fails with InvalidArg.
    let h_addr = h as usize;
    let uri1 = format!("ws://{}/nve-wire", srv1.wss_addr());
    let uri1_bg = uri1.clone();
    let bg_rc = std::thread::spawn(move || {
        // SAFETY: Handle stays alive until thread joins.
        unsafe {
            (api.pair_connect)(
                h_addr as *mut c_void,
                NveStrSlice::from_str(&uri1_bg),
                NveStrSlice::from_str(token),
            )
        }
    })
    .join()
    .unwrap();
    assert_eq!(bg_rc, NveRuntimeStatus::InvalidArg.as_i32());

    // 2. `pair_connect` while `scene_acquire` is held returns Busy.
    let mut view = NveRuntimeSceneView::default();
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
        assert_eq!(
            (api.pair_connect)(h, NveStrSlice::from_str(&uri1), NveStrSlice::from_str(token)),
            NveRuntimeStatus::Busy.as_i32()
        );
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 3. Connect to srv1 and receive initial frame.
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!(
            (api.pair_connect)(h, NveStrSlice::from_str(&uri1), NveStrSlice::from_str(token)),
            NveRuntimeStatus::Ok.as_i32()
        );
    }
    assert!(wait_until(Duration::from_secs(3), || {
        let mut ch = 0u32;
        // SAFETY: Owner thread.
        unsafe { (api.tick)(h, 0.016, &mut ch) == NveRuntimeStatus::Ok.as_i32() && ch == 1 }
    }));

    // 4. `attach_app_interface(h, NULL)` while paired MUST be a no-op (does NOT wipe paired replica!).
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!((api.attach_app_interface)(h, None), NveRuntimeStatus::Ok.as_i32());
        assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
        assert!(read_text_slab(&view).contains("EDGE_SRV1_V1"));
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }
    // Subsequent delta on srv1 MUST still apply cleanly!
    let f2 = scene1.update_label_delta("EDGE_SRV1_DELTA2");
    assert_eq!(srv1.broadcast_command_delta(2, &f2).unwrap(), 1);
    assert!(wait_until(Duration::from_secs(2), || {
        let mut ch = 0u32;
        // SAFETY: Owner thread.
        let _ = unsafe { (api.tick)(h, 0.016, &mut ch) };
        if ch == 1 {
            // SAFETY: Valid handle.
            unsafe {
                assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
                let ok = read_text_slab(&view).contains("EDGE_SRV1_DELTA2");
                assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
                ok
            }
        } else {
            false
        }
    }));

    // 5. `pair_connect` while ALREADY paired cleanly replaces srv1 with srv2!
    let uri2 = format!("ws://{}/nve-wire", srv2.wss_addr());
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!(
            (api.pair_connect)(h, NveStrSlice::from_str(&uri2), NveStrSlice::from_str(token)),
            NveRuntimeStatus::Ok.as_i32()
        );
    }
    assert!(wait_until(Duration::from_secs(3), || {
        let mut ch = 0u32;
        // SAFETY: Owner thread.
        let _ = unsafe { (api.tick)(h, 0.016, &mut ch) };
        if ch == 1 {
            // SAFETY: Valid handle.
            unsafe {
                assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
                let ok = read_text_slab(&view).contains("EDGE_SRV2_V1");
                assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
                ok
            }
        } else {
            false
        }
    }));

    // 6. Install wake callback, destroy while paired, then broadcast frames -> ZERO wakes after destroy!
    static AFTER_DESTROY_WAKES: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn on_destroy_wake(ctx: *mut c_void) {
        // SAFETY: `ctx` points to static `AFTER_DESTROY_WAKES`.
        unsafe { (*ctx.cast::<AtomicUsize>()).fetch_add(1, Ordering::Release) };
    }
    AFTER_DESTROY_WAKES.store(0, Ordering::Release);
    // SAFETY: Valid handle & static callback.
    unsafe {
        assert_eq!(
            (api.set_wake_callback)(
                h,
                Some(on_destroy_wake),
                (&raw const AFTER_DESTROY_WAKES).cast_mut().cast::<c_void>(),
            ),
            NveRuntimeStatus::Ok.as_i32()
        );
        (api.destroy)(h);
    }
    let wakes_at_destroy = AFTER_DESTROY_WAKES.load(Ordering::Acquire);
    let _ = srv2.broadcast_scene_keyframe(9, &encode_scene("AFTER_DESTROY"));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        AFTER_DESTROY_WAKES.load(Ordering::Acquire),
        wakes_at_destroy,
        "wake callback fired after destroy!"
    );

    // 7. Rust API: attaching a local app while paired cleanly disconnects pairing.
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        })
        .build();
    rt.pair_connect(&uri2, token).unwrap();
    assert!(wait_until(Duration::from_secs(3), || rt.pair_is_connected()));
    rt.attach_app(Box::new(DummyLocalApp)).unwrap();
    assert!(!rt.pair_is_connected());

    srv1.shutdown();
    srv2.shutdown();
}

#[test]
fn pair_connect_and_disconnect_each_start_a_new_scene_epoch() {
    let epoch = |rt: &nve_runtime::SurfaceRuntime| rt.acquire_scene().unwrap().view().scene_epoch;
    let mut rt = RuntimeBuilder::new().build();
    rt.submit_nve2(&encode_scene("LOCAL")).expect("local keyframe");
    let e0 = epoch(&rt);
    let token = "epoch-tok";
    let srv = NveWireServer::bind_sync(NveWireServerConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        token,
    ))
    .unwrap();
    let _ = srv.broadcast_scene_keyframe(1, &encode_scene("REMOTE"));
    rt.pair_connect(&format!("ws://{}/nve-wire", srv.wss_addr()), token).expect("pair");
    let e1 = epoch(&rt);
    assert_eq!(e1, e0.wrapping_add(1), "pairing discards the local scene");
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        read_text_slab(rt.acquire_scene().unwrap().view()).contains("REMOTE")
    }));
    assert_eq!(epoch(&rt), e1, "the paired keyframe lands in the pairing's epoch");
    rt.pair_disconnect().expect("disconnect");
    assert_eq!(epoch(&rt), e1.wrapping_add(1), "disconnecting discards the remote scene");
    rt.pair_disconnect().expect("idempotent");
    assert_eq!(epoch(&rt), e1.wrapping_add(1), "disconnecting nothing keeps the epoch");
    srv.shutdown();
}
