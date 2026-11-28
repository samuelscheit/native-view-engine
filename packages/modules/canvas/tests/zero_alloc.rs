//! Thread-local `CountingAlloc` zero-allocation hot-path guards for `nve-canvas`:
//! `DrawList::validate`, `DrawListView::ops`, and `PolylinePoints::iter`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use nve_abi::ColorRgba;
use nve_canvas::{CanvasOpRef, DrawList, DrawListRecorder, MAX_CLIP_DEPTH};
use nve_store::{FlatResource, ValidatedWords};

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
fn draw_list_validate_and_view_ops_allocate_zero_bytes() {
    // Positive probe: verify `CountingAlloc` observes real heap allocations on this thread.
    let (probe_allocs, probe_vec) = measure_allocs(|| {
        let mut v = Vec::with_capacity(64);
        v.push(black_box(42u32));
        v
    });
    black_box(&probe_vec);
    assert!(
        probe_allocs >= 1,
        "CountingAlloc positive probe must observe >= 1 allocation, got {probe_allocs}"
    );

    let mut rec = DrawListRecorder::new();
    rec.clear(ColorRgba::BLACK)
        .fill_rect(1.0, 2.0, 40.0, 20.0, ColorRgba::WHITE)
        .fill_rounded_rect(4.0, 5.0, 20.0, 10.0, 3.0, ColorRgba::rgb(1, 2, 3))
        .fill_circle(12.0, 14.0, 6.0, ColorRgba::rgb(4, 5, 6))
        .stroke_line(0.0, 0.0, 10.0, 10.0, 2.0, ColorRgba::rgb(7, 8, 9))
        .stroke_polyline(
            &[(0.0, 0.0), (5.0, 10.0), (10.0, 2.0), (16.0, 8.0)],
            1.5,
            ColorRgba::rgb(9, 8, 7),
        );
    for _ in 0..(MAX_CLIP_DEPTH + 2) {
        rec.clip_rect(0.0, 0.0, 50.0, 50.0);
    }
    rec.restore_clip().restore_clip().restore_clip();

    let mut words = Vec::new();
    assert!(rec.finish().write_words(&mut words));
    let mut bad_words = words.clone();
    // word 0 = op_count, word 1 = OP_CLEAR, word 2 = color, word 3 = OP_FILL_RECT, word 4 = x
    bad_words[4] = f32::NAN.to_bits();

    let (allocs, (validated_count, ops_seen, pts_seen)) = measure_allocs(|| {
        let mut valid_hits = 0usize;
        let mut ops = 0usize;
        let mut pts = 0usize;
        for _ in 0..64 {
            assert!(DrawList::validate(black_box(&words)));
            assert!(!DrawList::validate(black_box(&bad_words)));
            valid_hits += 1;

            let vw = ValidatedWords::<DrawList>::try_new(black_box(&words))
                .expect("valid draw list");
            let view = DrawList::view(vw);
            for op in view.ops() {
                ops += 1;
                match op {
                    CanvasOpRef::StrokePolyline { points, width, color } => {
                        black_box((width, color));
                        for pt in points.iter() {
                            black_box(pt);
                            pts += 1;
                        }
                    }
                    other => {
                        black_box(other);
                    }
                }
            }
        }
        (valid_hits, ops, pts)
    });

    assert_eq!(validated_count, 64);
    assert_eq!(ops_seen, 64 * (6 + MAX_CLIP_DEPTH + 2 + 3));
    assert_eq!(pts_seen, 64 * 4);
    assert_eq!(
        allocs, 0,
        "DrawList::validate and DrawListView::ops must allocate 0 bytes"
    );
}
