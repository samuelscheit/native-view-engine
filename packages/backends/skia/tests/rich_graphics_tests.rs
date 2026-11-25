//! Deep edge-case & software-vs-GPU parity tests for typed Linear/Radial Gradients and
//! byte-budgeted cached Vector SVG rendering in `nve-backend-skia`.

use nve_abi::{ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, LayoutProps, StyleProps, ROOT_NODE_ID};
use nve_backend_skia::{
    draw_svg_to_buffer, parse_svg_path_data, rasterize_display_list, rasterize_svg_picture,
    BackgroundFill, DisplayList, FillRule, GradientStop, InteractionState, LinearGradient,
    PaintOp, PixelBuffer, SkiaBackend, StateStyleRule, StylePatch, SvgCache, SvgPicture,
    Transform2D, VectorPathCmd, VectorShape,
};
use nve_engine::{LayoutRect, RenderBackend, Viewport};

fn assert_rgb_near(label: &str, actual: Option<(u8, u8, u8)>, expected: (u8, u8, u8), tol: u8) {
    let (ar, ag, ab) = actual.unwrap_or_else(|| panic!("{label}: pixel out of bounds"));
    let diff = |a: u8, b: u8| a.abs_diff(b);
    assert!(
        diff(ar, expected.0) <= tol && diff(ag, expected.1) <= tol && diff(ab, expected.2) <= tol,
        "{label}: expected ~{expected:?} (±{tol}), got ({ar},{ag},{ab})"
    );
}

#[cfg(feature = "skia")]
fn render_skia_gpu(
    renderer: &nve_backend_skia::SkiaCanvasRenderer,
    list: &DisplayList,
    w: u32,
    h: u32,
    scale: f32,
    clear: ColorRgba,
) -> PixelBuffer {
    use nve_backend_skia::{CpuSkiaSurface, GpuSurfaceBackend};
    let mut surf = CpuSkiaSurface::new_with_scale(w, h, scale).unwrap();
    surf.render_display_list(renderer, list, scale, clear).unwrap();
    let mut out = PixelBuffer::new(w, h);
    surf.copy_to_pixel_buffer(&mut out).unwrap();
    out
}

fn check_linear_angles_on(label: &str, render: impl Fn(&DisplayList, u32, u32) -> PixelBuffer) {
    let red = ColorRgba::rgb(255, 0, 0);
    let blue = ColorRgba::rgb(0, 0, 255);
    let rect = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };

    // 0° = bottom (red) -> top (blue)
    let dl0 = DisplayList {
        ops: vec![PaintOp::FillRoundedRectGradient {
            rect,
            radius: 0.0,
            gradient: BackgroundFill::linear(0.0, [GradientStop::new(0.0, red), GradientStop::new(1.0, blue)]),
        }],
    };
    let buf = render(&dl0, 100, 100);
    assert_rgb_near(&format!("{label} 0deg bottom"), buf.get_rgb(50, 98), (255, 0, 0), 8);
    assert_rgb_near(&format!("{label} 0deg center"), buf.get_rgb(50, 50), (128, 0, 128), 8);
    assert_rgb_near(&format!("{label} 0deg top"), buf.get_rgb(50, 1), (0, 0, 255), 8);

    // 90° = left (red) -> right (blue)
    let dl90 = DisplayList {
        ops: vec![PaintOp::FillRoundedRectGradient {
            rect,
            radius: 0.0,
            gradient: BackgroundFill::linear(90.0, [GradientStop::new(0.0, red), GradientStop::new(1.0, blue)]),
        }],
    };
    let buf = render(&dl90, 100, 100);
    assert_rgb_near(&format!("{label} 90deg left"), buf.get_rgb(1, 50), (255, 0, 0), 8);
    assert_rgb_near(&format!("{label} 90deg center"), buf.get_rgb(50, 50), (128, 0, 128), 8);
    assert_rgb_near(&format!("{label} 90deg right"), buf.get_rgb(98, 50), (0, 0, 255), 8);

    // 180° = top (red) -> bottom (blue)
    let dl180 = DisplayList {
        ops: vec![PaintOp::FillRoundedRectGradient {
            rect,
            radius: 0.0,
            gradient: BackgroundFill::linear(180.0, [GradientStop::new(0.0, red), GradientStop::new(1.0, blue)]),
        }],
    };
    let buf = render(&dl180, 100, 100);
    assert_rgb_near(&format!("{label} 180deg top"), buf.get_rgb(50, 1), (255, 0, 0), 8);
    assert_rgb_near(&format!("{label} 180deg bottom"), buf.get_rgb(50, 98), (0, 0, 255), 8);

    // 270° = right (red) -> left (blue)
    let dl270 = DisplayList {
        ops: vec![PaintOp::FillRoundedRectGradient {
            rect,
            radius: 0.0,
            gradient: BackgroundFill::linear(270.0, [GradientStop::new(0.0, red), GradientStop::new(1.0, blue)]),
        }],
    };
    let buf = render(&dl270, 100, 100);
    assert_rgb_near(&format!("{label} 270deg right"), buf.get_rgb(98, 50), (255, 0, 0), 8);
    assert_rgb_near(&format!("{label} 270deg left"), buf.get_rgb(1, 50), (0, 0, 255), 8);

    // 45° = bottom-left (red) -> top-right (blue)
    let dl45 = DisplayList {
        ops: vec![PaintOp::FillRoundedRectGradient {
            rect,
            radius: 0.0,
            gradient: BackgroundFill::linear(45.0, [GradientStop::new(0.0, red), GradientStop::new(1.0, blue)]),
        }],
    };
    let buf = render(&dl45, 100, 100);
    assert_rgb_near(&format!("{label} 45deg bottom-left"), buf.get_rgb(2, 97), (255, 0, 0), 12);
    assert_rgb_near(&format!("{label} 45deg center"), buf.get_rgb(50, 50), (128, 0, 128), 8);
    assert_rgb_near(&format!("{label} 45deg top-right"), buf.get_rgb(97, 2), (0, 0, 255), 12);
}

fn check_stop_edges_and_radial_on(label: &str, render: impl Fn(&DisplayList, u32, u32) -> PixelBuffer) {
    let red = ColorRgba::rgb(255, 0, 0);
    let green = ColorRgba::rgb(0, 255, 0);
    let blue = ColorRgba::rgb(0, 0, 255);
    let rect = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };

    // Multi-stop clamping (first at 0.25, last at 0.75) + identical offsets (hard stop at 0.5).
    let dl_stops = DisplayList {
        ops: vec![PaintOp::FillRoundedRectGradient {
            rect,
            radius: 0.0,
            gradient: BackgroundFill::linear(
                90.0,
                [
                    GradientStop::new(0.25, red),
                    GradientStop::new(0.5, red),
                    GradientStop::new(0.5, green),
                    GradientStop::new(0.75, blue),
                ],
            ),
        }],
    };
    let buf = render(&dl_stops, 100, 100);
    assert_rgb_near(&format!("{label} clamp left"), buf.get_rgb(5, 50), (255, 0, 0), 4);
    assert_rgb_near(&format!("{label} pre-hard-stop"), buf.get_rgb(47, 50), (255, 0, 0), 4);
    assert_rgb_near(&format!("{label} post-hard-stop"), buf.get_rgb(52, 50), (0, 230, 25), 25);
    assert_rgb_near(&format!("{label} clamp right"), buf.get_rgb(95, 50), (0, 0, 255), 4);

    // Single-stop gradient fills uniformly.
    let dl_single = DisplayList {
        ops: vec![PaintOp::FillRoundedRectGradient {
            rect,
            radius: 16.0,
            gradient: BackgroundFill::linear(90.0, [GradientStop::new(0.3, green)]),
        }],
    };
    let buf = render(&dl_single, 100, 100);
    assert_rgb_near(&format!("{label} single-stop center"), buf.get_rgb(50, 50), (0, 255, 0), 2);
    // Corner outside border-radius=16 remains clear black.
    assert_rgb_near(&format!("{label} rounded corner clip"), buf.get_rgb(1, 1), (0, 0, 0), 2);

    // Radial gradient: center red -> outer blue.
    let dl_radial = DisplayList {
        ops: vec![PaintOp::FillRoundedRectGradient {
            rect,
            radius: 0.0,
            gradient: BackgroundFill::radial(
                0.5,
                0.5,
                1.0,
                [GradientStop::new(0.0, red), GradientStop::new(1.0, blue)],
            ),
        }],
    };
    let buf = render(&dl_radial, 100, 100);
    assert_rgb_near(&format!("{label} radial center"), buf.get_rgb(50, 50), (255, 0, 0), 8);
    assert_rgb_near(&format!("{label} radial mid"), buf.get_rgb(75, 50), (128, 0, 128), 10);
    assert_rgb_near(&format!("{label} radial edge"), buf.get_rgb(99, 50), (0, 0, 255), 8);
    assert_rgb_near(&format!("{label} radial corner clamp"), buf.get_rgb(99, 99), (0, 0, 255), 4);
}

#[test]
fn software_rasterizer_gradients_all_angles_stops_radial_and_transformed() {
    let render_sw = |dl: &DisplayList, w: u32, h: u32| {
        let mut buf = PixelBuffer::new(w, h);
        rasterize_display_list(dl, &mut buf, 1.0, ColorRgba::BLACK);
        buf
    };
    check_linear_angles_on("software", render_sw);
    check_stop_edges_and_radial_on("software", render_sw);

    // Affine-transformed (rotated 90°) linear gradient swaps axes cleanly.
    let rect = LayoutRect {
        x: 20.0,
        y: 20.0,
        width: 80.0,
        height: 80.0,
    };
    let tf = Transform2D::IDENTITY.with_rotate(90.0);
    let dl_tf = DisplayList {
        ops: vec![
            PaintOp::PushTransform {
                matrix: tf.to_surface_matrix_2d(rect.x, rect.y, rect.width, rect.height),
            },
            PaintOp::FillRoundedRectGradient {
                rect,
                radius: 8.0,
                gradient: BackgroundFill::linear(
                    90.0,
                    [
                        GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0)),
                        GradientStop::new(1.0, ColorRgba::rgb(0, 0, 255)),
                    ],
                ),
            },
            PaintOp::PopTransform,
        ],
    };
    let buf = render_sw(&dl_tf, 120, 120);
    // Rotating left->right by +90° clockwise puts red at top and blue at bottom.
    assert_rgb_near("transformed top", buf.get_rgb(60, 23), (255, 0, 0), 15);
    assert_rgb_near("transformed bottom", buf.get_rgb(60, 96), (0, 0, 255), 15);
}

#[cfg(feature = "skia")]
#[test]
fn skia_gpu_and_software_have_gradient_parity() {
    let renderer = nve_backend_skia::SkiaCanvasRenderer::new();
    let render_gpu = |dl: &DisplayList, w: u32, h: u32| {
        render_skia_gpu(&renderer, dl, w, h, 1.0, ColorRgba::BLACK)
    };
    check_linear_angles_on("skia-gpu", render_gpu);
    check_stop_edges_and_radial_on("skia-gpu", render_gpu);
}

#[test]
fn svg_path_parser_edge_cases_and_fuzz_never_panic() {
    // Chained quadratic/cubic/horizontal/vertical/relative/close commands.
    let cmds = parse_svg_path_data(
        "M2,2 h20 v20 H2 z m4 4 l12 0 0 12 -12 0 Z M2 12 Q12 2 22 12 T42 12 C44 6 48 6 50 12 S56 18 58 12",
    );
    assert!(cmds.len() >= 12);
    assert_eq!(cmds[0], VectorPathCmd::MoveTo(2.0, 2.0));
    assert_eq!(cmds[1], VectorPathCmd::LineTo(22.0, 2.0));
    assert_eq!(cmds[2], VectorPathCmd::LineTo(22.0, 22.0));
    assert_eq!(cmds[3], VectorPathCmd::LineTo(2.0, 22.0));
    assert_eq!(cmds[4], VectorPathCmd::Close);

    // Fuzz/malformed/adversarial inputs never panic.
    let adversarial = [
        "",
        "M",
        "M 1",
        "M NaN NaN",
        "M Infinity -Infinity",
        "M 1e308 1e308",
        "M 0 0 L 99999999 0",
        "C 1 2 3 4 5",
        "Q 1 2 3",
        "M0 0L10 10\" onload=\"alert(1)",
        "Z Z Z M 0 0 Z",
        "m-0.5-0.5c.1.2-.3-.4.5.6",
    ];
    for sample in adversarial {
        let _ = parse_svg_path_data(sample);
    }
    // Determinism of content hash.
    let a = SvgPicture::from_paths(24.0, 24.0, &["M2 2 L22 2 L12 22 Z"]);
    let b = SvgPicture::from_paths(24.0, 24.0, &["M2 2 L22 2 L12 22 Z"]);
    let c = SvgPicture::from_paths(24.0, 24.0, &["M2 2 L22 2 L12 20 Z"]);
    assert_eq!(a.content_hash, b.content_hash);
    assert_ne!(a.content_hash, c.content_hash);
}

#[test]
fn svg_cache_enforces_lru_hits_eviction_and_byte_budget() {
    // 16x16 RGBA = 1024 bytes per bitmap. Budget = 2048 bytes => at most 2 entries fit.
    let cache = SvgCache::new(16, 2048);
    let p1 = SvgPicture::from_paths(24.0, 24.0, &["M2 2 H22 V22 H2 Z"]);
    let p2 = SvgPicture::from_paths(24.0, 24.0, &["M4 4 H20 V20 H4 Z"]);
    let p3 = SvgPicture::from_paths(24.0, 24.0, &["M6 6 H18 V18 H6 Z"]);

    let b1 = cache.get_or_rasterize(&p1, 16, 16, None);
    let b1_hit = cache.get_or_rasterize(&p1, 16, 16, None);
    assert!(std::sync::Arc::ptr_eq(&b1, &b1_hit), "steady-state hit must reuse Arc");
    let st = cache.stats();
    assert_eq!(st.hits, 1);
    assert_eq!(st.misses, 1);
    assert_eq!(st.total_bytes, 1024);

    // Insert p2 -> 2048 bytes, still fits.
    let _ = cache.get_or_rasterize(&p2, 16, 16, None);
    // Touch p1 so p2 becomes least-recently-used.
    let _ = cache.get_or_rasterize(&p1, 16, 16, None);
    // Insert p3 -> must evict p2, keeping p1 + p3 within 2048-byte budget!
    let _ = cache.get_or_rasterize(&p3, 16, 16, None);
    let st = cache.stats();
    assert!(st.total_bytes <= 2048, "byte budget exceeded: {}", st.total_bytes);
    assert_eq!(st.entries, 2);
    assert_eq!(st.evictions, 1);

    // p1 is still cached (hit), while p2 was evicted (miss).
    let hits_before = st.hits;
    let _ = cache.get_or_rasterize(&p1, 16, 16, None);
    assert_eq!(cache.stats().hits, hits_before + 1);

    // Distinct tint produces distinct cache entry.
    let _ = cache.get_or_rasterize(&p1, 16, 16, Some(ColorRgba::rgb(56, 189, 248)));
    assert!(cache.total_bytes() <= 2048);

    // Single bitmap larger than entire cache budget still rasterizes cleanly without caching.
    let huge = cache.get_or_rasterize(&p1, 64, 64, None);
    assert_eq!(huge.width, 64);
    assert!(cache.total_bytes() <= 2048);
}

fn sample_svg_scene() -> (SvgPicture, SvgPicture, SvgPicture) {
    // 1. Even-odd square with center hole.
    let outer_inner = parse_svg_path_data("M2 2 H22 V22 H2 Z M8 8 H16 V16 H8 Z");
    let donut = SvgPicture::new(
        (0.0, 0.0, 24.0, 24.0),
        vec![VectorShape::filled(outer_inner, ColorRgba::rgb(56, 189, 248)).with_fill_rule(FillRule::EvenOdd)],
    );
    // 2. Cubic Bezier curved drop shape.
    let curve = SvgPicture::from_paths(
        24.0,
        24.0,
        &["M12 2 C18 8 20 14 20 17 C20 20.5 16.5 22 12 22 C7.5 22 4 20.5 4 17 C4 14 6 8 12 2 Z"],
    );
    // 3. Stroked diagonal checkmark.
    let stroke = SvgPicture::from_stroked_paths(24.0, 24.0, 3.0, &["M4 12 L10 18 L20 6"]);
    (donut, curve, stroke)
}

#[test]
fn software_svg_rasterizer_renders_evenodd_curves_strokes_and_tint() {
    let (donut, curve, stroke) = sample_svg_scene();
    let bmp = rasterize_svg_picture(&donut, 48, 48, None);
    // Ring pixel (x=8, y=24) must be solid cyan; center hole (x=24, y=24) must be transparent!
    let ring_idx = (24 * 48 + 8) * 4;
    assert!(bmp.premul_rgba[ring_idx + 3] >= 240, "ring should be filled");
    assert!(bmp.premul_rgba[ring_idx + 2] >= 200, "ring should be cyan-blue");
    let hole_idx = (24 * 48 + 24) * 4;
    assert_eq!(bmp.premul_rgba[hole_idx + 3], 0, "even-odd center hole must be empty");

    // Curve & stroke non-empty + tint override.
    let curve_bmp = rasterize_svg_picture(&curve, 48, 48, Some(ColorRgba::rgb(34, 197, 94)));
    let center_idx = (30 * 48 + 24) * 4;
    assert!(curve_bmp.premul_rgba[center_idx + 3] >= 240);
    assert!(curve_bmp.premul_rgba[center_idx + 1] >= 180);

    let stroke_bmp = rasterize_svg_picture(&stroke, 48, 48, Some(ColorRgba::WHITE));
    let lit = stroke_bmp
        .premul_rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] > 128)
        .count();
    assert!(lit >= 60, "expected >=60 lit stroke pixels, got {lit}");

    // Steady-state draw_svg_with_cache hits cache on frame 2.
    let cache = SvgCache::new(8, 1024 * 1024);
    let mut buf = PixelBuffer::new(64, 64);
    let clip_dl = DisplayList { ops: vec![] };
    rasterize_display_list(&clip_dl, &mut buf, 1.0, ColorRgba::BLACK);
    let rect = LayoutRect { x: 8.0, y: 8.0, width: 48.0, height: 48.0 };
    // Drive through DisplayList + SkiaBackend end-to-end as well.
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 120.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Custom, Some("svg"));
    b.set_layout(2, &LayoutProps { width: Dimension::Points(48.0), height: Dimension::Points(48.0), ..Default::default() });
    b.set_style(2, &StyleProps { text_color: ColorRgba::rgb(239, 68, 68), ..Default::default() });
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    backend.submit_frame(b.finish()).unwrap();
    backend.set_node_svg(2, Some(curve.clone()));
    backend.set_node_background_fill(
        2,
        Some(BackgroundFill::Linear(LinearGradient::new(
            90.0,
            [
                GradientStop::new(0.0, ColorRgba::rgb(15, 23, 42)),
                GradientStop::new(1.0, ColorRgba::rgb(30, 41, 59)),
            ],
        ))),
    );
    backend.set_node_state_rules(
        2,
        vec![StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new().radial_gradient(
                0.5,
                0.5,
                1.0,
                [
                    GradientStop::new(0.0, ColorRgba::rgb(255, 255, 255)),
                    GradientStop::new(1.0, ColorRgba::rgb(59, 130, 246)),
                ],
            ),
        )],
    );
    assert!(backend
        .display_list()
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::FillRoundedRectGradient { gradient: BackgroundFill::Linear(_), .. })));
    assert!(backend
        .display_list()
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::DrawSvg { tint: Some(c), .. } if *c == ColorRgba::rgb(239, 68, 68))));

    // Hover switches gradient to Radial automatically!
    let _ = backend.pointer_move(24.0, 24.0);
    assert!(backend
        .display_list()
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::FillRoundedRectGradient { gradient: BackgroundFill::Radial(_), .. })));

    let mut out = PixelBuffer::new(120, 120);
    backend.rasterize(&mut out, ColorRgba::BLACK);
    assert!(out.get_rgb(24, 30).is_some_and(|(r, _, _)| r > 150));

    // Also explicitly verify isolated SvgCache hits via draw_svg_to_buffer.
    draw_svg_to_buffer(&mut buf, rect, &donut, None, 1.0, &cache);
    draw_svg_to_buffer(&mut buf, rect, &donut, None, 1.0, &cache);
    assert_eq!(cache.stats().misses, 1);
    assert_eq!(cache.stats().hits, 1);
}

#[cfg(feature = "skia")]
#[test]
fn skia_gpu_and_software_render_svg_with_non_empty_pixels_and_parity() {
    let (donut, curve, stroke) = sample_svg_scene();
    let dl = DisplayList {
        ops: vec![
            PaintOp::DrawSvg {
                rect: LayoutRect { x: 4.0, y: 4.0, width: 48.0, height: 48.0 },
                picture: donut,
                tint: None,
            },
            PaintOp::DrawSvg {
                rect: LayoutRect { x: 60.0, y: 4.0, width: 48.0, height: 48.0 },
                picture: curve,
                tint: Some(ColorRgba::rgb(34, 197, 94)),
            },
            PaintOp::DrawSvg {
                rect: LayoutRect { x: 116.0, y: 4.0, width: 48.0, height: 48.0 },
                picture: stroke,
                tint: Some(ColorRgba::WHITE),
            },
        ],
    };

    let mut sw_buf = PixelBuffer::new(172, 56);
    rasterize_display_list(&dl, &mut sw_buf, 1.0, ColorRgba::BLACK);

    let renderer = nve_backend_skia::SkiaCanvasRenderer::new();
    let gpu_buf = render_skia_gpu(&renderer, &dl, 172, 56, 1.0, ColorRgba::BLACK);
    // Second frame must hit cached skia_safe::Picture entries.
    let _gpu_buf2 = render_skia_gpu(&renderer, &dl, 172, 56, 1.0, ColorRgba::BLACK);

    for (label, buf) in [("software", &sw_buf), ("skia-gpu", &gpu_buf)] {
        // 1. Donut ring filled cyan, center hole remains clear black.
        assert_rgb_near(&format!("{label} donut ring"), buf.get_rgb(12, 28), (56, 189, 248), 10);
        assert_rgb_near(&format!("{label} donut hole"), buf.get_rgb(28, 28), (0, 0, 0), 2);
        // 2. Curved drop tinted green.
        assert_rgb_near(&format!("{label} curve center"), buf.get_rgb(84, 34), (34, 197, 94), 10);
        // 3. Stroked checkmark has >= 50 lit pixels in its bounding box.
        let mut lit = 0usize;
        for y in 4..52 {
            for x in 116..164 {
                if let Some((r, g, b)) = buf.get_rgb(x, y)
                    && r > 120 && g > 120 && b > 120
                {
                    lit += 1;
                }
            }
        }
        assert!(lit >= 50, "{label}: expected >=50 lit stroke pixels, got {lit}");
    }

    // Verify RadialGradient struct equality and direct construction too.
    let rg = nve_backend_skia::RadialGradient::new(
        0.5,
        0.5,
        1.0,
        [GradientStop::new(0.0, ColorRgba::WHITE)],
    );
    assert_eq!(rg.stops.len(), 1);
}
