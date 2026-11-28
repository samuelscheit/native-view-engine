//! Runtime ABI v6 measurement, intrinsic-size invalidation, and Frame-0 save/boot contract tests.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use nve_abi::{
    FrameId, InterfaceId, NveBufferSlice, NveEventBatch, NveIntrinsicSize, NveLayoutApi,
    NveRuntimeApi, NveRuntimeConfig, NveRuntimeHandle, NveRuntimeStatus, NveSceneView,
    NveSizeConstraints, NveStrSlice, NveSurfaceApi, NveSurfaceMeasure, SurfaceId, NVE_IFACE_LAYOUT,
    NVE_IFACE_SURFACE, NVE_MEASURE_FLAG_DRY_RUN, NVE_MEASURE_FLAG_IDEAL, NVE_RUNTIME_ABI_VERSION,
};
use nve_engine::persistence::FRAME0_FOOTER_SIZE;
use nve_layout::{HeuristicTextMeasurer, TaffyStoreLayout};
use nve_runtime::{nve_runtime_get_interface, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{Layout, TextContent, Visual};

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

        // 1. Corrupt payload -> auto-removed from disk & NotFound.
        let mut corrupt = saved.clone();
        corrupt[0] ^= 0xFF;
        std::fs::write(&path, &corrupt).expect("write corrupt frame0");
        assert_eq!((api.boot_frame0_file)(h3, str_slice(&path), BUILD_ID, ENV_HASH), NOT_FOUND);
        assert!(!path.exists(), "corrupt frame0 file must be auto-removed");
        assert_eq!(
            (api.boot_frame0_bytes)(h3, NveBufferSlice::from_slice(&corrupt), BUILD_ID, ENV_HASH),
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
