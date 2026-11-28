//! Thread-local `CountingAlloc` zero-allocation hot-path guard for `CanvasSkiaSystem::paint_node`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_backend_skia::{PixelBuffer, SkiaSystemPaintCtx, SkiaTableSystem};
use nve_canvas::{Canvas, DrawListRecorder, MAX_CLIP_DEPTH};
use nve_canvas_skia::CanvasSkiaSystem;
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

// SAFETY: Delegates directly to `std::alloc::System` while counting per-thread allocations.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let r = f();
    let after = THREAD_ALLOCS.with(Cell::get);
    (after.wrapping_sub(before), r)
}

#[test]
fn canvas_skia_system_paint_node_allocates_zero_bytes() {
    // Positive probe: verify `CountingAlloc` observes real heap allocations on this thread.
    let (probe_allocs, probe_box) = measure_allocs(|| Box::new(black_box([1u64; 16])));
    black_box(&probe_box);
    assert!(
        probe_allocs >= 1,
        "CountingAlloc positive probe must observe >= 1 allocation, got {probe_allocs}"
    );

    let mut rec = DrawListRecorder::new();
    rec.clear(ColorRgba::BLACK)
        .fill_rect(2.0, 2.0, 28.0, 28.0, ColorRgba::rgb(20, 40, 60))
        .fill_rounded_rect(4.0, 4.0, 20.0, 12.0, 4.0, ColorRgba::rgb(0, 200, 120))
        .fill_circle(24.0, 24.0, 4.0, ColorRgba::rgb(0, 120, 255))
        .stroke_line(1.0, 1.0, 30.0, 30.0, 2.0, ColorRgba::rgb(255, 180, 0))
        .stroke_polyline(
            &[(2.0, 28.0), (12.0, 16.0), (22.0, 24.0), (30.0, 8.0)],
            2.0,
            ColorRgba::rgb(240, 80, 160),
        );
    for _ in 0..(MAX_CLIP_DEPTH + 2) {
        rec.clip_rect(4.0, 4.0, 16.0, 16.0);
    }
    rec.restore_clip()
        .restore_clip()
        .fill_rect(0.0, 0.0, 32.0, 32.0, ColorRgba::rgb(0, 255, 0))
        .restore_clip();

    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let node = tx.create();
    tx.set_children(NodeId::ROOT, &[node]);
    tx.edit::<Canvas>(node)
        .draw_list_ephemeral(rec.finish())
        .generation(1);
    let mut changes = ChangeSet::default();
    tx.commit_into(&mut changes);

    let mut buf = PixelBuffer::new(32, 32);
    let mut events = EventBatchBuf::new();
    let rect = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: 32.0,
        height: 32.0,
    };

    let (allocs, ()) = measure_allocs(|| {
        for _ in 0..16 {
            let mut ctx = SkiaSystemPaintCtx {
                buffer: &mut buf,
                rect,
                clip_rect: None,
                scale_factor: 1.0,
                opacity: 1.0,
                inherited_text_color: Rgba::WHITE,
                events: &mut events,
            };
            CanvasSkiaSystem.paint_node(&mut ctx, black_box(&store), node);
        }
    });

    assert_eq!(buf.get_rgb(8, 8), Some((0, 255, 0)));
    assert_eq!(
        allocs, 0,
        "CanvasSkiaSystem::paint_node must allocate 0 bytes during steady-state painting"
    );
}
