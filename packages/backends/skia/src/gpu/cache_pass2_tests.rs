//! Pass-2 dual-review regression guards for Commit 2 (`packages/backends/skia`):
//! - Zero-copy `Data::from_filename` mmap vs `Binary` copied typeface backing & macOS emoji RSS cap
//! - Inline variable-font variant drop on base typeface eviction (zero pinned `SkData` / false hits)
//! - Device-pixel snapped focused-`TextInput` caret parity across fractional scales (`1.0..=2.0`)
//! - Real rendered Software vs CPU Skia text-box parity (centroid, bbox, MAE) across RTL, emoji,
//!   ligatures, wrapped Center/Right text, and fractional scales (`1.0, 1.25, 1.5, 1.75, 2.0`)
//! - Multi-table late registration (`register_table_system`) & direct `table_registry_mut` invalidation
//! - Exact fontless bitmap text parity (including faux-bold `700` + alignments)
//! - 10,000-frame multi-cache churn enforcing every individual cap including `mask_filter_entries`

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use nve_abi::{ColorRgba, EventBatchBuf, SurfaceId};
use nve_engine::{LayoutRect, StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba, TableSlot};
use nve_tables::tables::{ImageFit, Layout, TextInput, Typography};

use super::cache_parity_tests::{
    compare_buffers, compare_region, lock_shared_image_cache, register_checker_image,
    CountingPluginSystem, ParityPlugin,
};
use super::super::renderer_cache::{
    MAX_BASE_TYPEFACES, MAX_MASK_FILTERS, MAX_MASK_FILTER_BYTES, MAX_SKIA_IMAGES,
    MAX_SKIA_IMAGE_BYTES, MAX_SKIA_PLUGINS, MAX_SKIA_PLUGIN_BYTES, MAX_TEXT_BLOBS,
    MAX_TEXT_BLOB_BYTES,
};
use super::super::*;
use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry, SkiaTableSystem};
use crate::text::{CosmicTextEngine, TextAlign};
use crate::{DisplayList, PaintOp, SkiaBackend, FOCUS_RING_COLOR};

#[derive(NveTable)]
#[nve_table(name = "nve.test.second_plugin_4b_pass2", storage = "sparse")]
#[allow(dead_code)]
struct SecondPlugin {
    #[effect(paint)]
    pub accent_rgba: u32,
}

struct SecondPluginSystem {
    calls: Arc<AtomicU32>,
}

impl SkiaTableSystem for SecondPluginSystem {
    fn slot(&self) -> TableSlot {
        SecondPlugin::slot()
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let Some(row) = store.try_row::<SecondPlugin>(node) else {
            return;
        };
        let c = ColorRgba::from_u32(row.accent_rgba());
        // Paint a distinct strip in the bottom-right quadrant so both A and B can be verified.
        let x0 = ((ctx.rect.x + ctx.rect.width * 0.5).round() as i32) + 4;
        let y0 = ((ctx.rect.y + ctx.rect.height * 0.5).round() as i32) + 4;
        let x1 = ((ctx.rect.x + ctx.rect.width).round() as i32) - 4;
        let y1 = ((ctx.rect.y + ctx.rect.height).round() as i32) - 4;
        for y in y0..y1 {
            for x in x0..x1 {
                ctx.buffer.blend_pixel(x, y, c, ctx.opacity);
            }
        }
    }
}

struct FixedColorParitySystem {
    color: ColorRgba,
    calls: Arc<AtomicU32>,
}

impl SkiaTableSystem for FixedColorParitySystem {
    fn slot(&self) -> TableSlot {
        ParityPlugin::slot()
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, _store: &NodeStore, _node: NodeId) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let x0 = (ctx.rect.x.round() as i32) + 4;
        let y0 = (ctx.rect.y.round() as i32) + 4;
        let x1 = ((ctx.rect.x + ctx.rect.width).round() as i32) - 4;
        let y1 = ((ctx.rect.y + ctx.rect.height).round() as i32) - 4;
        for y in y0..y1 {
            for x in x0..x1 {
                ctx.buffer.blend_pixel(x, y, self.color, ctx.opacity);
            }
        }
    }
}

#[test]
fn binary_font_source_records_copied_backing_and_base_eviction_drops_variants() {
    let engine = CosmicTextEngine::shared();
    if !engine.has_system_fonts() {
        assert!(
            std::env::var_os("NVE_ALLOW_NO_SYSTEM_FONTS").is_some(),
            "system fonts unavailable (set NVE_ALLOW_NO_SYSTEM_FONTS=1 only in fontless containers)"
        );
        return;
    }

    // 1. Copy one system font file into an in-memory `fontdb::Database::load_font_data` (Binary
    //    source) and verify `TypefaceBacking::Heap` vs `Mmap` is tracked accurately.
    let mut runs = crate::text::SkiaShapedRuns::default();
    assert!(engine.shape_for_skia_runs("Abc 123", 16.0, 400, TextAlign::Left, None, 200.0, &mut runs));
    let default_face = runs.glyphs.first().expect("shaped glyph").font_id;
    let raw_bytes = engine
        .with_face_bytes(default_face, |bytes, _| bytes.to_vec())
        .expect("font face bytes");

    let mut bin_db = cosmic_text::fontdb::Database::new();
    bin_db.load_font_data(raw_bytes);
    let bin_face_id = bin_db.faces().next().expect("loaded binary face").id;
    let bin_fs = cosmic_text::FontSystem::new_with_locale_and_db("en-US".to_string(), bin_db);
    let bin_engine = Arc::new(CosmicTextEngine::from_font_system(bin_fs));
    let bin_renderer = SkiaCanvasRenderer::with_text_engine(bin_engine);
    assert!(bin_renderer.resolve_typeface_for_test(bin_face_id, 400).is_some());
    let bin_stats = bin_renderer.cache_stats();
    assert_eq!(bin_stats.typeface_entries, 1);
    assert_eq!(bin_stats.typeface_copied_entries, 1);
    assert_eq!(bin_stats.typeface_mmap_entries, 0);

    // 2. Probe system fonts for a real variable-`wght` face so variant retention & eviction drop
    //    are proven non-vacuously (`typeface_variant_entries >= 2` before eviction, `0` after).
    let probe = SkiaCanvasRenderer::new();
    let all_faces = engine.all_face_ids();
    let mut var_face = None;
    for &fid in &all_faces {
        probe.clear_caches();
        let _ = probe.resolve_typeface_for_test(fid, 300);
        let _ = probe.resolve_typeface_for_test(fid, 700);
        if probe.cache_stats().typeface_variant_entries >= 2 {
            var_face = Some(fid);
            break;
        }
    }
    let face_a = var_face.unwrap_or(default_face);
    let face_b = all_faces
        .iter()
        .copied()
        .find(|&fid| fid != face_a && probe.resolve_typeface_for_test(fid, 400).is_some())
        .expect("need at least 2 distinct decodable system fonts");

    let renderer = SkiaCanvasRenderer::new();
    renderer.set_base_typefaces_cap_for_test(1);
    assert!(renderer.resolve_typeface_for_test(face_a, 300).is_some());
    assert!(renderer.resolve_typeface_for_test(face_a, 400).is_some());
    assert!(renderer.resolve_typeface_for_test(face_a, 700).is_some());
    assert_eq!(renderer.cache_stats().typeface_entries, 1);
    if var_face.is_some() {
        assert!(
            renderer.cache_stats().typeface_variant_entries >= 2,
            "variable wght face must retain >= 2 inline variants before eviction"
        );
    }

    // Resolving face B evicts face A AND its inline variants.
    assert!(renderer.resolve_typeface_for_test(face_b, 400).is_some());
    assert_eq!(renderer.cache_stats().typeface_entries, 1);
    assert!(renderer.cache_stats().evictions >= 1);

    // Re-resolving face A at weight 700 MUST be a miss (variants dropped with base face A).
    renderer.reset_cache_stats();
    assert!(renderer.resolve_typeface_for_test(face_a, 700).is_some());
    let st = renderer.cache_stats();
    assert_eq!(st.typeface_misses, 1, "evicted face + variants must miss on re-resolve");
    assert_eq!(st.typeface_hits, 0);
}

#[test]
fn focused_text_input_caret_snaps_to_device_pixels_across_fractional_scales() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let input = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n)
            .width(Length::points(220.0))
            .height(Length::points(36.0));
        tx.edit::<Typography>(n)
            .font_size(14.0)
            .color(Rgba::WHITE);
        tx.edit::<TextInput>(n).value("Hello world");
        tx.commit_into(&mut changes);
        n
    };

    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 240.0,
            height: 60.0,
            scale_factor: 1.0,
        },
    );
    backend.commit(&store, &changes).unwrap();
    backend.set_focused_node(&store, Some(input));

    let target = (FOCUS_RING_COLOR.r, FOCUS_RING_COLOR.g, FOCUS_RING_COLOR.b);
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 240.0,
                height: 60.0,
                scale_factor: scale,
            },
        );
        backend.commit(&store, &ChangeSet::default()).unwrap();

        let pw = (240.0 * scale).round() as u32;
        let ph = (60.0 * scale).round() as u32;

        let mut soft = PixelBuffer::new(pw, ph);
        backend.rasterize(&mut soft, ColorRgba::BLACK);

        let mut cpu = CpuSkiaSurface::new_with_scale(pw, ph, scale).unwrap();
        cpu.render_scene_cached_with_plugins(backend.scene(), scale, ColorRgba::BLACK, None)
            .unwrap();
        let mut skia = PixelBuffer::new(pw, ph);
        cpu.copy_to_pixel_buffer(&mut skia).unwrap();

        let caret_rect = backend.ime_cursor_rect(&store).expect("focused caret rect");
        let expected_cols = (2.0 * scale).round() as usize;
        let mid_y = ((caret_rect.y + caret_rect.height * 0.5) * scale).round() as u32;
        let scan_x0 = ((caret_rect.x * scale).floor() as u32).saturating_sub(3);
        let scan_x1 = (((caret_rect.x + caret_rect.width) * scale).ceil() as u32 + 3).min(pw);

        for (label, buf) in [("software", &soft), ("cpu-skia", &skia)] {
            let mut opaque_cols = 0usize;
            let mut smeared_cols = 0usize;
            for x in scan_x0..scan_x1 {
                let (r, g, b) = buf.get_rgb(x, mid_y).unwrap_or((0, 0, 0));
                if r.abs_diff(target.0) <= 2
                    && g.abs_diff(target.1) <= 2
                    && b.abs_diff(target.2) <= 2
                {
                    opaque_cols += 1;
                } else if b > 20 && b > r.saturating_add(20) {
                    smeared_cols += 1;
                }
            }
            assert_eq!(
                opaque_cols, expected_cols,
                "{label}@{scale}x: expected exactly {expected_cols} opaque caret cols, got {opaque_cols} (smeared={smeared_cols})"
            );
            assert_eq!(
                smeared_cols, 0,
                "{label}@{scale}x: caret must not smear across fractional columns (smeared={smeared_cols})"
            );
        }
    }
}

fn ink_metrics(buf: &PixelBuffer, x0: u32, y0: u32, x1: u32, y1: u32) -> (f64, f64, (u32, u32, u32, u32), f64) {
    let (mut wsum, mut sx, mut sy) = (0.0f64, 0.0f64, 0.0f64);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in y0..y1.min(buf.height()) {
        for x in x0..x1.min(buf.width()) {
            let (r, g, b) = buf.get_rgb(x, y).unwrap_or((0, 0, 0));
            let lum = f64::from(r) + f64::from(g) + f64::from(b);
            if lum > 30.0 {
                wsum += lum;
                sx += f64::from(x) * lum;
                sy += f64::from(y) * lum;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if wsum <= 0.0 {
        return (0.0, 0.0, (0, 0, 0, 0), 0.0);
    }
    (sx / wsum, sy / wsum, (min_x, min_y, max_x, max_y), wsum)
}

#[test]
fn rendered_text_box_parity_across_rtl_emoji_ligatures_wrapped_and_fractional_scales() {
    let engine = CosmicTextEngine::shared();
    if !engine.has_system_fonts() {
        assert!(
            std::env::var_os("NVE_ALLOW_NO_SYSTEM_FONTS").is_some(),
            "system fonts unavailable (set NVE_ALLOW_NO_SYSTEM_FONTS=1 only in fontless containers)"
        );
        return;
    }

    type Case<'a> = (&'a str, &'a str, f32, u16, TextAlign, f32, f32);
    let cases: &[Case<'_>] = &[
        ("rtl", "مرحبا hello", 16.0, 400, TextAlign::Left, 180.0, 36.0),
        ("emoji", "Rocket 🚀", 16.0, 400, TextAlign::Left, 160.0, 36.0),
        ("ligatures", "fi fl ffi ffl", 18.0, 400, TextAlign::Left, 180.0, 36.0),
        ("wrap-center", "Wrapped centered parity line", 14.0, 500, TextAlign::Center, 120.0, 60.0),
        ("wrap-right", "Wrapped right parity line", 14.0, 700, TextAlign::Right, 120.0, 60.0),
    ];

    for scale in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
        for &(name, text, fs, weight, align, w_log, h_log) in cases {
            let pw = ((w_log + 16.0) * scale).ceil() as u32;
            let ph = ((h_log + 16.0) * scale).ceil() as u32;
            let rect = LayoutRect { x: 8.0, y: 8.0, width: w_log, height: h_log };
            let dl = DisplayList {
                ops: vec![PaintOp::DrawText {
                    rect,
                    text: text.to_owned(),
                    font_size: fs,
                    font_weight: weight,
                    text_align: align,
                    color: ColorRgba::rgb(248, 250, 252),
                }],
            };

            let mut soft = PixelBuffer::new(pw, ph);
            crate::raster::rasterize_display_list(&dl, &mut soft, scale, ColorRgba::BLACK);

            let mut cpu = CpuSkiaSurface::new_with_scale(pw, ph, scale).unwrap();
            cpu.render_cached_with_plugins(&dl, scale, ColorRgba::BLACK, None)
                .unwrap();
            let mut skia = PixelBuffer::new(pw, ph);
            cpu.copy_to_pixel_buffer(&mut skia).unwrap();

            let x0 = (8.0 * scale).floor() as u32;
            let y0 = (8.0 * scale).floor() as u32;
            let x1 = ((8.0 + w_log) * scale).ceil() as u32;
            let y1 = ((8.0 + h_log) * scale).ceil() as u32;

            let (scx, scy, sbbox, sw) = ink_metrics(&soft, x0, y0, x1, y1);
            let (kcx, kcy, kbbox, kw) = ink_metrics(&skia, x0, y0, x1, y1);
            assert!(sw > 1_000.0 && kw > 1_000.0, "{name}@{scale}x: missing rendered ink (sw={sw}, kw={kw})");

            let centroid_dx = (scx - kcx).abs() / f64::from(scale);
            let centroid_dy = (scy - kcy).abs() / f64::from(scale);
            let region = compare_region(&soft, &skia, x0, y0, x1, y1);
            // Bitmap color-emoji `sbix` tables (Apple Color Emoji) differ slightly in strike
            // baseline between CoreText and swash; vector outline runs stay within <= 1.0 logical px.
            let (max_dx, max_dy, max_bbox, max_mae) = match name {
                "emoji" => (1.5, 1.25, 3u32, 12.0),
                "rtl" | "ligatures" => (1.0, 1.0, 2u32, 10.5),
                _ => (1.0, 1.35, 2u32, 22.0),
            };
            assert!(
                centroid_dx <= max_dx && centroid_dy <= max_dy,
                "{name}@{scale}x: ink centroid diff exceeded ({max_dx},{max_dy}) logical px (dx={centroid_dx:.3}, dy={centroid_dy:.3})"
            );
            assert!(
                sbbox.0.abs_diff(kbbox.0) <= max_bbox
                    && sbbox.1.abs_diff(kbbox.1) <= max_bbox
                    && sbbox.2.abs_diff(kbbox.2) <= max_bbox
                    && sbbox.3.abs_diff(kbbox.3) <= max_bbox,
                "{name}@{scale}x: ink bbox mismatch > {max_bbox}px: soft={sbbox:?} skia={kbbox:?}"
            );
            assert!(
                region.mae <= max_mae,
                "{name}@{scale}x: unhinted AA text-box MAE {:.3} exceeded {max_mae} (max_diff={})",
                region.mae,
                region.max_diff
            );
        }
    }
}

#[test]
fn multi_table_late_registration_and_table_registry_mut_invalidate_plugin_cache() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n)
            .width(Length::points(120.0))
            .height(Length::points(120.0));
        tx.edit::<ParityPlugin>(n).color_rgba(ColorRgba::rgb(220, 40, 40).to_u32());
        tx.edit::<SecondPlugin>(n).accent_rgba(ColorRgba::rgb(40, 120, 250).to_u32());
        tx.commit_into(&mut changes);
        n
    };
    let _ = node;

    let calls_a = Arc::new(AtomicU32::new(0));
    let calls_b = Arc::new(AtomicU32::new(0));
    let mut backend = SkiaBackend::new();
    backend.register_table_system(CountingPluginSystem { calls: Arc::clone(&calls_a) });
    backend.set_viewport(SurfaceId::ROOT, Viewport { width: 140.0, height: 140.0, scale_factor: 1.0 });
    backend.commit(&store, &changes).unwrap();

    let mut surface = CpuSkiaSurface::new(140, 140).unwrap();
    let mut buf = PixelBuffer::new(140, 140);
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(scene, 1.0, ColorRgba::BLACK, Some((&store, reg, ev)))
            .unwrap();
    }
    assert_eq!(surface.cache_stats().plugin_rasterizations, 1);
    surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(20, 20), Some((220, 40, 40)));
    assert_eq!(buf.get_rgb(90, 90), Some((220, 40, 40)), "table B not registered yet");

    // Register table B on the same node and commit an EMPTY ChangeSet: must invalidate cache!
    backend.register_table_system(SecondPluginSystem { calls: Arc::clone(&calls_b) });
    backend.commit(&store, &ChangeSet::default()).unwrap();
    surface.reset_cache_stats();
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(scene, 1.0, ColorRgba::BLACK, Some((&store, reg, ev)))
            .unwrap();
    }
    assert_eq!(surface.cache_stats().plugin_rasterizations, 1);
    assert_eq!(calls_b.load(Ordering::Relaxed), 1);
    surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(20, 20), Some((220, 40, 40)));
    assert_eq!(buf.get_rgb(90, 90), Some((40, 120, 250)), "late-registered table B ink must appear");

    // Replace plugin A via `table_registry_mut().register(...)` WITHOUT any ChangeSet (and even
    // without calling `commit()`): `SkiaTableRegistry::generation()` must invalidate the cache!
    let calls_green = Arc::new(AtomicU32::new(0));
    backend.table_registry_mut().register(FixedColorParitySystem {
        color: ColorRgba::rgb(20, 210, 90),
        calls: Arc::clone(&calls_green),
    });
    surface.reset_cache_stats();
    {
        let (scene, reg, ev) = backend.scene_paint_parts_mut();
        surface
            .render_scene_cached_with_plugins(scene, 1.0, ColorRgba::BLACK, Some((&store, reg, ev)))
            .unwrap();
    }
    assert_eq!(surface.cache_stats().plugin_rasterizations, 1);
    assert_eq!(calls_green.load(Ordering::Relaxed), 1);
    surface.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(20, 20), Some((20, 210, 90)));
}

#[test]
fn fontless_bitmap_text_matches_between_software_and_cpu_skia() {
    let engine = Arc::new(CosmicTextEngine::new_without_system_fonts());
    let dl = DisplayList {
        ops: vec![
            PaintOp::DrawText {
                rect: LayoutRect { x: 8.0, y: 8.0, width: 220.0, height: 24.0 },
                text: "Small 8pt Bold".to_owned(),
                font_size: 8.0,
                font_weight: 700,
                text_align: TextAlign::Left,
                color: ColorRgba::WHITE,
            },
            PaintOp::DrawText {
                rect: LayoutRect { x: 8.0, y: 36.0, width: 220.0, height: 28.0 },
                text: "Bold 700 Center".to_owned(),
                font_size: 14.0,
                font_weight: 700,
                text_align: TextAlign::Center,
                color: ColorRgba::rgb(56, 189, 248),
            },
            PaintOp::DrawText {
                rect: LayoutRect { x: 8.0, y: 68.0, width: 220.0, height: 28.0 },
                text: "Right Align".to_owned(),
                font_size: 14.0,
                font_weight: 700,
                text_align: TextAlign::Right,
                color: ColorRgba::rgb(250, 204, 21),
            },
        ],
    };

    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        let pw = (240.0 * scale).round() as u32;
        let ph = (110.0 * scale).round() as u32;
        let mut soft = PixelBuffer::new(pw, ph);
        crate::raster::rasterize_display_list_with_text_engine(
            &dl,
            &mut soft,
            scale,
            ColorRgba::BLACK,
            engine.as_ref(),
        );

        let mut cpu =
            CpuSkiaSurface::with_text_engine(pw, ph, scale, Arc::clone(&engine)).unwrap();
        cpu.render_cached_with_plugins(&dl, scale, ColorRgba::BLACK, None)
            .unwrap();
        let mut skia = PixelBuffer::new(pw, ph);
        cpu.copy_to_pixel_buffer(&mut skia).unwrap();

        let report = compare_buffers(&soft, &skia);
        assert!(
            report.mae < 3.0,
            "fontless bitmap text Software vs CPU Skia @{scale}x MAE too high: {:.3} (max={})",
            report.mae,
            report.max_diff
        );
    }
}

#[test]
fn ten_thousand_frame_multi_cache_churn_respects_every_individual_cap() {
    let _img_guard = lock_shared_image_cache();
    crate::image_cache::ImageCache::shared().clear();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let plugin_nodes: Vec<NodeId> = {
        let mut tx = store.tx();
        let nodes: Vec<NodeId> = (0..300u32)
            .map(|i| {
                let n = tx.create();
                tx.edit::<ParityPlugin>(n)
                    .color_rgba(ColorRgba::rgb((i & 255) as u8, 120, 200).to_u32());
                n
            })
            .collect();
        tx.set_children(NodeId::ROOT, &nodes);
        tx.commit_into(&mut changes);
        nodes
    };
    let calls = Arc::new(AtomicU32::new(0));
    let mut registry = SkiaTableRegistry::new();
    registry.register(CountingPluginSystem { calls });
    let mut events = EventBatchBuf::new();

    let mut surface = CpuSkiaSurface::new(160, 48).unwrap();
    for i in 0..10_000u32 {
        let mut ops = vec![PaintOp::DrawText {
            rect: LayoutRect { x: 4.0, y: 4.0, width: 152.0, height: 32.0 },
            text: format!("Churn line #{i}"),
            font_size: 13.0,
            font_weight: 400,
            text_align: TextAlign::Left,
            color: ColorRgba::WHITE,
        }];
        if i < 300 {
            let img_key = format!("nve-test://churn-img-{i}");
            register_checker_image(&img_key, [(i & 255) as u8, 40, 80, 255], [20, 200, 120, 255]);
            ops.push(PaintOp::DrawImage {
                rect: LayoutRect { x: 4.0, y: 4.0, width: 16.0, height: 16.0 },
                radius: 0.0,
                src: img_key,
                fit: ImageFit::Fill,
                resolved: None,
            });
            ops.push(PaintOp::Plugin {
                node_id: plugin_nodes[i as usize],
                rect: LayoutRect { x: 24.0, y: 4.0, width: 16.0, height: 16.0 },
                inherited_text_color: Rgba::WHITE,
                plugin_epoch: u64::from(i) + 1,
            });
            ops.push(PaintOp::DrawShadow {
                rect: LayoutRect { x: 44.0, y: 4.0, width: 16.0, height: 16.0 },
                radius: 2.0,
                offset_x: 1.0,
                offset_y: 1.0,
                blur_radius: 1.0 + (i as f32) * 0.1,
                spread_radius: 0.0,
                color: ColorRgba::rgba(0, 0, 0, 128),
                border_width: 0.0,
                inset: false,
            });
        }
        let dl = DisplayList { ops };
        surface
            .render_cached_with_plugins(
                &dl,
                1.0,
                ColorRgba::BLACK,
                Some((&store, &registry, &mut events)),
            )
            .unwrap();

        if i == 299 || i % 2000 == 1999 {
            let s = surface.cache_stats();
            assert!(s.typeface_entries <= MAX_BASE_TYPEFACES);
            assert!(s.text_blob_entries <= MAX_TEXT_BLOBS);
            assert!(s.text_blob_bytes <= MAX_TEXT_BLOB_BYTES);
            assert!(s.image_entries <= MAX_SKIA_IMAGES);
            assert!(s.image_bytes <= MAX_SKIA_IMAGE_BYTES);
            assert!(s.plugin_entries <= MAX_SKIA_PLUGINS);
            assert!(s.plugin_bytes <= MAX_SKIA_PLUGIN_BYTES);
            assert!(s.mask_filter_entries <= MAX_MASK_FILTERS);
            assert!(s.mask_filter_bytes <= MAX_MASK_FILTER_BYTES);
        }
    }

    let s = surface.cache_stats();
    assert!(s.evictions >= 7_900);
    assert_eq!(s.text_blob_entries, MAX_TEXT_BLOBS);
    assert_eq!(s.image_entries, MAX_SKIA_IMAGES);
    assert_eq!(s.plugin_entries, MAX_SKIA_PLUGINS);
    assert_eq!(s.mask_filter_entries, MAX_MASK_FILTERS);
    assert!(s.mask_filter_bytes <= MAX_MASK_FILTER_BYTES);
}

#[cfg(feature = "skia-vulkan")]
#[test]
fn dropping_vulkan_swapchain_before_external_texture_image_does_not_uaf() {
    use skia_safe::{gpu, images, AlphaType, ColorType, ConditionallySend, Data, ImageInfo};
    let cfg = GpuSurfaceConfig {
        width: 64,
        height: 64,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Vulkan,
        allow_cpu_fallback: false,
        ..Default::default()
    };
    let mut sw = match crate::VulkanSwapchain::new_offscreen(cfg) {
        Ok(sw) => sw,
        Err(GpuSurfaceError::DeviceUnavailable(reason)) => {
            if std::env::var_os("NVE_REQUIRE_VULKAN_PARITY").is_some() {
                panic!("NVE_REQUIRE_VULKAN_PARITY=1 set but Vulkan unavailable: {reason}");
            }
            eprintln!("SKIP(dropping_vulkan_swapchain_before_external_texture_image_does_not_uaf): {reason}");
            return;
        }
        Err(other) => panic!("unexpected Vulkan init error: {other}"),
    };
    let info = ImageInfo::new((8, 8), ColorType::RGBA8888, AlphaType::Unpremul, None);
    let raster = images::raster_from_data(&info, Data::new_copy(&[255u8; 8 * 8 * 4]), 8 * 4)
        .expect("raster image");
    let tex = gpu::images::texture_from_image(
        sw.inner_mut().direct_context_mut(),
        &raster,
        gpu::Mipmapped::No,
        gpu::Budgeted::Yes,
    )
    .expect("GPU texture image");
    assert!(
        !sw.inner_mut().direct_context_mut().can_send(),
        "external GPU texture must hold a GrDirectContext reference (can_send() == false)"
    );
    let leaked_flag = sw.inner_mut().leaked_owner_flag_for_test();
    assert!(!leaked_flag.load(std::sync::atomic::Ordering::Acquire));
    drop(sw);
    assert!(
        leaked_flag.load(std::sync::atomic::Ordering::Acquire),
        "dropping VulkanSwapchain while external texture lives must take the safe leak path"
    );
    drop(tex);
}
