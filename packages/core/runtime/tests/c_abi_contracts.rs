//! Comprehensive C ABI, memory-safety, concurrency, panic-boundary, thread-affinity, and
//! validation contract tests for `nve-runtime`.

#[path = "c_abi_contracts/direct_app.rs"]
mod direct_app;
#[path = "c_abi_contracts/entry_points.rs"]
mod entry_points;

use std::ffi::c_void;

use nve_abi::{
    EventBatchBuf, FrameId, InteractionEvent, InterfaceId, NveBufferSlice, NveRuntimeApi,
    NveRuntimeConfig, NveRuntimeMetrics, NveRuntimeStatus, NveSceneView, SurfaceId, TimestampUs,
    ValidatedEvents, NVE_RUNTIME_ABI_VERSION,
};
use nve_engine::NveApp;
use nve_runtime::nve_runtime_get_interface;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{Interaction, Layout, Scroll, TextContent, Visual};

pub(crate) fn get_api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable returned for matching ABI version.
    unsafe { *ptr }
}

pub(crate) fn read_last_error(api: &NveRuntimeApi) -> String {
    let mut buf = [0u8; 512];
    // SAFETY: Stack buffer valid for 512 bytes.
    let len = unsafe { (api.last_error)(buf.as_mut_ptr(), buf.len()) };
    let n = len.min(buf.len() - 1);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

pub(crate) fn build_nve2_with_nodes(count: usize, with_scroll: bool) -> Vec<u8> {
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
            tx.edit::<Interaction>(item).listen(InteractionEvent::Press);
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
    fn create(_surface_id: SurfaceId) -> Self {
        Self {
            clicks: 0,
            store: NodeStore::new(),
            changes: ChangeSet::default(),
            enc: WireEncoder::default(),
            buf: Vec::new(),
            dirty: true,
        }
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
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

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
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
    fn nve_runtime_test_mini_app_get_interface(interface_id: InterfaceId) -> *const c_void;
}

unsafe extern "C" fn failing_null_surface_get_interface(_interface_id: InterfaceId) -> *const c_void {
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
    let mut ev_buf = EventBatchBuf::new();
    ev_buf.push_press(SurfaceId::ROOT, NodeId::new(2, 0), TimestampUs(100), 10.0, 10.0);
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
            (api.dispatch_events)(h, ev_buf.as_raw_batch(), &mut ev_changed),
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
    nve_tables::tables::register_core_tables();
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
    let child = NodeId::new(2, 0);
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
        let leaked = NodeId::new(99, 0);
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
