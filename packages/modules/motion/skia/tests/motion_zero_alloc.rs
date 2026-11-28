//! End-to-end 0-allocation guard proving that `MotionDriver` + `SkiaBackend`'s retained
//! `CompositorScene` runs steady-state 120 Hz `opacity` + `transform` ticks AND live retarget
//! commits with ZERO Rust heap allocations and ZERO chunk re-emissions.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nve_abi::SurfaceId;
use nve_backend_skia::retained::SceneDirtyKind;
use nve_backend_skia::SkiaBackend;
use nve_engine::{StoreBackend, Viewport};
use nve_motion::{EasingCurve, Motion, TransitionProperty, TransitionSpec, Transitions};
use nve_motion_skia::MotionDriver;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{Layout, Transform, Visual};

struct CountingAlloc;

static TRACKING: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

// SAFETY: Delegates directly to `std::alloc::System` while updating lock-free atomic counters.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: `layout` satisfies `GlobalAlloc::alloc` preconditions from the caller.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: `ptr` and `layout` were returned by `System.alloc` via this allocator.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        // SAFETY: `ptr`, `layout`, and `new_size` satisfy `GlobalAlloc::realloc` preconditions.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn measure_allocs<F: FnOnce()>(f: F) -> (usize, usize) {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    ALLOC_BYTES.store(0, Ordering::SeqCst);
    TRACKING.store(true, Ordering::SeqCst);
    f();
    TRACKING.store(false, Ordering::SeqCst);
    (
        ALLOC_COUNT.load(Ordering::SeqCst),
        ALLOC_BYTES.load(Ordering::SeqCst),
    )
}

#[test]
fn motion_driver_compositor_120hz_composite_ticks_and_retargets_are_zero_alloc() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.register_frame_hook(MotionDriver::new());
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        },
    );

    let nodes: Vec<NodeId> = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(320.0))
            .height(Length::points(240.0));
        let mut out = Vec::with_capacity(16);
        for i in 0..16u8 {
            let n = tx.create();
            tx.edit::<Layout>(n)
                .width(Length::points(60.0))
                .height(Length::points(24.0));
            tx.edit::<Visual>(n)
                .background(Rgba::rgb(40 + i * 10, 90, 200))
                .opacity(1.0);
            tx.edit::<Transform>(n).translate_x(0.0);
            tx.edit::<Motion>(n).transitions(Some(Transitions::new([
                TransitionSpec::new(TransitionProperty::Opacity, 400).easing(EasingCurve::Linear),
                TransitionSpec::new(TransitionProperty::Transform, 400)
                    .easing(EasingCurve::Linear),
            ])));
            out.push(n);
        }
        tx.set_children(NodeId::ROOT, &out);
        tx.commit_into(&mut changes);
        out
    };
    backend.commit(&store, &changes).expect("initial commit");

    // Warm up transition state + scratch buffers with an initial trigger and a few ticks.
    {
        let mut tx = store.tx();
        for (i, &n) in nodes.iter().enumerate() {
            tx.edit::<Visual>(n).opacity(0.25 + (i as f32) * 0.02);
            tx.edit::<Transform>(n).translate_x(80.0 + (i as f32) * 4.0);
        }
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("warmup trigger");
    assert!(backend.needs_next_frame());
    assert!(backend.advance_animations(&store, 1.0 / 120.0));
    assert!(backend.advance_animations(&store, 1.0 / 120.0));

    // Steady-state 120 Hz advance_animations ticks MUST allocate 0 bytes and emit 0 paint chunks!
    let emitted_before = backend.scene().stats().chunks_emitted;
    let (tick_allocs, tick_bytes) = measure_allocs(|| {
        for _ in 0..30 {
            let active = backend.advance_animations(&store, 1.0 / 120.0);
            assert!(active);
        }
    });
    assert_eq!(
        (tick_allocs, tick_bytes),
        (0, 0),
        "steady-state 120 Hz MotionDriver composite ticks must allocate 0 bytes"
    );
    assert_eq!(
        backend.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::CompositeOnly)
    );
    assert_eq!(
        backend.scene().stats().chunks_emitted,
        emitted_before,
        "composite-only motion ticks must not re-emit paint chunks"
    );

    // Steady-state retarget commits while transitions are already allocated MUST also allocate 0 bytes!
    let target_a = nodes[0];
    let target_b = nodes[1];
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(target_a).opacity(0.6);
        tx.edit::<Transform>(target_b).translate_x(40.0);
        tx.commit_into(&mut changes);
        backend.commit(&store, &changes).expect("warmup retarget");
    }
    let emitted_before_retarget = backend.scene().stats().chunks_emitted;
    let (retarget_allocs, retarget_bytes) = measure_allocs(|| {
        for step in 0..16u32 {
            let mut tx = store.tx();
            tx.edit::<Visual>(target_a)
                .opacity(0.3 + ((step & 7) as f32) * 0.05);
            tx.edit::<Transform>(target_b)
                .translate_x(20.0 + ((step & 7) as f32) * 5.0);
            tx.commit_into(&mut changes);
            backend.commit(&store, &changes).expect("retarget commit");
            let _ = backend.advance_animations(&store, 1.0 / 120.0);
        }
    });
    assert_eq!(
        (retarget_allocs, retarget_bytes),
        (0, 0),
        "steady-state MotionDriver retarget + tick must allocate 0 bytes"
    );
    assert_eq!(
        backend.scene().stats().chunks_emitted,
        emitted_before_retarget
    );

    // Reduced-motion snap completes cleanly with 0 allocs and puts display link to sleep.
    let (rm_allocs, rm_bytes) = measure_allocs(|| {
        backend.set_reduced_motion(&store, true);
    });
    assert_eq!(
        (rm_allocs, rm_bytes),
        (0, 0),
        "set_reduced_motion snap must allocate 0 bytes"
    );
    assert!(!backend.needs_next_frame());
}
