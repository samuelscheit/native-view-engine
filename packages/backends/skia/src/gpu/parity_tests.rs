use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use super::super::*;
use super::sample_parity_display_list;
use crate::{DisplayList, PaintOp};

fn assert_parity_pixels(label: &str, buf: &PixelBuffer, scale: f32) {
    let pt = |x: f32, y: f32| -> (u32, u32) { ((x * scale) as u32, (y * scale) as u32) };

    // Rounded blue card interior.
    let (cx, cy) = pt(70.0, 60.0);
    assert_eq!(
        buf.get_rgb(cx, cy),
        Some((37, 99, 235)),
        "{label}: rounded rect center mismatch at ({cx}, {cy})"
    );
    // Clipped outer corner of radius-20 rounded rect must remain black.
    let (kx, ky) = pt(20.0, 20.0);
    assert_eq!(
        buf.get_rgb(kx, ky),
        Some((0, 0, 0)),
        "{label}: rounded rect corner not clipped at ({kx}, {ky})"
    );
    // Top stroke border pixel is bright white; interior remains hollow black.
    let (sx, sy) = pt(180.0, 21.5);
    let (sr, sg, sb) = buf.get_rgb(sx, sy).unwrap();
    assert!(
        sr >= 240 && sg >= 240 && sb >= 240,
        "{label}: stroke pixel expected white at ({sx}, {sy}), got ({sr},{sg},{sb})"
    );
    let (hx, hy) = pt(180.0, 50.0);
    assert_eq!(
        buf.get_rgb(hx, hy),
        Some((0, 0, 0)),
        "{label}: stroke interior not hollow at ({hx}, {hy})"
    );
    // 50% opacity red layer over black -> ~(128, 0, 0).
    let (ox, oy) = pt(45.0, 145.0);
    let (r, g, b) = buf.get_rgb(ox, oy).unwrap();
    assert!(
        (124..=132).contains(&r) && g <= 2 && b <= 2,
        "{label}: 50% opacity red mismatch at ({ox}, {oy}), got ({r},{g},{b})"
    );
    // Restored full-opacity green square.
    let (gx, gy) = pt(115.0, 145.0);
    assert_eq!(
        buf.get_rgb(gx, gy),
        Some((0, 255, 0)),
        "{label}: restored green mismatch at ({gx}, {gy})"
    );
    // Clipped text box contains non-zero cyan glyph coverage inside bounds and zero leakage outside.
    let x0 = (150.0 * scale) as u32;
    let y0 = (110.0 * scale) as u32;
    let x1 = (270.0 * scale) as u32;
    let y1 = (150.0 * scale) as u32;
    let mut lit_text_pixels = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some((_, g, b)) = buf.get_rgb(x, y)
                && g > 40
                && b > 60
            {
                lit_text_pixels += 1;
            }
        }
    }
    assert!(
        lit_text_pixels >= 25,
        "{label}: expected rendered cyan text glyphs, found only {lit_text_pixels} pixels"
    );
}

#[test]
fn cpu_skia_and_software_rasterizer_have_rendering_parity_at_1x_and_2x_hidpi() {
    let list = sample_parity_display_list();
    let renderer = SkiaCanvasRenderer::new();

    for scale in [1.0f32, 2.0f32] {
        let w = (280.0 * scale) as u32;
        let h = (190.0 * scale) as u32;

        // 1. Software rasterizer
        let mut sw = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
            width: w,
            height: h,
            scale_factor: scale,
            preferred_backend: GpuBackendKind::Softbuffer,
            allow_cpu_fallback: true,
            ..Default::default()
        })
        .unwrap();
        sw.execute_frame(&list, scale, ColorRgba::BLACK, None)
            .unwrap();
        let mut soft_buf = PixelBuffer::new(w, h);
        sw.copy_to_pixel_buffer(&mut soft_buf).unwrap();
        assert_parity_pixels(&format!("softbuffer@{scale}x"), &soft_buf, scale);

        // 2. CPU Skia via SkiaCanvasRenderer
        let mut cpu_skia = CpuSkiaSurface::new_with_scale(w, h, scale).unwrap();
        GpuSurfaceBackend::render_display_list(
            &mut cpu_skia,
            &renderer,
            &list,
            scale,
            ColorRgba::BLACK,
        )
        .unwrap();
        let mut skia_buf = PixelBuffer::new(w, h);
        cpu_skia.copy_to_pixel_buffer(&mut skia_buf).unwrap();
        assert_parity_pixels(&format!("cpu-skia@{scale}x"), &skia_buf, scale);

        // 3. CpuSkia via GpuFramePresenter lifecycle
        let tel = cpu_skia
            .execute_frame(&list, scale, ColorRgba::BLACK, Some("HUD"))
            .unwrap();
        assert_eq!(tel.backend, GpuBackendKind::CpuRaster);
        assert_eq!(tel.width, w);
        assert_eq!(tel.height, h);
    }
}

#[cfg(all(feature = "skia-metal", target_os = "macos"))]
#[test]
fn real_hardware_metal_swapchain_offscreen_renders_with_cpu_parity_and_hidpi_resize() {
    let list = sample_parity_display_list();
    let mut metal_sw = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
        width: 280,
        height: 190,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Metal,
        allow_cpu_fallback: false,
        ..Default::default()
    })
    .expect("macOS hardware Metal swapchain should initialize");

    assert_eq!(metal_sw.kind(), GpuBackendKind::Metal);
    assert!(!metal_sw.presents_directly()); // offscreen VRAM surface
    assert_eq!(metal_sw.fallback_reason(), None);

    // Frame 1 at 1x
    let t1 = metal_sw
        .execute_frame(&list, 1.0, ColorRgba::BLACK, None)
        .expect("Metal frame 1 should render & submit");
    assert_eq!(t1.backend, GpuBackendKind::Metal);
    assert_eq!(t1.frame_index, 1);
    let mut metal_buf_1x = PixelBuffer::new(280, 190);
    metal_sw.copy_to_pixel_buffer(&mut metal_buf_1x).unwrap();
    assert_parity_pixels("metal-gpu@1x", &metal_buf_1x, 1.0);

    // Resize to 2x Retina + odd pixel test -> back to 2x HiDPI
    assert_eq!(
        metal_sw.resize(0, 0, 2.0),
        Err(GpuSurfaceError::InvalidDimensions {
            width: 0,
            height: 0
        })
    );
    metal_sw.resize(560, 380, 2.0).unwrap();
    assert_eq!(metal_sw.swapchain_recreations(), 1);
    let t2 = metal_sw
        .execute_frame(&list, 2.0, ColorRgba::BLACK, None)
        .expect("Metal frame 2 at 2x HiDPI should render & submit");
    assert_eq!(t2.frame_index, 2);
    assert_eq!(t2.width, 560);
    assert_eq!(t2.height, 380);
    assert_eq!(t2.swapchain_recreations, 1);

    let mut metal_buf_2x = PixelBuffer::new(560, 380);
    metal_sw.copy_to_pixel_buffer(&mut metal_buf_2x).unwrap();
    assert_parity_pixels("metal-gpu@2x", &metal_buf_2x, 2.0);

    // Frame 3: verify GPU-resident HUD banner renders cleanly over the 2x Metal surface.
    let t3 = metal_sw
        .execute_frame(&list, 2.0, ColorRgba::BLACK, Some("Metal 2x"))
        .expect("Metal frame 3 with HUD should render & submit");
    assert_eq!(t3.frame_index, 3);
    metal_sw.copy_to_pixel_buffer(&mut metal_buf_2x).unwrap();
    let (hr, hg, hb) = metal_buf_2x.get_rgb(40, 40).unwrap();
    assert!(
        (10..=18).contains(&hr) && (16..=25).contains(&hg) && (30..=42).contains(&hb),
        "expected translucent slate HUD background at (40,40), got ({hr},{hg},{hb})"
    );

    // Also verify GpuSurface::new picks real Metal when requested.
    let mut gpu_surface = GpuSurface::new(GpuSurfaceConfig {
        width: 280,
        height: 190,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Metal,
        allow_cpu_fallback: false,
        ..Default::default()
    })
    .expect("GpuSurface::new(Metal) should initialize real Metal render target");
    assert_eq!(gpu_surface.kind(), GpuBackendKind::Metal);
    let renderer = SkiaCanvasRenderer::new();
    gpu_surface
        .render_display_list(&renderer, &list, 1.0, ColorRgba::BLACK)
        .unwrap();
    let mut surf_buf = PixelBuffer::new(280, 190);
    gpu_surface.copy_to_pixel_buffer(&mut surf_buf).unwrap();
    assert_parity_pixels("gpu-surface-metal@1x", &surf_buf, 1.0);
}

fn sample_image_canvas_clip_display_list() -> DisplayList {
    use nve_tables::tables::ImageFit;

    const AVATAR_MAYA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";

    DisplayList {
        ops: vec![
            // 1. Clipped container (10, 10, 80x60) containing an overflowing purple rect (10, 10, 80x140).
            PaintOp::PushClip {
                rect: LayoutRect {
                    x: 10.0,
                    y: 10.0,
                    width: 80.0,
                    height: 60.0,
                },
                radius: 0.0,
            },
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 10.0,
                    y: 10.0,
                    width: 80.0,
                    height: 140.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(168, 85, 247),
            },
            PaintOp::PopClip,
            // 2. Real Studio Workspace PNG avatar at (10, 90, 64x64) with Cover + rounded clip.
            PaintOp::DrawImage {
                rect: LayoutRect {
                    x: 10.0,
                    y: 90.0,
                    width: 64.0,
                    height: 64.0,
                },
                radius: 12.0,
                src: AVATAR_MAYA.to_string(),
                fit: ImageFit::Cover,
            },
            // 3. Secondary clipped card at (110, 10, 160x130) with yellow accent inside clip.
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 110.0,
                    y: 10.0,
                    width: 160.0,
                    height: 60.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(17, 24, 39),
            },
            PaintOp::PushClip {
                rect: LayoutRect {
                    x: 120.0,
                    y: 110.0,
                    width: 25.0,
                    height: 20.0,
                },
                radius: 0.0,
            },
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 120.0,
                    y: 110.0,
                    width: 80.0,
                    height: 20.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(250, 204, 21),
            },
            PaintOp::PopClip,
        ],
    }
}

fn assert_image_canvas_clip_parity(label: &str, buf: &PixelBuffer) {
    // 1. PushClip / PopClip: purple inside (50, 40), clipped to black below at (50, 80).
    assert_eq!(
        buf.get_rgb(50, 40),
        Some((168, 85, 247)),
        "{label}: clipped purple interior mismatch"
    );
    assert_eq!(
        buf.get_rgb(50, 80),
        Some((0, 0, 0)),
        "{label}: PushClip leaked outside bottom edge"
    );

    // 2. DrawImage: real PNG avatar produced non-zero pixels inside (10..74, 90..154),
    // and rounded corner at (10, 90) stays clipped to black.
    let mut avatar_lit = 0usize;
    for y in 90..154 {
        for x in 10..74 {
            if buf.get_rgb(x, y) != Some((0, 0, 0)) {
                avatar_lit += 1;
            }
        }
    }
    assert!(
        avatar_lit > 1000,
        "{label}: expected rendered PNG avatar pixels, got only {avatar_lit}"
    );
    assert_eq!(
        buf.get_rgb(10, 90),
        Some((0, 0, 0)),
        "{label}: DrawImage rounded corner not clipped"
    );

    // 3. Secondary card background + clipped yellow interior.
    assert_eq!(
        buf.get_rgb(120, 20),
        Some((17, 24, 39)),
        "{label}: card bg mismatch"
    );
    assert_eq!(
        buf.get_rgb(130, 120),
        Some((250, 204, 21)),
        "{label}: clipped yellow interior mismatch"
    );
    assert_eq!(
        buf.get_rgb(160, 120),
        Some((0, 0, 0)),
        "{label}: PushClip leaked past right boundary"
    );
}

#[test]
fn image_canvas_and_clip_primitives_have_software_and_cpu_skia_parity() {
    let list = sample_image_canvas_clip_display_list();
    let renderer = SkiaCanvasRenderer::new();

    // Software rasterizer
    let mut soft = PixelBuffer::new(280, 170);
    crate::raster::rasterize_display_list(&list, &mut soft, 1.0, ColorRgba::BLACK);
    assert_image_canvas_clip_parity("software", &soft);

    // CPU Skia
    let mut cpu_skia = CpuSkiaSurface::new(280, 170).unwrap();
    GpuSurfaceBackend::render_display_list(&mut cpu_skia, &renderer, &list, 1.0, ColorRgba::BLACK)
        .unwrap();
    let mut skia_buf = PixelBuffer::new(280, 170);
    cpu_skia.copy_to_pixel_buffer(&mut skia_buf).unwrap();
    assert_image_canvas_clip_parity("cpu-skia", &skia_buf);
}

#[cfg(all(feature = "skia-metal", target_os = "macos"))]
#[test]
fn hardware_metal_gpu_renders_real_draw_image_nve_draw1_and_clips_with_software_parity() {
    let list = sample_image_canvas_clip_display_list();

    // Software reference
    let mut soft = PixelBuffer::new(280, 170);
    crate::raster::rasterize_display_list(&list, &mut soft, 1.0, ColorRgba::BLACK);
    assert_image_canvas_clip_parity("software-ref", &soft);

    // Real hardware Metal GPU swapchain
    let mut metal_sw = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
        width: 280,
        height: 170,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Metal,
        allow_cpu_fallback: false,
        ..Default::default()
    })
    .expect("macOS hardware Metal swapchain should initialize");
    assert_eq!(metal_sw.kind(), GpuBackendKind::Metal);
    metal_sw
        .execute_frame(&list, 1.0, ColorRgba::BLACK, None)
        .expect("Metal frame should render");
    let mut metal_buf = PixelBuffer::new(280, 170);
    metal_sw.copy_to_pixel_buffer(&mut metal_buf).unwrap();
    assert_image_canvas_clip_parity("metal-gpu", &metal_buf);
}
