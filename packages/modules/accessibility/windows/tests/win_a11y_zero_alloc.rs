//! Zero-allocation guards for `nve-accessibility-windows::WinA11yBridge`:
//! - Inactive mirrors perform 0 heap allocations per `sync_scene` call.
//! - Active mirrors on an unchanged `(scene_epoch, frame_id, focus, scale, title)` perform 0 heap
//!   allocations.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use nve_abi::{NveLayoutRect, NveSceneView};
use nve_accessibility_windows::{NoopResources, WinA11yBridge};
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_tables::tables::{core_realization_order, Accessibility, AriaRole};

struct CountingAlloc;

static RECORDING: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static SERIAL_TEST: Mutex<()> = Mutex::new(());

// SAFETY: delegates directly to `System` global allocator and tracks allocation count atomically.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        if RECORDING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegated directly to the OS `System` allocator with identical layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: delegated directly to `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        if RECORDING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegated directly to `System::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs(f: impl FnOnce()) -> usize {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    RECORDING.store(true, Ordering::SeqCst);
    f();
    RECORDING.store(false, Ordering::SeqCst);
    ALLOC_COUNT.load(Ordering::SeqCst)
}

#[test]
fn counting_alloc_positive_probe() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let allocs = measure_allocs(|| {
        let v: Vec<u8> = black_box(vec![3u8; 64]);
        black_box(v);
    });
    assert!(allocs >= 1, "CountingAlloc must observe real heap allocations, got {allocs}");
}

#[test]
fn inactive_and_unchanged_scene_sync_allocate_zero_times() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());

    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let focus = {
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(128);
        for i in 0..128usize {
            let n = tx.create();
            tx.edit::<Accessibility>(n)
                .role(Some(AriaRole::Button))
                .label(&format!("Action {i}"));
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        tx.commit_into(&mut changes);
        children[4]
    };

    let mut tables = Vec::new();
    let mut dirty_buf = Vec::new();
    store.publish_table_views(&changes, &mut tables, &mut dirty_buf);
    let rows = store.node_rows();
    let rects = vec![
        NveLayoutRect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 24.0,
        };
        rows.len()
    ];
    let scene = NveSceneView {
        frame_id: 1,
        scene_epoch: 1,
        node_capacity: rows.len() as u32,
        rows: rows.as_ptr().cast(),
        rects: rects.as_ptr(),
        text_slab: store.text_slab().as_ptr(),
        text_slab_len: store.text_slab().len() as u32,
        children_slab: store.children_slab().as_ptr().cast(),
        children_slab_len: store.children_slab().len() as u32,
        tables: tables.as_ptr(),
        table_count: tables.len() as u32,
        ..NveSceneView::default()
    };

    let mut inactive = WinA11yBridge::new();
    let inactive_allocs = measure_allocs(|| {
        for _ in 0..50 {
            // SAFETY: `scene` references live local slices for this call.
            let upd = unsafe {
                inactive.sync_scene(&scene, &NoopResources, focus.raw(), 1.0, "ZeroAlloc")
            };
            assert!(upd.is_none());
        }
    });
    assert_eq!(inactive_allocs, 0, "inactive WinA11yBridge must allocate 0 times");

    let mut active = WinA11yBridge::new();
    active.activate();
    // SAFETY: `scene` references live local slices for this call.
    let initial = unsafe { active.sync_scene(&scene, &NoopResources, focus.raw(), 1.0, "ZeroAlloc") };
    assert!(initial.is_some());

    let unchanged_allocs = measure_allocs(|| {
        for _ in 0..50 {
            // SAFETY: `scene` references live local slices for this call.
            let upd = unsafe {
                active.sync_scene(&scene, &NoopResources, focus.raw(), 1.0, "ZeroAlloc")
            };
            assert!(upd.is_none());
        }
    });
    assert_eq!(
        unchanged_allocs, 0,
        "unchanged active WinA11yBridge frame must allocate 0 times"
    );
}
