//! Runtime ABI v6 measurement, intrinsic-size invalidation, and Frame-0 save/boot contract tests.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use nve_abi::{
    FrameId, InterfaceId, NveBufferSlice, NveEventBatch, NveHostCallbacks, NveIntrinsicSize,
    NveLayoutApi, NveRuntimeApi, NveRuntimeConfig, NveRuntimeHandle, NveRuntimeStatus,
    NveSceneView, NveSizeConstraints, NveStrSlice, NveSurfaceApi, NveSurfaceMeasure, SurfaceId,
    ValidatedEvents, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE, NVE_MEASURE_FLAG_DRY_RUN,
    NVE_MEASURE_FLAG_IDEAL, NVE_RUNTIME_ABI_VERSION,
};
use nve_engine::persistence::{Frame0File, Frame0Footer, Frame0Meta, FRAME0_FOOTER_SIZE};
use nve_engine::{AppFrame, FrameTickHook, RuntimeApp, Viewport};
use nve_layout::{HeuristicTextMeasurer, SizeConstraints, TaffyStoreLayout};
use nve_runtime::{nve_runtime_get_interface, RuntimeBuilder, RuntimeError, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba, Transaction};
use nve_tables::tables::{
    core_tables_schema_hash, AlignItems, FlexDirection, Layout, TextContent, Visual,
};

const OK: i32 = NveRuntimeStatus::Ok as i32;
const INVALID: i32 = NveRuntimeStatus::InvalidArg as i32;
const NOT_FOUND: i32 = NveRuntimeStatus::NotFound as i32;
const BUSY: i32 = NveRuntimeStatus::Busy as i32;

fn get_api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable for matching Runtime ABI v6.
    unsafe { *ptr }
}

fn str_slice(path: &Path) -> NveStrSlice {
    NveStrSlice::from_str(path.to_str().expect("UTF-8 test path"))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("nve-v6-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn create_handle(api: &NveRuntimeApi, w: f32, h: f32) -> NveRuntimeHandle {
    let cfg = NveRuntimeConfig {
        surface_id: 1,
        width: w,
        height: h,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Stack config outlives `create`.
    let handle = unsafe { (api.create)(&cfg) };
    assert!(!handle.is_null());
    handle
}

fn build_card_tree(card_w: f32, card_h: f32, label_text: &str) -> (NodeStore, NodeId, NodeId, Vec<u8>) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (card, label) = {
        let mut tx = store.tx();
        let card = tx.create();
        tx.edit::<Layout>(card)
            .width(Length::points(card_w))
            .height(Length::points(card_h));
        tx.edit::<Visual>(card).background(Rgba::rgb(20, 30, 40));
        let label = tx.create();
        tx.edit::<TextContent>(label).text(label_text);
        tx.set_children(card, &[label]);
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut changes);
        (card, label)
    };
    let mut enc = WireEncoder::default();
    let mut buf = Vec::new();
    enc.encode(&store, &changes, &mut buf);
    (store, card, label, buf)
}

unsafe extern "C" fn layout_app_create(_: SurfaceId) -> *mut c_void {
    Box::into_raw(Box::new(1u32)).cast()
}
unsafe extern "C" fn layout_app_destroy(h: *mut c_void) {
    if !h.is_null() {
        // SAFETY: Allocated by `layout_app_create`.
        drop(unsafe { Box::from_raw(h.cast::<u32>()) });
    }
}
unsafe extern "C" fn layout_app_render(_: *mut c_void, _: FrameId) -> NveBufferSlice {
    NveBufferSlice::EMPTY
}
unsafe extern "C" fn layout_app_dispatch(_: *mut c_void, _: NveEventBatch) -> u32 {
    0
}
unsafe extern "C" fn layout_app_measure(_: *mut c_void, c: NveSizeConstraints) -> NveIntrinsicSize {
    NveIntrinsicSize {
        width: 333.0f32.clamp(c.min_w, c.max_w),
        height: 77.0f32.clamp(c.min_h, c.max_h),
    }
}
unsafe extern "C" fn layout_app_hit_test(_: *mut c_void, _: f32, _: f32, _: u8) -> u8 {
    0
}
static LAYOUT_SURFACE_API: NveSurfaceApi = NveSurfaceApi {
    create: layout_app_create,
    destroy: layout_app_destroy,
    render: layout_app_render,
    dispatch_events: layout_app_dispatch,
};
static LAYOUT_LAYOUT_API: NveLayoutApi = NveLayoutApi {
    measure: layout_app_measure,
    hit_test: layout_app_hit_test,
};
unsafe extern "C" fn layout_app_get_interface(id: InterfaceId) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => (&raw const LAYOUT_SURFACE_API).cast(),
        NVE_IFACE_LAYOUT => (&raw const LAYOUT_LAYOUT_API).cast(),
        _ => std::ptr::null(),
    }
}

#[test]
fn measure_surface_dry_run_ideal_validation_and_layout_interface_delegation() {
    let api = get_api();
    let h = create_handle(&api, 400.0, 300.0);
    let (_, _, _, frame) = build_card_tree(220.0, 80.0, "Hello Measure");

    // SAFETY: Owner thread, valid pointers & handles.
    unsafe {
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&frame)), OK);

        let mut before = NveSceneView::default();
        assert_eq!((api.scene_acquire)(h, &mut before), OK);
        let before_frame_id = before.frame_id;
        let before_root_rect = *before.rects.add(1);
        let before_card_rect = *before.rects.add(2);
        let mut busy_out = NveSurfaceMeasure::default();
        let tight = NveSizeConstraints {
            min_w: 100.0,
            min_h: 30.0,
            max_w: 150.0,
            max_h: 60.0,
        };
        assert_eq!(
            (api.measure_surface)(h, &tight, NVE_MEASURE_FLAG_DRY_RUN, &mut busy_out),
            BUSY
        );
        assert_eq!((api.scene_release)(h), OK);

        // 1. Constrained DRY_RUN returns bounded size and leaves committed scene untouched.
        let mut constrained = NveSurfaceMeasure::default();
        assert_eq!(
            (api.measure_surface)(h, &tight, NVE_MEASURE_FLAG_DRY_RUN, &mut constrained),
            OK
        );
        assert!(
            (100.0..=150.0).contains(&constrained.width)
                && (30.0..=60.0).contains(&constrained.height),
            "constrained measure must respect bounds: {constrained:?}"
        );

        let mut after = NveSceneView::default();
        assert_eq!((api.scene_acquire)(h, &mut after), OK);
        assert_eq!(after.frame_id, before_frame_id);
        assert_eq!(*after.rects.add(1), before_root_rect);
        assert_eq!(*after.rects.add(2), before_card_rect);
        assert_eq!((api.scene_release)(h), OK);

        // 2. IDEAL ignores tight max bounds and returns unconstrained intrinsic dimensions.
        let mut ideal = NveSurfaceMeasure::default();
        assert_eq!(
            (api.measure_surface)(
                h,
                &tight,
                NVE_MEASURE_FLAG_DRY_RUN | NVE_MEASURE_FLAG_IDEAL,
                &mut ideal,
            ),
            OK
        );
        assert_eq!((ideal.width, ideal.height), (220.0, 80.0));

        // 3. Invalid flags, null pointers, NaN, and inverted min > max return `InvalidArg`.
        assert_eq!((api.measure_surface)(h, std::ptr::null(), 0, &mut ideal), INVALID);
        assert_eq!((api.measure_surface)(h, &tight, 0, std::ptr::null_mut()), INVALID);
        assert_eq!((api.measure_surface)(h, &tight, 1 << 5, &mut ideal), INVALID);
        let bad_nan = NveSizeConstraints {
            min_w: f32::NAN,
            min_h: 0.0,
            max_w: 100.0,
            max_h: 100.0,
        };
        let bad_inv = NveSizeConstraints {
            min_w: 200.0,
            min_h: 0.0,
            max_w: 100.0,
            max_h: 100.0,
        };
        assert_eq!((api.measure_surface)(h, &bad_nan, 0, &mut ideal), INVALID);
        assert_eq!((api.measure_surface)(h, &bad_inv, 0, &mut ideal), INVALID);

        // 4. App providing `NVE_IFACE_LAYOUT = 2` delegates `measure_surface`.
        assert_eq!((api.attach_app_interface)(h, Some(layout_app_get_interface)), OK);
        let loose = NveSizeConstraints::loose(500.0, 500.0);
        let mut delegated = NveSurfaceMeasure::default();
        assert_eq!(
            (api.measure_surface)(h, &loose, NVE_MEASURE_FLAG_DRY_RUN, &mut delegated),
            OK
        );
        assert_eq!((delegated.width, delegated.height), (333.0, 77.0));
        assert!(delegated.first_baseline.is_nan() && delegated.last_baseline.is_nan());

        (api.destroy)(h);
    }
}

#[test]
fn intrinsic_size_invalidated_fires_only_on_layout_changes_and_drains_cleanly() {
    static HITS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn on_invalidated(ctx: *mut c_void) {
        // SAFETY: `ctx` points to static `HITS`.
        unsafe { (*ctx.cast::<AtomicUsize>()).fetch_add(1, Ordering::Release) };
    }
    HITS.store(0, Ordering::Release);

    let api = get_api();
    let h = create_handle(&api, 400.0, 300.0);
    let (mut src, card, label, frame1) = build_card_tree(200.0, 80.0, "Initial");
    let mut enc = WireEncoder::default();
    let mut dummy = Vec::new();
    enc.encode_full(&src, &mut dummy);

    let encode_delta =
        |src: &mut NodeStore, enc: &mut WireEncoder, f: &dyn Fn(&mut nve_store::Transaction<'_>)| {
            let mut ch = ChangeSet::default();
            {
                let mut tx = src.tx();
                f(&mut tx);
                tx.commit_into(&mut ch);
            }
            let mut buf = Vec::new();
            enc.encode(src, &ch, &mut buf);
            buf
        };

    // SAFETY: Valid handle on owner thread.
    unsafe {
        assert_eq!(
            (api.set_intrinsic_size_invalidated_callback)(
                h,
                Some(on_invalidated),
                (&raw const HITS).cast_mut().cast(),
            ),
            OK
        );

        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&frame1)), OK);
        assert_eq!(HITS.load(Ordering::Acquire), 1);

        // Paint-only Visual mutation -> MUST NOT fire!
        let paint_only = encode_delta(&mut src, &mut enc, &|tx| {
            tx.edit::<Visual>(card).background(Rgba::rgb(255, 0, 128));
        });
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&paint_only)), OK);
        assert_eq!(HITS.load(Ordering::Acquire), 1);

        // Viewport resize + tick -> MUST NOT fire!
        assert_eq!((api.set_viewport)(h, 640.0, 480.0, 2.0), OK);
        let mut changed = 0u32;
        assert_eq!((api.tick)(h, 0.016, &mut changed), OK);
        assert_eq!(changed, 1);
        assert_eq!(HITS.load(Ordering::Acquire), 1);

        // Layout width change -> fires (2).
        let layout_edit = encode_delta(&mut src, &mut enc, &|tx| {
            tx.edit::<Layout>(card).width(Length::points(260.0));
        });
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&layout_edit)), OK);
        assert_eq!(HITS.load(Ordering::Acquire), 2);

        // TextContent edit -> fires (3).
        let text_edit = encode_delta(&mut src, &mut enc, &|tx| {
            tx.edit::<TextContent>(label).text("Longer intrinsic text string");
        });
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&text_edit)), OK);
        assert_eq!(HITS.load(Ordering::Acquire), 3);

        // Child create + set_children -> fires (4).
        let child_edit = encode_delta(&mut src, &mut enc, &|tx| {
            let extra = tx.create();
            tx.edit::<Layout>(extra)
                .width(Length::points(40.0))
                .height(Length::points(20.0));
            tx.set_children(card, &[label, extra]);
        });
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&child_edit)), OK);
        assert_eq!(HITS.load(Ordering::Acquire), 4);

        // Text measurer swap & layout engine swap -> fire (5, 6).
        let rt = &mut *(h.cast::<SurfaceRuntime>());
        rt.set_text_measurer(Some(Arc::new(HeuristicTextMeasurer)))
            .expect("set_text_measurer");
        assert_eq!(HITS.load(Ordering::Acquire), 5);
        rt.swap_layout_engine(Box::new(TaffyStoreLayout::new()))
            .expect("swap_layout_engine");
        assert_eq!(HITS.load(Ordering::Acquire), 6);

        // Unregister with NULL -> subsequent layout edits MUST NOT fire!
        assert_eq!(
            (api.set_intrinsic_size_invalidated_callback)(h, None, std::ptr::null_mut()),
            OK
        );
        let after_clear = encode_delta(&mut src, &mut enc, &|tx| {
            tx.edit::<Layout>(card).width(Length::points(300.0));
        });
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&after_clear)), OK);
        assert_eq!(HITS.load(Ordering::Acquire), 6);

        // Re-register then destroy -> destroy drains cleanly without firing.
        assert_eq!(
            (api.set_intrinsic_size_invalidated_callback)(
                h,
                Some(on_invalidated),
                (&raw const HITS).cast_mut().cast(),
            ),
            OK
        );
        (api.destroy)(h);
        assert_eq!(HITS.load(Ordering::Acquire), 6);
    }
}

#[test]
fn frame0_file_and_bytes_round_trip_and_auto_quarantine_mismatches() {
    let api = get_api();
    let dir = Scratch::new("frame0");
    let path = dir.0.join("nested").join("frame0.nve2");
    const BUILD_ID: u64 = 0xCAFE_BABE_0001;
    const ENV_HASH: u64 = 0xBEEF_F00D_0002;

    let h1 = create_handle(&api, 400.0, 300.0);
    // SAFETY: Owner thread, valid handles & slices.
    unsafe {
        assert_eq!((api.save_frame0_file)(h1, str_slice(&path), BUILD_ID, ENV_HASH), NOT_FOUND);
        assert!(!path.exists());

        let (_, _, _, frame) = build_card_tree(210.0, 90.0, "Hello Frame0");
        assert_eq!((api.submit_nve2)(h1, NveBufferSlice::from_slice(&frame)), OK);
        assert_eq!((api.save_frame0_file)(h1, str_slice(&path), BUILD_ID, ENV_HASH), OK);
        assert!(path.exists());
        let entries: Vec<_> = std::fs::read_dir(path.parent().expect("parent dir"))
            .expect("read parent dir")
            .collect();
        assert_eq!(entries.len(), 1, "atomic write leaves no temporary file");
        let saved = std::fs::read(&path).expect("read saved frame0");
        (api.destroy)(h1);

        // Boot from file into fresh surface + verify scene, text slab, and rect.
        let h2 = create_handle(&api, 400.0, 300.0);
        assert_eq!((api.boot_frame0_file)(h2, str_slice(&path), BUILD_ID, ENV_HASH), OK);
        let mut view = NveSceneView::default();
        assert_eq!((api.scene_acquire)(h2, &mut view), OK);
        assert_eq!(view.frame_id, 1);
        assert!(view.node_capacity >= 4);
        let rect2 = *view.rects.add(2);
        assert_eq!((rect2.width, rect2.height), (210.0, 90.0));
        let text = std::str::from_utf8(std::slice::from_raw_parts(
            view.text_slab,
            view.text_slab_len as usize,
        ))
        .expect("UTF-8 text slab");
        assert!(text.contains("Hello Frame0"), "text slab=`{text}`");
        // Busy check while scene is held MUST refuse without deleting the valid file!
        assert_eq!((api.boot_frame0_file)(h2, str_slice(&path), BUILD_ID, ENV_HASH), BUSY);
        assert!(path.exists(), "busy boot_frame0_file must leave file untouched");
        assert_eq!((api.scene_release)(h2), OK);
        (api.destroy)(h2);

        // Boot from in-memory bytes into fresh surface.
        let h3 = create_handle(&api, 400.0, 300.0);
        assert_eq!(
            (api.boot_frame0_bytes)(h3, NveBufferSlice::from_slice(&saved), BUILD_ID, ENV_HASH),
            OK
        );
        let mut view3 = NveSceneView::default();
        assert_eq!((api.scene_acquire)(h3, &mut view3), OK);
        assert_eq!(view3.frame_id, 1);
        assert_eq!((*view3.rects.add(2)).width, 210.0);
        assert_eq!((api.scene_release)(h3), OK);

        // 1. Corrupt NVE2 payload (valid magic, corrupt body) -> auto-removed from disk & NotFound.
        let mut corrupt = saved.clone();
        corrupt[16] ^= 0xFF;
        std::fs::write(&path, &corrupt).expect("write corrupt frame0");
        assert_eq!((api.boot_frame0_file)(h3, str_slice(&path), BUILD_ID, ENV_HASH), NOT_FOUND);
        assert!(!path.exists(), "corrupt NVE2 frame0 file must be auto-removed");
        assert_eq!(
            (api.boot_frame0_bytes)(h3, NveBufferSlice::from_slice(&corrupt), BUILD_ID, ENV_HASH),
            NOT_FOUND
        );

        // 1b. Non-NVE2 file (corrupted magic / arbitrary file) -> NotFound and PRESERVED on disk.
        let mut bad_magic = saved.clone();
        bad_magic[0] ^= 0xFF;
        std::fs::write(&path, &bad_magic).expect("write non-nve2 file");
        assert_eq!((api.boot_frame0_file)(h3, str_slice(&path), BUILD_ID, ENV_HASH), NOT_FOUND);
        assert!(path.exists(), "non-NVE2 file must not be deleted by boot_frame0_file");
        assert_eq!(
            (api.boot_frame0_bytes)(h3, NveBufferSlice::from_slice(&bad_magic), BUILD_ID, ENV_HASH),
            NOT_FOUND
        );

        // 2. app_build_id mismatch -> auto-removed & NotFound.
        std::fs::write(&path, &saved).expect("write frame0 for build_id mismatch");
        assert_eq!(
            (api.boot_frame0_file)(h3, str_slice(&path), BUILD_ID ^ 1, ENV_HASH),
            NOT_FOUND
        );
        assert!(!path.exists(), "build_id mismatch frame0 file must be auto-removed");
        assert_eq!(
            (api.boot_frame0_bytes)(h3, NveBufferSlice::from_slice(&saved), BUILD_ID ^ 1, ENV_HASH),
            NOT_FOUND
        );

        // 3. env_hash mismatch -> auto-removed & NotFound.
        std::fs::write(&path, &saved).expect("write frame0 for env_hash mismatch");
        assert_eq!(
            (api.boot_frame0_file)(h3, str_slice(&path), BUILD_ID, ENV_HASH ^ 1),
            NOT_FOUND
        );
        assert!(!path.exists(), "env_hash mismatch frame0 file must be auto-removed");
        assert_eq!(
            (api.boot_frame0_bytes)(h3, NveBufferSlice::from_slice(&saved), BUILD_ID, ENV_HASH ^ 1),
            NOT_FOUND
        );

        // 4. table_schema_hash mismatch (footer bytes 8..16) -> auto-removed & NotFound.
        let mut bad_schema = saved.clone();
        let footer_start = bad_schema.len() - FRAME0_FOOTER_SIZE;
        bad_schema[footer_start + 8] ^= 0x01;
        std::fs::write(&path, &bad_schema).expect("write bad_schema frame0");
        assert_eq!((api.boot_frame0_file)(h3, str_slice(&path), BUILD_ID, ENV_HASH), NOT_FOUND);
        assert!(!path.exists(), "schema mismatch frame0 file must be auto-removed");
        assert_eq!(
            (api.boot_frame0_bytes)(
                h3,
                NveBufferSlice::from_slice(&bad_schema),
                BUILD_ID,
                ENV_HASH
            ),
            NOT_FOUND
        );

        // 5. Missing file -> NotFound.
        assert_eq!((api.boot_frame0_file)(h3, str_slice(&path), BUILD_ID, ENV_HASH), NOT_FOUND);
        (api.destroy)(h3);
    }
}

unsafe extern "C" fn unclamped_app_measure(_: *mut c_void, _: NveSizeConstraints) -> NveIntrinsicSize {
    NveIntrinsicSize {
        width: 999.0,
        height: 1.0,
    }
}
static UNCLAMPED_LAYOUT_API: NveLayoutApi = NveLayoutApi {
    measure: unclamped_app_measure,
    hit_test: layout_app_hit_test,
};
unsafe extern "C" fn unclamped_app_get_interface(id: InterfaceId) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => (&raw const LAYOUT_SURFACE_API).cast(),
        NVE_IFACE_LAYOUT => (&raw const UNCLAMPED_LAYOUT_API).cast(),
        _ => std::ptr::null(),
    }
}
unsafe extern "C" fn empty_app_get_interface(id: InterfaceId) -> *const c_void {
    match id {
        NVE_IFACE_SURFACE => (&raw const LAYOUT_SURFACE_API).cast(),
        _ => std::ptr::null(),
    }
}

#[test]
fn measure_surface_baselines_min_max_rewrap_validation_alignment_and_cabi_clamping() {
    let api = get_api();
    let h = create_handle(&api, 400.0, 300.0);

    // 1. Finite CSS flex baseline returned from `measure_surface` (Row container with fixed item).
    let mut store = NodeStore::new();
    let mut ch = ChangeSet::default();
    {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .flex_direction(FlexDirection::Row);
        let box_item = tx.create();
        tx.edit::<Layout>(box_item)
            .width(Length::points(80.0))
            .height(Length::points(24.0));
        tx.set_children(NodeId::ROOT, &[box_item]);
        tx.commit_into(&mut ch);
    }
    let mut enc = WireEncoder::default();
    let mut baseline_frame = Vec::new();
    enc.encode(&store, &ch, &mut baseline_frame);

    // SAFETY: Owner thread, valid handles & buffers.
    unsafe {
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&baseline_frame)), OK);
        let loose = NveSizeConstraints::loose(300.0, 200.0);
        let mut m = NveSurfaceMeasure::default();
        assert_eq!((api.measure_surface)(h, &loose, NVE_MEASURE_FLAG_DRY_RUN, &mut m), OK);
        assert_eq!((m.width, m.height), (80.0, 24.0));
        assert!(
            m.first_baseline.is_finite() && m.first_baseline > 0.0 && m.first_baseline <= m.height,
            "flex container must expose finite first_baseline, got {m:?}"
        );
    }

    // 2. `0 < min_w < max_w`: row container (`align_items: flex-start`) with `flex_grow: 1,
    //    aspect_ratio: 2.0` child has Pass-1 intrinsic width `0.0 < min_w=120` and MUST re-measure
    //    Pass 2 at `min_w = 120` -> `120x60`!
    let mut ar_store = NodeStore::new();
    let mut ar_ch = ChangeSet::default();
    {
        let mut tx = ar_store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .flex_direction(FlexDirection::Row)
            .align_items(AlignItems::FlexStart);
        let child = tx.create();
        tx.edit::<Layout>(child)
            .flex_grow(1.0)
            .aspect_ratio(Some(2.0));
        tx.set_children(NodeId::ROOT, &[child]);
        tx.commit_into(&mut ar_ch);
    }
    let mut ar_buf = Vec::new();
    WireEncoder::default().encode(&ar_store, &ar_ch, &mut ar_buf);
    let h_ar = create_handle(&api, 400.0, 300.0);
    // SAFETY: Owner thread, valid handles.
    unsafe {
        assert_eq!((api.submit_nve2)(h_ar, NveBufferSlice::from_slice(&ar_buf)), OK);
        let c = NveSizeConstraints {
            min_w: 120.0,
            min_h: 0.0,
            max_w: 300.0,
            max_h: 300.0,
        };
        let mut m = NveSurfaceMeasure::default();
        assert_eq!((api.measure_surface)(h_ar, &c, NVE_MEASURE_FLAG_DRY_RUN, &mut m), OK);
        assert!(
            (m.width - 120.0).abs() <= 1.0 && (m.height - 60.0).abs() <= 1.0,
            "0 < min_w < max_w must re-measure height at min_w=120 -> 60, got {m:?}"
        );

        // 3. Negative min/max, -Inf, NaN, min > max, and misaligned FFI pointers -> `InvalidArg`.
        for bad in [
            NveSizeConstraints { min_w: -1.0, min_h: 0.0, max_w: 100.0, max_h: 100.0 },
            NveSizeConstraints { min_w: 0.0, min_h: -0.5, max_w: 100.0, max_h: 100.0 },
            NveSizeConstraints { min_w: 0.0, min_h: 0.0, max_w: -1.0, max_h: 100.0 },
            NveSizeConstraints { min_w: 0.0, min_h: 0.0, max_w: 100.0, max_h: f32::NEG_INFINITY },
            NveSizeConstraints { min_w: 0.0, min_h: f32::NEG_INFINITY, max_w: 100.0, max_h: 100.0 },
            NveSizeConstraints { min_w: 0.0, min_h: 0.0, max_w: f32::NAN, max_h: 100.0 },
            NveSizeConstraints { min_w: 0.0, min_h: 90.0, max_w: 100.0, max_h: 50.0 },
        ] {
            assert_eq!((api.measure_surface)(h_ar, &bad, NVE_MEASURE_FLAG_DRY_RUN, &mut m), INVALID);
        }
        let mut unaligned_storage = [0u8; 64];
        let bad_c_ptr = unaligned_storage.as_ptr().add(1).cast::<NveSizeConstraints>();
        let bad_m_ptr = unaligned_storage.as_mut_ptr().add(1).cast::<NveSurfaceMeasure>();
        let bad_cb_ptr = unaligned_storage.as_ptr().add(1).cast::<NveHostCallbacks>();
        assert_eq!((api.measure_surface)(h_ar, bad_c_ptr, NVE_MEASURE_FLAG_DRY_RUN, &mut m), INVALID);
        assert_eq!((api.measure_surface)(h_ar, &c, NVE_MEASURE_FLAG_DRY_RUN, bad_m_ptr), INVALID);
        assert_eq!((api.set_host_callbacks)(h_ar, bad_cb_ptr), INVALID);

        // 4. `CAbiAppAdapter::measure_surface` clamps non-IDEAL output to `[min, max]` while
        //    leaving `IDEAL` unconstrained.
        assert_eq!((api.attach_app_interface)(h_ar, Some(unclamped_app_get_interface)), OK);
        let bounded = NveSizeConstraints {
            min_w: 50.0,
            min_h: 20.0,
            max_w: 200.0,
            max_h: 80.0,
        };
        let mut clamped = NveSurfaceMeasure::default();
        assert_eq!(
            (api.measure_surface)(h_ar, &bounded, NVE_MEASURE_FLAG_DRY_RUN, &mut clamped),
            OK
        );
        assert_eq!((clamped.width, clamped.height), (200.0, 20.0));
        let mut ideal = NveSurfaceMeasure::default();
        assert_eq!(
            (api.measure_surface)(
                h_ar,
                &bounded,
                NVE_MEASURE_FLAG_DRY_RUN | NVE_MEASURE_FLAG_IDEAL,
                &mut ideal,
            ),
            OK
        );
        assert_eq!((ideal.width, ideal.height), (999.0, 1.0));

        (api.destroy)(h_ar);
        (api.destroy)(h);
    }

    // 5. `TaffyStoreLayout::measure_root` purity:
    //    - Calling `measure_root` BEFORE first `compute_layout` leaves engine dirty (`true`).
    //    - Calling `measure_root` AFTER `compute_layout` with varying constraints & `IDEAL`
    //      leaves committed rects untouched AND next `compute_layout` returns `false`!
    let (card_store, card, _, _) = build_card_tree(220.0, 80.0, "Taffy Purity");
    let mut taffy = TaffyStoreLayout::new();
    let vp = Viewport {
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
    };
    let pre = taffy.measure_root(
        &card_store,
        NodeId::ROOT,
        SizeConstraints::loose(300.0, 300.0),
        false,
    );
    assert_eq!((pre.width, pre.height), (220.0, 80.0));
    assert!(
        taffy.compute_layout(&card_store, NodeId::ROOT, vp),
        "measure_root before first compute_layout must leave layout dirty"
    );
    let committed_root = taffy.layout_of(NodeId::ROOT);
    let committed_card = taffy.layout_of(card);

    let _ = taffy.measure_root(
        &card_store,
        NodeId::ROOT,
        SizeConstraints { min_w: 100.0, min_h: 30.0, max_w: 150.0, max_h: 60.0 },
        false,
    );
    let _ = taffy.measure_root(
        &card_store,
        NodeId::ROOT,
        SizeConstraints::loose(50.0, 50.0),
        true,
    );
    assert_eq!(taffy.layout_of(NodeId::ROOT), committed_root);
    assert_eq!(taffy.layout_of(card), committed_card);
    assert!(
        !taffy.compute_layout(&card_store, NodeId::ROOT, vp),
        "measure_root must NOT bust Taffy clean-frame caching"
    );

    // 6. Store mutation without `sync_changes` AND two-store `measure_root` must NEVER return
    //    stale cached `SurfaceMeasure` or bleed `committed_inherit` across stores.
    let mut mut_store = card_store;
    let mut mut_ch = ChangeSet::default();
    {
        let mut tx = mut_store.tx();
        tx.edit::<Layout>(card)
            .width(Length::points(180.0))
            .height(Length::points(55.0));
        tx.commit_into(&mut mut_ch);
    }
    let updated = taffy.measure_root(
        &mut_store,
        NodeId::ROOT,
        SizeConstraints::loose(300.0, 300.0),
        false,
    );
    assert_eq!(
        (updated.width, updated.height),
        (180.0, 55.0),
        "mutating NodeStore must invalidate DryRunScratch even before sync_changes"
    );
    let (other_store, _, _, _) = build_card_tree(90.0, 40.0, "Store B");
    let other_m = taffy.measure_root(
        &other_store,
        NodeId::ROOT,
        SizeConstraints::loose(300.0, 300.0),
        false,
    );
    assert_eq!(
        (other_m.width, other_m.height),
        (90.0, 40.0),
        "measuring a distinct NodeStore must not reuse prior store's committed_inherit"
    );
}

#[test]
fn intrinsic_size_reentrant_unregister_and_detach_pair_empty_app_invalidations() {
    struct ReentrantState {
        api: NveRuntimeApi,
        handle: NveRuntimeHandle,
        hits: AtomicUsize,
        unregister_rc: std::sync::atomic::AtomicI32,
    }
    unsafe extern "C" fn reentrant_cb(ctx: *mut c_void) {
        // SAFETY: `ctx` points to live `ReentrantState` on owner thread.
        let s = unsafe { &*ctx.cast::<ReentrantState>() };
        s.hits.fetch_add(1, Ordering::AcqRel);
        // Re-entrantly unregister from INSIDE the callback -> MUST NOT deadlock!
        // SAFETY: Valid handle on owner thread.
        let rc = unsafe {
            (s.api.set_intrinsic_size_invalidated_callback)(s.handle, None, std::ptr::null_mut())
        };
        s.unregister_rc.store(rc, Ordering::Release);
    }

    let api = get_api();
    let h = create_handle(&api, 400.0, 300.0);
    let state = Box::new(ReentrantState {
        api,
        handle: h,
        hits: AtomicUsize::new(0),
        unregister_rc: std::sync::atomic::AtomicI32::new(-1),
    });
    let (_, _, _, frame) = build_card_tree(200.0, 80.0, "Reentrant");

    // SAFETY: Owner thread, valid handles.
    unsafe {
        assert_eq!(
            (api.set_intrinsic_size_invalidated_callback)(
                h,
                Some(reentrant_cb),
                (&*state as *const ReentrantState).cast_mut().cast(),
            ),
            OK
        );
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&frame)), OK);
        assert_eq!(state.hits.load(Ordering::Acquire), 1);
        assert_eq!(state.unregister_rc.load(Ordering::Acquire), OK);

        // Subsequent layout change must NOT fire after re-entrant unregister.
        let (_, _, _, frame2) = build_card_tree(250.0, 90.0, "AfterUnreg");
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&frame2)), OK);
        assert_eq!(state.hits.load(Ordering::Acquire), 1);

        // Now install a counting callback and verify invalidation on empty-app attach, detach, and pair_disconnect.
        let hits = Box::new(AtomicUsize::new(0));
        unsafe extern "C" fn count_cb(ctx: *mut c_void) {
            // SAFETY: `ctx` points to live `AtomicUsize`.
            unsafe { (*ctx.cast::<AtomicUsize>()).fetch_add(1, Ordering::AcqRel) };
        }
        assert_eq!(
            (api.set_intrinsic_size_invalidated_callback)(
                h,
                Some(count_cb),
                (&*hits as *const AtomicUsize).cast_mut().cast(),
            ),
            OK
        );

        // Attaching an empty app over a non-empty scene invalidates intrinsic size.
        assert_eq!((api.attach_app_interface)(h, Some(empty_app_get_interface)), OK);
        assert!(
            hits.load(Ordering::Acquire) >= 1,
            "attaching empty app over non-empty scene must fire intrinsic invalidation"
        );

        // Detaching when NO app is attached must preserve a `submit_nve2` scene (no wipe/invalidation).
        assert_eq!((api.attach_app_interface)(h, None), OK);
        assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&frame)), OK);
        let before_noop_detach = hits.load(Ordering::Acquire);
        assert_eq!((api.attach_app_interface)(h, None), OK);
        assert_eq!(hits.load(Ordering::Acquire), before_noop_detach);

        // Attaching a non-empty app, rendering a frame, and then detaching MUST fire invalidation.
        struct CardApp {
            buf: Vec<u8>,
            sent: bool,
        }
        unsafe extern "C" fn card_app_create(_: SurfaceId) -> *mut c_void {
            let (_, _, _, buf) = build_card_tree(160.0, 44.0, "AppCard");
            Box::into_raw(Box::new(CardApp { buf, sent: false })).cast()
        }
        unsafe extern "C" fn card_app_destroy(p: *mut c_void) {
            if !p.is_null() {
                // SAFETY: Allocated by `card_app_create`.
                drop(unsafe { Box::from_raw(p.cast::<CardApp>()) });
            }
        }
        unsafe extern "C" fn card_app_render(p: *mut c_void, _: FrameId) -> NveBufferSlice {
            // SAFETY: Valid `CardApp` owned by runtime.
            let app = unsafe { &mut *p.cast::<CardApp>() };
            if app.sent {
                NveBufferSlice::EMPTY
            } else {
                app.sent = true;
                NveBufferSlice::from_slice(&app.buf)
            }
        }
        static CARD_SURFACE_API: NveSurfaceApi = NveSurfaceApi {
            create: card_app_create,
            destroy: card_app_destroy,
            render: card_app_render,
            dispatch_events: layout_app_dispatch,
        };
        unsafe extern "C" fn card_app_get_interface(id: InterfaceId) -> *const c_void {
            match id {
                NVE_IFACE_SURFACE => (&raw const CARD_SURFACE_API).cast(),
                _ => std::ptr::null(),
            }
        }
        assert_eq!((api.attach_app_interface)(h, Some(card_app_get_interface)), OK);
        let mut changed = 0u32;
        assert_eq!((api.tick)(h, 1.0 / 60.0, &mut changed), OK);
        assert_eq!(changed, 1);
        let before_detach = hits.load(Ordering::Acquire);
        assert_eq!((api.attach_app_interface)(h, None), OK);
        assert!(
            hits.load(Ordering::Acquire) > before_detach,
            "detaching a live app over a non-empty scene must fire intrinsic invalidation"
        );

        #[cfg(feature = "wire")]
        {
            assert_eq!((api.submit_nve2)(h, NveBufferSlice::from_slice(&frame)), OK);
            let before_pair = hits.load(Ordering::Acquire);
            let uri = NveStrSlice::from_str("ws://127.0.0.1:9/nve-wire");
            let tok = NveStrSlice::from_str("tok");
            assert_eq!((api.pair_connect)(h, uri, tok), OK);
            assert!(
                hits.load(Ordering::Acquire) > before_pair,
                "pair_connect replacing non-empty scene must fire intrinsic invalidation"
            );
            assert_eq!((api.pair_disconnect)(h), OK);
        }

        assert_eq!((api.set_intrinsic_size_invalidated_callback)(h, None, std::ptr::null_mut()), OK);
        (api.destroy)(h);
    }
}

#[derive(NveTable)]
#[nve_table(name = "nve.test.frame0_module", storage = "sparse", flags = "publish")]
#[allow(dead_code)]
struct TestFrame0ModuleTable {
    #[effect(paint)]
    pub marker: u32,
}

struct DirectModuleApp {
    store: NodeStore,
    changes: ChangeSet,
    dirty: bool,
}

impl RuntimeApp for DirectModuleApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        if !self.dirty {
            return AppFrame::Clean;
        }
        self.dirty = false;
        AppFrame::Direct {
            changes: &self.changes,
        }
    }
    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
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
fn frame0_atomic_failure_preserves_scene_app_guard_and_module_table_schema() {
    let dir = Scratch::new("frame0-atomic");
    const BUILD_ID: u64 = 0x1234_5678;
    const ENV_HASH: u64 = 0x9ABC_DEF0;
    let meta = Frame0Meta {
        table_schema_hash: core_tables_schema_hash(),
        app_build_id: BUILD_ID,
        env_hash: ENV_HASH,
    };

    // 1. Arbitrary non-NVE2 file on disk is NEVER deleted on `boot_frame0_file`.
    let user_file = dir.0.join("important_notes.txt");
    std::fs::write(&user_file, b"do-not-delete-me-important-user-data").expect("write user file");
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .build();
    assert!(matches!(
        rt.boot_frame0_file(&user_file, BUILD_ID, ENV_HASH),
        Err(RuntimeError::NotFound(_))
    ));
    assert_eq!(
        std::fs::read(&user_file).expect("user file preserved"),
        b"do-not-delete-me-important-user-data"
    );

    // 2. Atomic staging failure: establish live scene (card 210x90, frame_id == 1), then attempt
    //    `boot_frame0_bytes` with (a) mismatched `node_count` and (b) truncated mid-stream NVE2 body
    //    carrying valid footers + XXH64. Both MUST fail with `NotFound` and preserve prior scene!
    let (src_store, _, _, frame) = build_card_tree(210.0, 90.0, "Prior Live Scene");
    rt.submit_nve2(&frame).expect("submit live scene");
    let mut full_nve2 = Vec::new();
    WireEncoder::default().encode_full(&src_store, &mut full_nve2);
    assert!(src_store.stats().live_nodes >= 3);

    // (a) Valid NVE2 payload, valid footer + XXH64, wrong `node_count` (1 != 3).
    let wrong_count_env =
        Frame0File::encode_nve2(&full_nve2, 1, meta).expect("encode wrong node_count");
    assert!(matches!(
        rt.boot_frame0_bytes(&wrong_count_env, BUILD_ID, ENV_HASH),
        Err(RuntimeError::NotFound(_))
    ));
    {
        let g = rt.acquire_scene().expect("acquire after node_count fail");
        assert_eq!(g.view().frame_id, 1);
        // SAFETY: `g` holds reader lock and tree has >= 3 nodes.
        let r2 = unsafe { *g.view().rects.add(2) };
        assert_eq!((r2.width, r2.height), (210.0, 90.0));
    }

    // (b) Valid 24-byte NVE2 header + corrupt mid-stream op tag + valid footer/XXH64 -> staging decode error.
    let mut corrupt_nve2 = full_nve2.clone();
    assert!(corrupt_nve2.len() >= 28);
    corrupt_nve2[24..28].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    let corrupt_env =
        Frame0File::encode_nve2(&corrupt_nve2, 3, meta).expect("encode corrupt op body");
    assert!(matches!(
        rt.boot_frame0_bytes(&corrupt_env, BUILD_ID, ENV_HASH),
        Err(RuntimeError::NotFound(_))
    ));
    {
        let g = rt.acquire_scene().expect("acquire after decode fail");
        assert_eq!(g.view().frame_id, 1);
        // SAFETY: `g` holds reader lock and tree has >= 3 nodes.
        let r2 = unsafe { *g.view().rects.add(2) };
        assert_eq!((r2.width, r2.height), (210.0, 90.0));
    }

    // 3. `boot_frame0_file` & `boot_frame0_bytes` refused with `InvalidArg` while app or pair attached!
    let valid_path = dir.0.join("valid.nve2");
    rt.save_frame0_file(&valid_path, BUILD_ID, ENV_HASH)
        .expect("save valid frame0");
    let valid_bytes = std::fs::read(&valid_path).expect("read valid frame0");
    let (app_store, _, _, _) = build_card_tree(100.0, 50.0, "App");
    let mut app_changes = ChangeSet::default();
    let mut s_mut = app_store;
    {
        let tx = s_mut.tx();
        tx.commit_into(&mut app_changes);
    }
    rt.attach_app(Box::new(DirectModuleApp {
        store: s_mut,
        changes: app_changes,
        dirty: true,
    }))
    .expect("attach app");
    assert!(matches!(
        rt.boot_frame0_file(&valid_path, BUILD_ID, ENV_HASH),
        Err(RuntimeError::InvalidArg(_))
    ));
    assert!(matches!(
        rt.boot_frame0_bytes(&valid_bytes, BUILD_ID, ENV_HASH),
        Err(RuntimeError::InvalidArg(_))
    ));
    assert!(valid_path.exists(), "refused boot must leave valid file intact");
    rt.detach_app().expect("detach app");

    #[cfg(feature = "wire")]
    {
        rt.pair_connect("ws://127.0.0.1:9/nve-wire", "tok")
            .expect("pair connect");
        assert!(matches!(
            rt.boot_frame0_file(&valid_path, BUILD_ID, ENV_HASH),
            Err(RuntimeError::InvalidArg(_))
        ));
        assert!(matches!(
            rt.boot_frame0_bytes(&valid_bytes, BUILD_ID, ENV_HASH),
            Err(RuntimeError::InvalidArg(_))
        ));
        rt.pair_disconnect().expect("pair disconnect");
    }

    // 4. `save_frame0_file` with an optional custom module table attached to a live node MUST emit
    //    a schema hash distinct from `core_tables_schema_hash()`, rejected by core-only boot!
    let mut mod_store = NodeStore::new();
    let mut mod_changes = ChangeSet::default();
    {
        let mut tx = mod_store.tx();
        let n = tx.create();
        tx.edit::<Layout>(n)
            .width(Length::points(100.0))
            .height(Length::points(40.0));
        tx.edit::<TestFrame0ModuleTable>(n).marker(777);
        tx.set_children(NodeId::ROOT, &[n]);
        tx.commit_into(&mut mod_changes);
        assert_eq!(mod_store.row::<TestFrame0ModuleTable>(n).marker(), 777);
    }
    let mut rt_mod = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .app(Box::new(DirectModuleApp {
            store: mod_store,
            changes: mod_changes,
            dirty: true,
        }))
        .build();
    assert!(rt_mod.tick(1.0 / 60.0).expect("tick module app"));
    let mod_path = dir.0.join("module_frame0.nve2");
    rt_mod
        .save_frame0_file(&mod_path, BUILD_ID, ENV_HASH)
        .expect("save module frame0");
    let mod_bytes = std::fs::read(&mod_path).expect("read module frame0");
    let footer_arr: [u8; FRAME0_FOOTER_SIZE] = mod_bytes[mod_bytes.len() - FRAME0_FOOTER_SIZE..]
        .try_into()
        .expect("footer slice");
    let footer = Frame0Footer::from_bytes(&footer_arr);
    assert_ne!(
        footer.table_schema_hash,
        core_tables_schema_hash(),
        "Frame0 with live custom module table must NOT be tagged with core-only schema hash"
    );
    let mut core_only_rt = RuntimeBuilder::new().build();
    assert!(matches!(
        core_only_rt.boot_frame0_bytes(&mod_bytes, BUILD_ID, ENV_HASH),
        Err(RuntimeError::NotFound(_))
    ));
}

#[test]
fn tick_hook_request_frame_without_changes_wakes_without_bumping_frame_id() {
    struct RequestOnlyHook(bool);
    impl FrameTickHook for RequestOnlyHook {
        fn tick(&mut self, _tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
            self.0
        }
    }

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let n = tx.create();
        tx.edit::<Layout>(n)
            .width(Length::points(120.0))
            .height(Length::points(40.0));
        tx.set_children(NodeId::ROOT, &[n]);
        tx.commit_into(&mut changes);
    }
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .app(Box::new(DirectModuleApp {
            store,
            changes,
            dirty: true,
        }))
        .tick_hook(Box::new(RequestOnlyHook(true)))
        .build();

    // Tick 1 mounts initial tree (`frame_id == 1`).
    assert!(rt.tick(1.0 / 60.0).expect("tick 1"));
    assert_eq!(rt.acquire_scene().expect("acquire 1").view().frame_id, 1);

    // Tick 2: app clean, hook returns `true` WITHOUT writing any changes -> MUST return `false`,
    // MUST NOT bump `frame_id`, and MUST arm `wake_bridge` for follow-up scheduling!
    assert!(
        !rt.tick(1.0 / 60.0).expect("tick 2"),
        "empty hook changes must not report a newly published scene"
    );
    assert_eq!(
        rt.acquire_scene().expect("acquire 2").view().frame_id,
        1,
        "empty hook changes must not bump frame_id"
    );
    assert!(
        rt.wake_bridge().is_pending(),
        "hook returning true must still schedule follow-up wake"
    );
}
