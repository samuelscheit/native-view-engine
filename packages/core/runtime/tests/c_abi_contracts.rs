//! Comprehensive C ABI, memory-safety, concurrency, panic-boundary, thread-affinity, and
//! validation contract tests for `nve-runtime`.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use nve_abi::{
    EventMask, NativeEventKind, NveBufferSlice, NveDirectEvent, NveEventBatch, NveEventBatchRef,
    NveRuntimeApi, NveRuntimeConfig, NveRuntimeMetrics, NveSceneView, NveRuntimeStatus,
    NveStrSlice, NVE_RUNTIME_ABI_VERSION, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_components::tables::{Interaction, Layout, Scroll, TextContent, Visual};
use nve_engine::{NveApp, Viewport};
use nve_layout::TaffyStoreLayout;
use nve_runtime::{
    AppFrame, FrameTickHook, RuntimeApp, RuntimeBuilder, RuntimeError, SurfaceRuntime, WakeBridge,
    nve_runtime_get_interface,
};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba, Transaction};

fn get_api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable returned for matching ABI version.
    unsafe { *ptr }
}

fn read_last_error(api: &NveRuntimeApi) -> String {
    let mut buf = [0u8; 512];
    // SAFETY: Stack buffer valid for 512 bytes.
    let len = unsafe { (api.last_error)(buf.as_mut_ptr(), buf.len()) };
    let n = len.min(buf.len() - 1);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

fn build_nve2_with_nodes(count: usize, with_scroll: bool) -> Vec<u8> {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let container = tx.create();
        tx.edit::<Layout>(container)
            .width(Length::points(200.0))
            .height(Length::points(100.0));
        if with_scroll {
            tx.edit::<Scroll>(container);
        }
        let mut kids = Vec::with_capacity(count);
        for _ in 0..count {
            let item = tx.create();
            tx.edit::<Layout>(item)
                .width(Length::points(200.0))
                .height(Length::points(50.0))
                .flex_shrink(0.0);
            tx.edit::<Visual>(item).background(Rgba::rgb(30, 41, 59));
            tx.edit::<Interaction>(item)
                .event_mask(EventMask::PRESS as u32);
            kids.push(item);
        }
        tx.set_children(container, &kids);
        tx.set_children(NodeId::ROOT, &[container]);
        tx.commit_into(&mut changes);
    }
    let mut enc = WireEncoder::default();
    let mut out = Vec::new();
    enc.encode(&store, &changes, &mut out);
    out
}

/// Minimal `NveApp` exported via `export_nve_app!` for C ABI `attach_app_interface` testing.
struct MiniCAbiApp {
    clicks: u32,
    store: NodeStore,
    changes: ChangeSet,
    enc: WireEncoder,
    buf: Vec<u8>,
    dirty: bool,
}

impl NveApp for MiniCAbiApp {
    fn create(_surface_id: u32) -> Self {
        Self {
            clicks: 0,
            store: NodeStore::new(),
            changes: ChangeSet::default(),
            enc: WireEncoder::default(),
            buf: Vec::new(),
            dirty: true,
        }
    }

    fn render(&mut self, _frame_id: u64) -> &[u8] {
        if !self.dirty {
            return &[];
        }
        self.dirty = false;
        let label = format!("clicks={}", self.clicks);
        {
            let mut tx = self.store.tx();
            let node = tx.store().id_at(2).unwrap_or_else(|| {
                let n = tx.create();
                tx.edit::<Layout>(n)
                    .width(Length::points(160.0))
                    .height(Length::points(48.0));
                tx.set_children(NodeId::ROOT, &[n]);
                n
            });
            tx.edit::<TextContent>(node).text(&label);
            tx.commit_into(&mut self.changes);
        }
        self.buf.clear();
        self.enc.encode(&self.store, &self.changes, &mut self.buf);
        &self.buf
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        if events.is_empty() {
            return false;
        }
        self.clicks += events.len() as u32;
        self.dirty = true;
        true
    }
}

nve_engine::export_nve_app!(MiniCAbiApp, symbol = nve_runtime_test_mini_app_get_interface);

unsafe extern "C" {
    fn nve_runtime_test_mini_app_get_interface(interface_id: u32) -> *const c_void;
}

unsafe extern "C" fn failing_null_surface_get_interface(_interface_id: u32) -> *const c_void {
    std::ptr::null()
}

/// Appends an unknown opcode (`0xFF`, 0 words) to a valid `NVE2` frame while updating its 24-byte
/// header (`ops += 1`, `words += 2`) so `WireDecoder::validate_header` SUCCEEDS and every earlier
/// op executes (including `NodeStore` reallocations) before failing mid-frame with `UnknownOp`.
fn append_failing_op_to_valid_nve2(frame: &mut Vec<u8>) {
    assert!(frame.len() >= 24 && frame.len().is_multiple_of(4));
    let ops = u32::from_le_bytes(frame[16..20].try_into().expect("4-byte ops field")) + 1;
    let words = u32::from_le_bytes(frame[20..24].try_into().expect("4-byte words field")) + 2;
    frame[16..20].copy_from_slice(&ops.to_le_bytes());
    frame[20..24].copy_from_slice(&words.to_le_bytes());
    frame.extend_from_slice(&0x0000_00FFu32.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
}

#[test]
fn failed_reallocating_wire_frame_and_detach_never_leave_dangling_published_view() {
    let api = get_api();
    // SAFETY: Null config uses defaults.
    let h = unsafe { (api.create)(std::ptr::null()) };
    assert!(!h.is_null());

    // 1. Build a stateful `src_store` + `WireEncoder` and submit frame 1 (16 nodes).
    let mut src_store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut enc = WireEncoder::default();
    let container = {
        let mut tx = src_store.tx();
        let container = tx.create();
        tx.edit::<Layout>(container)
            .width(Length::points(200.0))
            .height(Length::points(100.0));
        let mut kids = Vec::with_capacity(16);
        for _ in 0..16 {
            let item = tx.create();
            tx.edit::<Layout>(item)
                .width(Length::points(100.0))
                .height(Length::points(20.0));
            tx.edit::<Visual>(item).background(Rgba::rgb(30, 41, 59));
            kids.push(item);
        }
        tx.set_children(container, &kids);
        tx.set_children(NodeId::ROOT, &[container]);
        tx.commit_into(&mut changes);
        container
    };
    let mut frame1 = Vec::new();
    enc.encode(&src_store, &changes, &mut frame1);

    let mut view1 = NveSceneView::default();
    // SAFETY: Valid handle, slice, and stack out pointer on owner thread.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&frame1)),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(
            (api.scene_acquire)(h, &mut view1),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(!view1.rows.is_null() && view1.node_capacity >= 18);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }
    let orig_rows_ptr = view1.rows;
    let orig_cap = view1.node_capacity;

    // 1b. A header-invalid frame (misaligned trailing garbage) must fail BEFORE mutating store
    //     and MUST preserve the live `published_view` intact!
    let mut bad_header = frame1.clone();
    bad_header.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0xFF]);
    // SAFETY: Valid handle and slice.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&bad_header)),
            NveRuntimeStatus::Io.as_i32()
        );
        let mut preserved = NveSceneView::default();
        assert_eq!(
            (api.scene_acquire)(h, &mut preserved),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(
            preserved.rows, orig_rows_ptr,
            "header-invalid wire frame must not blank the live published_view"
        );
        assert_eq!(preserved.node_capacity, orig_cap);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 2. Non-vacuous mid-frame reallocation failure: encode 256 NEW nodes (valid header, forces
    //    `replica_store.nodes` reallocation!) followed by a trailing unknown op `0xFF`.
    {
        let mut tx = src_store.tx();
        for _ in 0..256 {
            let n = tx.create();
            tx.edit::<Layout>(n)
                .width(Length::points(10.0))
                .height(Length::points(10.0));
        }
        tx.commit_into(&mut changes);
    }
    let mut mid_fail_frame = Vec::new();
    enc.encode(&src_store, &changes, &mut mid_fail_frame);
    append_failing_op_to_valid_nve2(&mut mid_fail_frame);
    // SAFETY: Valid handle and slice.
    let rc = unsafe { (api.submit_nve2)(h, NveBufferSlice::from_slice(&mid_fail_frame)) };
    assert_eq!(rc, NveRuntimeStatus::Io.as_i32());
    assert!(!read_last_error(&api).is_empty());

    // 3. `scene_acquire` MUST NOT return dangling pointers: `rows` must be NULL and capacity 0!
    let mut view_after_err = NveSceneView::default();
    // SAFETY: Valid handle and out pointer.
    unsafe {
        assert_eq!(
            (api.scene_acquire)(h, &mut view_after_err),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(
            view_after_err.rows.is_null() && view_after_err.node_capacity == 0,
            "mid-frame wire failure after reallocation must clear published_view"
        );
        assert!(view_after_err.tables.is_null() && view_after_err.table_count == 0);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 3b. Because step 2 failed mid-frame after mutating `replica_store`, the runtime MUST have
    //     reset `replica_store` and `LaidOutScene` cleanly (zero leaked 256-node partial state!):
    //     a stale incremental Frame-3 delta against `container` must therefore fail cleanly, and a
    //     subsequent full Frame-1 keyframe carrying all 16 + 256 nodes must recover cleanly with
    //     reallocated `rows` and full capacity!
    {
        let mut tx = src_store.tx();
        tx.edit::<Visual>(container).opacity(0.75);
        tx.commit_into(&mut changes);
    }
    let mut stale_delta = Vec::new();
    enc.encode(&src_store, &changes, &mut stale_delta);
    // SAFETY: Valid handle and slice on owner thread.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&stale_delta)),
            NveRuntimeStatus::Io.as_i32(),
            "stale incremental delta must fail after mid-frame error reset (no leaked nodes)"
        );
    }

    let mut recovery_frame = Vec::new();
    WireEncoder::default().encode_full(&src_store, &mut recovery_frame);
    let mut recovered = NveSceneView::default();
    // SAFETY: Valid handle, slice, and out pointer.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&recovery_frame)),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(
            (api.scene_acquire)(h, &mut recovered),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(!recovered.rows.is_null() && !recovered.rects.is_null() && !recovered.tables.is_null());
        assert!(
            recovered.node_capacity >= orig_cap + 256,
            "expected recovered keyframe to carry all >= 256+ nodes (was {orig_cap}, now {})",
            recovered.node_capacity
        );
        assert_ne!(
            recovered.rows, orig_rows_ptr,
            "expected Vec<NodeRow> backing buffer to have actually reallocated"
        );
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 4. Attach C ABI app, verify failed attach preserves live app & view, verify `submit_nve2`
    //    error preserves live view, then detach cleanly.
    let ev = [NveDirectEvent::press(1, 2, 100, 10.0, 10.0)];
    let mut changed = 0u32;
    let mut ev_changed = 0u32;
    let mut detached_view = NveSceneView::default();
    // SAFETY: Valid handle, exported `nve_get_interface` symbol, and stack slices on owner thread.
    unsafe {
        assert_eq!(
            (api.attach_app_interface)(h, Some(nve_runtime_test_mini_app_get_interface)),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(
            (api.tick)(h, 0.016, &mut changed),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(changed, 1);

        // Non-mutating errors (`submit_nve2` while app attached AND failing `attach_app_interface`)
        // MUST preserve both the existing app and the live `published_view`!
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&frame1)),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.attach_app_interface)(h, Some(failing_null_surface_get_interface)),
            NveRuntimeStatus::Unsupported.as_i32()
        );
        {
            let mut intact = NveSceneView::default();
            assert_eq!(
                (api.scene_acquire)(h, &mut intact),
                NveRuntimeStatus::Ok.as_i32()
            );
            assert!(
                !intact.rows.is_null() && intact.node_capacity >= 3 && intact.frame_id >= 1,
                "failed submit_nve2 / failed attach_app_interface must not blank live published_view"
            );
            assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
        }

        // Dispatch an event to the STILL-ATTACHED C ABI app and re-tick.
        assert_eq!(
            (api.dispatch_events)(h, NveEventBatch::from_slice(&ev), &mut ev_changed),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(ev_changed, 1);
        assert_eq!(
            (api.tick)(h, 0.016, &mut changed),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(changed, 1);
        {
            let mut v = NveSceneView::default();
            assert_eq!(
                (api.scene_acquire)(h, &mut v),
                NveRuntimeStatus::Ok.as_i32()
            );
            let text = std::str::from_utf8(std::slice::from_raw_parts(
                v.text_slab,
                v.text_slab_len as usize,
            ))
            .unwrap();
            assert!(text.contains("clicks=1"), "expected clicks=1 in `{text}`");
            assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
        }

        // Detach app (`NULL`) -> published_view must be invalidated!
        assert_eq!(
            (api.attach_app_interface)(h, None),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(
            (api.scene_acquire)(h, &mut detached_view),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(
            detached_view.rows.is_null() && detached_view.node_capacity == 0,
            "detaching app via attach_app_interface(NULL) must invalidate published_view"
        );
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // SAFETY: Destroy handle once on owner thread.
    unsafe { (api.destroy)(h) };
}

#[test]
fn submit_nve2_corrupt_header_preserves_scene_while_mid_frame_error_resets_store_and_layout() {
    nve_components::tables::register_core_tables();
    let api = get_api();
    let cfg = NveRuntimeConfig {
        surface_id: 1,
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Valid config on owner thread.
    let h = unsafe { (api.create)(&cfg) };
    assert!(!h.is_null());

    // Frame 1: ROOT(1) + child(2) sized 120x40.
    let mut src = NodeStore::new();
    let mut enc = WireEncoder::default();
    let mut changes = ChangeSet::default();
    let child = NodeId(2);
    {
        let mut tx = src.tx();
        let _ = tx.create_at(NodeId::ROOT);
        assert!(tx.create_at(child));
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(400.0))
            .height(Length::points(300.0));
        tx.edit::<Layout>(child)
            .width(Length::points(120.0))
            .height(Length::points(40.0));
        tx.set_children(NodeId::ROOT, &[child]);
        tx.commit_into(&mut changes);
    }
    let mut frame1 = Vec::new();
    enc.encode(&src, &changes, &mut frame1);

    let mut view = NveSceneView::default();
    // SAFETY: Valid handle, slice, and out pointers.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&frame1)),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
        let rects = std::slice::from_raw_parts(view.rects, view.node_capacity as usize);
        assert_eq!(rects[2].width, 120.0);
        assert_eq!(rects[2].height, 40.0);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 1. Corrupt header fails before touching store -> live scene & layout rect stay 100% intact.
    let bad_hdr = b"NOT_NVE2_HEADER_BYTES";
    // SAFETY: Valid handle and slice.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(bad_hdr)),
            NveRuntimeStatus::Io.as_i32()
        );
        assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
        let rects = std::slice::from_raw_parts(view.rects, view.node_capacity as usize);
        assert_eq!(rects[2].width, 120.0);
        assert_eq!(rects[2].height, 40.0);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 2. Mid-frame corrupt body (valid header + CreateNodes(99) + invalid op 0xFF) MUST reset both
    //    `replica_store` and `LaidOutScene` cleanly (zero leaked nodes, NULL published view).
    {
        let mut tx = src.tx();
        let leaked = NodeId(99);
        assert!(tx.create_at(leaked));
        tx.edit::<Layout>(leaked)
            .width(Length::points(55.0))
            .height(Length::points(55.0));
        tx.commit_into(&mut changes);
    }
    let mut bad_mid = Vec::new();
    enc.encode(&src, &changes, &mut bad_mid);
    append_failing_op_to_valid_nve2(&mut bad_mid);

    let mut metrics = NveRuntimeMetrics::default();
    // SAFETY: Valid handle and slices.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&bad_mid)),
            NveRuntimeStatus::Io.as_i32()
        );
        assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
        assert!(view.rows.is_null() && view.node_capacity == 0);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
        assert_eq!((api.get_metrics)(h, &mut metrics), NveRuntimeStatus::Ok.as_i32());
        assert_eq!(metrics.live_nodes, 0);
    }

    // Stale delta against child(2) fails because replica_store was wiped cleanly.
    {
        let mut tx = src.tx();
        tx.edit::<Layout>(child).width(Length::points(200.0));
        tx.commit_into(&mut changes);
    }
    let mut stale_f3 = Vec::new();
    enc.encode(&src, &changes, &mut stale_f3);
    // SAFETY: Valid handle and slice.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&stale_f3)),
            NveRuntimeStatus::Io.as_i32()
        );
    }

    // 3. Fresh Frame-1 keyframe recovers store + layout cleanly (child 2 = 200x50, no node 99).
    let mut fresh = NodeStore::new();
    {
        let mut tx = fresh.tx();
        let _ = tx.create_at(NodeId::ROOT);
        assert!(tx.create_at(child));
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(400.0))
            .height(Length::points(300.0));
        tx.edit::<Layout>(child)
            .width(Length::points(200.0))
            .height(Length::points(50.0));
        tx.set_children(NodeId::ROOT, &[child]);
        tx.commit_into(&mut changes);
    }
    let mut fresh_kf = Vec::new();
    WireEncoder::default().encode_full(&fresh, &mut fresh_kf);
    // SAFETY: Valid handle, slice, and out pointers.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&fresh_kf)),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!((api.scene_acquire)(h, &mut view), NveRuntimeStatus::Ok.as_i32());
        assert!(view.node_capacity >= 3 && view.node_capacity < 99);
        let rects = std::slice::from_raw_parts(view.rects, view.node_capacity as usize);
        assert_eq!(rects[2].width, 200.0);
        assert_eq!(rects[2].height, 50.0);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
        (api.destroy)(h);
    }
}

#[test]
fn wake_bridge_clear_sink_waits_for_in_flight_callback() {
    let bridge = Arc::new(WakeBridge::new());
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let in_flight = Arc::new(AtomicBool::new(false));

    {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        let in_flight = Arc::clone(&in_flight);
        bridge.set_sink(Arc::new(move || {
            in_flight.store(true, Ordering::Release);
            entered.store(true, Ordering::Release);
            while !release.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            in_flight.store(false, Ordering::Release);
        }));
    }

    let b_worker = Arc::clone(&bridge);
    let trigger_thread = std::thread::spawn(move || {
        b_worker.trigger();
    });

    while !entered.load(Ordering::Acquire) {
        std::thread::yield_now();
    }

    let clear_finished = Arc::new(AtomicBool::new(false));
    let b_clear = Arc::clone(&bridge);
    let cf = Arc::clone(&clear_finished);
    let clear_thread = std::thread::spawn(move || {
        b_clear.clear_sink();
        cf.store(true, Ordering::Release);
    });

    std::thread::sleep(Duration::from_millis(25));
    assert!(
        !clear_finished.load(Ordering::Acquire),
        "clear_sink() MUST block while a wake callback is still in flight"
    );
    assert!(in_flight.load(Ordering::Acquire));

    release.store(true, Ordering::Release);
    trigger_thread.join().expect("trigger thread");
    clear_thread.join().expect("clear thread");
    assert!(clear_finished.load(Ordering::Acquire));
    assert!(!in_flight.load(Ordering::Acquire));
}

#[test]
fn c_abi_all_entry_points_null_handles_validation_scroll_viewport_and_threading() {
    assert!(nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION + 1).is_null());
    let api = get_api();
    assert_eq!(api.abi_version, NVE_RUNTIME_ABI_VERSION);
    assert_eq!(api.schema_hash, NVE_RUNTIME_SCHEMA_HASH);

    // 1. Null handle guards across all handle-taking entry points.
    let null_h = std::ptr::null_mut();
    let mut u32_out = 0u32;
    let mut f32_out = 0.0f32;
    let mut view = NveSceneView::default();
    let mut metrics = NveRuntimeMetrics::default();
    // SAFETY: Null handles are explicitly guarded by every C ABI entry point.
    unsafe {
        (api.destroy)(null_h);
        assert_eq!(
            (api.set_wake_callback)(null_h, None, std::ptr::null_mut()),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.set_viewport)(null_h, 400.0, 300.0, 1.0),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.attach_app_interface)(null_h, None),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.submit_nve2)(null_h, NveBufferSlice::EMPTY),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.tick)(null_h, 0.016, &mut u32_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.dispatch_events)(null_h, NveEventBatch::EMPTY, &mut u32_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.hit_test)(null_h, 0.0, 0.0, &mut u32_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.scroll_by)(null_h, 1, 0.0, 10.0, &mut f32_out, &mut f32_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.scene_acquire)(null_h, &mut view),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.scene_release)(null_h),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.get_metrics)(null_h, &mut metrics),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.pair_connect)(null_h, NveStrSlice::EMPTY, NveStrSlice::EMPTY),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.pair_disconnect)(null_h),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!((api.pair_is_connected)(null_h), 0);
    }

    // 2. Live handle validation, wake callback, viewport bump, scroll_by, spurious release, pairing.
    let cfg = NveRuntimeConfig {
        surface_id: 7,
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Valid stack config.
    let h = unsafe { (api.create)(&cfg) };
    assert!(!h.is_null());

    // Invalid `NveBufferSlice` (null ptr with non-zero len, or len > isize::MAX).
    let bad_buf_null = NveBufferSlice {
        ptr: std::ptr::null(),
        len: 8,
    };
    let bad_buf_huge = NveBufferSlice {
        ptr: &1u8 as *const u8,
        len: usize::MAX,
    };
    // SAFETY: Guarded inside `try_as_slice` before dereference.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, bad_buf_null),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.submit_nve2)(h, bad_buf_huge),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
    }

    // Invalid `NveEventBatch` (null ptr with non-zero len, and misaligned pointer).
    let bad_batch_null = NveEventBatch {
        ptr: std::ptr::null(),
        len: 2,
        _pad: 0,
    };
    let unaligned_storage = [0u64; 2];
    let misaligned_ptr = unaligned_storage
        .as_ptr()
        .cast::<u8>()
        .wrapping_add(1)
        .cast::<NveDirectEvent>();
    let bad_batch_align = NveEventBatch {
        ptr: misaligned_ptr,
        len: 1,
        _pad: 0,
    };
    // SAFETY: Guarded inside `NveEventBatchRef::try_from_raw` before dereference.
    unsafe {
        assert_eq!(
            (api.dispatch_events)(h, bad_batch_null, &mut u32_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.dispatch_events)(h, bad_batch_align, &mut u32_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
    }

    // Invalid viewport dimensions.
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!(
            (api.set_viewport)(h, 0.0, 300.0, 1.0),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.set_viewport)(h, 400.0, f32::NAN, 1.0),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.set_viewport)(h, 400.0, 300.0, -1.0),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
    }

    // Submit scrollable tree (1 scroll container id=2 of 200x100 with 6 children of 50px = 300px).
    let frame = build_nve2_with_nodes(6, true);
    // SAFETY: Valid handle and slice.
    let sub_rc = unsafe { (api.submit_nve2)(h, NveBufferSlice::from_slice(&frame)) };
    assert_eq!(sub_rc, NveRuntimeStatus::Ok.as_i32());

    // Valid `scroll_by` on scroll container (id=2), `NotFound` on non-scroll child (id=3) & unknown (999).
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    // SAFETY: Valid handle and stack out pointers.
    unsafe {
        assert_eq!(
            (api.scroll_by)(h, 2, 0.0, 75.0, &mut sx, &mut sy),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!((sx, sy), (0.0, 75.0));
        assert_eq!(
            (api.scroll_by)(h, 3, 0.0, 10.0, &mut sx, &mut sy),
            NveRuntimeStatus::NotFound.as_i32()
        );
        assert_eq!(
            (api.scroll_by)(h, 999, 0.0, 10.0, &mut sx, &mut sy),
            NveRuntimeStatus::NotFound.as_i32()
        );
        assert_eq!(
            (api.scroll_by)(h, 2, f32::NAN, 0.0, &mut sx, &mut sy),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        // Non-mutating `scroll_by` errors (`NotFound`, `InvalidArg`) MUST preserve `published_view`!
        let mut after_scroll_err = NveSceneView::default();
        assert_eq!(
            (api.scene_acquire)(h, &mut after_scroll_err),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(
            !after_scroll_err.rows.is_null() && after_scroll_err.node_capacity >= 8,
            "scroll_by validation errors must not blank published_view"
        );
        // Owner-thread `hit_test` MUST succeed even while `scene_acquire` reader lock is held.
        assert_eq!(
            (api.hit_test)(h, 20.0, 20.0, &mut u32_out),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(u32_out != 0);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // `set_viewport` + `tick` must advance `frame_id`.
    // SAFETY: Valid handle.
    unsafe {
        assert_eq!(
            (api.set_viewport)(h, 640.0, 480.0, 2.0),
            NveRuntimeStatus::Ok.as_i32()
        );
        let mut changed = 0u32;
        assert_eq!(
            (api.tick)(h, 0.016, &mut changed),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(changed, 1);
        assert_eq!(
            (api.get_metrics)(h, &mut metrics),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(metrics.frame_id, 2);
    }

    // Spurious `scene_release` without matching `scene_acquire` returns `InvalidArg`.
    // SAFETY: Valid handle.
    let sp_rc = unsafe { (api.scene_release)(h) };
    assert_eq!(sp_rc, NveRuntimeStatus::InvalidArg.as_i32());

    // `set_wake_callback` + cross-thread wake + clear.
    static WAKE_HITS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn test_wake_cb(ctx: *mut c_void) {
        let counter = ctx.cast::<AtomicUsize>();
        // SAFETY: `ctx` points to static `WAKE_HITS`.
        unsafe { (*counter).fetch_add(1, Ordering::Release) };
    }
    WAKE_HITS.store(0, Ordering::Release);
    // SAFETY: Valid handle and static callback/context.
    unsafe {
        assert_eq!(
            (api.set_wake_callback)(
                h,
                Some(test_wake_cb),
                (&raw const WAKE_HITS).cast_mut().cast::<c_void>(),
            ),
            NveRuntimeStatus::Ok.as_i32()
        );
        let rt = &*(h.cast::<SurfaceRuntime>());
        let wb = Arc::clone(rt.wake_bridge());
        std::thread::spawn(move || wb.trigger()).join().unwrap();
        assert_eq!(WAKE_HITS.load(Ordering::Acquire), 1);
        assert_eq!(
            (api.set_wake_callback)(h, None, std::ptr::null_mut()),
            NveRuntimeStatus::Ok.as_i32()
        );
    }

    // Pairing validation & feature-gated status contracts on owner thread.
    // SAFETY: Valid handle.
    unsafe {
        #[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
        {
            assert_eq!(
                (api.pair_connect)(h, NveStrSlice::EMPTY, NveStrSlice::EMPTY),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!((api.pair_disconnect)(h), NveRuntimeStatus::Ok.as_i32());
            assert_eq!((api.pair_is_connected)(h), 0);
        }
        #[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
        {
            assert_eq!(
                (api.pair_connect)(h, NveStrSlice::EMPTY, NveStrSlice::EMPTY),
                NveRuntimeStatus::Unsupported.as_i32()
            );
            assert_eq!(
                (api.pair_disconnect)(h),
                NveRuntimeStatus::Unsupported.as_i32()
            );
            assert_eq!((api.pair_is_connected)(h), 0);
        }
    }

    // 3. Cross-thread contract: mutating & layout/app calls (including `hit_test`) from another
    //    thread MUST fail with `InvalidArg`, while `scene_acquire`/`scene_release`/`get_metrics` succeed!
    let h_addr = h as usize;
    std::thread::spawn(move || {
        let h_bg = h_addr as *mut c_void;
        let mut out_id = 0u32;
        let mut out_changed = 0u32;
        let mut out_v = NveSceneView::default();
        let mut out_m = NveRuntimeMetrics::default();
        // SAFETY: Handle stays alive on main thread until this thread joins.
        unsafe {
            assert_eq!(
                (api.tick)(h_bg, 0.016, &mut out_changed),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert!(read_last_error(&api).contains("non-owner thread"));
            assert_eq!(
                (api.hit_test)(h_bg, 20.0, 20.0, &mut out_id),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert!(read_last_error(&api).contains("non-owner thread"));
            assert_eq!(
                (api.set_viewport)(h_bg, 500.0, 500.0, 1.0),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.submit_nve2)(h_bg, NveBufferSlice::EMPTY),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.scroll_by)(h_bg, 2, 0.0, 5.0, std::ptr::null_mut(), std::ptr::null_mut()),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.dispatch_events)(h_bg, NveEventBatch::EMPTY, &mut out_changed),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.attach_app_interface)(h_bg, None),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.pair_connect)(h_bg, NveStrSlice::EMPTY, NveStrSlice::EMPTY),
                NveRuntimeStatus::InvalidArg.as_i32()
            );

            // Read-only cross-thread calls MUST succeed!
            assert_eq!(
                (api.scene_acquire)(h_bg, &mut out_v),
                NveRuntimeStatus::Ok.as_i32()
            );
            assert_eq!(out_v.frame_id, 2);
            assert_eq!(
                (api.get_metrics)(h_bg, &mut out_m),
                NveRuntimeStatus::Ok.as_i32()
            );
            assert_eq!(out_m.frame_id, 2);
            assert_eq!((api.scene_release)(h_bg), NveRuntimeStatus::Ok.as_i32());
        }
    })
    .join()
    .expect("cross-thread probe");

    // SAFETY: Destroy once on owner thread.
    unsafe { (api.destroy)(h) };
}

struct PanickingHook;

impl FrameTickHook for PanickingHook {
    fn tick(&mut self, _tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        #[expect(
            clippy::panic,
            reason = "intentional panic to verify C ABI catch_unwind + last_error + poisoning"
        )]
        {
            panic!("intentional hook explosion");
        }
    }
}

#[test]
fn c_abi_caught_panic_records_last_error_and_poisons_surface() {
    let api = get_api();
    let rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .tick_hook(Box::new(PanickingHook))
        .build();
    let h = Box::into_raw(Box::new(rt)).cast::<c_void>();

    let mut changed = 0u32;
    // SAFETY: Valid handle on owner thread; panic is caught inside `rt_tick`.
    let rc = unsafe { (api.tick)(h, 0.016, &mut changed) };
    assert_eq!(rc, NveRuntimeStatus::Panic.as_i32());
    let err = read_last_error(&api);
    assert!(
        err.contains("intentional hook explosion"),
        "expected caught panic message in last_error, got `{err}`"
    );

    // Subsequent `scene_acquire` on the poisoned runtime must fail cleanly rather than expose torn state.
    let mut view = NveSceneView::default();
    // SAFETY: Valid handle.
    let acq_rc = unsafe { (api.scene_acquire)(h, &mut view) };
    assert_eq!(acq_rc, NveRuntimeStatus::Panic.as_i32());

    // SAFETY: Destroy frees poisoned runtime cleanly on owner thread.
    unsafe { (api.destroy)(h) };
}

#[test]
fn swap_layout_engine_bumps_frame_id_and_reconciles_scroll_offsets() {
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .build();
    let frame = build_nve2_with_nodes(6, true);
    rt.submit_nve2(&frame).expect("submit");
    assert_eq!(rt.acquire_scene().expect("acquire 1").view().frame_id, 1);

    let (sx, sy) = rt.scroll_by(2, 0.0, 50.0).expect("scroll");
    assert_eq!((sx, sy), (0.0, 50.0));

    rt.swap_layout_engine(Box::new(TaffyStoreLayout::new()))
        .expect("swap");
    let guard = rt.acquire_scene().expect("acquire 2");
    assert_eq!(
        guard.view().frame_id,
        2,
        "swap_layout_engine must advance frame_id"
    );
    assert!(guard.view().node_capacity >= 8 && !guard.view().rects.is_null());
    // SAFETY: `guard` holds the reader lock and `node_capacity >= 8`.
    let (container_rect, child0_rect) = unsafe { (*guard.view().rects.add(2), *guard.view().rects.add(3)) };
    assert_eq!((container_rect.width, container_rect.height), (200.0, 100.0));
    assert_eq!((child0_rect.width, child0_rect.height), (200.0, 50.0));
    assert_eq!(rt.layout_engine_name().expect("engine name"), "taffy");
    drop(guard);

    // Scroll offset `(0, 50)` must remain preserved and clamped after `swap_layout_engine`.
    assert_eq!(rt.scroll_by(2, 0.0, 0.0).expect("query scroll"), (0.0, 50.0));

    // Confirm scroll_by returns NotFound on a missing node in Rust API too.
    assert!(matches!(
        rt.scroll_by(9999, 0.0, 10.0),
        Err(RuntimeError::NotFound(_))
    ));
    let _ = NativeEventKind::Press;
}

/// Direct-mode `RuntimeApp` whose `dispatch_events` reallocates the sparse `Scroll` column
/// (`rows` + `owners`) WITHOUT modifying `node_rows`, `text_slab`, or `children_slab` (P0 #1).
struct TableReallocInEventApp {
    store: NodeStore,
    changes: ChangeSet,
    nodes: Vec<NodeId>,
    mounted: bool,
}

impl TableReallocInEventApp {
    fn new() -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let mut nodes = Vec::with_capacity(64);
        {
            let mut tx = store.tx();
            for _ in 0..64 {
                let id = tx.create();
                tx.edit::<Layout>(id)
                    .width(Length::points(20.0))
                    .height(Length::points(10.0));
                nodes.push(id);
            }
            // Attach sparse `Scroll` on ONLY node 0 initially (`row_count == 1`).
            tx.edit::<Scroll>(nodes[0]);
            tx.set_children(NodeId::ROOT, &nodes);
            tx.commit_into(&mut changes);
        }
        Self {
            store,
            changes,
            nodes,
            mounted: false,
        }
    }
}

impl RuntimeApp for TableReallocInEventApp {
    fn render_frame(&mut self, _frame_id: u64) -> AppFrame<'_> {
        if !self.mounted {
            self.mounted = true;
            AppFrame::Direct {
                changes: &self.changes,
            }
        } else {
            AppFrame::Clean
        }
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        // Attach sparse `Scroll` on all 63 remaining existing nodes: reallocates `Scroll`'s
        // `rows` and `owners` Vecs WITHOUT reallocating `node_rows`, `text_slab`, or `children_slab`!
        let mut tx = self.store.tx();
        for &id in &self.nodes[1..] {
            tx.edit::<Scroll>(id);
        }
        tx.commit_into(&mut self.changes);
        true
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

#[test]
fn direct_app_table_column_reallocation_in_dispatch_events_invalidates_published_view() {
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .app(Box::new(TableReallocInEventApp::new()))
        .build();

    assert!(rt.tick(0.016).expect("initial mount tick"));
    let (orig_node_rows, orig_scroll_rows, orig_scroll_owners) = {
        let g = rt.acquire_scene().expect("acquire 1");
        let v = g.view();
        assert!(!v.tables.is_null() && v.table_count >= 1);
        // SAFETY: `g` holds the reader lock for `table_count` entries.
        let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
        let scr = tables
            .iter()
            .find(|t| t.slot == Scroll::slot().0 as u32)
            .expect("Scroll table published");
        assert_eq!(scr.row_count, 1);
        assert!(!scr.rows.is_null() && !scr.owners.is_null());
        (v.rows, scr.rows, scr.owners)
    };

    // Trigger sparse `Scroll` column reallocation during `dispatch_events` (without touching node_rows).
    let ev = [NveDirectEvent::press(1, 2, 100, 5.0, 5.0)];
    // SAFETY: `NveDirectEvent::press` has no borrowed pointers.
    let batch = unsafe { NveEventBatchRef::from_slice(&ev) };
    assert!(rt.dispatch_events(batch).expect("dispatch_events"));

    // `acquire_scene` BEFORE the next `tick` MUST return a cleared view rather than dangling pointers!
    {
        let g = rt.acquire_scene().expect("acquire after table realloc");
        assert!(
            g.view().tables.is_null() && g.view().table_count == 0 && g.view().rows.is_null(),
            "reallocating Scroll table column in dispatch_events must invalidate published_view"
        );
    }

    // Next `tick` republishes fresh valid `Scroll` table view with all 64 rows while `node_rows`
    // never moved (proving the catch came strictly from `published_tables` validation!).
    assert!(rt.tick(0.016).expect("republish tick"));
    {
        let g = rt.acquire_scene().expect("acquire after republish");
        let v = g.view();
        assert_eq!(v.rows, orig_node_rows, "node_rows must not have moved");
        // SAFETY: `g` holds the reader lock for `table_count` entries.
        let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
        let scr = tables
            .iter()
            .find(|t| t.slot == Scroll::slot().0 as u32)
            .expect("Scroll table republished");
        assert_eq!(scr.row_count, 64);
        assert_ne!(scr.rows, orig_scroll_rows);
        assert_ne!(scr.owners, orig_scroll_owners);
    }
}

/// Direct app + `FrameTickHook` exercising create-in-app -> remove-in-hook within ONE `tick()`
/// AND duplicate `Visual` edits across app + hook (regression test for P1 #4 and P2 #2).
struct CreateInAppApp {
    store: NodeStore,
    changes: ChangeSet,
    persistent: NodeId,
    mounted: bool,
}

impl CreateInAppApp {
    fn new() -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let persistent = {
            let mut tx = store.tx();
            let p = tx.create();
            tx.edit::<Layout>(p)
                .width(Length::points(100.0))
                .height(Length::points(40.0));
            tx.edit::<Visual>(p).opacity(1.0);
            let transient = tx.create();
            tx.edit::<Layout>(transient)
                .width(Length::points(50.0))
                .height(Length::points(20.0));
            tx.edit::<Visual>(transient).opacity(0.9);
            tx.set_children(NodeId::ROOT, &[p, transient]);
            tx.commit_into(&mut changes);
            p
        };
        Self {
            store,
            changes,
            persistent,
            mounted: false,
        }
    }
}

impl RuntimeApp for CreateInAppApp {
    fn render_frame(&mut self, _frame_id: u64) -> AppFrame<'_> {
        if !self.mounted {
            self.mounted = true;
            AppFrame::Direct {
                changes: &self.changes,
            }
        } else {
            AppFrame::Clean
        }
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

struct RemoveTransientAndEditPersistentHook {
    persistent: NodeId,
}

impl FrameTickHook for RemoveTransientAndEditPersistentHook {
    fn tick(&mut self, tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        let kids: Vec<NodeId> = tx.store().children(NodeId::ROOT).to_vec();
        if kids.len() == 2 {
            let transient = kids[1];
            tx.set_children(NodeId::ROOT, &[self.persistent]);
            tx.remove(transient);
            tx.edit::<Visual>(self.persistent).opacity(0.4);
            return true;
        }
        false
    }
}

#[test]
fn app_create_then_hook_remove_cancels_cleanly_and_deduplicates_table_dirty_rows() {
    let app = CreateInAppApp::new();
    let persistent = app.persistent;
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .app(Box::new(app))
        .tick_hook(Box::new(RemoveTransientAndEditPersistentHook { persistent }))
        .build();

    assert!(rt.tick(0.016).expect("app+hook tick"));
    let guard = rt.acquire_scene().expect("acquire scene");
    let v = guard.view();
    assert_eq!(v.removed_count, 0, "transient node born in app and removed in hook must cancel");
    // SAFETY: `guard` holds the reader lock for `dirty_count` indices.
    let dirty = unsafe { std::slice::from_raw_parts(v.dirty_indices, v.dirty_count as usize) };
    for &idx in dirty {
        // SAFETY: `idx < v.node_capacity`.
        let row = unsafe { *v.rows.add(idx as usize) };
        assert!(row.is_live(), "dead transient slot {idx} leaked into dirty_indices");
    }
    // SAFETY: `guard` holds the reader lock for `table_count` entries.
    let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
    let vis = tables
        .iter()
        .find(|t| t.slot == Visual::slot().0 as u32)
        .expect("Visual table published");
    assert_eq!(
        vis.dirty_count, 1,
        "duplicate Visual edits in app + hook must deduplicate to 1 dirty row"
    );
}

struct TwoPumpTransientApp {
    store: NodeStore,
    changes: ChangeSet,
    persistent: NodeId,
    transient_idx: usize,
    phase: u8,
}

impl TwoPumpTransientApp {
    fn new() -> Self {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let persistent = {
            let mut tx = store.tx();
            let p = tx.create();
            tx.edit::<Layout>(p)
                .width(Length::points(120.0))
                .height(Length::points(40.0));
            tx.edit::<Visual>(p).opacity(1.0);
            tx.set_children(NodeId::ROOT, &[p]);
            tx.commit_into(&mut changes);
            p
        };
        Self {
            store,
            changes,
            persistent,
            transient_idx: 0,
            phase: 0,
        }
    }
}

impl RuntimeApp for TwoPumpTransientApp {
    fn poll_async_work(&mut self) -> bool {
        if self.phase == 1 {
            self.phase = 2;
            true
        } else {
            false
        }
    }

    fn render_frame(&mut self, _frame_id: u64) -> AppFrame<'_> {
        match self.phase {
            0 => {
                // Pump 1: create a transient node AND edit `persistent`'s `Visual`.
                self.phase = 1;
                let mut tx = self.store.tx();
                let t = tx.create();
                self.transient_idx = t.index();
                tx.edit::<Layout>(t)
                    .width(Length::points(40.0))
                    .height(Length::points(20.0));
                tx.edit::<Visual>(t).opacity(0.9);
                tx.edit::<Visual>(self.persistent).opacity(0.7);
                tx.set_children(NodeId::ROOT, &[self.persistent, t]);
                tx.commit_into(&mut self.changes);
                AppFrame::Direct {
                    changes: &self.changes,
                }
            }
            2 => {
                // Pump 2 in the SAME `tick()`: remove the transient node AND edit `persistent`'s
                // `Visual` a second time (producing duplicate `Visual` dirty row before dedup!).
                self.phase = 3;
                let transient = self.store.children(NodeId::ROOT)[1];
                let mut tx = self.store.tx();
                tx.set_children(NodeId::ROOT, &[self.persistent]);
                tx.remove(transient);
                tx.edit::<Visual>(self.persistent).opacity(0.3);
                tx.commit_into(&mut self.changes);
                AppFrame::Direct {
                    changes: &self.changes,
                }
            }
            _ => AppFrame::Clean,
        }
    }

    fn dispatch_events(&mut self, _events: NveEventBatchRef<'_>) -> bool {
        false
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

#[test]
fn two_pump_create_then_remove_and_duplicate_table_edits_are_clean_and_deduplicated() {
    let app = TwoPumpTransientApp::new();
    let persistent = app.persistent;
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .app(Box::new(app))
        .tick_hook(Box::new(RemoveTransientAndEditPersistentHook { persistent }))
        .build();

    // Single `tick()` runs Pump 1 (create transient + edit Visual) AND Pump 2 (remove transient +
    // edit Visual again).
    assert!(rt.tick(0.016).expect("two-pump tick"));
    let guard = rt.acquire_scene().expect("acquire scene");
    let v = guard.view();
    assert_eq!(v.removed_count, 0, "transient node born and killed in same tick must cancel out");
    // SAFETY: `guard` holds the reader lock for `dirty_count` items.
    let dirty = unsafe { std::slice::from_raw_parts(v.dirty_indices, v.dirty_count as usize) };
    // Every dirty index must point to a LIVE row (never the dead transient slot 3!).
    for &idx in dirty {
        // SAFETY: `idx < v.node_capacity`.
        let row = unsafe { *v.rows.add(idx as usize) };
        assert!(row.is_live(), "dead transient slot {idx} leaked into dirty_indices");
    }
    // `Visual` table `dirty_rows` must be deduplicated (no duplicate row index for `persistent`)!
    // SAFETY: `guard` holds the reader lock for `table_count` entries.
    let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
    let vis = tables
        .iter()
        .find(|t| t.slot == Visual::slot().0 as u32)
        .expect("Visual table published");
    assert_eq!(vis.dirty_count, 1, "duplicate Visual edits across pumps must deduplicate to 1 dirty row");
    // SAFETY: `vis.dirty_count == 1` and `guard` holds the reader lock.
    let vis_dirty = unsafe { std::slice::from_raw_parts(vis.dirty_rows, vis.dirty_count as usize) };
    assert_eq!(vis_dirty.len(), 1);
}

struct Utf8PanickingHook;

impl FrameTickHook for Utf8PanickingHook {
    fn tick(&mut self, _tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        #[expect(
            clippy::panic,
            reason = "intentional UTF-8 panic to verify rt_last_error char-boundary truncation"
        )]
        {
            panic!("boom🔥é");
        }
    }
}

#[test]
fn rt_last_error_handles_null_cap1_utf8_truncation_and_clears_on_ok() {
    let api = get_api();
    let rt = RuntimeBuilder::new()
        .tick_hook(Box::new(Utf8PanickingHook))
        .build();
    let h = Box::into_raw(Box::new(rt)).cast::<c_void>();

    let mut changed = 0u32;
    // SAFETY: Valid handle on owner thread; triggers caught panic with multi-byte UTF-8 payload.
    let tick_rc = unsafe { (api.tick)(h, 0.016, &mut changed) };
    assert_eq!(tick_rc, NveRuntimeStatus::Panic.as_i32());

    // 1. `(NULL, 0)` returns full byte length without writing.
    // SAFETY: Null pointer with cap=0 is explicitly supported for length queries.
    let full_len = unsafe { (api.last_error)(std::ptr::null_mut(), 0) };
    assert!(full_len > 10);

    // 2. `cap == 1` writes only NUL terminator while returning full byte length.
    let mut tiny = [0xAAu8; 1];
    // SAFETY: Valid 1-byte buffer.
    let len1 = unsafe { (api.last_error)(tiny.as_mut_ptr(), tiny.len()) };
    assert_eq!(len1, full_len);
    assert_eq!(tiny[0], 0);

    // 3. Every small truncation capacity `2..=full_len` must produce valid NUL-terminated UTF-8!
    let mut buf = vec![0xFFu8; full_len + 2];
    for cap in 2..=full_len {
        buf.fill(0xFF);
        // SAFETY: Valid buffer of length `cap`.
        let reported = unsafe { (api.last_error)(buf.as_mut_ptr(), cap) };
        assert_eq!(reported, full_len);
        let nul_pos = buf[..cap]
            .iter()
            .position(|&b| b == 0)
            .expect("must NUL-terminate within cap");
        assert!(
            std::str::from_utf8(&buf[..nul_pos]).is_ok(),
            "rt_last_error split a multi-byte UTF-8 sequence at cap={cap}: {:?}",
            &buf[..nul_pos]
        );
    }

    // 4. Subsequent `Ok` C ABI call clears `last_error` back to 0 bytes.
    // SAFETY: Valid handle; `set_wake_callback(NULL)` succeeds even on poisoned handle.
    let clear_rc = unsafe { (api.set_wake_callback)(h, None, std::ptr::null_mut()) };
    assert_eq!(clear_rc, NveRuntimeStatus::Ok.as_i32());
    // SAFETY: Length query with null/0.
    let cleared_len = unsafe { (api.last_error)(std::ptr::null_mut(), 0) };
    assert_eq!(cleared_len, 0);

    // SAFETY: Destroy handle once on owner thread.
    unsafe { (api.destroy)(h) };
}
