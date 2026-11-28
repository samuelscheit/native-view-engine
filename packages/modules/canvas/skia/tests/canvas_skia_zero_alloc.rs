//! Thread-local `CountingAlloc` zero-allocation hot-path guards and `--features skia` GPU cache
//! invalidation / zero-leak tests for `nve-canvas-skia`.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{ColorRgba, EventBatchBuf, SurfaceId};
use nve_backend_skia::{PixelBuffer, SkiaBackend, SkiaSystemPaintCtx, SkiaTableSystem};
use nve_canvas::{
    Canvas, CanvasOp, CanvasPainter, DirectCanvasRegistry, DrawList, MAX_CLIP_DEPTH,
};
use nve_canvas_skia::{CanvasSkiaSystem, SkiaCanvasPainter};
use nve_engine::{LayoutRect, StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::Layout;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

static SERIAL_TEST: Mutex<()> = Mutex::new(());

struct CountingAlloc;

// SAFETY: Delegates directly to `std::alloc::System` while counting per-thread allocations.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Preconditions upheld by caller and delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
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
fn counting_alloc_probe_detects_real_heap_allocation() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let (probe_allocs, probe_box) = measure_allocs(|| Box::new(black_box([1u64; 16])));
    black_box(&probe_box);
    assert!(
        probe_allocs >= 1,
        "CountingAlloc positive probe must observe >= 1 allocation, got {probe_allocs}"
    );
}

#[test]
fn direct_skia_canvas_painter_all_8_ops_allocate_zero() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let mut buf = PixelBuffer::new(32, 32);
    let rect = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: 32.0,
        height: 32.0,
    };
    let poly = [(2.0f32, 28.0), (12.0, 16.0), (22.0, 24.0), (30.0, 8.0)];

    let (allocs, ()) = measure_allocs(|| {
        for _ in 0..16 {
            let (mut p, _) = SkiaCanvasPainter::new(&mut buf, rect, None, 1.0, 1.0)
                .expect("valid SkiaCanvasPainter");
            p.clear(ColorRgba::BLACK);
            p.fill_rect(2.0, 2.0, 28.0, 28.0, ColorRgba::rgb(20, 40, 60));
            p.fill_rounded_rect(4.0, 4.0, 20.0, 12.0, 4.0, ColorRgba::rgb(0, 200, 120));
            p.fill_circle(24.0, 24.0, 4.0, ColorRgba::rgb(0, 120, 255));
            p.stroke_line(1.0, 1.0, 30.0, 30.0, 2.0, ColorRgba::rgb(255, 180, 0));
            p.stroke_polyline(black_box(&poly), 2.0, ColorRgba::rgb(240, 80, 160));
            for _ in 0..(MAX_CLIP_DEPTH + 2) {
                p.clip_rect(4.0, 4.0, 16.0, 16.0);
            }
            p.restore_clip();
            p.restore_clip();
            p.fill_rect(0.0, 0.0, 32.0, 32.0, ColorRgba::rgb(0, 255, 0));
            p.restore_clip();
        }
    });

    assert_eq!(buf.get_rgb(8, 8), Some((0, 255, 0)));
    assert_eq!(
        allocs, 0,
        "direct SkiaCanvasPainter across all 8 ops + clip overflow must allocate 0 bytes"
    );
}

#[test]
fn draw_list_view_replay_into_skia_canvas_painter_allocates_zero() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let mut ops = vec![
        CanvasOp::Clear {
            color: ColorRgba::BLACK,
        },
        CanvasOp::FillRect {
            x: 2.0,
            y: 2.0,
            w: 28.0,
            h: 28.0,
            color: ColorRgba::rgb(20, 40, 60),
        },
        CanvasOp::FillRoundedRect {
            x: 4.0,
            y: 4.0,
            w: 20.0,
            h: 12.0,
            radius: 4.0,
            color: ColorRgba::rgb(0, 200, 120),
        },
        CanvasOp::FillCircle {
            cx: 24.0,
            cy: 24.0,
            radius: 4.0,
            color: ColorRgba::rgb(0, 120, 255),
        },
        CanvasOp::StrokeLine {
            x0: 1.0,
            y0: 1.0,
            x1: 30.0,
            y1: 30.0,
            width: 2.0,
            color: ColorRgba::rgb(255, 180, 0),
        },
        CanvasOp::StrokePolyline {
            points: vec![(2.0, 28.0), (12.0, 16.0), (22.0, 24.0), (30.0, 8.0)],
            width: 2.0,
            color: ColorRgba::rgb(240, 80, 160),
        },
    ];
    for _ in 0..(MAX_CLIP_DEPTH + 2) {
        ops.push(CanvasOp::ClipRect {
            x: 4.0,
            y: 4.0,
            w: 16.0,
            h: 16.0,
        });
    }
    ops.push(CanvasOp::RestoreClip);
    ops.push(CanvasOp::RestoreClip);
    ops.push(CanvasOp::FillRect {
        x: 0.0,
        y: 0.0,
        w: 32.0,
        h: 32.0,
        color: ColorRgba::rgb(0, 255, 0),
    });
    ops.push(CanvasOp::RestoreClip);

    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let node = tx.create();
    tx.set_children(NodeId::ROOT, &[node]);
    tx.edit::<Canvas>(node)
        .draw_list_ephemeral(DrawList::from_ops(ops))
        .generation(1);
    let mut changes = ChangeSet::default();
    tx.commit_into(&mut changes);

    let sys = CanvasSkiaSystem::new();
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
            sys.paint_node(&mut ctx, black_box(&store), node);
        }
    });

    assert_eq!(buf.get_rgb(8, 8), Some((0, 255, 0)));
    assert_eq!(
        allocs, 0,
        "CanvasSkiaSystem::paint_node recorded replay must allocate 0 bytes"
    );
}

#[test]
fn direct_mode_120hz_invalidate_and_rasterize_allocates_zero() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let reg = DirectCanvasRegistry::new();
    let frame = Arc::new(AtomicU32::new(0));

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n)
            .width(Length::points(32.0))
            .height(Length::points(32.0));
        let f = Arc::clone(&frame);
        reg.bind(&mut tx, n, move |p, size| {
            let step = (f.load(Ordering::Relaxed) % 255) as u8;
            p.clear(ColorRgba::rgb(step, 40, 80));
            p.fill_rect(4.0, 4.0, size.width - 8.0, size.height - 8.0, ColorRgba::rgb(20, step, 200));
        });
        tx.commit_into(&mut changes);
        n
    };

    let mut backend = SkiaBackend::new();
    backend.register_table_system(CanvasSkiaSystem::with_direct(reg.clone()));
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 32.0,
            height: 32.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store, &changes).expect("initial commit");

    let mut buf = PixelBuffer::new(32, 32);
    // Warm 2 frames so `ChangeSet`, `CompositorScene`, and `SkiaBackend` scratch buffers stabilize.
    for warm in 1..=2u32 {
        frame.store(warm, Ordering::Relaxed);
        {
            let mut tx = store.tx();
            assert!(reg.invalidate(&mut tx, node).is_some());
            tx.commit_into(&mut changes);
        }
        backend.commit(&store, &changes).expect("warm commit");
        backend.rasterize_with_store(&store, &mut buf, ColorRgba::BLACK);
    }

    let (allocs, ()) = measure_allocs(|| {
        for f in 3..63u32 {
            frame.store(f, Ordering::Relaxed);
            {
                let mut tx = store.tx();
                let g = reg.invalidate(&mut tx, node).expect("bound node");
                black_box(g);
                tx.commit_into(&mut changes);
            }
            backend.commit(&store, &changes).expect("steady commit");
            backend.rasterize_with_store(&store, &mut buf, ColorRgba::BLACK);
        }
    });

    assert_eq!(buf.get_rgb(10, 10), Some((20, 62, 200)));
    assert_eq!(
        allocs, 0,
        "120 Hz DirectCanvasRegistry::invalidate + SkiaBackend::commit + rasterize_with_store must allocate 0 bytes"
    );
}

#[cfg(feature = "skia")]
#[test]
fn direct_mode_skia_gpu_native_fast_path_cached_static_sibling_and_zero_leak() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    use nve_backend_skia::{CpuSkiaSurface, GpuSurfaceBackend};

    let reg = DirectCanvasRegistry::new();
    let anim_calls = Arc::new(AtomicU32::new(0));
    let anim_red = Arc::new(AtomicU32::new(10));

    let static_dl = DrawList::record(|p| {
        p.clear(ColorRgba::rgb(16, 185, 129));
        p.fill_rect(4.0, 4.0, 32.0, 32.0, ColorRgba::rgb(16, 185, 129));
    });

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (static_node, anim_node) = {
        let mut tx = store.tx();
        let s = tx.create();
        let a = tx.create();
        tx.set_children(NodeId::ROOT, &[s, a]);
        tx.edit::<Layout>(s)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Canvas>(s)
            .draw_list_ephemeral(static_dl)
            .generation(1);

        tx.edit::<Layout>(a)
            .width(Length::points(40.0))
            .height(Length::points(40.0));

        let ac = Arc::clone(&anim_calls);
        let ar = Arc::clone(&anim_red);
        reg.bind(&mut tx, a, move |p, size| {
            ac.fetch_add(1, Ordering::Relaxed);
            let r = (ar.load(Ordering::Relaxed) % 255) as u8;
            p.clear(ColorRgba::rgb(r, 40, 220));
            p.fill_rect(0.0, 0.0, size.width, size.height, ColorRgba::rgb(r, 40, 220));
        });

        tx.commit_into(&mut changes);
        (s, a)
    };

    let mut backend = SkiaBackend::new();
    backend.register_table_system(CanvasSkiaSystem::with_direct(reg.clone()));
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 120.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store, &changes).expect("initial commit");

    let mut surface = CpuSkiaSurface::new(120, 120).expect("CpuSkiaSurface");
    let render_once = |surface: &mut CpuSkiaSurface, b: &mut SkiaBackend, s: &NodeStore| {
        let (scene, table_reg, ev) = b.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(
                scene,
                1.0,
                ColorRgba::BLACK,
                Some((s, table_reg, ev)),
            )
            .expect("render_scene_cached_with_plugins");
    };

    // Frame 0: static recorded canvas populates the GPU plugin texture cache (1 rasterization);
    // direct canvas paints natively on `skia_safe::Canvas` (0 PixelBuffer rasterizations!).
    render_once(&mut surface, &mut backend, &store);
    let f0_stats = surface.cache_stats();
    assert_eq!(f0_stats.plugin_rasterizations, 1);
    assert_eq!(anim_calls.load(Ordering::Relaxed), 1);

    // Frames 1..=10: invalidate `anim_node` -> native `SkiaNativeSink` updates pixels every frame
    // with ZERO plugin cache rasterizations, while static recorded sibling stays 100% cache-hit!
    for frame in 1..=10u32 {
        anim_red.store(frame * 20, Ordering::Relaxed);
        {
            let mut tx = store.tx();
            assert!(reg.invalidate(&mut tx, anim_node).is_some());
            tx.commit_into(&mut changes);
        }
        backend.commit(&store, &changes).expect("anim commit");
        surface.reset_cache_stats();
        render_once(&mut surface, &mut backend, &store);
        let st = surface.cache_stats();
        assert_eq!(
            st.plugin_rasterizations, 0,
            "direct canvas must paint natively on skia_safe::Canvas with 0 CPU texture uploads"
        );
        assert_eq!(st.plugin_hits, 1, "static recorded sibling must stay 100% warm hit");
    }
    assert_eq!(anim_calls.load(Ordering::Relaxed), 11);

    let mut readback = PixelBuffer::new(120, 120);
    surface
        .copy_to_pixel_buffer(&mut readback)
        .expect("readback");
    assert_eq!(readback.get_rgb(20, 20), Some((16, 185, 129)));
    assert_eq!(readback.get_rgb(20, 60), Some((200, 40, 220)));

    // Detach/remove -> zero leaks!
    {
        let mut tx = store.tx();
        tx.detach::<Canvas>(static_node);
        tx.remove(anim_node);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("cleanup commit");
    assert_eq!(reg.len(), 0);
    assert_eq!(Arc::strong_count(&anim_calls), 1);
    assert_eq!(Arc::strong_count(&anim_red), 1);
}

#[cfg(feature = "skia")]
#[test]
fn skia_native_sink_matches_pixel_sink_across_all_8_ops_and_clips() {
    let _guard = SERIAL_TEST.lock().unwrap_or_else(|e| e.into_inner());
    use nve_backend_skia::{CpuSkiaSurface, GpuSurfaceBackend};
    use nve_canvas_skia::SkiaNativeCanvasPainter;
    use nve_store::{FlatResource, ValidatedWords};

    let paint_scene = |p: &mut dyn CanvasPainter| {
        p.clear(ColorRgba::rgb(12, 16, 24));
        p.fill_rect(2.0, 2.0, 18.0, 12.0, ColorRgba::rgb(220, 40, 40));
        p.fill_rounded_rect(4.0, 20.0, 22.0, 14.0, 4.0, ColorRgba::rgb(40, 200, 110));
        p.fill_circle(46.0, 14.0, 8.0, ColorRgba::rgb(50, 120, 240));
        p.stroke_line(6.0, 42.0, 28.0, 42.0, 4.0, ColorRgba::rgb(245, 190, 40));
        // 1-point polyline dot + NaN-split polyline.
        p.stroke_polyline(&[(50.0, 44.0)], 6.0, ColorRgba::rgb(255, 90, 180));
        p.stroke_polyline(
            &[
                (6.0, 56.0),
                (24.0, 56.0),
                (f32::NAN, 10.0),
                (38.0, 56.0),
                (56.0, 56.0),
            ],
            4.0,
            ColorRgba::rgb(180, 240, 255),
        );
        // 66 nested clips + 2 restores -> 64th clip `(10..24, 24..38)` stays active; 3rd pops it.
        for _ in 0..(MAX_CLIP_DEPTH - 1) {
            p.clip_rect(0.0, 0.0, 64.0, 64.0);
        }
        p.clip_rect(34.0, 26.0, 12.0, 12.0);
        p.clip_rect(0.0, 0.0, 1.0, 1.0);
        p.clip_rect(0.0, 0.0, 1.0, 1.0);
        p.restore_clip();
        p.restore_clip();
        p.fill_rect(0.0, 0.0, 64.0, 64.0, ColorRgba::rgb(0, 255, 128));
        p.restore_clip();
        p.fill_rect(50.0, 26.0, 10.0, 10.0, ColorRgba::rgb(255, 255, 0));
        // Leave 63 clips open to verify RAII Drop unwind on `skia_safe::Canvas`.
    };

    // Also exercise the translucent polyline (`PathBuilder`) branch + translucent `clear` overwrite
    // inside a clipped sub-region (`(0..12, 46..54)`).
    let paint_scene_full = |p: &mut dyn CanvasPainter| {
        paint_scene(p);
        p.stroke_polyline(
            &[(6.0, 36.0), (24.0, 36.0), (24.0, 46.0)],
            4.0,
            ColorRgba::rgba(255, 40, 200, 128),
        );
        p.clip_rect(2.0, 46.0, 12.0, 8.0);
        p.clear(ColorRgba::rgba(30, 140, 250, 128));
        p.restore_clip();
    };

    // 1. Render via software `SkiaCanvasPainter` (`SkiaPixelSink`) on an isolated RGBA buffer.
    let mut pixel_buf = PixelBuffer::new_transparent_rgba(64, 64);
    let rect = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: 64.0,
        height: 64.0,
    };
    {
        let (mut p, _) =
            SkiaCanvasPainter::new(&mut pixel_buf, rect, None, 1.0, 1.0).expect("pixel painter");
        paint_scene_full(&mut p);
    }

    // 2. Render via native `SkiaNativeCanvasPainter` (`SkiaNativeSink`) onto `CpuSkiaSurface`.
    let mut surface = CpuSkiaSurface::new(64, 64).expect("CpuSkiaSurface");
    let save_before = surface.canvas().save_count();
    {
        let (mut p, _) = SkiaNativeCanvasPainter::new(surface.canvas(), 64.0, 64.0, 1.0);
        paint_scene_full(&mut p);
    }
    assert_eq!(
        surface.canvas().save_count(),
        save_before,
        "GuardedPainter Drop must unwind all 63 unclosed clips on skia_safe::Canvas"
    );

    let mut native_buf = PixelBuffer::new(64, 64);
    surface
        .copy_to_pixel_buffer(&mut native_buf)
        .expect("readback");

    // Verify exact RGB parity across interior sample points of every primitive, NaN gap, and clip!
    for (label, x, y) in [
        ("clear bg", 60, 4),
        ("fill_rect", 10, 8),
        ("fill_rounded_rect", 14, 27),
        ("fill_circle", 46, 14),
        ("stroke_line", 16, 42),
        ("1-pt polyline dot", 50, 44),
        ("polyline seg1", 15, 56),
        ("polyline NaN gap stays bg", 31, 56),
        ("polyline seg2", 47, 56),
        ("64th clip active", 40, 32),
        ("outside 64th clip stays bg", 30, 32),
        ("after 3rd restore", 55, 31),
    ] {
        assert_eq!(
            native_buf.get_rgb(x, y),
            pixel_buf.get_rgb(x, y),
            "SkiaNativeSink vs SkiaPixelSink mismatch at {label} ({x},{y})"
        );
    }
    // Translucent polyline (`PathBuilder` branch) matches software blend within +-2 rounding LSB.
    let (nr, ng, nb) = native_buf.get_rgb(15, 36).expect("native polyline");
    let (pr, pg, pb) = pixel_buf.get_rgb(15, 36).expect("pixel polyline");
    assert!(
        (i16::from(nr) - i16::from(pr)).abs() <= 2
            && (i16::from(ng) - i16::from(pg)).abs() <= 2
            && (i16::from(nb) - i16::from(pb)).abs() <= 2,
        "translucent polyline mismatch: native=({nr},{ng},{nb}) pixel=({pr},{pg},{pb})"
    );
    // Translucent `clear(rgba(30, 140, 250, 128))` inside `(2..14, 46..54)` replaces prior pixels
    // (readback returns unpremultiplied `(30, 140, 250)` or premul-rounded close to `(15, 70, 125)`
    // over transparent, strictly matching non-bg).
    assert_ne!(native_buf.get_rgb(8, 50), Some((12, 16, 24)));

    // 3. Hostile wire `DrawList::from_ops` (unmatched leading restore + 66 clips) replayed into
    //    `SkiaNativeCanvasPainter` must also leave `save_count` strictly balanced AND actually clear
    //    the clipped region `(0..32, 0..32)` to transparent `(0, 0, 0)`.
    let mut hostile_ops = vec![CanvasOp::RestoreClip];
    for _ in 0..(MAX_CLIP_DEPTH + 2) {
        hostile_ops.push(CanvasOp::ClipRect {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
        });
    }
    hostile_ops.push(CanvasOp::Clear {
        color: ColorRgba::TRANSPARENT,
    });
    let mut words = Vec::new();
    assert!(DrawList::from_ops(hostile_ops).write_words(&mut words));
    let vw = ValidatedWords::<DrawList>::try_new(&words).expect("valid wire");
    {
        let (mut p, _) = SkiaNativeCanvasPainter::new(surface.canvas(), 64.0, 64.0, 1.0);
        DrawList::view(vw).replay(&mut p);
    }
    assert_eq!(surface.canvas().save_count(), save_before);
    surface
        .copy_to_pixel_buffer(&mut native_buf)
        .expect("post-clear readback");
    assert_eq!(
        native_buf.get_rgb(10, 8),
        Some((0, 0, 0)),
        "clear(TRANSPARENT) inside clip must wipe prior pixels in isolated surface"
    );
    assert_eq!(
        native_buf.get_rgb(46, 14),
        Some((50, 120, 240)),
        "pixels outside the hostile clip must remain untouched"
    );
}
