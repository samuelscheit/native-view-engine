use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask,
    FlexDirection, LayoutProps, RawPropValue, StyleProps, ROOT_NODE_ID,
};
use nve_components::canvas_ir::{DrawListBuilder, ImageResizeMode};
use nve_engine::{RenderBackend, Viewport};

use crate::{PaintOp, PixelBuffer, SkiaBackend};

const STUDIO_AVATAR_MAYA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";

#[test]
fn renders_studio_png_avatar_under_all_four_resize_modes_and_rounded_clip() {
    let viewport = Viewport {
        width: 120.0,
        height: 80.0,
        scale_factor: 1.0,
    };

    for mode in [
        ImageResizeMode::Cover,
        ImageResizeMode::Contain,
        ImageResizeMode::Stretch,
        ImageResizeMode::Center,
    ] {
        let mut skia = SkiaBackend::new();
        skia.set_viewport(1, viewport);

        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::Image, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(80.0),
                height: Dimension::Points(40.0),
                margin: EdgeInsets {
                    left: 10.0,
                    top: 10.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                border_radius: 12.0,
                ..Default::default()
            },
        );
        b.set_raw_prop(2, "src", &RawPropValue::String(STUDIO_AVATAR_MAYA.to_string()));
        b.set_raw_prop(2, "resizeMode", &RawPropValue::String(mode.as_str().to_string()));
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);

        skia.submit_frame(b.finish()).unwrap();
        assert!(skia
            .display_list()
            .ops
            .iter()
            .any(|op| matches!(op, PaintOp::DrawImage { resize_mode, .. } if *resize_mode == mode)));

        let mut buf = PixelBuffer::new(120, 80);
        skia.rasterize(&mut buf, ColorRgba::BLACK);

        let mut non_black_inside = 0usize;
        for y in 10..50 {
            for x in 10..90 {
                if buf.get_rgb(x, y) != Some((0, 0, 0)) {
                    non_black_inside += 1;
                }
            }
        }
        assert!(non_black_inside > 0);
        assert_eq!(buf.get_rgb(10, 10), Some((0, 0, 0)));
        assert_eq!(buf.get_rgb(5, 5), Some((0, 0, 0)));
        assert_eq!(buf.get_rgb(105, 60), Some((0, 0, 0)));

        if mode == ImageResizeMode::Contain {
            assert_eq!(buf.get_rgb(15, 30), Some((0, 0, 0)));
            assert_eq!(buf.get_rgb(85, 30), Some((0, 0, 0)));
        }
        if mode == ImageResizeMode::Center {
            assert_eq!(buf.get_rgb(20, 30), Some((0, 0, 0)));
            assert!(non_black_inside <= 64);
        }
    }
}

#[test]
fn executes_nve_draw1_sparkline_primitives_and_survives_corrupt_buffers() {
    let viewport = Viewport {
        width: 340.0,
        height: 160.0,
        scale_factor: 1.0,
    };

    let stroke_cyan = ColorRgba::rgb(56, 189, 248);
    let bg_slate = ColorRgba::rgb(17, 24, 39);
    let dot_green = ColorRgba::rgb(16, 185, 129);
    let line_red = ColorRgba::rgb(239, 68, 68);

    let mut draw = DrawListBuilder::new();
    draw.sparkline(
        &[120.0, 180.0, 95.0, 240.0, 210.0, 310.0],
        320.0,
        72.0,
        stroke_cyan,
        bg_slate,
    );
    draw.fill_circle(40.0, 110.0, 12.0, dot_green);
    draw.stroke_line(80.0, 110.0, 180.0, 110.0, 4.0, line_red);
    draw.clip_rect(200.0, 95.0, 30.0, 30.0);
    draw.fill_rect(200.0, 95.0, 100.0, 30.0, ColorRgba::rgb(250, 204, 21));
    draw.restore_clip();
    let draw_bytes = draw.finish();

    let mut skia = SkiaBackend::new();
    skia.set_viewport(1, viewport);

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::SkiaCanvas, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(320.0),
            height: Dimension::Points(140.0),
            margin: EdgeInsets {
                left: 10.0,
                top: 10.0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    b.set_raw_prop(2, "drawList", &RawPropValue::Bytes(draw_bytes));
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);

    skia.submit_frame(b.finish()).unwrap();

    let mut buf = PixelBuffer::new(340, 160);
    skia.rasterize(&mut buf, ColorRgba::BLACK);

    assert_eq!(buf.get_rgb(20, 20), Some((17, 24, 39)));

    let mut cyan_hits = 0usize;
    for y in 10..82 {
        for x in 10..330 {
            let (r, g, b) = buf.get_rgb(x, y).unwrap();
            if r <= 80 && g >= 160 && b >= 220 {
                cyan_hits += 1;
            }
        }
    }
    assert!(cyan_hits >= 40);
    assert_eq!(buf.get_rgb(50, 120), Some((16, 185, 129)));
    assert_eq!(buf.get_rgb(140, 120), Some((239, 68, 68)));
    assert_eq!(buf.get_rgb(225, 120), Some((250, 204, 21)));
    assert_eq!(buf.get_rgb(270, 120), Some((0, 0, 0)));

    for bad in [
        RawPropValue::Bytes(vec![]),
        RawPropValue::Bytes(b"NVE_DRAW1".to_vec()),
        RawPropValue::Bytes(b"BAD_MAGIC_1234567890".to_vec()),
        RawPropValue::Bytes(vec![
            b'N', b'V', b'E', b'_', b'D', b'R', b'A', b'W', b'1', 1, 0, 255, 255, 0, 0,
        ]),
        RawPropValue::String("nve-draw1;b64,!!!invalid!!!".to_string()),
    ] {
        let mut b = CommandBufferBuilder::new(1, 2);
        b.set_raw_prop(2, "drawList", &bad);
        b.commit(false);
        skia.submit_frame(b.finish()).unwrap();
        let mut tmp = PixelBuffer::new(340, 160);
        skia.rasterize(&mut tmp, ColorRgba::BLACK);
    }
}

#[test]
fn scrollview_clips_overflow_shifts_pixels_and_updates_hit_testing_on_scroll() {
    let viewport = Viewport {
        width: 200.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    let mut skia = SkiaBackend::new();
    skia.set_viewport(1, viewport);

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::ScrollView, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(160.0),
            height: Dimension::Points(80.0),
            flex_direction: FlexDirection::Column,
            margin: EdgeInsets {
                left: 10.0,
                top: 10.0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    b.set_event_mask(2, EventMask::SCROLL);

    for (id, color) in [
        (10u32, ColorRgba::rgb(239, 68, 68)),
        (11u32, ColorRgba::rgb(16, 185, 129)),
        (12u32, ColorRgba::rgb(37, 99, 235)),
    ] {
        b.create_node(id, ComponentKind::Pressable, None);
        b.set_layout(
            id,
            &LayoutProps {
                width: Dimension::Points(160.0),
                height: Dimension::Points(60.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
        );
        b.set_style(
            id,
            &StyleProps {
                background: color,
                ..Default::default()
            },
        );
        b.set_event_mask(id, EventMask::PRESS);
    }
    b.set_children(2, &[10, 11, 12]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    assert!(skia
        .display_list()
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::PushClip { .. })));
    assert!(skia
        .display_list()
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::PopClip)));
    assert_eq!(skia.hit_test_scroll_view(50.0, 50.0), Some(2));
    assert_eq!(skia.hit_test_scroll_view(190.0, 190.0), None);

    assert_eq!(skia.hit_test(50.0, 30.0), Some(10));
    assert_eq!(skia.hit_test(50.0, 80.0), Some(11));
    assert_eq!(skia.hit_test(50.0, 110.0), None);

    let mut buf = PixelBuffer::new(200, 200);
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(50, 30), Some((239, 68, 68)));
    assert_eq!(buf.get_rgb(50, 80), Some((16, 185, 129)));
    assert_eq!(buf.get_rgb(50, 110), Some((0, 0, 0)));

    let (sx, sy) = skia.apply_scroll_delta(2, 0.0, 90.0);
    assert_eq!((sx, sy), (0.0, 90.0));

    assert_eq!(skia.hit_test(50.0, 25.0), Some(11));
    assert_eq!(skia.hit_test(50.0, 60.0), Some(12));
    assert_eq!(skia.hit_test(50.0, 95.0), None);

    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(50, 25), Some((16, 185, 129)));
    assert_eq!(buf.get_rgb(50, 60), Some((37, 99, 235)));
    assert_eq!(buf.get_rgb(50, 5), Some((0, 0, 0)));
    assert_eq!(buf.get_rgb(50, 95), Some((0, 0, 0)));

    assert_eq!(skia.apply_scroll_delta(2, 0.0, 500.0), (0.0, 100.0));
    assert_eq!(skia.apply_scroll_delta(2, 0.0, -999.0), (0.0, 0.0));
}
