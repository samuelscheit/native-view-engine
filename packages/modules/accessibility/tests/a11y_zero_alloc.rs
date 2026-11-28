//! Zero-allocation regression guards proving that `AccessKitTreeBridge::sync_changes` performs
//! **0** heap allocations when inactive AND **0** heap allocations on `COMPOSITE`-only 120 Hz
//! animation frames (with positive `CountingAlloc` probe).

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use nve_accessibility::{A11yGeometryProvider, AccessKitTreeBridge};
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_tables::tables::{Accessibility, AriaRole, Transform, Visual};

struct CountingAlloc;

static RECORDING: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static SERIAL_TEST: Mutex<()> = Mutex::new(());

// SAFETY: delegates directly to `System` global allocator and bumps a lock-free counter.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        if RECORDING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegated directly to the OS `System` allocator with identical layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: delegated directly to `System::dealloc` for memory allocated by `System`.
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

struct DummyGeo;
impl A11yGeometryProvider for DummyGeo {
    fn layout_rect(&self, id: NodeId) -> Option<LayoutRect> {
        Some(LayoutRect {
            x: 0.0,
            y: (id.index() as f32) * 20.0,
            width: 240.0,
            height: 20.0,
        })
    }
}

#[test]
fn counting_alloc_positive_probe() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let allocs = measure_allocs(|| {
        let v: Vec<u8> = black_box(vec![7u8; 64]);
        black_box(v);
    });
    assert!(allocs >= 1, "CountingAlloc must observe real heap allocations, got {allocs}");
}

#[test]
fn inactive_and_composite_only_frames_allocate_zero_times() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let target = {
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(200);
        for i in 0..200usize {
            let n = tx.create();
            tx.edit::<Accessibility>(n)
                .role(Some(AriaRole::Button))
                .label(&format!("Btn {i}"));
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        tx.commit_into(&mut changes);
        children[10]
    };

    let geo = DummyGeo;
    let mut inactive = AccessKitTreeBridge::new();
    let inactive_allocs = measure_allocs(|| {
        for _ in 0..32 {
            assert!(inactive.sync_changes(&store, &changes, &geo, "Demo", false).is_none());
        }
    });
    assert_eq!(inactive_allocs, 0, "inactive bridge must perform 0 heap allocations");

    // Warm up active bridge + Transform/Visual rows + ChangeSet buffers.
    let mut active = AccessKitTreeBridge::new();
    active.activate();
    let _ = active.build_full_tree_update(&store, &geo, "Demo");
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(target).opacity(0.9);
        tx.edit::<Transform>(target).translate_x(2.0);
        tx.commit_into(&mut changes);
    }
    assert!(active.sync_changes(&store, &changes, &geo, "Demo", false).is_none());

    let composite_allocs = measure_allocs(|| {
        for step in 1..=32u32 {
            let mut tx = store.tx();
            tx.edit::<Visual>(target).opacity(0.5 + (step as f32) * 0.01);
            tx.edit::<Transform>(target).translate_x((step as f32) * 1.5);
            tx.commit_into(&mut changes);
            assert!(active.sync_changes(&store, &changes, &geo, "Demo", false).is_none());
        }
    });
    assert_eq!(
        composite_allocs, 0,
        "32x COMPOSITE-only frames must perform 0 heap allocations on active AccessKitTreeBridge"
    );
}
