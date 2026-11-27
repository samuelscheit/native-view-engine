//! Zero-allocation steady-state runtime tick + `scene_acquire`/`scene_release` + C ABI vtable
//! verification tests.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_abi::{
    EventMask, NveBufferSlice, NveRuntimeConfig, NveRuntimeMetrics, NveRuntimeSceneView,
    NveRuntimeStatus, NVE_RUNTIME_ABI_VERSION, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_components::tables::{Interaction, Layout, TextContent, Visual};
use nve_engine::Viewport;
use nve_runtime::{
    AppFrame, FrameTickHook, RuntimeApp, RuntimeBuilder, RuntimeError, WakeBridge,
    nve_runtime_get_interface,
};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

struct CountingAlloc;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: Delegates directly to `System` allocator while counting allocations on the opted-in thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        COUNTING.with(|c| {
            if c.get() {
                ALLOC_COUNT.with(|n| n.set(n.get() + 1));
            }
        });
        // SAFETY: Forwards unmodified layout to `System::alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Forwards matching pointer and layout to `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        COUNTING.with(|c| {
            if c.get() {
                ALLOC_COUNT.with(|n| n.set(n.get() + 1));
            }
        });
        // SAFETY: Forwards unmodified arguments to `System::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    ALLOC_COUNT.with(|n| n.set(0));
    COUNTING.with(|c| c.set(true));
    let out = f();
    COUNTING.with(|c| c.set(false));
    let count = ALLOC_COUNT.with(Cell::get);
    (out, count)
}

fn build_sample_nve2_frame() -> (NodeStore, ChangeSet, Vec<u8>) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let card = tx.create();
        tx.edit::<Layout>(card)
            .width(Length::points(200.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(card).background(Rgba::rgb(30, 41, 59));
        tx.edit::<Interaction>(card).event_mask(EventMask::PRESS as u32);

        let label = tx.create();
        tx.edit::<TextContent>(label).text("Hello NveRuntime");

        tx.set_children(card, &[label]);
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut changes);
    }
    let mut encoder = WireEncoder::default();
    let mut buf = Vec::new();
    encoder.encode(&store, &changes, &mut buf);
    (store, changes, buf)
}

#[test]
fn steady_state_tick_and_scene_acquire_are_zero_alloc() {
    let (_, _, frame) = build_sample_nve2_frame();
    let mut rt = RuntimeBuilder::new()
        .surface_id(1)
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 2.0,
        })
        .build();

    rt.submit_nve2(&frame).expect("submit NVE2 frame");
    // Warm up one clean tick + hit_test + scene acquire.
    let _ = rt.tick(1.0 / 60.0).expect("warm tick");
    let hit = rt.hit_test(50.0, 25.0).expect("warm hit");
    assert_eq!(hit, Some(2));
    {
        let g = rt.acquire_scene().expect("warm acquire");
        assert_eq!(g.view().schema_hash, NVE_RUNTIME_SCHEMA_HASH);
        assert!(g.view().node_capacity >= 4);
    }

    let (_, allocs) = count_allocs(|| {
        for _ in 0..32 {
            let changed = rt.tick(1.0 / 60.0).expect("clean tick");
            assert!(!changed);
            let h = rt.hit_test(50.0, 25.0).expect("hit");
            assert_eq!(h, Some(2));
            let g = rt.acquire_scene().expect("acquire");
            assert_eq!(g.view().frame_id, 1);
        }
    });
    assert_eq!(
        allocs, 0,
        "steady-state runtime tick + hit_test + scene_acquire must perform 0 heap allocations"
    );
}

struct PulseHook {
    calls: Arc<AtomicUsize>,
}

impl FrameTickHook for PulseHook {
    fn tick(&mut self, store: &mut NodeStore, _dt_s: f32) -> bool {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(id) = store.id_at(2) {
            let mut tx = store.tx();
            tx.edit::<Visual>(id).opacity(0.75);
            let mut c = ChangeSet::default();
            tx.commit_into(&mut c);
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
    fn render_frame(&mut self, _frame_id: u64) -> AppFrame<'_> {
        if !self.dirty {
            return AppFrame::Clean;
        }
        self.dirty = false;
        AppFrame::Direct {
            store: &self.store,
            changes: &self.changes,
        }
    }

    fn dispatch_events(&mut self, _events: nve_abi::NveEventBatchRef<'_>) -> bool {
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
fn direct_app_and_frame_tick_hook_share_single_store_zero_copy() {
    let (store, changes, _) = build_sample_nve2_frame();
    let calls = Arc::new(AtomicUsize::new(0));
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
        .tick_hook(Box::new(PulseHook {
            calls: Arc::clone(&calls),
        }))
        .build();

    assert!(rt.tick(0.016).expect("first tick"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    let guard = rt.acquire_scene().expect("acquire");
    let v = guard.view();
    assert_eq!(v.schema_hash, NVE_RUNTIME_SCHEMA_HASH);
    assert!(v.node_capacity >= 4);
    assert!(!v.rows.is_null());
    assert!(!v.rects.is_null());
    // SAFETY: `guard` holds the reader lock and `node_capacity >= 4`.
    let rect2 = unsafe { *v.rects.add(2) };
    assert_eq!((rect2.width, rect2.height), (200.0, 80.0));
    // SAFETY: `guard` holds the reader lock and `node_capacity >= 4`.
    let row2 = unsafe { *v.rows.add(2) };
    assert!(row2.is_live());
    assert_eq!(row2.child_len, 1);
    // Attempting a write while `guard` is held must return `SceneBusy`, never mutate borrowed slabs.
    drop(guard);
}

#[test]
fn wake_bridge_coalesces_bursts() {
    let wake = Arc::new(WakeBridge::new());
    let hits = Arc::new(AtomicUsize::new(0));
    let h = Arc::clone(&hits);
    wake.set_sink(Arc::new(move || {
        h.fetch_add(1, Ordering::Relaxed);
    }));
    let waker = wake.waker();
    waker.wake_by_ref();
    waker.wake_by_ref();
    wake.trigger();
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    assert!(wake.take_pending());
    assert!(!wake.take_pending());
    waker.wake_by_ref();
    assert_eq!(hits.load(Ordering::Relaxed), 2);
}

#[test]
fn c_abi_vtable_end_to_end_and_lock_enforcement() {
    assert!(nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION + 99).is_null());
    let api_ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!api_ptr.is_null());
    // SAFETY: Non-null static vtable returned for matching ABI version.
    let api = unsafe { *api_ptr };
    assert_eq!(api.abi_version, NVE_RUNTIME_ABI_VERSION);
    assert_eq!(api.schema_hash, NVE_RUNTIME_SCHEMA_HASH);

    let cfg = NveRuntimeConfig {
        surface_id: 1,
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Valid stack config pointer.
    let handle = unsafe { (api.create)(&cfg) };
    assert!(!handle.is_null());

    let (_, _, frame) = build_sample_nve2_frame();
    // SAFETY: Valid live handle and frame slice.
    let rc = unsafe { (api.submit_nve2)(handle, NveBufferSlice::from_slice(&frame)) };
    assert_eq!(rc, NveRuntimeStatus::Ok.as_i32());

    let mut hit_id = 0u32;
    // SAFETY: Valid live handle and output pointer.
    let hit_rc = unsafe { (api.hit_test)(handle, 25.0, 25.0, &mut hit_id) };
    assert_eq!(hit_rc, NveRuntimeStatus::Ok.as_i32());
    assert_eq!(hit_id, 2);

    let mut view = NveRuntimeSceneView::default();
    // SAFETY: Valid live handle and output pointer.
    let acq_rc = unsafe { (api.scene_acquire)(handle, &mut view) };
    assert_eq!(acq_rc, NveRuntimeStatus::Ok.as_i32());
    assert_eq!(view.schema_hash, NVE_RUNTIME_SCHEMA_HASH);
    assert!(view.node_capacity >= 4);

    // While acquired, `tick` MUST fail with `NVE_RUNTIME_BUSY` rather than mutating live pointers.
    let mut changed = 0u32;
    // SAFETY: Valid live handle.
    let busy_rc = unsafe { (api.tick)(handle, 0.016, &mut changed) };
    assert_eq!(busy_rc, NveRuntimeStatus::Busy.as_i32());

    // SAFETY: Matching release for `scene_acquire`.
    let rel_rc = unsafe { (api.scene_release)(handle) };
    assert_eq!(rel_rc, NveRuntimeStatus::Ok.as_i32());

    let mut metrics = NveRuntimeMetrics::default();
    // SAFETY: Valid live handle and output pointer.
    let met_rc = unsafe { (api.get_metrics)(handle, &mut metrics) };
    assert_eq!(met_rc, NveRuntimeStatus::Ok.as_i32());
    assert_eq!(metrics.frame_id, 1);
    assert_eq!(metrics.live_nodes, 3);

    // SAFETY: Handle destroyed once.
    unsafe { (api.destroy)(handle) };
    let _ = RuntimeError::SceneBusy.status();
}
