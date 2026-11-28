//! End-to-end live loopback integration tests for `SurfaceRuntime` embedded `nve-wire[client]`
//! pairing over both the Rust API and the C ABI (`pair_connect`, `pair_is_connected`, `tick`,
//! `scene_acquire`, `dispatch_events`, automatic reconnect + `KeyframeRequest` + `SceneKeyframe`
//! recovery, and `pair_disconnect`).

#![cfg(all(feature = "wire", not(target_arch = "wasm32")))]

#[path = "pair_integration/lifecycle.rs"]
mod lifecycle;
#[path = "pair_integration/recovery.rs"]
mod recovery;

use std::ffi::c_void;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nve_abi::{
    EventBatchBuf, FrameId, InteractionEvent, NveBufferSlice, NveRuntimeApi, NveRuntimeConfig,
    NveRuntimeMetrics, NveRuntimeStatus, NveSceneView, NveStrSlice, SurfaceId, TimestampUs,
    ValidatedEvents, NVE_RUNTIME_ABI_VERSION,
};
use nve_engine::Viewport;
use nve_runtime::{
    nve_runtime_get_interface, AppFrame, RuntimeApp, RuntimeBuilder, RuntimeError,
};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{Interaction, Layout, TextContent, Visual};
use nve_wire::{NveWireKind, NveWireServer, NveWireServerConfig};

pub(crate) fn get_api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable returned for matching ABI version.
    unsafe { *ptr }
}

pub(crate) struct StatefulSceneServer {
    store: NodeStore,
    enc: WireEncoder,
    btn: NodeId,
}

impl StatefulSceneServer {
    pub(crate) fn new(initial_label: &str) -> (Self, Vec<u8>) {
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
            tx.edit::<Interaction>(btn).listen(InteractionEvent::Press);
            tx.set_children(NodeId::ROOT, &[btn]);
            tx.commit_into(&mut changes);
            btn
        };
        let mut enc = WireEncoder::default();
        let mut out = Vec::new();
        enc.encode(&store, &changes, &mut out);
        (Self { store, enc, btn }, out)
    }

    pub(crate) fn update_label_delta(&mut self, next_label: &str) -> Vec<u8> {
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

pub(crate) fn encode_scene(label: &str) -> Vec<u8> {
    let (srv, _) = StatefulSceneServer::new(label);
    let mut enc = WireEncoder::default();
    let mut out = Vec::new();
    enc.encode_full(&srv.store, &mut out);
    out
}

pub(crate) fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    pred()
}

pub(crate) fn read_text_slab(view: &NveSceneView) -> String {
    if view.text_slab.is_null() || view.text_slab_len == 0 {
        return String::new();
    }
    // SAFETY: Caller holds an active `scene_acquire` guard over `view`.
    let bytes = unsafe { std::slice::from_raw_parts(view.text_slab, view.text_slab_len as usize) };
    std::str::from_utf8(bytes).expect("UTF-8 text slab").to_owned()
}

pub(crate) struct DummyLocalApp;

impl RuntimeApp for DummyLocalApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        AppFrame::Clean
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
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
            (api.pair_connect)(
                h,
                NveStrSlice::from_str("ws://127.0.0.1:9999/nve-wire"),
                NveStrSlice::EMPTY
            ),
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
    let mut view = NveSceneView::default();
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
    let mut ev_buf = EventBatchBuf::new();
    ev_buf.push_press(SurfaceId::ROOT, NodeId::new(2, 0), TimestampUs(123), 15.0, 15.0);
    let mut ev_changed = 1u32;
    // SAFETY: Valid handle and stack event batch.
    let drc = unsafe { (api.dispatch_events)(h, ev_buf.as_raw_batch(), &mut ev_changed) };
    assert_eq!(drc, NveRuntimeStatus::Ok.as_i32());
    assert_eq!(ev_changed, 0);

    let inbound = server1
        .recv_inbound_timeout(Duration::from_secs(2))
        .expect("server1 must receive upstream EventBatch");
    assert_eq!(inbound.frame.kind, NveWireKind::EventBatch);
    let decoded = EventBatchBuf::decode_wire(&inbound.frame.payload).expect("decode upstream events");
    let decoded_view = decoded.as_validated();
    assert_eq!(decoded_view.len(), 1);
    let first = decoded_view.get(0).expect("first event");
    assert_eq!(first.interaction(), Some(InteractionEvent::Press));
    assert_eq!(first.node(), NodeId::new(2, 0));

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
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        if std::mem::replace(&mut self.rendered, true) {
            AppFrame::Clean
        } else {
            AppFrame::Wire(&self.frame)
        }
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
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
    let scene_text =
        |rt: &nve_runtime::SurfaceRuntime| read_text_slab(rt.acquire_scene().unwrap().view());
    assert!(scene_text(&rt).contains("LOCAL_APP"));

    // Rejected connects never touch the running app, its scene, or its state.
    for bad in [
        "",
        "not-a-url",
        "wss://127.0.0.1:8092/nve-wire",
        "ws://127.0.0.1:0/nve-wire",
    ] {
        assert!(
            matches!(rt.pair_connect(bad, "tok"), Err(RuntimeError::InvalidArg(_))),
            "{bad}"
        );
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
    assert_eq!(
        DROPS.load(Ordering::Acquire),
        1,
        "the local app is dropped by the switch"
    );
    assert!(
        matches!(rt.snapshot_state(), Err(RuntimeError::Unsupported(_))),
        "no app remains"
    );
    assert!(
        !scene_text(&rt).contains("LOCAL_APP"),
        "the app's scene is reset at the switch"
    );
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
    rt.pair_connect(&format!("ws://{}/nve-wire", srv.wss_addr()), token)
        .expect("pair");
    let e1 = epoch(&rt);
    assert_eq!(e1, e0.wrapping_add(1), "pairing discards the local scene");
    assert!(wait_until(Duration::from_secs(3), || {
        let _ = rt.tick(0.016).unwrap();
        read_text_slab(rt.acquire_scene().unwrap().view()).contains("REMOTE")
    }));
    assert_eq!(epoch(&rt), e1, "the paired keyframe lands in the pairing's epoch");
    rt.pair_disconnect().expect("disconnect");
    assert_eq!(
        epoch(&rt),
        e1.wrapping_add(1),
        "disconnecting discards the remote scene"
    );
    rt.pair_disconnect().expect("idempotent");
    assert_eq!(
        epoch(&rt),
        e1.wrapping_add(1),
        "disconnecting nothing keeps the epoch"
    );
    srv.shutdown();
}
