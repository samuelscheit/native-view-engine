//! Zero-allocation steady-state guards (`[INV:bench-and-alloc-guards]`) for `SlabLru`,
//! warm `SkiaCanvasRenderer` `TextBlob` lookups, and warm `CpuSkiaSurface` frames.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_backend_skia::slab_lru::{IdentityU64BuildHasher, SlabLru};

struct CountingAlloc;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: Delegates directly to `System` allocator while counting allocations on the active thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNTING.with(|active| {
            if active.get() {
                ALLOC_COUNT.with(|c| c.set(c.get() + 1));
            }
        });
        // SAFETY: Forwards valid `GlobalAlloc::alloc` layout directly to `std::alloc::System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forwards pointer and layout previously allocated by `System` back to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        COUNTING.with(|active| {
            if active.get() {
                ALLOC_COUNT.with(|c| c.set(c.get() + 1));
            }
        });
        // SAFETY: Forwards pointer, layout, and new size previously allocated by `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    ALLOC_COUNT.with(|c| c.set(0));
    COUNTING.with(|a| a.set(true));
    let out = f();
    COUNTING.with(|a| a.set(false));
    let count = ALLOC_COUNT.with(|c| c.get());
    (out, count)
}

#[test]
fn slab_lru_warm_get_mut_allocates_zero_times() {
    let mut lru: SlabLru<u64, u64, IdentityU64BuildHasher> =
        SlabLru::with_initial_capacity_and_hasher(128, 128 * 64, 128, IdentityU64BuildHasher);
    for i in 0..64u64 {
        lru.insert(i, i * 10, 64);
    }

    let (sum, allocs) = count_allocs(|| {
        let mut acc = 0u64;
        for round in 0..100u64 {
            for k in 0..64u64 {
                if let Some(v) = lru.get_mut(&((k + round) % 64)) {
                    acc = acc.wrapping_add(*v);
                }
            }
        }
        acc
    });
    assert!(sum > 0);
    assert_eq!(allocs, 0, "SlabLru warm get_mut must allocate 0 times");
}

#[cfg(feature = "skia")]
#[test]
fn warm_skia_text_blob_lookup_and_cpu_frame_allocate_zero_rust_heap() {
    use nve_abi::ColorRgba;
    use nve_backend_skia::image_cache::ImageCache;
    use nve_backend_skia::text::TextAlign;
    use nve_backend_skia::{CpuSkiaSurface, DisplayList, PaintOp, SkiaCanvasRenderer};
    use nve_tables::paint::{BackgroundFill, GradientStop, LinearGradient};
    use nve_tables::tables::ImageFit;
    use nve_engine::LayoutRect;

    // 1. Warm `SkiaCanvasRenderer::build_cached_text_blob` lookup allocates 0 Rust heap bytes.
    let renderer = SkiaCanvasRenderer::new();
    let warm_blob = renderer.build_cached_text_blob(
        "Zero-alloc cached HarfBuzz TextBlob",
        16.0,
        400,
        TextAlign::Left,
        Some(240.0),
        240.0,
    );
    assert!(warm_blob.is_some());

    let (hits, blob_allocs) = count_allocs(|| {
        let mut count = 0usize;
        for _ in 0..100 {
            if renderer
                .build_cached_text_blob(
                    "Zero-alloc cached HarfBuzz TextBlob",
                    16.0,
                    400,
                    TextAlign::Left,
                    Some(240.0),
                    240.0,
                )
                .is_some()
            {
                count += 1;
            }
        }
        count
    });
    assert_eq!(hits, 100);
    assert_eq!(
        blob_allocs, 0,
        "warm build_cached_text_blob hit must perform 0 Rust heap allocations"
    );

    // 2. Warm `CpuSkiaSurface::render_cached_with_plugins` frame allocates 0 Rust heap bytes.
    let img_key = "nve-test://zero-alloc-img";
    ImageCache::shared()
        .insert_raw_rgba(img_key, 8, 8, vec![200u8; 8 * 8 * 4])
        .expect("insert image");

    let dl = DisplayList {
        ops: vec![
            PaintOp::DrawShadow {
                rect: LayoutRect { x: 8.0, y: 8.0, width: 120.0, height: 60.0 },
                radius: 8.0,
                offset_x: 2.0,
                offset_y: 4.0,
                blur_radius: 8.0,
                spread_radius: 0.0,
                color: ColorRgba::rgba(0, 0, 0, 160),
                border_width: 0.0,
                inset: false,
            },
            PaintOp::FillRoundedRectGradient {
                rect: LayoutRect { x: 8.0, y: 8.0, width: 120.0, height: 60.0 },
                radius: 8.0,
                gradient: BackgroundFill::Linear(LinearGradient::new(
                    90.0,
                    vec![
                        GradientStop::new(0.0, ColorRgba::rgb(37, 99, 235)),
                        GradientStop::new(1.0, ColorRgba::rgb(16, 185, 129)),
                    ],
                )),
            },
            PaintOp::DrawImage {
                rect: LayoutRect { x: 140.0, y: 8.0, width: 32.0, height: 32.0 },
                radius: 4.0,
                src: img_key.to_owned(),
                fit: ImageFit::Cover,
                resolved: ImageCache::shared().resolve_cached_handle(img_key),
            },
            PaintOp::DrawText {
                rect: LayoutRect { x: 8.0, y: 80.0, width: 180.0, height: 28.0 },
                text: "Steady-state Skia frame".to_owned(),
                font_size: 14.0,
                font_weight: 500,
                text_align: TextAlign::Left,
                color: ColorRgba::WHITE,
            },
        ],
    };

    let mut surface = CpuSkiaSurface::new(220, 120).expect("cpu skia surface");
    // Prime every cache (typeface, text blob, Skia image, mask filter, gradient scratch).
    surface
        .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None)
        .expect("cold frame");
    surface.reset_cache_stats();

    let (_, frame_allocs) = count_allocs(|| {
        for _ in 0..20 {
            surface
                .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None)
                .expect("warm frame");
        }
    });
    let stats = surface.cache_stats();
    assert_eq!(stats.text_blob_misses, 0);
    assert_eq!(stats.image_misses, 0);
    assert_eq!(stats.image_fast_hits, 20);
    assert_eq!(stats.mask_filter_misses, 0);
    assert_eq!(
        frame_allocs, 0,
        "warm CpuSkiaSurface frame must perform 0 Rust heap allocations"
    );
}
