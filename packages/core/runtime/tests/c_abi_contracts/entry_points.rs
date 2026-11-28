//! C ABI entry-point validation, threading, wake-bridge synchronization, panic poisoning, and
//! layout-engine hot-swap contract tests.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nve_abi::{
    ArenaOffset, ByteLen, EventId, HostRequestId, HostServiceId, InteractionEvent, NodeId,
    NveBufferSlice, NveEvent, NveEventBatch, NveHostCallbacks, NveHostStatus, NveRuntimeConfig,
    NveRuntimeMetrics, NveRuntimeStatus, NveSceneView, NveSizeConstraints, NveStrSlice,
    NveSurfaceMeasure, RawEventBytes, SurfaceId, TimestampUs, NVE_MEASURE_FLAG_DRY_RUN,
    NVE_RUNTIME_ABI_VERSION, NVE_RUNTIME_SCHEMA_HASH, NVE_TABLE_INTERACTION,
};
use nve_engine::Viewport;
use nve_layout::TaffyStoreLayout;
use nve_runtime::{
    nve_runtime_get_interface, FrameTickHook, RuntimeBuilder, RuntimeError, SurfaceRuntime,
    WakeBridge,
};
use nve_store::Transaction;

use super::{build_nve2_with_nodes, get_api, read_last_error};

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
        let mut node_out = NodeId::NULL;
        assert_eq!(
            (api.hit_test)(null_h, 0.0, 0.0, &mut node_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.scroll_by)(null_h, NodeId::ROOT, 0.0, 10.0, &mut f32_out, &mut f32_out),
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
        // Runtime ABI v6 slots 28..=36 null-handle guards.
        let constraints = NveSizeConstraints::loose(200.0, 100.0);
        let mut measure = NveSurfaceMeasure::default();
        assert_eq!(
            (api.measure_surface)(null_h, &constraints, NVE_MEASURE_FLAG_DRY_RUN, &mut measure),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.set_intrinsic_size_invalidated_callback)(null_h, None, std::ptr::null_mut()),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.save_frame0_file)(null_h, NveStrSlice::EMPTY, 0, 0),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.boot_frame0_file)(null_h, NveStrSlice::EMPTY, 0, 0),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.boot_frame0_bytes)(null_h, NveBufferSlice::EMPTY, 0, 0),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.set_host_callbacks)(null_h, std::ptr::null::<NveHostCallbacks>()),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.complete_host_request)(
                null_h,
                HostRequestId(1),
                NveHostStatus::Ok.as_i32(),
                NveBufferSlice::EMPTY
            ),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.push_host_event)(null_h, HostServiceId(1), 0, NveBufferSlice::EMPTY),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.publish_host_channel)(null_h, 1, 0, NveBufferSlice::EMPTY),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
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

    // Invalid `NveEventBatch` (null events ptr with event_count > 0, null bytes ptr with
    // bytes_len > 0, and out-of-bounds event byte span).
    let bad_batch_null_events = NveEventBatch {
        events: std::ptr::null(),
        bytes: std::ptr::null(),
        event_count: 2,
        bytes_len: ByteLen::ZERO,
    };
    let bad_batch_null_bytes = NveEventBatch {
        events: std::ptr::null(),
        bytes: std::ptr::null(),
        event_count: 0,
        bytes_len: ByteLen(4),
    };
    let oob_event = [NveEvent::new(
        NVE_TABLE_INTERACTION,
        SurfaceId::ROOT,
        NodeId::new(2, 0),
        TimestampUs(1),
        EventId(0),
    )
    .with_bytes(RawEventBytes::new(ArenaOffset(0), ByteLen(8)))];
    let bad_batch_oob = NveEventBatch::from_parts(&oob_event, &[]);
    // SAFETY: Guarded inside `ValidatedEvents::try_from_raw` before dereference.
    unsafe {
        assert_eq!(
            (api.dispatch_events)(h, bad_batch_null_events, &mut u32_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.dispatch_events)(h, bad_batch_null_bytes, &mut u32_out),
            NveRuntimeStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            (api.dispatch_events)(h, bad_batch_oob, &mut u32_out),
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
            (api.scroll_by)(h, NodeId::new(2, 0), 0.0, 75.0, &mut sx, &mut sy),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert_eq!((sx, sy), (0.0, 75.0));
        assert_eq!(
            (api.scroll_by)(h, NodeId::new(3, 0), 0.0, 10.0, &mut sx, &mut sy),
            NveRuntimeStatus::NotFound.as_i32()
        );
        assert_eq!(
            (api.scroll_by)(h, NodeId::new(999, 0), 0.0, 10.0, &mut sx, &mut sy),
            NveRuntimeStatus::NotFound.as_i32()
        );
        assert_eq!(
            (api.scroll_by)(h, NodeId::new(2, 0), f32::NAN, 0.0, &mut sx, &mut sy),
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
        let mut hit_out = NodeId::NULL;
        assert_eq!(
            (api.hit_test)(h, 20.0, 20.0, &mut hit_out),
            NveRuntimeStatus::Ok.as_i32()
        );
        assert!(!hit_out.is_null());
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
        let mut out_id = NodeId::NULL;
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
                (api.scroll_by)(h_bg, NodeId::new(2, 0), 0.0, 5.0, std::ptr::null_mut(), std::ptr::null_mut()),
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
            // Runtime ABI v6 owner-thread slots reject cross-thread calls.
            let c = NveSizeConstraints::loose(200.0, 100.0);
            let mut m = NveSurfaceMeasure::default();
            assert_eq!(
                (api.measure_surface)(h_bg, &c, NVE_MEASURE_FLAG_DRY_RUN, &mut m),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.set_intrinsic_size_invalidated_callback)(h_bg, None, std::ptr::null_mut()),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.save_frame0_file)(h_bg, NveStrSlice::EMPTY, 0, 0),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.boot_frame0_file)(h_bg, NveStrSlice::EMPTY, 0, 0),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.boot_frame0_bytes)(h_bg, NveBufferSlice::EMPTY, 0, 0),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            assert_eq!(
                (api.set_host_callbacks)(h_bg, std::ptr::null()),
                NveRuntimeStatus::InvalidArg.as_i32()
            );
            // Runtime ABI v6 ANY-thread host ingress slots MUST succeed cross-thread!
            assert_eq!(
                (api.complete_host_request)(
                    h_bg,
                    HostRequestId(999),
                    NveHostStatus::Ok.as_i32(),
                    NveBufferSlice::EMPTY
                ),
                NveRuntimeStatus::Ok.as_i32()
            );
            assert_eq!(
                (api.push_host_event)(h_bg, HostServiceId(0x42), 1, NveBufferSlice::EMPTY),
                NveRuntimeStatus::Ok.as_i32()
            );
            assert_eq!(
                (api.publish_host_channel)(h_bg, 7, 0, NveBufferSlice::EMPTY),
                NveRuntimeStatus::Ok.as_i32()
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

    let scroll_node = NodeId::new(2, 0);
    let (sx, sy) = rt.scroll_by(scroll_node, 0.0, 50.0).expect("scroll");
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
    let (container_rect, child0_rect) =
        unsafe { (*guard.view().rects.add(2), *guard.view().rects.add(3)) };
    assert_eq!((container_rect.width, container_rect.height), (200.0, 100.0));
    assert_eq!((child0_rect.width, child0_rect.height), (200.0, 50.0));
    assert_eq!(rt.layout_engine_name().expect("engine name"), "taffy");
    drop(guard);

    // Scroll offset `(0, 50)` must remain preserved and clamped after `swap_layout_engine`.
    assert_eq!(
        rt.scroll_by(scroll_node, 0.0, 0.0).expect("query scroll"),
        (0.0, 50.0)
    );

    // Confirm scroll_by returns NotFound on a missing node in Rust API too.
    assert!(matches!(
        rt.scroll_by(NodeId::new(9999, 0), 0.0, 10.0),
        Err(RuntimeError::NotFound(_))
    ));
    let _ = InteractionEvent::Press;
}
