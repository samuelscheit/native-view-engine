//! End-to-end 0-allocation guard proving that `MotionDriver` + `SkiaBackend`'s retained
//! `CompositorScene` runs steady-state 120 Hz `opacity` + `transform` ticks AND live retarget
//! commits with ZERO Rust heap allocations and ZERO chunk re-emissions.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use nve_abi::SurfaceId;
use nve_backend_skia::retained::SceneDirtyKind;
use nve_backend_skia::SkiaBackend;
use nve_engine::{StoreBackend, Viewport};
use nve_motion::{EasingCurve, Motion, TransitionProperty, TransitionSpec, Transitions};
use nve_motion_skia::MotionDriver;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{Layout, Transform, Visual};

struct CountingAlloc;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);
static SERIAL_TEST: Mutex<()> = Mutex::new(());

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

// SAFETY: Delegates directly to `std::alloc::System`; `try_with` isolates per-thread counting
// without ever panicking during TLS teardown inside `GlobalAlloc`.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
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
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
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
    let _ = COUNTING.try_with(|c| c.set(true));
    f();
    let _ = COUNTING.try_with(|c| c.set(false));
    (
        ALLOC_COUNT.load(Ordering::SeqCst),
        ALLOC_BYTES.load(Ordering::SeqCst),
    )
}

#[test]
fn counting_alloc_probe_detects_real_heap_allocation() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let (allocs, bytes) = measure_allocs(|| {
        let v: Vec<u8> = black_box(vec![7u8; 256]);
        black_box(v);
    });
    assert!(
        allocs >= 1 && bytes >= 256,
        "CountingAlloc positive probe must observe real heap allocations, got ({allocs}, {bytes})"
    );
}

#[test]
fn motion_driver_compositor_120hz_composite_ticks_and_retargets_are_zero_alloc() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
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

    let (nodes, paint_probe) = {
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
        let paint_probe = tx.create();
        tx.edit::<Layout>(paint_probe)
            .width(Length::points(20.0))
            .height(Length::points(20.0));
        tx.edit::<Visual>(paint_probe)
            .background(Rgba::rgb(10, 20, 30));
        let mut children = out.clone();
        children.push(paint_probe);
        tx.set_children(NodeId::ROOT, &children);
        tx.commit_into(&mut changes);
        (out, paint_probe)
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

    // Precede the 30 composite ticks with an unrelated `PaintOnly` commit so the `CompositeOnly`
    // `last_dirty_kind` assertion below genuinely falsifies stale dirty-kind reporting!
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(paint_probe)
            .background(Rgba::rgb(40, 50, 60));
        tx.commit_into(&mut changes);
    }
    backend
        .commit(&store, &changes)
        .expect("preceding paint commit");
    assert_eq!(
        backend.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::PaintOnly)
    );

    // Steady-state 120 Hz advance_animations ticks MUST allocate 0 bytes, emit 0 paint chunks,
    // and progress interpolated opacity & transform monotonically across all 30 ticks!
    let emitted_before = backend.scene().stats().chunks_emitted;
    let probe_node = nodes[0];
    let start_op = backend
        .scene()
        .effect_for_node(probe_node)
        .expect("probe effect")
        .opacity;
    let start_tx = backend
        .scene()
        .transform_for_node(probe_node)
        .expect("probe tf")
        .local_transform
        .translate_x;
    let mut prev_op = start_op;
    let mut prev_tx = start_tx;
    let mut monotonic = true;
    let (tick_allocs, tick_bytes) = measure_allocs(|| {
        for _ in 0..30 {
            let active = backend.advance_animations(&store, 1.0 / 120.0);
            assert!(active);
            let cur_op = backend
                .scene()
                .effect_for_node(probe_node)
                .unwrap()
                .opacity;
            let cur_tx = backend
                .scene()
                .transform_for_node(probe_node)
                .unwrap()
                .local_transform
                .translate_x;
            if !(cur_op < prev_op && cur_tx > prev_tx) {
                monotonic = false;
            }
            prev_op = cur_op;
            prev_tx = cur_tx;
        }
    });
    assert!(
        monotonic && prev_op < start_op - 0.3 && prev_tx > start_tx + 35.0,
        "opacity/transform must progress monotonically over 30 ticks (op {start_op}->{prev_op}, tx {start_tx}->{prev_tx})"
    );
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
