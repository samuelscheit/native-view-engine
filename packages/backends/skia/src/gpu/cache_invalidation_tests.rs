//! Regression guards for Skia renderer typeface accounting, fontless bitmap fallback, and
//! image cache slot-keyed invalidation. Plugin invalidation and non-finite op guards live in
//! `cache_plugin_invalidation_tests.rs`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;
use nve_tables::tables::ImageFit;

use super::cache_parity_tests::{
    compare_buffers, count_lit_ink, lock_shared_image_cache, register_checker_image,
};
use super::super::*;
use crate::image_cache::ImageCache;
use crate::text::{CosmicTextEngine, TextAlign};
use crate::{DisplayList, PaintOp};

#[test]
fn base_typeface_cache_loads_face_once_with_exact_bytes_across_three_weights() {
    let engine = CosmicTextEngine::shared();
    if !engine.has_system_fonts() {
        assert!(
            std::env::var_os("NVE_ALLOW_NO_SYSTEM_FONTS").is_some(),
            "system fonts unavailable (set NVE_ALLOW_NO_SYSTEM_FONTS=1 only in fontless containers)"
        );
        return;
    }

    let mut runs = crate::text::SkiaShapedRuns::default();
    assert!(engine.shape_for_skia_runs(
        "Typeface bytes probe",
        16.0,
        400,
        TextAlign::Left,
        None,
        200.0,
        &mut runs,
    ));
    let face_id = runs.glyphs.first().expect("shaped glyph").font_id;
    let exact_raw_len = engine
        .with_face_bytes(face_id, |bytes, _| bytes.len())
        .expect("font face bytes")
        .max(1);

    let renderer = SkiaCanvasRenderer::new();
    for weight in [300u16, 400u16, 700u16] {
        assert!(renderer.resolve_typeface_for_test(face_id, weight).is_some());
    }

    let stats = renderer.cache_stats();
    assert_eq!(stats.typeface_entries, 1);
    assert_eq!(stats.typeface_mmap_entries, 1, "file-backed system font must use zero-copy mmap");
    assert_eq!(stats.typeface_copied_entries, 0);
    assert_eq!(stats.typeface_bytes, exact_raw_len);
}

#[test]
fn failed_typeface_load_caches_failure_once_and_falls_back_to_bitmap_text() {
    let engine = Arc::new(CosmicTextEngine::new());
    if !engine.has_system_fonts() {
        assert!(
            std::env::var_os("NVE_ALLOW_NO_SYSTEM_FONTS").is_some(),
            "system fonts unavailable (set NVE_ALLOW_NO_SYSTEM_FONTS=1 only in fontless containers)"
        );
        return;
    }

    let mut surface = CpuSkiaSurface::with_text_engine(220, 60, 1.0, Arc::clone(&engine)).unwrap();
    // Verify Skia's FontMgr itself rejects corrupt font bytes cleanly.
    assert!(
        surface
            .renderer()
            .font_mgr()
            .new_from_bytes(b"NOT_A_VALID_SFNT_FONT_FILE", Some(0))
            .is_none()
    );

    // Inject typeface-load failure while HarfBuzz shaping succeeds: draw 100 DISTINCT strings
    // sharing the failed face (every string misses `text_blobs`, hitting `TypefaceBacking::Failed`)
    // and assert the face load is attempted ONCE across all 100 strings while bitmap fallback paints.
    engine.fail_face_load.store(true, Ordering::Relaxed);
    for idx in 0..100u32 {
        let dl = DisplayList {
            ops: vec![PaintOp::DrawText {
                rect: LayoutRect {
                    x: 8.0,
                    y: 8.0,
                    width: 200.0,
                    height: 40.0,
                },
                text: format!("Fallback Visible #{idx}"),
                font_size: 16.0,
                font_weight: 600,
                text_align: TextAlign::Left,
                color: ColorRgba::rgb(248, 250, 252),
            }],
        };
        surface
            .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None)
            .expect("failed font frame must not panic");
    }
    assert_eq!(
        engine.face_load_calls.load(Ordering::Relaxed),
        1,
        "TypefaceBacking::Failed must prevent retries across 100 distinct strings"
    );
    assert_eq!(surface.cache_stats().text_blob_misses, 100);
    let mut out = PixelBuffer::new(220, 60);
    surface.copy_to_pixel_buffer(&mut out).unwrap();
    assert!(
        count_lit_ink(&out, 8, 8, 208, 48) > 5_000,
        "text must remain visible via bitmap fallback when typeface decode fails"
    );

    // Clearing caches after recovery restores cached HarfBuzz TextBlob rendering cleanly.
    engine.fail_face_load.store(false, Ordering::Relaxed);
    surface.renderer().clear_caches();
    let recovered = surface
        .renderer()
        .build_cached_text_blob("Hello", 16.0, 400, TextAlign::Left, None, 120.0);
    assert!(recovered.is_some());
    assert_eq!(surface.cache_stats().text_blob_entries, 1);
}

#[test]
fn fontless_cpu_skia_surface_renders_visible_bitmap_fallback_text_and_hud() {
    let engine = Arc::new(CosmicTextEngine::new_without_system_fonts());
    assert!(!engine.has_system_fonts());

    let dl = DisplayList {
        ops: vec![PaintOp::DrawText {
            rect: LayoutRect { x: 16.0, y: 52.0, width: 280.0, height: 40.0 },
            text: "BITMAP FALLBACK VISIBLE".to_owned(),
            font_size: 18.0,
            font_weight: 700,
            text_align: TextAlign::Left,
            color: ColorRgba::rgb(248, 250, 252),
        }],
    };

    let mut surface = CpuSkiaSurface::with_text_engine(320, 120, 1.0, engine).unwrap();
    surface
        .execute_frame(&dl, 1.0, ColorRgba::BLACK, Some("HUD 60fps"))
        .expect("fontless frame");
    let mut out = PixelBuffer::new(320, 120);
    surface.copy_to_pixel_buffer(&mut out).unwrap();

    assert!(count_lit_ink(&out, 16, 52, 296, 92) > 5_000);
    assert!(count_lit_ink(&out, 16, 12, 160, 36) > 5_000);
}

#[test]
fn skia_image_cache_never_pins_evicted_decoded_images_and_invalidates_on_same_uri_swap() {
    let _img_guard = lock_shared_image_cache();
    let img_key = "nve-test://4b-zero-pin-swap";
    register_checker_image(img_key, [240, 20, 20, 255], [240, 20, 20, 255]);
    let old_arc = ImageCache::shared().get_or_decode(img_key).expect("old decoded");
    let old_weak = Arc::downgrade(&old_arc);
    let old_id = old_arc.id();
    assert_ne!(old_arc.as_ref().clone().id(), old_id, "DecodedImage::clone must mint a fresh ID");

    let dl = DisplayList {
        ops: vec![PaintOp::DrawImage {
            rect: LayoutRect { x: 8.0, y: 8.0, width: 32.0, height: 32.0 },
            radius: 0.0,
            src: img_key.to_owned(),
            fit: ImageFit::Fill,
            resolved: ImageCache::shared().resolve_cached_handle(img_key),
        }],
    };

    let mut warm_surface = CpuSkiaSurface::new(64, 64).unwrap();
    warm_surface
        .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None)
        .unwrap();
    let initial_bytes = warm_surface.cache_stats().image_bytes;
    assert_eq!(warm_surface.cache_stats().image_entries, 1);

    register_checker_image(img_key, [20, 220, 255, 255], [20, 220, 255, 255]);
    assert_eq!(Arc::strong_count(&old_arc), 1, "SkiaImageCache must not pin Arc<DecodedImage>");
    drop(old_arc);
    assert!(old_weak.upgrade().is_none());

    let new_arc = ImageCache::shared().get_or_decode(img_key).expect("new decoded");
    assert_ne!(new_arc.id(), old_id);
    drop(new_arc);

    warm_surface.reset_cache_stats();
    warm_surface
        .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None)
        .unwrap();
    assert_eq!(warm_surface.cache_stats().image_misses, 1);
    assert_eq!(warm_surface.cache_stats().image_entries, 1);
    assert_eq!(warm_surface.cache_stats().image_bytes, initial_bytes);

    // Swap the same URI 100x: slot-keyed SkiaImageCache must overwrite in-place with constant bytes.
    for step in 0..100u8 {
        register_checker_image(img_key, [step, 180, 255 - step, 255], [20, 220, 255, 255]);
        warm_surface
            .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None)
            .unwrap();
        let st = warm_surface.cache_stats();
        assert_eq!(st.image_entries, 1, "URI swap #{step} must overwrite in-place");
        assert_eq!(st.image_bytes, initial_bytes, "URI swap #{step} must keep constant bytes");
    }

    let mut warm_buf = PixelBuffer::new(64, 64);
    warm_surface.copy_to_pixel_buffer(&mut warm_buf).unwrap();

    let mut cold_surface = CpuSkiaSurface::new(64, 64).unwrap();
    cold_surface
        .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None)
        .unwrap();
    let mut cold_buf = PixelBuffer::new(64, 64);
    cold_surface.copy_to_pixel_buffer(&mut cold_buf).unwrap();
    assert!(compare_buffers(&warm_buf, &cold_buf).exact_match);
}

#[test]
fn hud_banner_bypasses_text_blob_cache_and_paints_visible_ink() {
    let engine = CosmicTextEngine::shared();
    if !engine.has_system_fonts() {
        assert!(
            std::env::var_os("NVE_ALLOW_NO_SYSTEM_FONTS").is_some(),
            "system fonts unavailable (set NVE_ALLOW_NO_SYSTEM_FONTS=1 only in fontless containers)"
        );
        return;
    }

    let mut surface = CpuSkiaSurface::new(320, 80).unwrap();
    let empty = DisplayList::default();
    for frame in 0..20u32 {
        let banner = format!("FPS {:>3}.0 | Frame 0.{frame:02}ms", 60 + frame);
        surface
            .execute_frame(&empty, 1.0, ColorRgba::BLACK, Some(&banner))
            .expect("hud frame");
    }

    let st = surface.cache_stats();
    assert_eq!(st.text_blob_entries, 0, "HUD banner must bypass text_blobs cache");
    assert_eq!(st.text_blob_misses, 0, "HUD banner must not pollute text_blob_misses");
    assert_eq!(st.text_blob_hits, 0, "HUD banner must not touch text_blob_hits");

    let mut out = PixelBuffer::new(320, 80);
    surface.copy_to_pixel_buffer(&mut out).unwrap();
    assert!(
        count_lit_ink(&out, 10, 10, 260, 44) > 5_000,
        "transient HUD TextBlob must still paint visible text"
    );
}

#[test]
fn small_typeface_byte_cap_still_caches_failed_typeface_once() {
    let engine = Arc::new(CosmicTextEngine::new());
    if !engine.has_system_fonts() {
        assert!(
            std::env::var_os("NVE_ALLOW_NO_SYSTEM_FONTS").is_some(),
            "system fonts unavailable (set NVE_ALLOW_NO_SYSTEM_FONTS=1 only in fontless containers)"
        );
        return;
    }

    let mut surface = CpuSkiaSurface::with_text_engine(220, 60, 1.0, Arc::clone(&engine)).unwrap();
    // Cap strictly below `FAILED_TYPEFACE_BYTES` (64): clamped charge must still admit the failure.
    surface.renderer().set_base_typefaces_byte_cap_for_test(16);
    engine.fail_face_load.store(true, Ordering::Relaxed);
    for idx in 0..20u32 {
        let dl = DisplayList {
            ops: vec![PaintOp::DrawText {
                rect: LayoutRect { x: 8.0, y: 8.0, width: 200.0, height: 40.0 },
                text: format!("Tiny Cap Fail #{idx}"),
                font_size: 16.0,
                font_weight: 500,
                text_align: TextAlign::Left,
                color: ColorRgba::WHITE,
            }],
        };
        surface
            .render_cached_with_plugins(&dl, 1.0, ColorRgba::BLACK, None)
            .expect("small-cap failed font frame");
    }
    assert_eq!(
        engine.face_load_calls.load(Ordering::Relaxed),
        1,
        "failed typeface must be cached even when byte cap < FAILED_TYPEFACE_BYTES"
    );
}

#[test]
fn opacity_wrapped_plugin_without_plugins_preserves_siblings_and_skips_plugin_draw() {
    use std::sync::atomic::AtomicU32;
    use nve_abi::SurfaceId;
    use nve_engine::{StoreBackend, Viewport};
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
    use nve_tables::tables::{FlexDirection, Layout, Visual};

    use super::cache_parity_tests::{CountingPluginSystem, ParityPlugin};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(80.0))
            .height(Length::points(40.0))
            .flex_direction(FlexDirection::Row);
        let p = tx.create();
        tx.edit::<Layout>(p)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(p).opacity(0.5);
        tx.edit::<ParityPlugin>(p).color_rgba(ColorRgba::rgb(250, 204, 21).to_u32());
        let sib = tx.create();
        tx.edit::<Layout>(sib)
            .width(Length::points(40.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(sib)
            .background(Rgba::rgb(40, 180, 220))
            .opacity(0.5);
        tx.set_children(NodeId::ROOT, &[p, sib]);
        tx.commit_into(&mut changes);
    }

    let calls = Arc::new(AtomicU32::new(0));
    let mut backend = crate::SkiaBackend::new();
    backend.register_table_system(CountingPluginSystem { calls: Arc::clone(&calls) });
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport { width: 80.0, height: 40.0, scale_factor: 1.0 },
    );
    backend.commit(&store, &changes).unwrap();

    let mut surface = CpuSkiaSurface::new(80, 40).unwrap();
    surface
        .render_scene_cached_with_plugins(backend.scene(), 1.0, ColorRgba::BLACK, None)
        .unwrap();
    let mut out = PixelBuffer::new(80, 40);
    surface.copy_to_pixel_buffer(&mut out).unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 0, "plugins=None must not invoke plugin");
    assert_eq!(out.get_rgb(20, 20), Some((0, 0, 0)), "plugin region must remain clear");
    let sib_px = out.get_rgb(60, 20).expect("sibling pixel");
    assert!(
        sib_px.1 > 70 && sib_px.2 > 90 && sib_px.2 < 140,
        "opacity-wrapped sibling must still paint at 0.5 alpha, got {sib_px:?}"
    );
}

#[cfg(all(feature = "skia-metal", any(target_os = "macos", target_os = "ios")))]
#[test]
fn gpu_to_cpu_context_switch_evicts_gpu_images_and_plugins() {
    use std::sync::atomic::AtomicU32;
    use nve_abi::EventBatchBuf;
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
    use nve_tables::tables::Layout;
    use objc2::rc::Retained;
    use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice};
    use skia_safe::gpu::{direct_contexts, mtl};

    use super::cache_parity_tests::{CountingPluginSystem, ParityPlugin};
    use crate::gpu::ganesh::create_ganesh_offscreen_surface;
    use crate::gpu::SwapchainExtent;
    use crate::table_system::SkiaTableRegistry;

    let _img_guard = lock_shared_image_cache();
    let img_key = "nve-test://p2-2-gpu-to-cpu-evict";
    register_checker_image(img_key, [220, 60, 40, 255], [40, 180, 240, 255]);

    let Some(device) = MTLCreateSystemDefaultDevice() else {
        return;
    };
    let queue = device.newCommandQueue().expect("Metal command queue");
    // SAFETY: `device` and `queue` outlive `direct_context` and `gpu_surface` in this test.
    let backend = unsafe {
        mtl::BackendContext::new(
            Retained::as_ptr(&device) as mtl::Handle,
            Retained::as_ptr(&queue) as mtl::Handle,
        )
    };
    let mut direct_context =
        direct_contexts::make_metal(&backend, None).expect("Metal DirectContext");
    let extent = SwapchainExtent { width: 64, height: 64, scale_factor: 1.0 };
    let mut gpu_surface =
        create_ganesh_offscreen_surface(&mut direct_context, extent, "Metal").expect("GPU surface");

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let plugin_node = {
        let mut tx = store.tx();
        let p = tx.create();
        tx.edit::<Layout>(p).width(Length::points(32.0)).height(Length::points(32.0));
        tx.edit::<ParityPlugin>(p).color_rgba(ColorRgba::rgb(250, 204, 21).to_u32());
        tx.set_children(NodeId::ROOT, &[p]);
        tx.commit_into(&mut changes);
        p
    };

    let mut registry = SkiaTableRegistry::new();
    registry.register(CountingPluginSystem { calls: Arc::new(AtomicU32::new(0)) });
    let mut events = EventBatchBuf::new();

    let dl = DisplayList {
        ops: vec![
            PaintOp::DrawImage {
                rect: LayoutRect { x: 0.0, y: 0.0, width: 32.0, height: 32.0 },
                radius: 0.0,
                src: img_key.to_owned(),
                fit: ImageFit::Fill,
                resolved: ImageCache::shared().resolve_cached_handle(img_key),
            },
            PaintOp::Plugin {
                node_id: plugin_node,
                rect: LayoutRect { x: 32.0, y: 0.0, width: 32.0, height: 32.0 },
                inherited_text_color: Rgba::WHITE,
                plugin_epoch: 1,
            },
        ],
    };

    let renderer = SkiaCanvasRenderer::new();
    renderer.paint_display_list_with_gpu_context(
        &dl,
        gpu_surface.canvas(),
        1.0,
        Some((&store, &registry, &mut events)),
        Some(&mut direct_context),
    );
    let _ = direct_context.flush_submit_and_sync_cpu();
    let gpu_stats = renderer.cache_stats();
    assert_eq!(gpu_stats.image_uploads, 1, "GPU frame must upload image texture");
    assert_eq!(gpu_stats.plugin_uploads, 1, "GPU frame must upload plugin texture");
    assert_eq!(gpu_stats.image_entries, 1);
    assert_eq!(gpu_stats.plugin_entries, 1);

    // Switching the SAME renderer to a CPU canvas (`direct_context = None`) MUST immediately evict
    // all GPU-backed images and plugins so old Ganesh textures are not kept pinned.
    let mut cpu_surface = skia_safe::surfaces::raster_n32_premul((64, 64)).expect("CPU surface");
    renderer.paint_display_list_opt_plugins(
        &DisplayList::default(),
        cpu_surface.canvas(),
        1.0,
        None,
    );
    let cpu_stats = renderer.cache_stats();
    assert_eq!(cpu_stats.image_entries, 0, "GPU→CPU switch must evict GPU-backed images");
    assert_eq!(cpu_stats.plugin_entries, 0, "GPU→CPU switch must evict GPU-backed plugins");

    // Subsequent CPU draw repopulates raster entries cleanly without incrementing GPU uploads.
    renderer.paint_display_list_opt_plugins(
        &dl,
        cpu_surface.canvas(),
        1.0,
        Some((&store, &registry, &mut events)),
    );
    let post_cpu = renderer.cache_stats();
    assert_eq!(post_cpu.image_entries, 1);
    assert_eq!(post_cpu.plugin_entries, 1);
    assert_eq!(post_cpu.image_uploads, 1, "CPU frame must not perform GPU uploads");
    assert_eq!(post_cpu.plugin_uploads, 1, "CPU frame must not perform GPU uploads");

    drop(gpu_surface);
    direct_context.release_resources_and_abandon();
    drop(direct_context);
}
