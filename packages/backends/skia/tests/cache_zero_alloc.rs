//! Zero-allocation steady-state guards (`[INV:bench-and-alloc-guards]`) for `SlabLru`,
//! warm `SkiaCanvasRenderer` `TextBlob` lookups, and warm `CpuSkiaSurface` frames (covering
//! outer + inset shadows, gradients, images, text, and plugins).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use nve_backend_skia::slab_lru::{IdentityU64BuildHasher, SlabLru};

struct CountingAlloc;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static SERIAL_TEST: Mutex<()> = Mutex::new(());

// SAFETY: Delegates directly to `System` allocator; `try_with` isolates per-thread counting
// without ever panicking during TLS teardown inside `GlobalAlloc`.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: Forwards valid `GlobalAlloc::alloc` layout directly to `std::alloc::System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forwards pointer and layout previously allocated by `System` back to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: Forwards pointer, layout, and new size previously allocated by `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    let _ = COUNTING.try_with(|c| c.set(true));
    let out = f();
    let _ = COUNTING.try_with(|c| c.set(false));
    let count = ALLOC_COUNT.load(Ordering::SeqCst);
    (out, count)
}

#[test]
fn counting_alloc_probe_detects_real_heap_allocation() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let (_, allocs) = count_allocs(|| {
        let v: Vec<u8> = black_box(vec![42u8; 128]);
        black_box(v);
    });
    assert!(
        allocs >= 1,
        "CountingAlloc positive probe must observe real heap allocations, got {allocs}"
    );
}

#[test]
fn slab_lru_warm_get_mut_allocates_zero_times() {
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
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
    let _serial = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    use nve_abi::{ColorRgba, EventBatchBuf};
    use nve_backend_skia::image_cache::ImageCache;
    use nve_backend_skia::text::TextAlign;
    use nve_backend_skia::{
        CpuSkiaSurface, DisplayList, PaintOp, SkiaCanvasRenderer, SkiaSystemPaintCtx,
        SkiaTableRegistry, SkiaTableSystem,
    };
    use nve_engine::LayoutRect;
    use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, Rgba, TableSlot};
    use nve_tables::paint::{BackgroundFill, GradientStop, LinearGradient};
    use nve_tables::tables::ImageFit;

    #[derive(NveTable)]
    #[nve_table(name = "nve.test.cache_zero_alloc.plugin", storage = "sparse")]
    #[allow(dead_code)]
    struct ZeroAllocPlugin {
        #[effect(paint)]
        color: u32,
    }
    struct ZeroAllocPluginSystem;
    impl SkiaTableSystem for ZeroAllocPluginSystem {
        fn slot(&self) -> TableSlot {
            ZeroAllocPlugin::slot()
        }
        fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
            if let Some(row) = store.try_row::<ZeroAllocPlugin>(node) {
                let c = ColorRgba::from_u32(row.color());
                for y in 0..(ctx.rect.height as i32).min(16) {
                    for x in 0..(ctx.rect.width as i32).min(16) {
                        ctx.buffer.blend_pixel(x, y, c, ctx.opacity);
                    }
                }
            }
        }
    }

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

    // 2. Warm `CpuSkiaSurface::render_cached_with_plugins` frame (including outer shadow,
    //    inset shadow, gradient, image, text, and plugin) allocates 0 Rust heap bytes.
    let img_key = "nve-test://zero-alloc-img";
    ImageCache::shared()
        .insert_raw_rgba(img_key, 8, 8, vec![200u8; 8 * 8 * 4])
        .expect("insert image");

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let plugin_node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<ZeroAllocPlugin>(n)
            .color(ColorRgba::rgb(56, 189, 248).to_u32());
        tx.commit_into(&mut changes);
        n
    };
    let mut registry = SkiaTableRegistry::new();
    registry.register(ZeroAllocPluginSystem);
    let mut events = EventBatchBuf::new();

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
            PaintOp::DrawShadow {
                rect: LayoutRect { x: 8.0, y: 8.0, width: 120.0, height: 60.0 },
                radius: 8.0,
                offset_x: 0.0,
                offset_y: 4.0,
                blur_radius: 6.0,
                spread_radius: 2.0,
                color: ColorRgba::rgba(15, 23, 42, 180),
                border_width: 2.0,
                inset: true,
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
            PaintOp::Plugin {
                node_id: plugin_node,
                rect: LayoutRect { x: 180.0, y: 8.0, width: 24.0, height: 24.0 },
                inherited_text_color: Rgba::WHITE,
                plugin_epoch: 1,
            },
        ],
    };

    let mut surface = CpuSkiaSurface::new(220, 120).expect("cpu skia surface");
    // Prime every cache (typeface, text blob, Skia image, plugin texture, mask filter, gradient scratch).
    surface
        .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, Some((&store, &registry, &mut events)))
        .expect("cold frame");
    surface.reset_cache_stats();

    let (_, frame_allocs) = count_allocs(|| {
        for _ in 0..20 {
            surface
                .render_cached_with_plugins(
                    &dl,
                    1.0,
                    ColorRgba::BLACK,
                    Some((&store, &registry, &mut events)),
                )
                .expect("warm frame");
        }
    });
    let stats = surface.cache_stats();
    assert_eq!(stats.text_blob_misses, 0);
    assert_eq!(stats.image_misses, 0);
    assert_eq!(stats.image_fast_hits, 20);
    assert_eq!(stats.plugin_misses, 0);
    assert_eq!(stats.plugin_hits, 20);
    assert_eq!(stats.mask_filter_misses, 0);
    assert_eq!(stats.mask_filter_hits, 40, "outer + inset blur mask filters must both hit cache");
    assert_eq!(
        frame_allocs, 0,
        "warm CpuSkiaSurface frame (outer+inset shadows, gradient, image, text, plugin) must perform 0 Rust heap allocations"
    );
}
