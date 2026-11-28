//! Zero-allocation steady-state runtime tick (clean + dirty hook + steady-state NVE2) and
//! `scene_acquire`/`scene_release` verification with positive + cross-thread `CountingAlloc`
//! isolation proofs.

use std::ffi::c_void;
use std::hint::{black_box, spin_loop};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nve_abi::{
    FrameId, HostServiceId, InteractionEvent, NveBufferSlice, NveHostCallbacks, NveHostStatus,
    NveRuntimeStatus, NveSizeConstraints, SurfaceId, ValidatedEvents, METHOD_WINDOW_REQUEST_FRAME,
    NVE_MEASURE_FLAG_DRY_RUN, NVE_MEASURE_FLAG_IDEAL, NVE_RUNTIME_ABI_VERSION,
    NVE_RUNTIME_SCHEMA_HASH, SERVICE_WINDOW,
};
use nve_tables::tables::{Interaction, Layout, Scroll, TextContent, Visual};
use nve_engine::Viewport;
use nve_runtime::{
    AppFrame, FrameTickHook, RuntimeApp, RuntimeBuilder, RuntimeError, SurfaceRuntime,
    nve_runtime_get_interface,
};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba, Transaction};

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::count_allocs;

#[test]
fn counting_alloc_positive_and_thread_isolation_probe() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let (_, own) = count_allocs(|| black_box(Vec::<u64>::with_capacity(32)));
    assert!(
        own >= 1,
        "CountingAlloc must observe allocations on the measuring thread"
    );

    const WORKER_ALLOCS: usize = 1_000;
    let go = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_allocs = Arc::new(AtomicUsize::new(0));
    let worker = {
        let (go, stop, worker_allocs) = (
            Arc::clone(&go),
            Arc::clone(&stop),
            Arc::clone(&worker_allocs),
        );
        std::thread::spawn(move || {
            while !go.load(Ordering::Acquire) {
                if Instant::now() > deadline {
                    return;
                }
                spin_loop();
            }
            while !stop.load(Ordering::Acquire) {
                black_box(vec![0u8; 256]);
                worker_allocs.fetch_add(1, Ordering::Release);
            }
        })
    };
    let (_, during) = count_allocs(|| {
        go.store(true, Ordering::Release);
        while worker_allocs.load(Ordering::Acquire) < WORKER_ALLOCS && Instant::now() < deadline {
            spin_loop();
        }
    });
    stop.store(true, Ordering::Release);
    assert!(worker.join().is_ok(), "allocation probe worker panicked");
    assert!(worker_allocs.load(Ordering::Acquire) >= WORKER_ALLOCS);
    assert_eq!(
        during, 0,
        "another thread's allocations must never leak into this thread's count"
    );
}

fn build_sample_nve2_frame() -> (NodeStore, ChangeSet, Vec<u8>) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let scroller = tx.create();
        tx.edit::<Layout>(scroller)
            .width(Length::points(240.0))
            .height(Length::points(120.0));
        tx.edit::<Scroll>(scroller);

        let card = tx.create();
        tx.edit::<Layout>(card)
            .width(Length::points(200.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(card).background(Rgba::rgb(30, 41, 59));
        tx.edit::<Interaction>(card)
            .listen(InteractionEvent::Press);

        let label = tx.create();
        tx.edit::<TextContent>(label).text("Hello NveRuntime");

        tx.set_children(card, &[label]);
        tx.set_children(scroller, &[card]);
        tx.set_children(NodeId::ROOT, &[scroller]);
        tx.commit_into(&mut changes);
    }
    let mut encoder = WireEncoder::default();
    let mut buf = Vec::new();
    encoder.encode(&store, &changes, &mut buf);
    (store, changes, buf)
}

#[test]
fn steady_state_clean_and_dirty_nve2_ticks_are_zero_alloc() {
    let (mut src_store, mut changes, initial_frame) = build_sample_nve2_frame();
    let mut rt = RuntimeBuilder::new()
        .surface_id(SurfaceId::ROOT)
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 2.0,
        })
        .build();

    rt.submit_nve2(&initial_frame).expect("submit initial NVE2");
    assert_eq!(
        rt.metrics().expect("metrics").wire_bytes_rx,
        initial_frame.len() as u64
    );
    // Warm clean tick + hit_test + scene acquire.
    let _ = rt.tick(1.0 / 60.0).expect("warm tick");
    let card_id = src_store.id_at(3).expect("card node");
    let hit = rt.hit_test(50.0, 25.0).expect("warm hit");
    assert_eq!(hit, Some(card_id));
    {
        let g = rt.acquire_scene().expect("warm acquire");
        assert_eq!(g.view().schema_hash, NVE_RUNTIME_SCHEMA_HASH);
        assert!(g.view().node_capacity >= 4);
    }

    // 1. Clean steady-state tick + hit_test + scene_acquire: 0 allocs.
    let (_, clean_allocs) = count_allocs(|| {
        for _ in 0..32 {
            let changed = rt.tick(1.0 / 60.0).expect("clean tick");
            assert!(!changed);
            let h = rt.hit_test(50.0, 25.0).expect("hit");
            assert_eq!(h, Some(card_id));
            let g = rt.acquire_scene().expect("acquire");
            assert_eq!(g.view().frame_id, 1);
        }
    });
    assert_eq!(
        clean_allocs, 0,
        "steady-state clean tick + hit_test + scene_acquire must perform 0 heap allocations"
    );

    // 2. Pre-encode two alternating delta NVE2 frames mutating Visual opacity (even WITH Scroll
    //    present so `reconcile_scroll_offsets` runs on every commit!).
    let mut enc = WireEncoder::default();
    let mut dummy = Vec::new();
    enc.encode(&src_store, &changes, &mut dummy);

    let mut delta_a = Vec::new();
    {
        let mut tx = src_store.tx();
        tx.edit::<Visual>(card_id).opacity(0.6);
        tx.commit_into(&mut changes);
    }
    enc.encode(&src_store, &changes, &mut delta_a);

    let mut delta_b = Vec::new();
    {
        let mut tx = src_store.tx();
        tx.edit::<Visual>(card_id).opacity(0.9);
        tx.commit_into(&mut changes);
    }
    enc.encode(&src_store, &changes, &mut delta_b);

    // Warm one delta submit so scratch buffers reach steady-state capacity.
    rt.submit_nve2(&delta_a).expect("warm delta A");
    rt.submit_nve2(&delta_b).expect("warm delta B");
    let rx_before = rt.metrics().expect("metrics before").wire_bytes_rx;

    let (_, dirty_nve2_allocs) = count_allocs(|| {
        for i in 0..32 {
            let payload = if i % 2 == 0 { &delta_a } else { &delta_b };
            rt.submit_nve2(payload).expect("steady-state dirty submit_nve2");
            let g = rt.acquire_scene().expect("acquire after submit");
            assert_eq!(g.view().dirty_count, 1);
        }
    });
    assert_eq!(
        dirty_nve2_allocs, 0,
        "steady-state dirty submit_nve2 + scroll reconcile + publish_scene must perform 0 allocs"
    );
    let rx_after = rt.metrics().expect("metrics after").wire_bytes_rx;
    assert_eq!(
        rx_after - rx_before,
        16 * (delta_a.len() as u64 + delta_b.len() as u64)
    );
}

struct PulseLayoutAndVisualHook {
    calls: Arc<AtomicUsize>,
    target_idx: usize,
    step: u32,
    animate_layout: Arc<AtomicBool>,
}

impl FrameTickHook for PulseLayoutAndVisualHook {
    fn tick(&mut self, tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.step += 1;
        if let Some(id) = tx.store().id_at(self.target_idx) {
            if self.animate_layout.load(Ordering::Acquire) {
                let w = if self.step.is_multiple_of(2) {
                    180.0
                } else {
                    220.0
                };
                tx.edit::<Layout>(id).width(Length::points(w));
            }
            let op = if self.step.is_multiple_of(2) { 0.5 } else { 0.85 };
            tx.edit::<Visual>(id).opacity(op);
            return true;
        }
        false
    }
}

struct DirectTestApp {
    store: NodeStore,
    changes: ChangeSet,
    dirty: bool,
}

impl RuntimeApp for DirectTestApp {
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
fn direct_app_frame_tick_hook_propagates_changes_is_zero_alloc_and_enforces_scene_busy() {
    let (store, changes, _) = build_sample_nve2_frame();
    let calls = Arc::new(AtomicUsize::new(0));
    let animate_layout = Arc::new(AtomicBool::new(true));
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .app(Box::new(DirectTestApp {
            store,
            changes,
            dirty: true,
        }))
        .tick_hook(Box::new(PulseLayoutAndVisualHook {
            calls: Arc::clone(&calls),
            target_idx: 3, // `card` node
            step: 0,
            animate_layout: Arc::clone(&animate_layout),
        }))
        .build();

    // Tick 1: app mounts initial tree AND hook sets card width = 220, opacity = 0.85.
    assert!(rt.tick(0.016).expect("tick 1"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    {
        let guard = rt.acquire_scene().expect("acquire 1");
        let v = guard.view();
        assert_eq!(v.schema_hash, NVE_RUNTIME_SCHEMA_HASH);
        assert_eq!(v.frame_id, 1);
        assert!(v.node_capacity >= 5);
        // SAFETY: `guard` holds the reader lock and `node_capacity >= 5`.
        let rect3 = unsafe { *v.rects.add(3) };
        assert_eq!((rect3.width, rect3.height), (220.0, 80.0));
        // SAFETY: `guard` holds the reader lock and `node_capacity >= 5`.
        let row3 = unsafe { *v.rows.add(3) };
        assert!(row3.is_live());
        assert_eq!(row3.child_len, 1);

        // Actual `SceneBusy` enforcement: attempting a write while `guard` is held MUST fail!
        let api_ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
        // SAFETY: Static vtable for matching ABI version.
        let api = unsafe { *api_ptr };
        let handle = (&rt as *const SurfaceRuntime)
            .cast_mut()
            .cast::<std::ffi::c_void>();
        let mut changed = 0u32;
        // SAFETY: Valid live `SurfaceRuntime` on owner thread while `guard` holds scene read lock.
        let busy_rc = unsafe { (api.tick)(handle, 0.016, &mut changed) };
        assert_eq!(busy_rc, NveRuntimeStatus::Busy.as_i32());
        // SAFETY: Valid live `SurfaceRuntime` on owner thread while `guard` holds scene read lock.
        let vp_rc = unsafe { (api.set_viewport)(handle, 500.0, 400.0, 1.0) };
        assert_eq!(vp_rc, NveRuntimeStatus::Busy.as_i32());
    }

    // Tick 2: app is clean, ONLY hook mutates card width (220 -> 180) and Visual opacity (0.85 -> 0.5).
    assert!(rt.tick(0.016).expect("tick 2"));
    {
        let guard = rt.acquire_scene().expect("acquire 2");
        let v = guard.view();
        assert_eq!(v.frame_id, 2, "FrameTickHook mutation MUST advance frame_id");
        // SAFETY: `guard` holds the reader lock and `node_capacity >= 5`.
        let rect3 = unsafe { *v.rects.add(3) };
        assert_eq!(
            rect3.width, 180.0,
            "FrameTickHook Layout mutation MUST relayout and update published rects"
        );
        // `dirty_indices`: only the touched card (3); its stretched label (4) only resized through
        // relayout, so it is in `moved_indices` instead.
        // SAFETY: `guard` holds the reader lock for `dirty_count` / `moved_count` indices.
        let (dirty, moved) = unsafe {
            (
                std::slice::from_raw_parts(v.dirty_indices, v.dirty_count as usize),
                std::slice::from_raw_parts(v.moved_indices, v.moved_count as usize),
            )
        };
        assert_eq!(dirty, [3u32]);
        assert_eq!(moved, [4u32]);
        // SAFETY: `guard` holds the reader lock and `node_capacity >= 5`.
        let rect4 = unsafe { *v.rects.add(4) };
        assert_eq!(rect4.width, 180.0, "stretched label follows the card width");

        // Published `NveTableView` for `Visual` must report dirty_count >= 1!
        // SAFETY: `guard` holds the reader lock for `table_count` views.
        let tables = unsafe { std::slice::from_raw_parts(v.tables, v.table_count as usize) };
        let visual_view = tables
            .iter()
            .find(|t| t.slot == Visual::slot().0 as u32)
            .expect("Visual table published");
        assert!(
            visual_view.dirty_count >= 1 && !visual_view.dirty_rows.is_null(),
            "FrameTickHook Visual edit must populate NveTableView::dirty_rows"
        );
    }

    // Switch hook to 60fps paint/visual animation (without relayout inside external Taffy):
    // steady-state dirty hook + ChangeSet merge + scroll reconcile + publish_scene MUST be 0 alloc!
    animate_layout.store(false, Ordering::Release);
    assert!(rt.tick(0.016).expect("warm visual hook tick"));
    let (_, hook_allocs) = count_allocs(|| {
        for _ in 0..32 {
            assert!(rt.tick(0.016).expect("steady-state dirty visual hook tick"));
            let g = rt.acquire_scene().expect("acquire");
            assert_eq!(g.view().dirty_count, 1);
        }
    });
    assert_eq!(
        hook_allocs, 0,
        "steady-state dirty Visual FrameTickHook + commit + scroll reconcile + publish_scene must perform 0 allocs"
    );

    // Reject `submit_nve2` while an app is attached.
    assert_eq!(
        rt.submit_nve2(&[1, 2, 3]),
        Err(RuntimeError::InvalidArg(
            "cannot submit NVE2 wire frames while an app is attached"
        ))
    );
}

struct HostBoundDirectApp {
    store: NodeStore,
    changes: ChangeSet,
    dirty: bool,
    host: Option<NveHostCallbacks>,
    dispatched_batches: usize,
}

impl RuntimeApp for HostBoundDirectApp {
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

    fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
        self.host = callbacks;
    }

    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        if !batch.is_empty() {
            self.dispatched_batches += 1;
        }
        false
    }
}

unsafe extern "C" fn noop_host_invoke(
    _ctx: *mut c_void,
    _service_id: u32,
    _method_id: u32,
    _request_id: u64,
    _request: NveBufferSlice,
    _out_buf: *mut u8,
    _out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if !out_len.is_null() {
        // SAFETY: Test stub writes 0 length.
        unsafe { *out_len = 0 };
    }
    NveHostStatus::Ok.as_i32()
}

#[test]
fn v6_warm_measure_surface_and_idle_host_router_are_zero_alloc() {
    // Positive probe so running this single test still proves `CountingAlloc` is active.
    let (_, probe) = count_allocs(|| black_box(Vec::<u8>::with_capacity(64)));
    assert!(probe >= 1, "CountingAlloc positive probe must observe >= 1 allocation");

    let (store, changes, _) = build_sample_nve2_frame();
    let app_host_slot: Arc<std::sync::Mutex<Option<NveHostCallbacks>>> =
        Arc::new(std::sync::Mutex::new(None));
    struct CapturingApp {
        inner: HostBoundDirectApp,
        slot: Arc<std::sync::Mutex<Option<NveHostCallbacks>>>,
    }
    impl RuntimeApp for CapturingApp {
        fn render_frame(&mut self, frame_id: FrameId) -> AppFrame<'_> {
            self.inner.render_frame(frame_id)
        }
        fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
            self.inner.dispatch_events(events)
        }
        fn direct_store(&self) -> Option<&NodeStore> {
            self.inner.direct_store()
        }
        fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
            self.inner.direct_store_mut()
        }
        fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
            *self.slot.lock().expect("slot lock") = callbacks;
            self.inner.bind_host(callbacks);
        }
        fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
            self.inner.dispatch_host_events(batch)
        }
    }

    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .app(Box::new(CapturingApp {
            inner: HostBoundDirectApp {
                store,
                changes,
                dirty: true,
                host: None,
                dispatched_batches: 0,
            },
            slot: Arc::clone(&app_host_slot),
        }))
        .build();
    rt.set_host_callbacks(Some(NveHostCallbacks {
        host_ctx: std::ptr::null_mut(),
        invoke: Some(noop_host_invoke),
        channel_publish: None,
    }))
    .expect("set_host_callbacks");

    // Warm mount + one host push drain + one dry-run measure so steady-state buffers are primed.
    assert!(rt.tick(1.0 / 60.0).expect("warm mount tick"));
    rt.push_host_event(HostServiceId(0x1001), 1, b"warm")
        .expect("warm push");
    assert!(!rt.tick(1.0 / 60.0).expect("warm drain tick"));
    let probes = [
        (NveSizeConstraints::loose(320.0, 240.0), NVE_MEASURE_FLAG_DRY_RUN),
        (
            NveSizeConstraints {
                min_w: 160.0,
                min_h: 90.0,
                max_w: 160.0,
                max_h: 90.0,
            },
            NVE_MEASURE_FLAG_DRY_RUN,
        ),
        (
            NveSizeConstraints {
                min_w: 120.0,
                min_h: 40.0,
                max_w: 280.0,
                max_h: 160.0,
            },
            NVE_MEASURE_FLAG_DRY_RUN,
        ),
        (NveSizeConstraints::loose(400.0, 300.0), NVE_MEASURE_FLAG_DRY_RUN),
        (
            NveSizeConstraints {
                min_w: 100.0,
                min_h: 40.0,
                max_w: 100.0,
                max_h: 40.0,
            },
            NVE_MEASURE_FLAG_DRY_RUN | NVE_MEASURE_FLAG_IDEAL,
        ),
    ];
    for &(c, flags) in &probes {
        let warm_m = rt.measure_surface(c, flags).expect("warm measure");
        assert!(warm_m.width > 0.0 && warm_m.height > 0.0);
    }

    // 1. Cycling multiple distinct constraints + `IDEAL` MUST perform 0 heap allocations AND keep
    //    distinct dimensions intact without clobbering committed clean-frame state.
    let (_, measure_allocs) = count_allocs(|| {
        for i in 0..40 {
            let (c, flags) = probes[i % probes.len()];
            let m = rt
                .measure_surface(c, flags)
                .expect("steady-state varying measure_surface");
            if (flags & NVE_MEASURE_FLAG_IDEAL) != 0 {
                assert!(m.width >= 200.0 && m.height >= 80.0);
            } else if c.min_w == 160.0 && c.max_w == 160.0 {
                assert_eq!((m.width, m.height), (160.0, 90.0));
            }
            black_box(m);
        }
    });
    assert_eq!(
        measure_allocs, 0,
        "warm measure_surface cycling varying constraints + IDEAL must perform 0 heap allocations"
    );

    // 2. Idle `RuntimeHostRouter::drain_into_app` + steady-state `tick()` MUST perform 0 allocs.
    let (_, idle_tick_allocs) = count_allocs(|| {
        for _ in 0..32 {
            assert!(!rt.tick(1.0 / 60.0).expect("steady-state idle tick"));
        }
    });
    assert_eq!(
        idle_tick_allocs, 0,
        "idle RuntimeHostRouter::drain_into_app + steady-state tick() must perform 0 allocs"
    );

    // 3. `SERVICE_WINDOW::METHOD_WINDOW_REQUEST_FRAME` routed through `RuntimeHostRouter` = 0 allocs.
    let cb = app_host_slot.lock().expect("slot lock").expect("bound host callbacks");
    let invoke = cb.invoke.expect("invoke fn");
    let (_, req_frame_allocs) = count_allocs(|| {
        let mut out_len = 0usize;
        for i in 1..=32u64 {
            // SAFETY: Bound host callbacks outlive `rt` while app stays attached.
            let rc = unsafe {
                invoke(
                    cb.host_ctx,
                    SERVICE_WINDOW,
                    METHOD_WINDOW_REQUEST_FRAME,
                    i,
                    NveBufferSlice::EMPTY,
                    std::ptr::null_mut(),
                    0,
                    &mut out_len,
                )
            };
            assert_eq!(rc, NveHostStatus::Ok.as_i32());
        }
    });
    assert_eq!(
        req_frame_allocs, 0,
        "SERVICE_WINDOW::METHOD_WINDOW_REQUEST_FRAME must perform 0 heap allocations"
    );
}
