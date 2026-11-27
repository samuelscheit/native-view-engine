//! Comprehensive C ABI, memory-safety, concurrency, panic-boundary, thread-affinity, and
//! validation contract tests for `nve-runtime`.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use nve_abi::{
    EventMask, NativeEventKind, NveBufferSlice, NveDirectEvent, NveEventBatch, NveEventBatchRef,
    NveRuntimeApi, NveRuntimeConfig, NveRuntimeMetrics, NveRuntimeSceneView, NveRuntimeStatus,
    NveStrSlice, NVE_RUNTIME_ABI_VERSION, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_components::tables::{Interaction, Layout, Scroll, TextContent, Visual};
use nve_engine::{NveApp, Viewport};
use nve_layout::TaffyStoreLayout;
use nve_runtime::{
    FrameTickHook, RuntimeBuilder, RuntimeError, SurfaceRuntime, WakeBridge,
    nve_runtime_get_interface,
};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, Transaction};

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

#[test]
fn failed_reallocating_wire_frame_and_detach_never_leave_dangling_published_view() {
    let api = get_api();
    // SAFETY: Null config uses defaults.
    let h = unsafe { (api.create)(std::ptr::null()) };
    assert!(!h.is_null());

    // 1. Submit valid initial frame (16 nodes) -> published_view populated.
    let valid = build_nve2_with_nodes(16, false);
    let mut view = NveRuntimeSceneView::default();
    // SAFETY: Valid handle, slice, and stack out pointer on owner thread.
    unsafe {
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&valid)),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(
            (api.scene_acquire)(h, &mut view),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(!view.rows.is_null() && view.node_capacity >= 18);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 2. Build a large frame (256 new nodes -> forces Vec reallocation in replica_store) and
    //    append trailing garbage bytes so `WireDecoder::apply` fails AFTER reallocating!
    let mut malformed = build_nve2_with_nodes(256, false);
    malformed.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0xFF]);
    // SAFETY: Valid handle and slice.
    let rc = unsafe { (api.submit_nve2)(h, NveBufferSlice::from_slice(&malformed)) };
    assert_eq!(rc, NveRuntimeStatus::Io.as_i32());
    assert!(!read_last_error(&api).is_empty());

    // 3. `scene_acquire` MUST NOT return dangling pointers: `rows` must be NULL and capacity 0!
    let mut view_after_err = NveRuntimeSceneView::default();
    // SAFETY: Valid handle and out pointer.
    unsafe {
        assert_eq!(
            (api.scene_acquire)(h, &mut view_after_err),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(
            view_after_err.rows.is_null() && view_after_err.node_capacity == 0,
            "failed wire decode must invalidate published_view rather than exposing reallocated pointers"
        );
        assert!(view_after_err.tables.is_null() && view_after_err.table_count == 0);
        assert_eq!((api.scene_release)(h), NveRuntimeStatus::Ok.as_i32());
    }

    // 4. Attach C ABI app, render, then detach with `None` -> published_view MUST be invalidated!
    let ev = [NveDirectEvent::press(1, 2, 100, 10.0, 10.0)];
    let mut changed = 0u32;
    let mut ev_changed = 0u32;
    let mut detached_view = NveRuntimeSceneView::default();
    // SAFETY: Valid handle, exported `nve_get_interface` symbol, and stack slices on owner thread.
    unsafe {
        assert_eq!(
            (api.attach_app_interface)(h, Some(nve_runtime_test_mini_app_get_interface)),
            NveRuntimeStatus::Ok.as_i32()
        );
        // Submitting NVE2 while app is attached MUST return `InvalidArg`.
        assert_eq!(
            (api.submit_nve2)(h, NveBufferSlice::from_slice(&valid)),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.tick)(h, 0.016, &mut changed),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!(changed, 1);

        // Dispatch an event to the attached C ABI app and re-tick.
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
            let mut v = NveRuntimeSceneView::default();
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
    let mut view = NveRuntimeSceneView::default();
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

    // Pairing unsupported stubs still guard panics and return Unsupported on owner thread.
    // SAFETY: Valid handle.
    unsafe {
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

    // 3. Cross-thread contract: mutating calls from another thread MUST fail with `InvalidArg`,
    //    while `scene_acquire`/`scene_release`/`get_metrics`/`hit_test` succeed!
    let h_addr = h as usize;
    std::thread::spawn(move || {
        let h_bg = h_addr as *mut c_void;
        let mut out_id = 0u32;
        let mut out_changed = 0u32;
        let mut out_v = NveRuntimeSceneView::default();
        let mut out_m = NveRuntimeMetrics::default();
        // SAFETY: Handle stays alive on main thread until this thread joins.
        unsafe {
            assert_eq!(
                (api.tick)(h_bg, 0.016, &mut out_changed),
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
                (api.hit_test)(h_bg, 20.0, 20.0, &mut out_id),
                NveRuntimeStatus::Ok.as_i32()
            );
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
    let mut view = NveRuntimeSceneView::default();
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
    assert_eq!(rt.layout_engine_name().expect("engine name"), "taffy");
    drop(guard);

    // Confirm scroll_by returns NotFound on a missing node in Rust API too.
    assert!(matches!(
        rt.scroll_by(9999, 0.0, 10.0),
        Err(RuntimeError::NotFound(_))
    ));
    let _ = NativeEventKind::Press;
}
