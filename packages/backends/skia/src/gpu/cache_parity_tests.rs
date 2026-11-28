//! Whole-frame pixel-diff parity, per-text bounding-box parity, variable-font `wght` ink
//! coverage, GPU texture cache warm-frame verification, and real multi-context / drop-order tests.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub(super) use crate::image_cache::lock_shared_image_cache;

use nve_abi::{ColorRgba, EventBatchBuf};
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, Rgba, TableSlot};
use nve_tables::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
use nve_tables::tables::ImageFit;
use nve_tables::transform::Transform2D;

use super::super::*;
use crate::image_cache::ImageCache;
use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry, SkiaTableSystem};
use crate::text::{CosmicTextEngine, TextAlign};
use crate::{DisplayList, PaintOp};

#[derive(NveTable)]
#[nve_table(name = "nve.test.cache_plugin_4b", storage = "sparse")]
#[allow(dead_code)]
pub(super) struct ParityPlugin {
    #[effect(paint)]
    pub color_rgba: u32,
}

pub(super) struct CountingPluginSystem {
    pub(super) calls: Arc<AtomicU32>,
}

impl SkiaTableSystem for CountingPluginSystem {
    fn slot(&self) -> TableSlot {
        ParityPlugin::slot()
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let Some(row) = store.try_row::<ParityPlugin>(node) else {
            return;
        };
        let c = ColorRgba::from_u32(row.color_rgba());
        let x0 = (ctx.rect.x.round() as i32) + 4;
        let y0 = (ctx.rect.y.round() as i32) + 4;
        let x1 = ((ctx.rect.x + ctx.rect.width).round() as i32) - 4;
        let y1 = ((ctx.rect.y + ctx.rect.height).round() as i32) - 4;
        for y in y0..y1 {
            for x in x0..x1 {
                ctx.buffer.blend_pixel(x, y, c, ctx.opacity);
            }
        }
    }
}

pub(super) struct ParityReport {
    pub(super) mae: f64,
    pub(super) max_diff: u8,
    pub(super) exact_match: bool,
}

pub(super) fn compare_buffers(a: &PixelBuffer, b: &PixelBuffer) -> ParityReport {
    assert_eq!(a.width(), b.width());
    assert_eq!(a.height(), b.height());
    compare_region(a, b, 0, 0, a.width(), a.height())
}

pub(super) fn compare_region(
    a: &PixelBuffer,
    b: &PixelBuffer,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> ParityReport {
    let (mut sum_diff, mut max_diff, mut exact, mut count) = (0u64, 0u8, true, 0u64);
    let (max_x, max_y) = (x1.min(a.width()).min(b.width()), y1.min(a.height()).min(b.height()));
    for y in y0..max_y {
        for x in x0..max_x {
            let (ra, ga, ba) = a.get_rgb(x, y).unwrap_or((0, 0, 0));
            let (rb, gb, bb) = b.get_rgb(x, y).unwrap_or((0, 0, 0));
            if (ra, ga, ba) != (rb, gb, bb) {
                exact = false;
            }
            let (dr, dg, db) = (ra.abs_diff(rb), ga.abs_diff(gb), ba.abs_diff(bb));
            sum_diff += u64::from(dr) + u64::from(dg) + u64::from(db);
            max_diff = max_diff.max(dr).max(dg).max(db);
            count += 1;
        }
    }
    ParityReport {
        mae: sum_diff as f64 / (count * 3).max(1) as f64,
        max_diff,
        exact_match: exact,
    }
}

pub(super) fn register_checker_image(key: &str, c0: [u8; 4], c1: [u8; 4]) {
    let mut rgba = vec![0u8; 16 * 16 * 4];
    for y in 0..16usize {
        for x in 0..16usize {
            let c = if ((x / 4) + (y / 4)) % 2 == 0 { c0 } else { c1 };
            let off = (y * 16 + x) * 4;
            rgba[off..off + 4].copy_from_slice(&c);
        }
    }
    ImageCache::shared()
        .insert_raw_rgba(key, 16, 16, rgba)
        .expect("insert checker image");
}

pub(super) fn build_rich_corpus(img_key: &str, plugin_node: NodeId, plugin_epoch: u64) -> DisplayList {
    let tf = Transform2D { rotate_deg: 12.0, ..Transform2D::IDENTITY };
    let r = |x, y, width, height| LayoutRect { x, y, width, height };
    DisplayList {
        ops: vec![
            PaintOp::DrawShadow {
                rect: r(16.0, 16.0, 96.0, 64.0),
                radius: 12.0,
                offset_x: 4.0,
                offset_y: 6.0,
                blur_radius: 10.0,
                spread_radius: 1.0,
                color: ColorRgba::rgba(15, 23, 42, 180),
                border_width: 0.0,
                inset: false,
            },
            PaintOp::FillRoundedRect {
                rect: r(16.0, 16.0, 96.0, 64.0),
                radius: 12.0,
                color: ColorRgba::rgb(37, 99, 235),
            },
            PaintOp::FillRoundedRectGradient {
                rect: r(124.0, 16.0, 96.0, 64.0),
                radius: 10.0,
                gradient: BackgroundFill::Linear(LinearGradient::new(
                    45.0,
                    vec![
                        GradientStop::new(0.0, ColorRgba::rgb(239, 68, 68)),
                        GradientStop::new(0.5, ColorRgba::rgb(245, 158, 11)),
                        GradientStop::new(1.0, ColorRgba::rgb(16, 185, 129)),
                    ],
                )),
            },
            PaintOp::FillRoundedRectGradient {
                rect: r(232.0, 16.0, 96.0, 64.0),
                radius: 10.0,
                gradient: BackgroundFill::Radial(RadialGradient::new(
                    0.5,
                    0.5,
                    0.5,
                    vec![
                        GradientStop::new(0.0, ColorRgba::rgb(244, 114, 182)),
                        GradientStop::new(1.0, ColorRgba::rgb(99, 102, 241)),
                    ],
                )),
            },
            PaintOp::PushClip { rect: r(16.0, 96.0, 110.0, 70.0), radius: 14.0 },
            PaintOp::SaveOpacity(0.6),
            PaintOp::PushTransform { matrix: tf.to_surface_matrix_2d(32.0, 106.0, 76.0, 48.0) },
            PaintOp::FillRoundedRect {
                rect: r(32.0, 106.0, 76.0, 48.0),
                radius: 8.0,
                color: ColorRgba::rgb(236, 72, 153),
            },
            PaintOp::PopTransform,
            PaintOp::Restore,
            PaintOp::PopClip,
            PaintOp::DrawImage {
                rect: r(138.0, 96.0, 48.0, 32.0),
                radius: 6.0,
                src: img_key.to_owned(),
                fit: ImageFit::Cover,
                resolved: ImageCache::shared().resolve_cached_handle(img_key),
            },
            PaintOp::DrawImage {
                rect: r(194.0, 96.0, 48.0, 32.0),
                radius: 6.0,
                src: img_key.to_owned(),
                fit: ImageFit::Contain,
                resolved: ImageCache::shared().resolve_cached_handle(img_key),
            },
            PaintOp::DrawImage {
                rect: r(250.0, 96.0, 48.0, 32.0),
                radius: 6.0,
                src: img_key.to_owned(),
                fit: ImageFit::Fill,
                resolved: ImageCache::shared().resolve_cached_handle(img_key),
            },
            PaintOp::DrawText {
                rect: r(16.0, 178.0, 150.0, 28.0),
                text: "Regular 400 Text".to_owned(),
                font_size: 16.0,
                font_weight: 400,
                text_align: TextAlign::Left,
                color: ColorRgba::rgb(248, 250, 252),
            },
            PaintOp::DrawText {
                rect: r(16.0, 208.0, 150.0, 28.0),
                text: "Regular 400 Text".to_owned(),
                font_size: 16.0,
                font_weight: 700,
                text_align: TextAlign::Left,
                color: ColorRgba::rgb(248, 250, 252),
            },
            PaintOp::DrawText {
                rect: r(176.0, 178.0, 150.0, 56.0),
                text: "Wrapped centered HarfBuzz parity line".to_owned(),
                font_size: 14.0,
                font_weight: 500,
                text_align: TextAlign::Center,
                color: ColorRgba::rgb(56, 189, 248),
            },
            PaintOp::Plugin {
                node_id: plugin_node,
                rect: r(306.0, 96.0, 32.0, 32.0),
                inherited_text_color: Rgba::WHITE,
                plugin_epoch,
            },
        ],
    }
}

pub(super) fn count_lit_ink(buf: &PixelBuffer, x0: u32, y0: u32, x1: u32, y1: u32) -> u64 {
    let mut total = 0u64;
    for y in y0..y1.min(buf.height()) {
        for x in x0..x1.min(buf.width()) {
            if let Some((r, g, b)) = buf.get_rgb(x, y) {
                total += u64::from(r) + u64::from(g) + u64::from(b);
            }
        }
    }
    total
}

struct ParityFixture {
    store: NodeStore,
    registry: SkiaTableRegistry,
    events: EventBatchBuf,
    calls: Arc<AtomicU32>,
    dl: DisplayList,
}

fn setup_parity_fixture(img_key: &str, plugin_color: ColorRgba) -> ParityFixture {
    register_checker_image(img_key, [255, 100, 50, 255], [40, 200, 140, 255]);
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let plugin_node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<ParityPlugin>(n).color_rgba(plugin_color.to_u32());
        tx.commit_into(&mut changes);
        n
    };
    let calls = Arc::new(AtomicU32::new(0));
    let mut registry = SkiaTableRegistry::new();
    registry.register(CountingPluginSystem { calls: Arc::clone(&calls) });
    ParityFixture {
        store,
        registry,
        events: EventBatchBuf::new(),
        calls,
        dl: build_rich_corpus(img_key, plugin_node, 1),
    }
}

#[cfg(any(
    all(feature = "skia-metal", target_os = "macos"),
    feature = "skia-vulkan",
    all(feature = "skia-d3d", target_os = "windows")
))]
fn verify_hardware_gpu_parity_and_warm_cache(
    label: &str,
    backend: GpuBackendKind,
    img_key: &str,
    mut sw: GpuSwapchain,
) -> (GpuSwapchain, PixelBuffer, ParityFixture) {
    assert_eq!(sw.kind(), backend);
    let mut fx = setup_parity_fixture(img_key, ColorRgba::rgb(250, 204, 21));

    let mut cpu = CpuSkiaSurface::new(360, 250).unwrap();
    cpu.render_cached_with_plugins(
        &fx.dl,
        1.0,
        ColorRgba::BLACK,
        Some((&fx.store, &fx.registry, &mut fx.events)),
    )
    .unwrap();
    let mut cpu_buf = PixelBuffer::new(360, 250);
    cpu.copy_to_pixel_buffer(&mut cpu_buf).unwrap();

    // Cold frame 1: uploads image + plugin textures once.
    sw.execute_frame_with_plugins(
        &fx.dl,
        1.0,
        ColorRgba::BLACK,
        None,
        Some((&fx.store, &fx.registry, &mut fx.events)),
    )
    .unwrap();
    let cold = sw.cache_stats().expect("skia stats");
    assert_eq!(cold.image_uploads, 1, "{label}: shared DrawImage must upload once");
    assert_eq!(cold.image_hits, 2);
    assert_eq!(cold.plugin_uploads, 1);
    assert_eq!(cold.plugin_rasterizations, 1);

    let mut gpu_buf1 = PixelBuffer::new(360, 250);
    sw.copy_to_pixel_buffer(&mut gpu_buf1).unwrap();
    let parity = compare_buffers(&cpu_buf, &gpu_buf1);
    assert!(
        parity.mae < 3.0,
        "{label} vs CPU Skia whole-frame MAE too high: {:.3} (max={})",
        parity.mae,
        parity.max_diff
    );

    // Warm frame 2: zero uploads, zero misses, zero plugin rasterizations.
    let calls_before = fx.calls.load(Ordering::Relaxed);
    sw.reset_cache_stats();
    sw.execute_frame_with_plugins(
        &fx.dl,
        1.0,
        ColorRgba::BLACK,
        None,
        Some((&fx.store, &fx.registry, &mut fx.events)),
    )
    .unwrap();
    let warm = sw.cache_stats().expect("skia stats");
    assert_eq!(warm.image_uploads, 0);
    assert_eq!(warm.image_misses, 0);
    assert_eq!(warm.image_hits, 3);
    assert_eq!(warm.plugin_uploads, 0);
    assert_eq!(warm.plugin_rasterizations, 0);
    assert_eq!(warm.plugin_hits, 1);
    assert_eq!(warm.text_blob_misses, 0);
    assert_eq!(warm.text_blob_hits, 3);
    assert_eq!(fx.calls.load(Ordering::Relaxed), calls_before);

    let mut gpu_buf2 = PixelBuffer::new(360, 250);
    sw.copy_to_pixel_buffer(&mut gpu_buf2).unwrap();
    assert!(compare_buffers(&gpu_buf1, &gpu_buf2).exact_match);
    (sw, gpu_buf1, fx)
}

#[test]
fn software_and_cpu_skia_rich_corpus_whole_frame_parity_and_bold_weight_ink() {
    let _img_guard = lock_shared_image_cache();
    let mut fx = setup_parity_fixture("nve-test://4b-parity-checker-1", ColorRgba::rgb(34, 197, 94));

    let mut soft = PixelBuffer::new(360, 250);
    crate::raster::rasterize_display_list_with_plugins(
        &fx.dl,
        &mut soft,
        1.0,
        ColorRgba::BLACK,
        CosmicTextEngine::shared().as_ref(),
        &fx.store,
        &fx.registry,
        &mut fx.events,
    );

    let mut cpu = CpuSkiaSurface::new(360, 250).expect("cpu skia surface");
    cpu.render_cached_with_plugins(
        &fx.dl,
        1.0,
        ColorRgba::BLACK,
        Some((&fx.store, &fx.registry, &mut fx.events)),
    )
    .expect("cpu skia render");
    let mut cpu_buf = PixelBuffer::new(360, 250);
    cpu.copy_to_pixel_buffer(&mut cpu_buf).expect("readback");

    let report = compare_buffers(&soft, &cpu_buf);
    assert!(
        report.mae < 6.0,
        "software vs cpu-skia whole-frame MAE too high: {:.3} (max_diff={})",
        report.mae,
        report.max_diff
    );

    for (label, x0, y0, x1, y1, max_mae) in [
        ("regular-400", 16, 178, 166, 206, 18.0),
        ("bold-700", 16, 208, 166, 236, 28.0),
        ("wrapped-center-500", 176, 178, 326, 234, 18.0),
    ] {
        let text_diff = compare_region(&soft, &cpu_buf, x0, y0, x1, y1);
        assert!(
            text_diff.mae < max_mae,
            "software vs cpu-skia text-box {label} MAE too high: {:.3} (limit={max_mae}, max_diff={})",
            text_diff.mae,
            text_diff.max_diff
        );
    }

    let allow_no_fonts = std::env::var("NVE_ALLOW_NO_SYSTEM_FONTS").as_deref() == Ok("1");
    if !allow_no_fonts {
        assert!(
            CosmicTextEngine::shared().has_system_fonts(),
            "system fonts required for bold weight ink verification (set NVE_ALLOW_NO_SYSTEM_FONTS=1 on headless fontless CI)"
        );
        let soft_reg = count_lit_ink(&soft, 16, 178, 166, 206);
        let soft_bold = count_lit_ink(&soft, 16, 208, 166, 236);
        let skia_reg = count_lit_ink(&cpu_buf, 16, 178, 166, 206);
        let skia_bold = count_lit_ink(&cpu_buf, 16, 208, 166, 236);
        assert!(skia_reg > 5_000, "expected visible regular text ink, got {skia_reg}");
        assert!(
            skia_bold > skia_reg && soft_bold > soft_reg,
            "font weight 700 must have strictly more ink than 400 (skia bold={skia_bold}, reg={skia_reg}; soft bold={soft_bold}, reg={soft_reg})"
        );
    }

    let calls_before = fx.calls.load(Ordering::Relaxed);
    cpu.reset_cache_stats();
    cpu.render_cached_with_plugins(
        &fx.dl,
        1.0,
        ColorRgba::BLACK,
        Some((&fx.store, &fx.registry, &mut fx.events)),
    )
    .expect("warm cpu render");
    let stats = cpu.cache_stats();
    assert_eq!(stats.text_blob_misses, 0);
    assert!(stats.text_blob_hits >= 3);
    assert_eq!(stats.image_misses, 0);
    assert!(stats.image_hits >= 3);
    assert_eq!(stats.plugin_misses, 0);
    assert_eq!(stats.plugin_rasterizations, 0);
    assert_eq!(stats.plugin_hits, 1);
    assert_eq!(fx.calls.load(Ordering::Relaxed), calls_before);
}

#[cfg(all(feature = "skia-metal", target_os = "macos"))]
#[test]
fn metal_vs_cpu_skia_whole_frame_parity_gpu_cache_and_context_switch_abandon() {
    let _img_guard = lock_shared_image_cache();
    let cfg = GpuSurfaceConfig {
        width: 360,
        height: 250,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Metal,
        allow_cpu_fallback: false,
        ..Default::default()
    };
    let metal_sw = GpuSwapchain::new_offscreen(cfg).expect("macOS Metal swapchain");
    let (metal_sw, _buf, mut fx) = verify_hardware_gpu_parity_and_warm_cache(
        "Metal",
        GpuBackendKind::Metal,
        "nve-test://4b-metal-parity-img",
        metal_sw,
    );
    drop(metal_sw);

    // Drive ONE shared `SkiaCanvasRenderer` across two real Metal `DirectContext`s + abandoned
    // `DirectContext` to lock `sync_gpu_context` eviction guarantees.
    let mut sw_a = crate::MetalSwapchain::new_offscreen(cfg).expect("MetalSwapchain A");
    let mut sw_b = crate::MetalSwapchain::new_offscreen(cfg).expect("MetalSwapchain B");
    let shared_renderer = SkiaCanvasRenderer::new();

    {
        let (surf_a, dc_a) = sw_a.inner.offscreen_surface_and_context_mut().expect("offscreen A");
        shared_renderer.paint_display_list_with_gpu_context(
            &fx.dl,
            surf_a.canvas(),
            1.0,
            Some((&fx.store, &fx.registry, &mut fx.events)),
            Some(dc_a),
        );
        let _ = dc_a.flush_submit_and_sync_cpu();
    }
    let st_a = shared_renderer.cache_stats();
    assert_eq!((st_a.image_entries, st_a.plugin_entries, st_a.image_uploads, st_a.plugin_uploads), (1, 1, 1, 1));

    {
        let (surf_b, dc_b) = sw_b.inner.offscreen_surface_and_context_mut().expect("offscreen B");
        shared_renderer.paint_display_list_with_gpu_context(
            &fx.dl,
            surf_b.canvas(),
            1.0,
            Some((&fx.store, &fx.registry, &mut fx.events)),
            Some(dc_b),
        );
        let _ = dc_b.flush_submit_and_sync_cpu();
    }
    let st_b = shared_renderer.cache_stats();
    assert_eq!((st_b.image_entries, st_b.plugin_entries, st_b.image_uploads, st_b.plugin_uploads), (1, 1, 2, 2));

    {
        let (surf_b, dc_b) = sw_b.inner.offscreen_surface_and_context_mut().expect("offscreen B");
        dc_b.abandon();
        shared_renderer.paint_display_list_with_gpu_context(
            &DisplayList::default(),
            surf_b.canvas(),
            1.0,
            None,
            Some(dc_b),
        );
    }
    let st_abandoned = shared_renderer.cache_stats();
    assert_eq!(st_abandoned.image_entries, 0, "abandoned context must evict GPU images");
    assert_eq!(st_abandoned.plugin_entries, 0, "abandoned context must evict GPU plugins");
    drop(shared_renderer);
    drop(sw_b);
    drop(sw_a);
}

#[cfg(all(feature = "skia-metal", target_os = "macos"))]
#[test]
fn sync_gpu_context_none_evicts_nonzero_tag_non_gpu_entries_and_retains_cpu_entries() {
    let cfg = GpuSurfaceConfig {
        width: 64,
        height: 64,
        preferred_backend: GpuBackendKind::Metal,
        allow_cpu_fallback: false,
        ..Default::default()
    };
    let mut sw = crate::MetalSwapchain::new_offscreen(cfg).expect("MetalSwapchain");
    let mut caches = crate::gpu::renderer_cache::SkiaRendererCaches::new();
    let local_imgs = crate::image_cache::ImageCache::new(4, 1024);
    local_imgs.insert_raw_rgba("k", 2, 2, vec![200u8; 16]).unwrap();
    let (slot, decoded) = local_imgs.get_or_decode_with_slot("k").unwrap();
    let slot = slot.expect("cached slot");
    let (store, empty_reg, mut ev) = (NodeStore::new(), SkiaTableRegistry::new(), EventBatchBuf::new());

    // Seed CPU (`context_tag == 0`) raster image + negative plugin (`painted == 0`) entries.
    assert!(caches.get_or_upload_image_for_slot(Some(slot), &decoded, 0, None).is_some());
    assert!(caches
        .get_or_render_plugin(NodeId::ROOT, 1, 8, 8, 1.0, Rgba::WHITE, &store, &empty_reg, &mut ev, 0, None)
        .is_none());

    // Activate Metal GPU context -> non-zero `context_tag`, then insert non-`gpu_backed` entries
    // under that tag (negative `painted == 0` plugin + failed-upload raster fallback).
    let gpu_tag = {
        let (_surf, dc) = sw.inner.offscreen_surface_and_context_mut().expect("offscreen");
        let tag = caches.sync_gpu_context(Some(dc));
        assert_ne!(tag, 0);
        assert!(caches
            .get_or_render_plugin(NodeId::ROOT, 1, 8, 8, 1.0, Rgba::WHITE, &store, &empty_reg, &mut ev, tag, Some(dc))
            .is_none());
        tag
    };
    assert!(caches.get_or_upload_image_for_slot(Some(slot), &decoded, gpu_tag, None).is_some());
    assert_eq!((caches.stats().image_entries, caches.stats().plugin_entries), (2, 2));

    // GPU -> CPU switch (`sync_gpu_context(None)`) evicts all `context_tag != 0` entries even
    // when `gpu_backed == false`, while preserving `context_tag == 0` CPU entries.
    assert_eq!(caches.sync_gpu_context(None), 0);
    assert_eq!((caches.stats().image_entries, caches.stats().plugin_entries), (1, 1));
    assert!(caches.get_cached_image_by_slot(slot, decoded.id(), 0).is_some());
    assert!(caches.get_cached_image_by_slot(slot, decoded.id(), gpu_tag).is_none());
}

#[cfg(feature = "skia-vulkan")]
#[test]
fn vulkan_vs_cpu_skia_whole_frame_parity_gpu_cache_and_clean_drop() {
    let _img_guard = lock_shared_image_cache();
    let cfg = GpuSurfaceConfig {
        width: 360,
        height: 250,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Vulkan,
        allow_cpu_fallback: false,
        ..Default::default()
    };
    let vk_sw = match GpuSwapchain::new_offscreen(cfg) {
        Ok(sw) => sw,
        Err(err) => {
            if std::env::var("NVE_REQUIRE_VULKAN_PARITY").as_deref() == Ok("1") {
                panic!("NVE_REQUIRE_VULKAN_PARITY=1 set but Vulkan init failed: {err}");
            }
            return;
        }
    };
    let (vk_sw, _buf, mut fx) = verify_hardware_gpu_parity_and_warm_cache(
        "Vulkan",
        GpuBackendKind::Vulkan,
        "nve-test://4b-vulkan-parity-img",
        vk_sw,
    );
    // Explicit drop + second swapchain locks Vulkan `DirectContext`-before-`VkDevice` teardown.
    drop(vk_sw);
    let mut vk_sw2 = GpuSwapchain::new_offscreen(cfg).expect("second Vulkan swapchain");
    assert_eq!(vk_sw2.kind(), GpuBackendKind::Vulkan);
    vk_sw2
        .execute_frame_with_plugins(
            &fx.dl,
            1.0,
            ColorRgba::BLACK,
            None,
            Some((&fx.store, &fx.registry, &mut fx.events)),
        )
        .unwrap();
    drop(vk_sw2);
}

#[cfg(all(feature = "skia-d3d", target_os = "windows"))]
#[test]
fn d3d12_vs_cpu_skia_whole_frame_parity_gpu_cache_and_clean_drop() {
    let _img_guard = lock_shared_image_cache();
    let cfg = GpuSurfaceConfig {
        width: 360,
        height: 250,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Direct3D12,
        allow_cpu_fallback: false,
        ..Default::default()
    };
    let d3d_sw = GpuSwapchain::new_offscreen(cfg).expect("Windows Direct3D 12 swapchain");
    let (d3d_sw, _buf, mut fx) = verify_hardware_gpu_parity_and_warm_cache(
        "D3D12",
        GpuBackendKind::Direct3D12,
        "nve-test://4b-d3d12-parity-img",
        d3d_sw,
    );
    drop(d3d_sw);
    let mut d3d_sw2 = GpuSwapchain::new_offscreen(cfg).expect("second D3D12 swapchain");
    assert_eq!(d3d_sw2.kind(), GpuBackendKind::Direct3D12);
    d3d_sw2
        .execute_frame_with_plugins(
            &fx.dl,
            1.0,
            ColorRgba::BLACK,
            None,
            Some((&fx.store, &fx.registry, &mut fx.events)),
        )
        .unwrap();
    drop(d3d_sw2);
}
