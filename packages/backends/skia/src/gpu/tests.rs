use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use super::d3d12::D3D12SwapchainConfig;
use super::metal::MetalSwapchainConfig;
use super::vulkan::VulkanSwapchainConfig;
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
                font_weight: 400,
                text_align: crate::TextAlign::Left,
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

#[test]
fn pixel_buffer_transparent_rgba_preserves_alpha_and_avoids_black_box() {
    let mut buf = PixelBuffer::new_transparent_rgba(8, 8);
    // Paint a single 50% alpha red pixel at (2, 2); untouched pixels must stay alpha = 0.
    buf.blend_pixel(2, 2, ColorRgba::rgb(255, 0, 0), 0.5);
    let mut rgba = Vec::new();
    buf.write_rgba8_into(&mut rgba);
    // Untouched corner (0, 0) is fully transparent (0, 0, 0, 0).
    assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
    // Blended pixel at (2, 2) has straight RGB (255, 0, 0) and ~128 alpha (zero black halo!).
    let off = (2 * 8 + 2) * 4;
    assert_eq!(rgba[off], 255);
    assert_eq!(rgba[off + 1], 0);
    assert_eq!(rgba[off + 2], 0);
    assert!((126..=130).contains(&rgba[off + 3]), "alpha={}", rgba[off + 3]);

    // Resizing from 0x0 must preserve explicit alpha tracking (`has_alpha`).
    let mut from_zero = PixelBuffer::new_transparent_rgba(0, 0);
    from_zero.resize(4, 4);
    from_zero.blend_pixel(1, 1, ColorRgba::rgb(0, 255, 0), 0.5);
    from_zero.write_rgba8_into(&mut rgba);
    assert_eq!(&rgba[0..4], &[0, 0, 0, 0], "untouched corner after 0x0->4x4 resize stays transparent");
    let off2 = (4 + 1) * 4;
    assert_eq!(rgba[off2 + 1], 255);
    assert!((126..=130).contains(&rgba[off2 + 3]));
}

#[cfg(feature = "skia")]
#[test]
fn cpu_skia_surface_plugin_preserves_transparent_background() {
    use nve_abi::EventBatchBuf;
    use nve_store::{ChangeSet, NodeId, NveTable, Rgba, TableSlot};
    use crate::table_system::{SkiaSystemPaintCtx, SkiaTableRegistry, SkiaTableSystem};

    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "skia.test.icon", storage = "sparse")]
    struct TestIcon {
        #[effect(paint)]
        tag: u32,
    }
    struct IconSys;
    impl SkiaTableSystem for IconSys {
        fn slot(&self) -> TableSlot {
            TestIcon::slot()
        }
        fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, _: &NodeStore, _: NodeId) {
            // Paint only a 4x4 red square inside a 20x20 plugin rect; outer pixels stay transparent.
            for y in 8..12 {
                for x in 8..12 {
                    ctx.buffer.blend_pixel(x, y, ColorRgba::rgb(255, 0, 0), 1.0);
                }
            }
        }
    }

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<TestIcon>(n).tag(1);
        tx.commit_into(&mut changes);
        n
    };
    let mut registry = SkiaTableRegistry::new();
    registry.register(IconSys);
    let mut events = EventBatchBuf::new();

    let dl = DisplayList {
        ops: vec![PaintOp::Plugin {
            node_id: n,
            rect: LayoutRect { x: 10.0, y: 10.0, width: 20.0, height: 20.0 },
            inherited_text_color: Rgba::WHITE,
        }],
    };
    let mut surface = CpuSkiaSurface::new(40, 40).unwrap();
    let renderer = SkiaCanvasRenderer::new();
    GpuSurfaceBackend::render_display_list_with_plugins(
        &mut surface,
        &renderer,
        &dl,
        1.0,
        ColorRgba::rgb(0, 200, 100),
        Some((&store, &registry, &mut events)),
    )
    .unwrap();
    let mut out = PixelBuffer::new(40, 40);
    surface.copy_to_pixel_buffer(&mut out).unwrap();
    // Inside the 4x4 painted square: red.
    assert_eq!(out.get_rgb(20, 20), Some((255, 0, 0)));
    // Inside the 20x20 plugin rect but outside the 4x4 icon: green clear color shows through (NOT black!).
    assert_eq!(out.get_rgb(12, 12), Some((0, 200, 100)));
}

#[cfg(feature = "skia")]
#[path = "parity_tests.rs"]
mod parity_tests;
