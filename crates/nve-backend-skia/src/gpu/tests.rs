use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use super::d3d12::D3D12SwapchainConfig;
use super::metal::MetalSwapchainConfig;
use super::vulkan::{VulkanSemaphoreSyncHooks, VulkanSwapchainConfig};
use super::*;
use crate::{DisplayList, PaintOp};

fn sample_parity_display_list() -> DisplayList {
    DisplayList {
        ops: vec![
            // Rounded blue card at (20, 20, 100x80) with radius 20.
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 20.0,
                    y: 20.0,
                    width: 100.0,
                    height: 80.0,
                },
                radius: 20.0,
                color: ColorRgba::rgb(37, 99, 235),
            },
            // Crisp white 4px stroke box at (140, 20, 80x60) with radius 8.
            PaintOp::StrokeRoundedRect {
                rect: LayoutRect {
                    x: 140.0,
                    y: 20.0,
                    width: 80.0,
                    height: 60.0,
                },
                radius: 8.0,
                width: 4.0,
                color: ColorRgba::WHITE,
            },
            // 50% opacity red square over black background -> ~(128, 0, 0).
            PaintOp::SaveOpacity(0.5),
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 20.0,
                    y: 120.0,
                    width: 50.0,
                    height: 50.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(255, 0, 0),
            },
            PaintOp::Restore,
            // Restored full-opacity green square at (90, 120, 50x50).
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 90.0,
                    y: 120.0,
                    width: 50.0,
                    height: 50.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(0, 255, 0),
            },
            // Clipped text box at (150, 110, 120x40) with cyan text.
            PaintOp::DrawText {
                rect: LayoutRect {
                    x: 150.0,
                    y: 110.0,
                    width: 120.0,
                    height: 40.0,
                },
                text: "NVE GPU".to_string(),
                font_size: 20.0,
                color: ColorRgba::rgb(56, 189, 248),
            },
        ],
    }
}

#[test]
fn platform_gpu_backend_matches_target_os() {
    let preferred = GpuBackendKind::platform_preferred();
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    assert_eq!(preferred, GpuBackendKind::Metal);
    #[cfg(target_os = "windows")]
    assert_eq!(preferred, GpuBackendKind::Direct3D12);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    assert_eq!(preferred, GpuBackendKind::Vulkan);

    let cfg = GpuSurfaceConfig::default();
    assert_eq!(cfg.preferred_backend, preferred);
    assert!(cfg.allow_cpu_fallback);
    assert!(cfg.vsync);
    assert_eq!(cfg.swapchain_image_count, 2);
    assert!(!preferred.as_str().is_empty());
    assert!(GpuBackendKind::Metal.is_hardware());
    assert!(GpuBackendKind::Vulkan.is_hardware());
    assert!(GpuBackendKind::Direct3D12.is_hardware());
    assert!(!GpuBackendKind::CpuRaster.is_hardware());
    assert!(!GpuBackendKind::Softbuffer.is_hardware());
}

#[test]
fn backend_selection_priority_is_deterministic_and_deduplicated() {
    // Disallowing fallback yields solely the requested backend.
    let strict = GpuBackendKind::selection_priority(GpuBackendKind::Direct3D12, false);
    assert_eq!(strict, vec![GpuBackendKind::Direct3D12]);

    // Allowing fallback always ends with Softbuffer (always compiled) and contains no duplicates.
    for requested in [
        GpuBackendKind::Metal,
        GpuBackendKind::Vulkan,
        GpuBackendKind::Direct3D12,
        GpuBackendKind::CpuRaster,
        GpuBackendKind::Softbuffer,
    ] {
        let order = GpuBackendKind::selection_priority(requested, true);
        assert!(!order.is_empty());
        assert_eq!(order.last().copied(), Some(GpuBackendKind::Softbuffer));

        // Verify strict uniqueness.
        let mut seen = std::collections::HashSet::new();
        for kind in &order {
            assert!(seen.insert(*kind), "duplicate backend {kind:?} in {order:?}");
            assert!(kind.is_compiled(), "uncompiled backend {kind:?} in {order:?}");
        }

        // If the requested backend is compiled, it must always be first.
        if requested.is_compiled() {
            assert_eq!(order[0], requested);
        }
    }
}

#[test]
fn swapchain_configs_validate_dimensions_and_clamp_buffer_counts() {
    let bad = GpuSurfaceConfig {
        width: 0,
        height: 480,
        ..Default::default()
    };
    assert_eq!(
        MetalSwapchainConfig::from_surface_config(bad),
        Err(GpuSurfaceError::InvalidDimensions {
            width: 0,
            height: 480
        })
    );
    assert_eq!(
        VulkanSwapchainConfig::from_surface_config(bad),
        Err(GpuSurfaceError::InvalidDimensions {
            width: 0,
            height: 480
        })
    );
    assert_eq!(
        D3D12SwapchainConfig::from_surface_config(bad),
        Err(GpuSurfaceError::InvalidDimensions {
            width: 0,
            height: 480
        })
    );

    let odd_hidpi = GpuSurfaceConfig {
        width: 333,
        height: 197,
        scale_factor: 1.5,
        swapchain_image_count: 1, // clamped up to 2
        vsync: false,
        ..Default::default()
    };
    let mtl = MetalSwapchainConfig::from_surface_config(odd_hidpi).unwrap();
    assert_eq!(mtl.maximum_drawable_count, 2);
    assert_eq!(mtl.extent.width, 333);
    assert_eq!(mtl.extent.height, 197);
    assert!(!mtl.vsync);

    let vk = VulkanSwapchainConfig::from_surface_config(odd_hidpi).unwrap();
    assert_eq!(vk.min_image_count, 2);
    assert_eq!(vk.frames_in_flight, 2);

    let d3d = D3D12SwapchainConfig::from_surface_config(odd_hidpi).unwrap();
    assert_eq!(d3d.buffer_count, 2);

    let hooks = VulkanSemaphoreSyncHooks::default();
    assert_eq!(hooks.image_available_semaphore, 0);
    assert_eq!(hooks.render_finished_semaphore, 0);
}

#[test]
fn hidpi_resize_rejects_zero_dimensions_preserves_odd_pixels_and_counts_recreations() {
    let mut sw = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
        width: 200,
        height: 120,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Softbuffer,
        allow_cpu_fallback: true,
        ..Default::default()
    })
    .unwrap();

    assert_eq!(sw.extent(), SwapchainExtent::new(200, 120, 1.0).unwrap());
    assert_eq!(sw.swapchain_recreations(), 0);

    // Zero dimensions rejected cleanly without mutating state.
    assert_eq!(
        sw.resize(0, 0, 2.0),
        Err(GpuSurfaceError::InvalidDimensions {
            width: 0,
            height: 0
        })
    );
    assert_eq!(
        sw.resize(0, 120, 2.0),
        Err(GpuSurfaceError::InvalidDimensions {
            width: 0,
            height: 120
        })
    );
    assert_eq!(
        sw.resize(200, 0, 2.0),
        Err(GpuSurfaceError::InvalidDimensions {
            width: 200,
            height: 0
        })
    );
    assert_eq!(sw.swapchain_recreations(), 0);

    // No-op resize with identical dimensions and scale does not bump recreation counter.
    sw.resize(200, 120, 1.0).unwrap();
    assert_eq!(sw.swapchain_recreations(), 0);

    // Odd pixel dimensions at fractional HiDPI scale factor 1.5x.
    sw.resize(333, 197, 1.5).unwrap();
    assert_eq!(sw.extent().width, 333);
    assert_eq!(sw.extent().height, 197);
    assert_eq!(sw.extent().scale_factor, 1.5);
    assert_eq!(sw.swapchain_recreations(), 1);
    let (lw, lh) = sw.extent().logical_size();
    assert!((lw - 222.0).abs() < 0.01);
    assert!((lh - 131.3333).abs() < 0.01);

    // Scale-factor-only change triggers swapchain recreation.
    sw.resize(333, 197, 2.0).unwrap();
    assert_eq!(sw.extent().scale_factor, 2.0);
    assert_eq!(sw.swapchain_recreations(), 2);

    // NaN / negative / zero scale factors sanitize safely to 1.0.
    sw.resize(333, 197, f32::NAN).unwrap();
    assert_eq!(sw.extent().scale_factor, 1.0);
    sw.resize(333, 197, -2.0).unwrap();
    assert_eq!(sw.extent().scale_factor, 1.0);
}

#[test]
fn graceful_fallback_on_uncompiled_or_unavailable_backend() {
    // Pick a hardware backend that is definitely not compiled for the current target/features.
    let uncompiled = if !GpuBackendKind::Direct3D12.is_compiled() {
        GpuBackendKind::Direct3D12
    } else if !GpuBackendKind::Metal.is_compiled() {
        GpuBackendKind::Metal
    } else {
        GpuBackendKind::Vulkan
    };

    if !uncompiled.is_compiled() {
        // Strict mode must fail with UnsupportedBackend.
        let strict = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
            width: 128,
            height: 128,
            scale_factor: 1.0,
            preferred_backend: uncompiled,
            allow_cpu_fallback: false,
            ..Default::default()
        });
        assert_eq!(
            strict.err(),
            Some(GpuSurfaceError::UnsupportedBackend(uncompiled))
        );

        // Fallback mode must succeed gracefully.
        let mut fallback = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
            width: 128,
            height: 128,
            scale_factor: 1.0,
            preferred_backend: uncompiled,
            allow_cpu_fallback: true,
            ..Default::default()
        })
        .expect("fallback swapchain must initialize cleanly");
        assert_ne!(fallback.kind(), uncompiled);
        let tel = fallback
            .execute_frame(&DisplayList::default(), 1.0, ColorRgba::BLACK, None)
            .expect("fallback frame should present");
        assert_eq!(tel.backend, fallback.kind());
        assert_eq!(tel.frame_index, 1);
    }
}

#[test]
fn frame_lifecycle_invariants_reject_out_of_order_calls_and_recover_on_abort() {
    let mut sw = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
        width: 280,
        height: 190,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::Softbuffer,
        allow_cpu_fallback: true,
        ..Default::default()
    })
    .unwrap();
    let list = sample_parity_display_list();

    assert_eq!(sw.lifecycle_state(), FrameLifecycleState::Idle);
    // Out-of-order calls from Idle must fail.
    assert!(matches!(
        sw.render_display_list(&list, 1.0, ColorRgba::BLACK, None),
        Err(GpuSurfaceError::InvalidFrameState {
            expected: "begun",
            actual: "idle"
        })
    ));
    assert!(matches!(
        sw.submit_frame(),
        Err(GpuSurfaceError::InvalidFrameState {
            expected: "rendered",
            actual: "idle"
        })
    ));
    assert!(matches!(
        sw.present_frame(),
        Err(GpuSurfaceError::InvalidFrameState {
            expected: "submitted",
            actual: "idle"
        })
    ));

    // Begin frame -> double begin & resize mid-frame must fail.
    sw.begin_frame().unwrap();
    assert_eq!(sw.lifecycle_state(), FrameLifecycleState::Begun);
    assert!(matches!(
        sw.begin_frame(),
        Err(GpuSurfaceError::InvalidFrameState {
            expected: "idle",
            actual: "begun"
        })
    ));
    assert!(matches!(
        sw.resize(300, 200, 1.0),
        Err(GpuSurfaceError::InvalidFrameState {
            expected: "idle",
            actual: "begun"
        })
    ));

    // Abort returns cleanly to Idle.
    sw.abort_frame();
    assert_eq!(sw.lifecycle_state(), FrameLifecycleState::Idle);

    // Full valid lifecycle: begin -> render -> submit -> present.
    sw.begin_frame().unwrap();
    sw.render_display_list(&list, 1.0, ColorRgba::BLACK, Some("NVE 60fps"))
        .unwrap();
    assert_eq!(sw.lifecycle_state(), FrameLifecycleState::Rendered);
    assert!(matches!(
        sw.present_frame(),
        Err(GpuSurfaceError::InvalidFrameState {
            expected: "submitted",
            actual: "rendered"
        })
    ));
    sw.submit_frame().unwrap();
    assert_eq!(sw.lifecycle_state(), FrameLifecycleState::Submitted);
    let t1 = sw.present_frame().unwrap();
    assert_eq!(sw.lifecycle_state(), FrameLifecycleState::Idle);
    assert_eq!(t1.frame_index, 1);
    assert_eq!(t1.width, 280);
    assert_eq!(t1.height, 190);
    assert_eq!(sw.last_telemetry(), Some(&t1));

    // Second frame via execute_frame increments frame_index monotonically.
    let t2 = sw
        .execute_frame(&list, 1.0, ColorRgba::BLACK, None)
        .unwrap();
    assert_eq!(t2.frame_index, 2);

    // Verify rendered pixels via readback.
    let mut buf = PixelBuffer::new(1, 1);
    sw.copy_to_pixel_buffer(&mut buf).unwrap();
    assert_eq!(buf.get_rgb(70, 60), Some((37, 99, 235))); // rounded rect center
    assert_eq!(buf.get_rgb(20, 20), Some((0, 0, 0))); // clipped corner
    assert_eq!(buf.get_rgb(180, 21), Some((255, 255, 255))); // top stroke
    assert_eq!(buf.get_rgb(180, 50), Some((0, 0, 0))); // hollow stroke center
    let (r, g, b) = buf.get_rgb(45, 145).unwrap();
    assert!((125..=131).contains(&r), "expected ~128 red, got {r}");
    assert_eq!((g, b), (0, 0));
    assert_eq!(buf.get_rgb(115, 145), Some((0, 255, 0))); // restored green
}

#[test]
fn gpu_surface_error_displays_clearly() {
    let e1 = GpuSurfaceError::InvalidDimensions {
        width: 0,
        height: 100,
    };
    assert!(e1.to_string().contains("0x100"));
    let e2 = GpuSurfaceError::UnsupportedBackend(GpuBackendKind::Vulkan);
    assert!(e2.to_string().contains("vulkan"));
    let e3 = GpuSurfaceError::InvalidFrameState {
        expected: "begun",
        actual: "idle",
    };
    assert!(e3.to_string().contains("begun") && e3.to_string().contains("idle"));
    assert!(GpuSurfaceError::DrawableUnavailable
        .to_string()
        .contains("drawable"));
}

#[cfg(feature = "skia")]
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
            if let Some((_, g, b)) = buf.get_rgb(x, y) {
                if g > 40 && b > 60 {
                    lit_text_pixels += 1;
                }
            }
        }
    }
    assert!(
        lit_text_pixels >= 25,
        "{label}: expected rendered cyan text glyphs, found only {lit_text_pixels} pixels"
    );
}

#[cfg(feature = "skia")]
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
