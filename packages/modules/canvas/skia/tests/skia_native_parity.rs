//! `--features skia` native `SkiaNativeSink` isolation, backdrop, HiDPI, opacity, transform,
//! out-of-bounds clip, and panic-unwind parity tests against GPU-cached `SkiaPixelSink`.
#![cfg(feature = "skia")]

use nve_abi::{ColorRgba, SurfaceId};
use nve_backend_skia::{CpuSkiaSurface, GpuSurfaceBackend, PixelBuffer, SkiaBackend};
use nve_canvas::{Canvas, CanvasPainter, DirectCanvasRegistry, DrawList, MAX_CLIP_DEPTH};
use nve_canvas_skia::{CanvasSkiaSystem, SkiaCanvasPainter, SkiaNativeCanvasPainter};
use nve_engine::{LayoutRect, StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::tables::{Layout, Transform, Visual};

fn assert_rgb_close(
    label: &str,
    actual: Option<(u8, u8, u8)>,
    expected: Option<(u8, u8, u8)>,
    tol: i16,
) {
    let (ar, ag, ab) = actual.expect("actual pixel in bounds");
    let (er, eg, eb) = expected.expect("expected pixel in bounds");
    let dr = (i16::from(ar) - i16::from(er)).abs();
    let dg = (i16::from(ag) - i16::from(eg)).abs();
    let db = (i16::from(ab) - i16::from(eb)).abs();
    assert!(
        dr <= tol && dg <= tol && db <= tol,
        "{label}: actual=({ar},{ag},{ab}) expected=({er},{eg},{eb}) exceeds tol={tol}"
    );
}

fn render_gpu_scene(
    backend: &mut SkiaBackend,
    store: &NodeStore,
    width: u32,
    height: u32,
    scale: f32,
    clear: ColorRgba,
) -> PixelBuffer {
    let mut surface = CpuSkiaSurface::new(width, height).expect("CpuSkiaSurface");
    let (scene, table_reg, ev) = backend.scene_paint_parts_mut();
    surface
        .render_scene_cached_with_plugins(scene, scale, clear, Some((store, table_reg, ev)))
        .expect("render_scene_cached_with_plugins");
    let mut out = PixelBuffer::new(width, height);
    surface.copy_to_pixel_buffer(&mut out).expect("readback");
    out
}

#[test]
fn clear_transparent_and_translucent_over_parent_backdrop_match_cached_pixel_sink() {
    let paint_isolation_scene = |p: &mut dyn CanvasPainter| {
        // 1. Paint solid white across the entire canvas, then clear the right half to TRANSPARENT
        //    so the node's own red background (and parent backdrop) must shine through with ZERO
        //    framebuffer hole!
        p.fill_rect(0.0, 0.0, 60.0, 60.0, ColorRgba::WHITE);
        p.clip_rect(30.0, 0.0, 30.0, 60.0);
        p.clear(ColorRgba::TRANSPARENT);
        p.restore_clip();

        // 2. Paint solid yellow over the bottom-left quadrant, then clear that quadrant with a
        //    translucent blue (`alpha = 128`): `BlendMode::Src` / overwrite must REPLACE yellow
        //    with translucent blue composited over the red node background (NOT blend over yellow!).
        p.clip_rect(0.0, 30.0, 30.0, 30.0);
        p.fill_rect(0.0, 30.0, 30.0, 30.0, ColorRgba::rgb(255, 255, 0));
        p.clear(ColorRgba::rgba(0, 80, 240, 128));
        p.restore_clip();

        // 3. Draw an oversized green rect extending outside the canvas bounds: must be clipped
        //    strictly to `(0..60, 0..60)` and never bleed onto the surrounding parent backdrop.
        p.fill_rect(-20.0, -20.0, 26.0, 26.0, ColorRgba::rgb(0, 220, 40));
    };

    let reg = DirectCanvasRegistry::new();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let recorded_dl = DrawList::record(|rec| paint_isolation_scene(rec));

    {
        let mut tx = store.tx();
        let root_bg = tx.create();
        let direct_node = tx.create();
        let cached_node = tx.create();
        tx.set_children(NodeId::ROOT, &[root_bg]);
        tx.set_children(root_bg, &[direct_node, cached_node]);

        // Bright magenta parent backdrop (`(220, 20, 180)`), tall enough for two 60px column children without flex-shrink.
        tx.edit::<Layout>(root_bg)
            .width(Length::points(180.0))
            .height(Length::points(160.0));
        tx.edit::<Visual>(root_bg)
            .background(Rgba::rgb(220, 20, 180));

        // Direct node at (10, 20) with red node background (`(200, 40, 20)`).
        tx.edit::<Layout>(direct_node)
            .width(Length::points(60.0))
            .height(Length::points(60.0));
        tx.edit::<Visual>(direct_node)
            .background(Rgba::rgb(200, 40, 20));
        tx.edit::<Transform>(direct_node)
            .translate_x(10.0)
            .translate_y(20.0);
        reg.bind(&mut tx, direct_node, move |p, _| {
            paint_isolation_scene(p);
        });

        // Identical static recorded sibling at (90, 20) using `get_or_render_plugin` (`SkiaPixelSink`).
        tx.edit::<Layout>(cached_node)
            .width(Length::points(60.0))
            .height(Length::points(60.0));
        tx.edit::<Visual>(cached_node)
            .background(Rgba::rgb(200, 40, 20));
        tx.edit::<Transform>(cached_node)
            .translate_x(90.0)
            .translate_y(-40.0);
        tx.edit::<Canvas>(cached_node)
            .draw_list_ephemeral(recorded_dl)
            .generation(1);

        tx.commit_into(&mut changes);
    }

    let mut backend = SkiaBackend::new();
    backend.register_table_system(CanvasSkiaSystem::with_direct(reg));
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 180.0,
            height: 160.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store, &changes).expect("commit");
    let img = render_gpu_scene(&mut backend, &store, 180, 160, 1.0, ColorRgba::BLACK);

    // Direct canvas bounds are `(10..70, 20..80)`; cached sibling is `(90..150, 20..80)`.
    // Outside both canvases at `(4, 14)`: must stay pure magenta parent backdrop (no OOB green bleed!).
    assert_eq!(img.get_rgb(4, 14), Some((220, 20, 180)));
    // Inside OOB green corner of direct canvas `(12, 22)`: green `(0, 220, 40)`.
    assert_eq!(img.get_rgb(12, 22), Some((0, 220, 40)));
    // Top-left white region: direct `(25, 30)` == cached `(105, 30)` == white.
    assert_eq!(img.get_rgb(25, 30), Some((255, 255, 255)));
    assert_eq!(img.get_rgb(105, 30), Some((255, 255, 255)));
    // Right half cleared to TRANSPARENT after white draw: MUST reveal red node background `(200, 40, 20)`
    // (proving `BlendMode::Clear` did NOT punch a hole into the main framebuffer!).
    assert_eq!(img.get_rgb(55, 45), Some((200, 40, 20)));
    assert_eq!(img.get_rgb(135, 45), Some((200, 40, 20)));
    // Bottom-left quadrant cleared to `rgba(0, 80, 240, 128)` after yellow fill:
    // Yellow MUST be replaced (NOT blended!), so result is ~50% blue over `(200, 40, 20)` = `(~100, 60, 130)`.
    let direct_tl = img.get_rgb(25, 65);
    let cached_tl = img.get_rgb(105, 65);
    assert_rgb_close("translucent clear direct vs cached", direct_tl, cached_tl, 2);
    assert_rgb_close(
        "translucent clear replaces yellow over red backdrop",
        direct_tl,
        Some((100, 60, 130)),
        3,
    );
}

#[test]
fn skia_native_hidpi_scale2_opacity_and_transform_match_cached_pixel_sink() {
    let paint_scene = |p: &mut dyn CanvasPainter| {
        p.clear(ColorRgba::rgb(20, 40, 80));
        p.fill_rect(4.0, 4.0, 20.0, 16.0, ColorRgba::rgb(240, 80, 40));
        // Translucent 3-vertex polyline exercises `SkiaNativeSink`'s `PathBuilder` branch.
        p.stroke_polyline(
            &[(6.0, 28.0), (20.0, 28.0), (20.0, 36.0)],
            4.0,
            ColorRgba::rgba(40, 240, 120, 128),
        );
    };

    // 1. Standalone `scale = 2.0` parity (`SkiaNativeSink` scaled 2x vs `SkiaPixelSink` scale 2.0).
    let mut pixel_buf = PixelBuffer::new_transparent_rgba(80, 80);
    let rect = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: 80.0,
        height: 80.0,
    };
    {
        let (mut p, size) =
            SkiaCanvasPainter::new(&mut pixel_buf, rect, None, 2.0, 1.0).expect("pixel painter");
        assert_eq!(size.width, 40.0);
        assert_eq!(size.scale_factor, 2.0);
        paint_scene(&mut p);
    }
    let mut surface = CpuSkiaSurface::new(80, 80).expect("CpuSkiaSurface");
    {
        surface.canvas().save();
        surface.canvas().scale((2.0, 2.0));
        {
            let (mut p, size) = SkiaNativeCanvasPainter::new(surface.canvas(), 40.0, 40.0, 2.0);
            assert_eq!(size.width, 40.0);
            assert_eq!(size.scale_factor, 2.0);
            paint_scene(&mut p);
        }
        surface.canvas().restore();
    }
    let mut native_buf = PixelBuffer::new(80, 80);
    surface
        .copy_to_pixel_buffer(&mut native_buf)
        .expect("readback");
    // Sample physical pixel coordinates at 2x scale: logical (14, 12) -> physical (28, 24),
    // logical (36, 36) -> physical (72, 72), translucent polyline horizontal interior (26, 56).
    assert_rgb_close("2x bg", native_buf.get_rgb(72, 72), pixel_buf.get_rgb(72, 72), 1);
    assert_rgb_close("2x fill_rect", native_buf.get_rgb(28, 24), pixel_buf.get_rgb(28, 24), 1);
    assert_rgb_close(
        "2x translucent polyline interior",
        native_buf.get_rgb(26, 56),
        pixel_buf.get_rgb(26, 56),
        2,
    );
    // At the translucent polyline corner vertex (logical 20, 28 -> physical 40, 56), `PathBuilder`
    // must NOT double-blend the two meeting segments (must stay within +-2 of single-blend interior!).
    assert_rgb_close(
        "translucent polyline corner has zero double-blend",
        native_buf.get_rgb(40, 56),
        native_buf.get_rgb(26, 56),
        2,
    );

    // 2. Full `SkiaBackend` + `CpuSkiaSurface` scene with `scale = 2.0`, `Opacity = 0.5`, and
    //    `Transform` (`translate_x` + `rotate_deg`): Direct `SkiaNativeSink` matches Cached `SkiaPixelSink`.
    let reg = DirectCanvasRegistry::new();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let dl = DrawList::record(|rec| paint_scene(rec));
    {
        let mut tx = store.tx();
        let bg = tx.create();
        let direct_n = tx.create();
        let cached_n = tx.create();
        tx.set_children(NodeId::ROOT, &[bg]);
        tx.set_children(bg, &[direct_n, cached_n]);

        tx.edit::<Layout>(bg)
            .width(Length::points(120.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(bg).background(Rgba::rgb(40, 40, 40));

        tx.edit::<Layout>(direct_n)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(direct_n).opacity(0.5);
        tx.edit::<Transform>(direct_n)
            .translate_x(10.0)
            .translate_y(16.0)
            .rotate_deg(12.0);
        reg.bind(&mut tx, direct_n, move |p, _| paint_scene(p));

        tx.edit::<Layout>(cached_n)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(cached_n).opacity(0.5);
        tx.edit::<Transform>(cached_n)
            .translate_x(66.0)
            .translate_y(-24.0)
            .rotate_deg(12.0);
        tx.edit::<Canvas>(cached_n)
            .draw_list_ephemeral(dl)
            .generation(1);

        tx.commit_into(&mut changes);
    }

    let mut backend = SkiaBackend::new();
    backend.register_table_system(CanvasSkiaSystem::with_direct(reg));
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 120.0,
            height: 80.0,
            scale_factor: 2.0,
        },
    );
    backend.commit(&store, &changes).expect("commit");
    let frame = render_gpu_scene(&mut backend, &store, 240, 160, 2.0, ColorRgba::BLACK);

    // Sample interior of the rotated, 0.5-opacity red rect in both direct and cached siblings
    // (direct starts at logical x=10 -> physical x=20; cached starts at logical x=66 -> physical x=132, dx=112px).
    let direct_sample = frame.get_rgb(46, 54);
    let cached_sample = frame.get_rgb(158, 54);
    assert_rgb_close(
        "opacity=0.5 + rotate(12deg) + scale=2.0 direct vs cached",
        direct_sample,
        cached_sample,
        4,
    );
    // Must be blended at 50% opacity against dark gray `(40, 40, 40)` (not opaque red `(240, 80, 40)`).
    assert_rgb_close(
        "opacity=0.5 actually attenuates direct native canvas",
        direct_sample,
        Some((140, 60, 40)),
        5,
    );
}

#[test]
#[allow(clippy::panic)]
fn native_skia_surface_drop_guard_unwinds_outer_save_and_layer_on_panic() {
    let reg = DirectCanvasRegistry::new();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        reg.bind(&mut tx, n, |p, _| {
            for _ in 0..(MAX_CLIP_DEPTH + 2) {
                p.clip_rect(0.0, 0.0, 20.0, 20.0);
            }
            panic!("simulated direct canvas closure panic");
        });
        tx.commit_into(&mut changes);
        n
    };

    let mut backend = SkiaBackend::new();
    backend.register_table_system(CanvasSkiaSystem::with_direct(reg));
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 64.0,
            height: 64.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store, &changes).expect("commit");

    let mut surface = CpuSkiaSurface::new(64, 64).expect("CpuSkiaSurface");
    let save_before = surface.canvas().save_count();
    let (_, table_reg, _) = backend.scene_paint_parts_mut();
    let rect = LayoutRect {
        x: 8.0,
        y: 8.0,
        width: 40.0,
        height: 40.0,
    };
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        table_reg.try_paint_skia_canvas(&store, node, surface.canvas(), rect, 1.0);
    }));
    assert!(res.is_err(), "panic must propagate out of direct closure");
    assert_eq!(
        surface.canvas().save_count(),
        save_before,
        "NativeSkiaSurface + GuardedPainter Drop guards must unwind all nested clips, save_layer, and outer save"
    );
}

#[derive(nve_store::NveTable)]
#[nve_table(name = "test.canvas.overlay_marker", storage = "sparse")]
#[allow(dead_code)]
struct OverlayMarker {
    #[effect(paint)]
    stamp: u32,
}

struct CachedOnlyOverlayPlugin;
impl nve_backend_skia::SkiaTableSystem for CachedOnlyOverlayPlugin {
    fn slot(&self) -> nve_store::TableSlot {
        <OverlayMarker as nve_store::NveTable>::slot()
    }

    fn paint_node(
        &self,
        ctx: &mut nve_backend_skia::SkiaSystemPaintCtx<'_>,
        store: &NodeStore,
        node: NodeId,
    ) {
        if store.try_row::<OverlayMarker>(node).is_some_and(|r| r.stamp() > 0) {
            let x0 = ctx.rect.x as i32 + 20;
            let y0 = ctx.rect.y as i32 + 20;
            ctx.buffer
                .fill_rect_rgba(x0, y0, x0 + 16, y0 + 16, ColorRgba::rgb(255, 220, 0));
        }
    }
}

#[test]
fn multi_plugin_mixed_native_and_cached_falls_back_cleanly_without_double_painting() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let reg = DirectCanvasRegistry::new();
    let direct_calls = Arc::new(AtomicU32::new(0));
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n)
            .width(Length::points(48.0))
            .height(Length::points(48.0));
        let dc = Arc::clone(&direct_calls);
        reg.bind(&mut tx, n, move |p, size| {
            dc.fetch_add(1, Ordering::Relaxed);
            p.clear(ColorRgba::rgb(20, 120, 220));
            p.fill_rect(0.0, 0.0, size.width, size.height, ColorRgba::rgb(20, 120, 220));
        });
        tx.edit::<OverlayMarker>(n).stamp(1);
        tx.commit_into(&mut changes);
        n
    };

    let mut backend = SkiaBackend::new();
    backend.register_table_system(CanvasSkiaSystem::with_direct(reg));
    backend.register_table_system(CachedOnlyOverlayPlugin);
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 64.0,
            height: 64.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store, &changes).expect("commit");

    let mut surface = CpuSkiaSurface::new(64, 64).expect("CpuSkiaSurface");
    let (scene, table_reg, ev) = backend.scene_paint_parts_mut();
    // Direct check: `try_paint_skia_canvas` MUST decline because `CachedOnlyOverlayPlugin` does
    // not prefer native Skia — and MUST NOT invoke `direct_calls`!
    let native_handled = table_reg.try_paint_skia_canvas(
        &store,
        node,
        surface.canvas(),
        LayoutRect {
            x: 0.0,
            y: 0.0,
            width: 48.0,
            height: 48.0,
        },
        1.0,
    );
    assert!(!native_handled);
    assert_eq!(
        direct_calls.load(Ordering::Relaxed),
        0,
        "declining mixed native/cached node must never invoke the native plugin closure"
    );

    // Full render falls back to `get_or_render_plugin`: paints BOTH plugins exactly once via
    // `PixelBuffer` and uploads 1 cached texture!
    surface
        .render_scene_cached_with_plugins(
            scene,
            1.0,
            ColorRgba::BLACK,
            Some((&store, table_reg, ev)),
        )
        .expect("render");
    assert_eq!(surface.cache_stats().plugin_rasterizations, 1);
    assert_eq!(direct_calls.load(Ordering::Relaxed), 1);

    let mut readback = PixelBuffer::new(64, 64);
    surface
        .copy_to_pixel_buffer(&mut readback)
        .expect("readback");
    assert_eq!(readback.get_rgb(8, 8), Some((20, 120, 220)));
    assert_eq!(readback.get_rgb(28, 28), Some((255, 220, 0)));
}
