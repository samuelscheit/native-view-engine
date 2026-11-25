//! Pixel-level & behavioral unit tests for rendering polish (`font-weight`, `text-align`,
//! `ScrollView` overlay scrollbars, `box-shadow`, and rounded-clip interior-span equivalence).

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, FlexDirection,
    LayoutProps, RawPropValue, StyleProps, ROOT_NODE_ID,
};
use nve_engine::{LayoutRect, RenderBackend, Viewport};

use crate::raster::clip::ClipStack;
use crate::{
    parse_box_shadow, rasterize_display_list, rasterize_display_list_bitmap, DisplayList, PaintOp,
    PixelBuffer, SkiaBackend, TextAlign, SCROLLBAR_THUMB_COLOR,
};

#[test]
fn bold_font_weight_renders_more_ink_than_normal_weight() {
    let rect = LayoutRect {
        x: 16.0,
        y: 16.0,
        width: 240.0,
        height: 48.0,
    };
    let fg = ColorRgba::WHITE;
    let bg = ColorRgba::BLACK;

    let render_luma = |weight: u16| -> u64 {
        let dl = DisplayList {
            ops: vec![PaintOp::DrawText {
                rect,
                text: "Bold Headline".to_string(),
                font_size: 24.0,
                font_weight: weight,
                text_align: TextAlign::Left,
                color: fg,
            }],
        };
        let mut buf = PixelBuffer::new(280, 80);
        rasterize_display_list(&dl, &mut buf, 1.0, bg);
        buf.pixels()
            .iter()
            .map(|&p| ((p >> 16) & 0xFF) as u64)
            .sum()
    };

    let regular_ink = render_luma(400);
    let bold_ink = render_luma(700);
    assert!(
        bold_ink > regular_ink,
        "expected weight=700 to produce denser ink than weight=400 (regular={regular_ink}, bold={bold_ink})"
    );

    // Bitmap fallback also differentiates bold (>= 600) vs normal weight.
    let render_bitmap_luma = |weight: u16| -> usize {
        let dl = DisplayList {
            ops: vec![PaintOp::DrawText {
                rect,
                text: "BOLD".to_string(),
                font_size: 20.0,
                font_weight: weight,
                text_align: TextAlign::Left,
                color: fg,
            }],
        };
        let mut buf = PixelBuffer::new(280, 80);
        rasterize_display_list_bitmap(&dl, &mut buf, 1.0, bg);
        buf.pixels().iter().filter(|&&p| p != 0).count()
    };
    assert!(render_bitmap_luma(700) > render_bitmap_luma(400));
}

#[test]
fn text_align_center_and_right_shift_glyph_centroid_accurately() {
    let centroid_x = |align: TextAlign| -> f64 {
        let mut skia = SkiaBackend::new();
        skia.set_viewport(
            1,
            Viewport {
                width: 300.0,
                height: 80.0,
                scale_factor: 1.0,
            },
        );
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::Text, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(260.0),
                height: Dimension::Points(40.0),
                margin: EdgeInsets {
                    left: 20.0,
                    top: 20.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                text_color: ColorRgba::WHITE,
                font_size: 20.0,
                ..Default::default()
            },
        );
        b.set_text(2, "Hi");
        let align_str = match align {
            TextAlign::Left => "left",
            TextAlign::Center => "center",
            TextAlign::Right => "right",
        };
        b.set_raw_prop(2, "text-align", &RawPropValue::String(align_str.to_string()));
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        skia.submit_frame(b.finish()).unwrap();

        let mut buf = PixelBuffer::new(300, 80);
        skia.rasterize(&mut buf, ColorRgba::BLACK);

        let mut sum_x = 0.0f64;
        let mut total_w = 0.0f64;
        for y in 0..80u32 {
            for x in 0..300u32 {
                let (r, _, _) = buf.get_rgb(x, y).unwrap();
                if r > 0 {
                    sum_x += (x as f64) * (r as f64);
                    total_w += r as f64;
                }
            }
        }
        assert!(total_w > 0.0, "expected lit pixels for align={align:?}");
        sum_x / total_w
    };

    let cx_left = centroid_x(TextAlign::Left);
    let cx_center = centroid_x(TextAlign::Center);
    let cx_right = centroid_x(TextAlign::Right);

    assert!(
        cx_left < 70.0,
        "expected left-aligned centroid near x=20..70, got {cx_left}"
    );
    assert!(
        (cx_center - 150.0).abs() < 20.0,
        "expected centered centroid near box midpoint x=150, got {cx_center}"
    );
    assert!(
        cx_right > 230.0,
        "expected right-aligned centroid near right edge x=280, got {cx_right}"
    );
}

#[test]
fn scrollview_renders_overlay_scrollbar_thumbs_and_moves_thumb_on_scroll() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 200.0,
            height: 140.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::ScrollView, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(180.0),
            height: Dimension::Points(100.0),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            ..Default::default()
        },
    );
    // Tall & wide content to exercise both vertical and horizontal overlay scrollbars.
    b.create_node(3, ComponentKind::View, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(360.0),
            height: Dimension::Points(300.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.set_style(
        3,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            ..Default::default()
        },
    );
    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    let thumb_ops = |ops: &[PaintOp]| -> Vec<LayoutRect> {
        ops.iter()
            .filter_map(|op| match op {
                PaintOp::FillRoundedRect { rect, color, .. } if *color == SCROLLBAR_THUMB_COLOR => {
                    Some(*rect)
                }
                _ => None,
            })
            .collect()
    };

    let thumbs_top = thumb_ops(&skia.display_list().ops);
    assert_eq!(thumbs_top.len(), 2, "expected vertical + horizontal scrollbar thumbs");
    let v_top = thumbs_top[0];
    let h_left = thumbs_top[1];
    assert!(v_top.x > 165.0 && v_top.width == 6.0);
    assert!(h_left.y > 85.0 && h_left.height == 6.0);

    // Verify rendered scrollbar pixels are visibly brighter than the dark slate background.
    let mut buf = PixelBuffer::new(200, 140);
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    let px = buf
        .get_rgb((v_top.x + 3.0) as u32, (v_top.y + 6.0) as u32)
        .unwrap();
    assert!(
        px.0 > 50 && px.1 > 60 && px.2 > 80,
        "expected visible vertical scrollbar thumb pixel, got {px:?}"
    );
    let hpx = buf
        .get_rgb((h_left.x + 8.0) as u32, (h_left.y + 3.0) as u32)
        .unwrap();
    assert!(
        hpx.0 > 50 && hpx.1 > 60 && hpx.2 > 80,
        "expected visible horizontal scrollbar thumb pixel, got {hpx:?}"
    );

    // Scrolling moves both thumbs forward along their tracks.
    skia.set_scroll_offset(2, 180.0, 200.0);
    let thumbs_end = thumb_ops(&skia.display_list().ops);
    assert!(thumbs_end[0].y > v_top.y + 30.0);
    assert!(thumbs_end[1].x > h_left.x + 40.0);
}

#[test]
fn box_shadow_parses_css_variants_and_rasterizes_soft_falloff_pixels() {
    let parsed = parse_box_shadow("0 8px 16px 2px rgba(0, 0, 0, 0.5), 2px 4px 6px #38bdf880");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].offset_x, 0.0);
    assert_eq!(parsed[0].offset_y, 8.0);
    assert_eq!(parsed[0].blur_radius, 16.0);
    assert_eq!(parsed[0].spread_radius, 2.0);
    assert_eq!(parsed[0].color.a, 128);
    assert_eq!(parsed[1].color, ColorRgba::rgba(56, 189, 248, 128));
    assert!(parse_box_shadow("none").is_empty());
    assert!(parse_box_shadow("inset 0 2px 4px #000").is_empty());

    // Render a white card with a dark blurred drop shadow on a bright canvas.
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 200.0,
            height: 160.0,
            scale_factor: 1.0,
        },
    );
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(60.0),
            margin: EdgeInsets {
                left: 50.0,
                top: 40.0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::WHITE,
            border_radius: 10.0,
            ..Default::default()
        },
    );
    b.set_raw_prop(
        2,
        "box-shadow",
        &RawPropValue::String("0 10px 14px rgba(0, 0, 0, 0.6)".to_string()),
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    assert!(skia
        .display_list()
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::DrawShadow { .. })));

    let mut buf = PixelBuffer::new(200, 160);
    skia.rasterize(&mut buf, ColorRgba::WHITE);

    // Card interior remains solid white.
    assert_eq!(buf.get_rgb(100, 70), Some((255, 255, 255)));
    // Directly beneath the card (y = 104, within offset_y=10 shadow zone) is darkened.
    let near_shadow = buf.get_rgb(100, 104).unwrap().0;
    // Further below (y = 114) has softer falloff than near_shadow, but still darker than white.
    let far_shadow = buf.get_rgb(100, 114).unwrap().0;
    // Far outside the shadow envelope (y = 155) remains pure white.
    let outside = buf.get_rgb(100, 155).unwrap().0;
    assert!(
        near_shadow < 180,
        "expected strong shadow pixel directly below card, got luma {near_shadow}"
    );
    assert!(
        near_shadow < far_shadow && far_shadow < 255,
        "expected smooth Gaussian falloff: near={near_shadow} < far={far_shadow} < 255"
    );
    assert_eq!(outside, 255);
}

#[test]
fn rounded_clip_interior_span_fast_path_matches_naive_sqrt_sdf_pixel_for_pixel() {
    fn naive_rounded_rect_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
        let qx = (px - cx).abs() - (hx - r);
        let qy = (py - cy).abs() - (hy - r);
        let out_x = qx.max(0.0);
        let out_y = qy.max(0.0);
        let outside = (out_x * out_x + out_y * out_y).sqrt();
        let inside = qx.max(qy).min(0.0);
        outside + inside - r
    }

    let clips = [
        (
            LayoutRect {
                x: 12.25,
                y: 8.5,
                width: 96.5,
                height: 74.0,
            },
            18.0f32,
        ),
        (
            LayoutRect {
                x: 24.0,
                y: 16.0,
                width: 72.0,
                height: 58.0,
            },
            14.0f32,
        ),
    ];

    let mut stack = ClipStack::new(128, 96);
    for &(r, rad) in &clips {
        stack.push(r, rad);
    }

    // Verify both `rounded_coverage` and `interior_span_for_row` against naive per-pixel sqrt SDF.
    for py in 0..96i32 {
        let fy = py as f32 + 0.5;
        let (span_l, span_r) = stack.interior_span_for_row(py, 0, 128);
        for px in 0..128i32 {
            let fx = px as f32 + 0.5;
            let mut naive_cov = 1.0f32;
            for &(r, rad) in &clips {
                let hx = r.width * 0.5;
                let hy = r.height * 0.5;
                let clamped_r = rad.clamp(0.0, hx.min(hy));
                let sdf = naive_rounded_rect_sdf(fx, fy, r.x + hx, r.y + hy, hx, hy, clamped_r);
                naive_cov = naive_cov.min((0.5 - sdf).clamp(0.0, 1.0));
            }
            let fast_cov = stack.rounded_coverage(fx, fy);
            assert!(
                (fast_cov - naive_cov).abs() <= 1e-5,
                "coverage mismatch at ({px}, {py}): fast={fast_cov}, naive={naive_cov}"
            );
            if px >= span_l && px < span_r {
                assert!(
                    (naive_cov - 1.0).abs() <= 1e-5,
                    "interior_span_for_row included non-opaque pixel ({px}, {py}) with naive_cov={naive_cov}"
                );
            }
        }
    }
}
