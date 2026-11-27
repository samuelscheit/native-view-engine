//! Unit & integration tests for `nve-engine`:
//! - `HotPatchMsg` protocol serialization, deserialization, error paths, and C ABI routing
//! - `FrameProfiler` span recording, ring eviction, percentiles, and Chrome Trace JSON output
//! - `SurfaceRunner` frame sequencing and error propagation
//! - `NveApp` default implementations, `export_nve_app!` C ABI null-safety, and `NVE_IFACE_HOST`

use std::ffi::c_void;

use nve_abi::{
    DecodeError, EventBatchBuf, FrameId, InterfaceId, NodeId, NveBufferSlice, NveComponentApi,
    NveDevApi, NveEventBatch, NveGestureDisposition, NveHostApi, NveHostCallbacks,
    NveIntrinsicSize, NveLayoutApi, NveMapEntry, NvePropsRef, NveSizeConstraints, NveSurfaceApi,
    NveValue, SurfaceId, TimestampUs, ValidatedEvents, NVE_IFACE_COMPONENT, NVE_IFACE_DEV,
    NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
use nve_engine::{
    BackendApplyStats, BackendError, FrameProfiler, FrameTiming, HotPatchMsg, LayoutRect, NveApp,
    StoreBackend, StoreWireReplica, SurfaceRunner, Viewport,
};
use nve_store::{ChangeSet, NodeStore, RealizationOrder};

// ============================================================================
// 1. Hot-reload protocol serialization / deserialization & patch routing
// ============================================================================

#[test]
fn hot_patch_msg_roundtrip_all_variants_and_wire_tags() {
    let cases = vec![
        (
            HotPatchMsg::TemplatePatch {
                generation: 42,
                template_id: "card::header".into(),
                attr_updates: vec![
                    ("opacity".into(), "0.85".into()),
                    ("background-color".into(), "#0f172a".into()),
                ],
                text_updates: vec![(0, "Hello 🚀".into()), (3, "".into())],
                target_test_id: None,
                node_path: vec![],
                text_replacements: vec![],
            },
            1,
        ),
        (
            HotPatchMsg::TemplatePatch {
                generation: 0,
                template_id: "".into(),
                attr_updates: vec![],
                text_updates: vec![],
                target_test_id: None,
                node_path: vec![],
                text_replacements: vec![],
            },
            1,
        ),
        (
            HotPatchMsg::DylibReload {
                generation: u64::MAX,
                dylib_path: "/tmp/libnve_app.dylib".into(),
                build_ms: 215,
            },
            2,
        ),
        (
            HotPatchMsg::DevStatus {
                message: "Hot swap ready (gen 7)".into(),
                latency_ms: 12,
            },
            3,
        ),
    ];

    for (msg, expected_tag) in cases {
        let bytes = msg.to_bytes().unwrap();
        assert_eq!(&bytes[..4], b"NVP1");
        assert_eq!(bytes[4], expected_tag, "variant tag");
        let decoded = HotPatchMsg::from_bytes(&bytes).expect("roundtrip decode");
        assert_eq!(decoded, msg);
    }
}

#[test]
fn hot_patch_msg_rejects_json_and_malformed_frames() {
    let invalid_payloads: [&[u8]; 6] = [
        b"",
        b"NVP",
        b"NVP1",
        b"null",
        br#"{"type":"dev_status","message":"x","latency_ms":5}"#,
        b"NVP1\x03\x05\x00\x00\x00hi",
    ];
    for raw in invalid_payloads {
        assert!(HotPatchMsg::from_bytes(raw).is_err(), "expected error for {raw:?}");
    }
}

// Local NveApp exported via `export_nve_app!` for testing patch routing and C ABI null safety.
struct RoutingTestApp {
    store: NodeStore,
    changes: ChangeSet,
    encoder: nve_store::wire::WireEncoder,
    patches_received: Vec<HotPatchMsg>,
    state: Vec<u8>,
    frame_buf: Vec<u8>,
    emitted: EventBatchBuf,
}

impl NveApp for RoutingTestApp {
    fn create(surface_id: SurfaceId) -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        store.tx().commit_into(&mut changes);
        let mut encoder = nve_store::wire::WireEncoder::default();
        let mut frame_buf = Vec::new();
        encoder.encode(&store, &changes, &mut frame_buf);
        let mut emitted = EventBatchBuf::new();
        emitted.push_press(surface_id, NodeId::new(2, 0), TimestampUs(10), 1.0, 2.0);
        Self {
            store,
            changes,
            encoder,
            patches_received: Vec::new(),
            state: b"initial".to_vec(),
            frame_buf,
            emitted,
        }
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        self.store.tx().commit_into(&mut self.changes);
        self.encoder
            .encode(&self.store, &self.changes, &mut self.frame_buf);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        events
            .iter()
            .any(|ev| ev.is_custom() && ev.custom_name() == "onPing" && ev.custom_data() == b"ok")
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
        props.get("count").and_then(|v| v.as_i64()).is_some()
    }

    fn drain_emitted_events(&mut self) -> ValidatedEvents<'_> {
        self.emitted.as_validated()
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        match patch {
            HotPatchMsg::TemplatePatch { .. } => {
                self.patches_received.push(patch.clone());
                true
            }
            _ => false,
        }
    }

    fn snapshot_state(&mut self) -> &[u8] {
        &self.state
    }

    fn restore_state(&mut self, state: &[u8]) {
        self.state = state.to_vec();
    }
}

nve_engine::export_nve_app!(RoutingTestApp, symbol = nve_engine_test_get_interface);

unsafe extern "C" {
    fn nve_engine_test_get_interface(interface_id: InterfaceId) -> *const c_void;
}

#[test]
fn export_nve_app_routes_hot_patches_handles_nulls_and_defaults() {
    // SAFETY: Exercises the locally exported `export_nve_app!` C ABI tables, null-handle guards,
    // valid stack-backed FFI slices, and balanced create/destroy lifecycle.
    unsafe {
        // Unknown interface IDs return NULL.
        assert!(nve_engine_test_get_interface(InterfaceId(0)).is_null());
        assert!(nve_engine_test_get_interface(InterfaceId(999)).is_null());

        let surface = *(nve_engine_test_get_interface(NVE_IFACE_SURFACE) as *const NveSurfaceApi);
        let layout = *(nve_engine_test_get_interface(NVE_IFACE_LAYOUT) as *const NveLayoutApi);
        let component =
            *(nve_engine_test_get_interface(NVE_IFACE_COMPONENT) as *const NveComponentApi);
        let dev = *(nve_engine_test_get_interface(NVE_IFACE_DEV) as *const NveDevApi);

        // Null handle guards across every exported C ABI function must never dereference or panic.
        let null = std::ptr::null_mut();
        (surface.destroy)(null);
        assert_eq!((surface.render)(null, FrameId(1)).len, 0);
        assert_eq!((surface.dispatch_events)(null, NveEventBatch::EMPTY), 0);
        assert_eq!(
            (layout.measure)(null, NveSizeConstraints::loose(100.0, 200.0)),
            NveIntrinsicSize::default()
        );
        assert_eq!((layout.hit_test)(null, 10.0, 20.0, 1), 2);
        assert_eq!((component.set_props)(null, std::ptr::null(), 0), 0);
        assert_eq!((component.drain_emitted_events)(null).event_count, 0);
        assert_eq!((dev.apply_hot_patch)(null, NveBufferSlice::EMPTY), 0);
        assert_eq!((dev.snapshot_state)(null).len, 0);
        (dev.restore_state)(null, NveBufferSlice::EMPTY);

        let handle = (surface.create)(SurfaceId::new(7, 0));
        assert!(!handle.is_null());

        // Direct zero-copy event dispatch & props update via C ABI
        let mut ev_buf = EventBatchBuf::new();
        ev_buf.push_custom(
            SurfaceId::new(7, 0),
            NodeId::new(42, 0),
            TimestampUs(1234),
            "onPing",
            b"ok",
        );
        let prop_entries = [NveMapEntry::new("count", NveValue::i64(99))];
        assert_eq!(
            (surface.dispatch_events)(handle, ev_buf.as_raw_batch()),
            1
        );
        assert_eq!(
            (component.set_props)(handle, prop_entries.as_ptr(), prop_entries.len() as u32),
            1
        );
        let emitted = (component.drain_emitted_events)(handle);
        assert_eq!(emitted.event_count, 1);

        // Default NveApp::measure handles finite and infinite constraints properly.
        let finite_size = (layout.measure)(
            handle,
            NveSizeConstraints {
                min_w: 50.0,
                min_h: 30.0,
                max_w: 300.0,
                max_h: 500.0,
            },
        );
        assert_eq!(
            finite_size,
            NveIntrinsicSize {
                width: 300.0,
                height: 500.0
            }
        );

        let inf_size = (layout.measure)(
            handle,
            NveSizeConstraints {
                min_w: 64.0,
                min_h: 48.0,
                max_w: f32::INFINITY,
                max_h: f32::INFINITY,
            },
        );
        assert_eq!(
            inf_size,
            NveIntrinsicSize {
                width: 64.0,
                height: 48.0
            }
        );
        assert_eq!(
            NveGestureDisposition::from_u8((layout.hit_test)(handle, 0.0, 0.0, 1)),
            NveGestureDisposition::PassThrough
        );

        // Valid TemplatePatch NVP1 frame routes cleanly -> returns 1.
        let valid_patch = HotPatchMsg::TemplatePatch {
            generation: 1,
            template_id: "root".into(),
            attr_updates: vec![("opacity".into(), "0.6".into())],
            text_updates: vec![],
            target_test_id: None,
            node_path: vec![],
            text_replacements: vec![],
        }
        .to_bytes()
        .unwrap();
        assert_eq!(
            (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(&valid_patch)),
            1
        );

        // Non-template patch returns 0.
        let status_patch = HotPatchMsg::DevStatus {
            message: "ping".into(),
            latency_ms: 1,
        }
        .to_bytes()
        .unwrap();
        assert_eq!(
            (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(&status_patch)),
            0
        );

        // Garbage, JSON text and truncated frames are rejected with 0 before reaching the app.
        let json_patch = br#"{"type":"template_patch","generation":1,"template_id":"root","attr_updates":[],"text_updates":[]}"#;
        for bad in [
            &[0xFFu8, 0xFE, 0x80][..],
            &json_patch[..],
            &valid_patch[..valid_patch.len() - 1],
        ] {
            assert_eq!(
                (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(bad)),
                0
            );
        }

        // State snapshot & restore roundtrip.
        let new_state = b"saved-v2";
        (dev.restore_state)(handle, NveBufferSlice::from_slice(new_state));
        let snapped = (dev.snapshot_state)(handle).as_slice().to_vec();
        assert_eq!(snapped, new_state);

        (surface.destroy)(handle);
    }
}

// ============================================================================
// 2. FrameProfiler & SurfaceRunner tests
// ============================================================================

#[test]
fn profiler_empty_capacity_eviction_and_percentiles() {
    let empty = FrameProfiler::new(8);
    assert_eq!(empty.p50_total_us(), 0);
    assert_eq!(empty.p99_total_us(), 0);
    let parsed_empty: serde_json::Value =
        serde_json::from_str(&empty.to_chrome_trace_json()).unwrap();
    assert_eq!(parsed_empty.as_array().unwrap().len(), 0);

    // Capacity 0 clamps safely to 1.
    let mut cap_zero = FrameProfiler::new(0);
    cap_zero.record(FrameTiming {
        frame_id: FrameId(1),
        reconcile_us: 10,
        ..Default::default()
    });
    cap_zero.record(FrameTiming {
        frame_id: FrameId(2),
        reconcile_us: 25,
        ..Default::default()
    });
    assert_eq!(cap_zero.frames().len(), 1);
    assert_eq!(cap_zero.frames()[0].frame_id, FrameId(2));

    // Ring buffer evicts oldest frames FIFO and computes accurate p50/p99 on unsorted totals.
    let mut profiler = FrameProfiler::new(5);
    let totals = [100u64, 500, 10, 40, 20, 30, 1000];
    for (i, total) in totals.iter().enumerate() {
        profiler.record(FrameTiming {
            frame_id: FrameId((i + 1) as u64),
            op_count: 5,
            mutation_count: 2,
            reconcile_us: *total,
            ..Default::default()
        });
    }
    assert_eq!(profiler.frames().len(), 5);
    assert_eq!(profiler.frames()[0].frame_id, FrameId(3));
    assert_eq!(profiler.frames()[4].frame_id, FrameId(7));
    assert_eq!(profiler.p50_total_us(), 30);
    assert_eq!(profiler.p99_total_us(), 1000);
}

#[test]
fn profiler_chrome_trace_json_emits_valid_spans_and_monotonic_timestamps() {
    let mut profiler = FrameProfiler::new(16);
    let f1 = FrameTiming {
        frame_id: FrameId(1),
        op_count: 10,
        mutation_count: 4,
        reconcile_us: 15,
        encode_us: 5,
        decode_us: 7,
        layout_us: 40,
        diff_us: 8,
        mount_us: 12,
    };
    assert_eq!(f1.total_us(), 87);
    let f2 = FrameTiming {
        frame_id: FrameId(2),
        op_count: 2,
        mutation_count: 1,
        ..Default::default()
    };
    profiler.record(f1);
    profiler.record(f2);

    let json = profiler.to_chrome_trace_json();
    let arr: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid trace JSON");
    assert_eq!(arr.len(), 12);

    let expected_stages = [
        ("rust.reconcile", 1u64, 15u64),
        ("abi.encode", 1, 5),
        ("backend.decode", 2, 7),
        ("backend.layout", 2, 40),
        ("backend.diff", 2, 8),
        ("backend.mount", 3, 12),
    ];

    let mut prev_ts = 0u64;
    for (idx, (name, tid, dur)) in expected_stages.iter().enumerate() {
        let ev = &arr[idx];
        assert_eq!(ev["name"].as_str(), Some(*name));
        assert_eq!(ev["cat"].as_str(), Some("nve.frame"));
        assert_eq!(ev["ph"].as_str(), Some("X"));
        assert_eq!(ev["pid"].as_u64(), Some(1));
        assert_eq!(ev["tid"].as_u64(), Some(*tid));
        assert_eq!(ev["dur"].as_u64(), Some(*dur));
        let ts = ev["ts"].as_u64().unwrap();
        if idx > 0 {
            assert!(ts > prev_ts, "timestamps must advance monotonically");
        }
        prev_ts = ts;
    }

    let f2_start = arr[6]["ts"].as_u64().unwrap();
    assert_eq!(f2_start, 87 + 100);
    for (offset, ev) in arr[7..12].iter().enumerate() {
        let ts = ev["ts"].as_u64().unwrap();
        assert_eq!(ts, f2_start + (offset + 1) as u64);
    }
}

struct StubBackend {
    viewport: Viewport,
    fail_next: bool,
}

impl StoreBackend for StubBackend {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn set_viewport(&mut self, _surface_id: SurfaceId, viewport: Viewport) {
        self.viewport = viewport;
    }

    fn commit(
        &mut self,
        _store: &NodeStore,
        changes: &ChangeSet,
    ) -> Result<BackendApplyStats, BackendError> {
        if self.fail_next {
            self.fail_next = false;
            return Err(BackendError::Decode(DecodeError::Truncated));
        }
        Ok(BackendApplyStats {
            op_count: (changes.created.len() + changes.touched.len()) as u32,
            mutation_count: 3,
            decode_us: 11,
            layout_us: 22,
            diff_us: 5,
            mount_us: 9,
        })
    }

    fn drain_events(&mut self, _out: &mut EventBatchBuf) {}

    fn layout_of(&self, _node_id: NodeId) -> Option<LayoutRect> {
        None
    }

    fn reset(&mut self) {}
}

#[test]
fn surface_runner_sequences_frames_records_profiler_and_propagates_errors() {
    let vp = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 2.0,
    };
    let mut runner = SurfaceRunner::new(
        SurfaceId::new(9, 0),
        vp,
        StubBackend {
            viewport: Viewport::default(),
            fail_next: false,
        },
    );
    assert_eq!(runner.surface_id(), SurfaceId::new(9, 0));
    assert_eq!(runner.backend().viewport, vp);

    let f1 = runner.next_frame_id();
    let f2 = runner.next_frame_id();
    assert_eq!((f1, f2), (FrameId(1), FrameId(2)));

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    store.tx().commit_into(&mut changes);

    let t1 = runner
        .commit_store(f1, 13, &store, &changes)
        .expect("frame 1 ok");
    assert_eq!(t1.total_us(), 13 + 11 + 22 + 5 + 9);
    assert_eq!(runner.profiler().frames().len(), 1);

    runner.backend_mut().fail_next = true;
    let err = runner.commit_store(f2, 1, &store, &changes).unwrap_err();
    assert!(matches!(err, BackendError::Decode(DecodeError::Truncated)));
    assert_eq!(runner.profiler().frames().len(), 1);

    // StoreWireReplica roundtrip over NVE2 bytes
    let mut enc = nve_store::wire::WireEncoder::default();
    let mut wire = Vec::new();
    enc.encode(&store, &changes, &mut wire);
    let mut replica = StoreWireReplica::new(
        StubBackend {
            viewport: vp,
            fail_next: false,
        },
        RealizationOrder::new(),
    );
    let stats = replica.submit_nve2_frame(&wire).expect("nve2 ok");
    assert_eq!(stats.mutation_count, 3);
}

// ============================================================================
// 3. Modular NVE_IFACE_HOST = 5 (`bind_host`, `dispatch_host_events`)
// ============================================================================

struct HostCapableApp {
    bound_host_ctx: usize,
    batches_seen: u32,
    state_buf: Vec<u8>,
}

impl NveApp for HostCapableApp {
    fn create(_surface_id: SurfaceId) -> Self {
        Self {
            bound_host_ctx: 0,
            batches_seen: 0,
            state_buf: Vec::new(),
        }
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        self.state_buf = format!("ctx={};batches={}", self.bound_host_ctx, self.batches_seen).into_bytes();
        &self.state_buf
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        false
    }

    fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
        self.bound_host_ctx = callbacks.map_or(0, |cb| cb.host_ctx as usize);
    }

    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        if batch.len() == 8 {
            self.batches_seen += u32::from_le_bytes([batch[0], batch[1], batch[2], batch[3]]);
            true
        } else {
            false
        }
    }
}

nve_engine::export_nve_app!(HostCapableApp, symbol = nve_engine_host_test_get_interface);

unsafe extern "C" {
    fn nve_engine_host_test_get_interface(interface_id: InterfaceId) -> *const c_void;
}

#[test]
fn export_nve_app_host_interface_bind_unbind_and_dispatch() {
    // SAFETY: Exercises locally exported `export_nve_app!` surface and host C ABI tables with valid stack data and balanced lifecycle.
    unsafe {
        let surface = *(nve_engine_host_test_get_interface(NVE_IFACE_SURFACE)
            as *const NveSurfaceApi);
        let host =
            *(nve_engine_host_test_get_interface(NVE_IFACE_HOST) as *const NveHostApi);

        let handle = (surface.create)(SurfaceId::ROOT);
        assert!(!handle.is_null());

        let cb = NveHostCallbacks {
            host_ctx: 0x1234usize as *mut c_void,
            invoke: None,
            channel_publish: None,
        };
        (host.bind_host)(handle, &cb);

        let mut payload = [0u8; 8];
        payload[..4].copy_from_slice(&2u32.to_le_bytes());

        assert_eq!(
            (host.dispatch_host_events)(handle, NveBufferSlice::from_slice(&payload)),
            1
        );
        assert_eq!(
            (host.dispatch_host_events)(handle, NveBufferSlice::from_slice(b"bad")),
            0
        );
        let rendered = (surface.render)(handle, FrameId(1)).as_slice().to_vec();
        assert_eq!(
            rendered,
            format!("ctx={};batches=2", 0x1234usize).into_bytes()
        );

        (host.bind_host)(handle, std::ptr::null());
        let rendered2 = (surface.render)(handle, FrameId(2)).as_slice().to_vec();
        assert_eq!(rendered2, b"ctx=0;batches=2");

        (surface.destroy)(handle);
    }
}
