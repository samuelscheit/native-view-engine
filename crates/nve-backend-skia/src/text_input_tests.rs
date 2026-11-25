use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask, LayoutProps,
    RawPropValue, StyleProps, ROOT_NODE_ID,
};
use nve_engine::{RenderBackend, Viewport};

use crate::{
    decode_png, encode_rgb_png, PaintOp, PixelBuffer, SkiaBackend, FOCUS_RING_COLOR,
    FOCUS_SELECTION_COLOR, IME_PREEDIT_UNDERLINE_COLOR,
};

#[test]
fn text_input_emits_placeholder_value_and_secure_mask_and_png_roundtrips() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 320.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );
    let mut b = CommandBufferBuilder::new(1, 1);
    for (id, ph, val, secure) in [
        (2u32, Some("Filter issues..."), None, false),
        (3u32, Some("Unused placeholder"), Some("hello"), false),
        (4u32, Some("Password"), Some("s3cr3t"), true),
    ] {
        b.create_node(id, ComponentKind::TextInput, None);
        b.set_layout(
            id,
            &LayoutProps {
                width: Dimension::Points(260.0),
                height: Dimension::Points(34.0),
                padding: EdgeInsets::symmetric(0.0, 12.0),
                ..Default::default()
            },
        );
        b.set_style(
            id,
            &StyleProps {
                background: ColorRgba::rgb(15, 23, 42),
                text_color: ColorRgba::rgb(241, 245, 249),
                font_size: 13.0,
                ..Default::default()
            },
        );
        if let Some(p) = ph {
            b.set_raw_prop(id, "placeholder", &RawPropValue::String(p.into()));
        }
        if let Some(v) = val {
            b.set_raw_prop(id, "value", &RawPropValue::String(v.into()));
        }
        if secure {
            b.set_raw_prop(id, "secureTextEntry", &RawPropValue::Bool(true));
        }
    }
    b.set_children(ROOT_NODE_ID, &[2, 3, 4]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    let drawn: Vec<(&str, ColorRgba, f32)> = skia
        .display_list()
        .ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::DrawText {
                rect, text, color, ..
            } => Some((text.as_str(), *color, rect.x)),
            _ => None,
        })
        .collect();
    assert_eq!(drawn.len(), 3);
    assert_eq!(drawn[0].0, "Filter issues...");
    assert!(drawn[0].1.a < 200, "placeholder should be muted alpha");
    assert!(drawn[0].2 >= 12.0, "text should respect left padding");
    assert_eq!(drawn[1].0, "hello");
    assert_eq!(drawn[1].1.a, 255);
    assert_eq!(drawn[2].0, "••••••");

    assert_eq!(skia.text_input_value(2).as_deref(), Some(""));
    assert_eq!(skia.text_input_value(3).as_deref(), Some("hello"));
    assert_eq!(skia.text_input_value(4).as_deref(), Some("s3cr3t"));
    assert_eq!(skia.text_input_value(ROOT_NODE_ID), None);

    let mut buf = PixelBuffer::new(320, 200);
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    let mut rgb = Vec::with_capacity(320 * 200 * 3);
    for y in 0..200 {
        for x in 0..320 {
            let (r, g, b) = buf.get_rgb(x, y).unwrap();
            rgb.extend_from_slice(&[r, g, b]);
        }
    }
    let png = encode_rgb_png(320, 200, &rgb).expect("encode PNG");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let decoded = decode_png(&png).expect("decode roundtripped PNG");
    assert_eq!((decoded.width, decoded.height), (320, 200));
    assert_eq!(&decoded.rgba[0..4], &[rgb[0], rgb[1], rgb[2], 255]);
}

#[test]
fn focused_text_input_paints_focus_ring_caret_selection_ime_and_secure_bullet_offset() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 320.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::TextInput, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(260.0),
            height: Dimension::Points(36.0),
            padding: EdgeInsets::symmetric(0.0, 12.0),
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
            background: ColorRgba::rgb(15, 23, 42),
            border_color: ColorRgba::rgb(51, 65, 85),
            border_width: 1.0,
            border_radius: 6.0,
            text_color: ColorRgba::rgb(241, 245, 249),
            font_size: 14.0,
            ..Default::default()
        },
    );
    b.set_raw_prop(2, "value", &RawPropValue::String("hello".into()));

    b.create_node(3, ComponentKind::TextInput, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(260.0),
            height: Dimension::Points(36.0),
            padding: EdgeInsets::symmetric(0.0, 12.0),
            margin: EdgeInsets {
                left: 10.0,
                top: 10.0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    b.set_style(
        3,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            text_color: ColorRgba::rgb(241, 245, 249),
            font_size: 14.0,
            ..Default::default()
        },
    );
    b.set_raw_prop(3, "value", &RawPropValue::String("secret".into()));
    b.set_raw_prop(3, "secureTextEntry", &RawPropValue::Bool(true));

    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    let r2 = skia.layout_of(2).unwrap();
    let r3 = skia.layout_of(3).unwrap();
    assert_eq!(skia.hit_test_focusable(r2.x + 20.0, r2.y + 18.0), Some(2));
    assert_eq!(skia.hit_test_focusable(r3.x + 20.0, r3.y + 18.0), Some(3));
    assert_eq!(skia.hit_test_focusable(310.0, 190.0), None);

    assert!(skia.set_focused_node(Some(2)));
    assert_eq!(skia.focused_node(), Some(2));
    assert_eq!(skia.caret_byte_index(), 5);
    assert_eq!(skia.caret_char_index(), 5);

    let find_caret_x = |ops: &[PaintOp]| -> Option<f32> {
        ops.iter().find_map(|op| match op {
            PaintOp::FillRoundedRect { rect, color, .. }
                if (rect.width - 2.0).abs() < 0.01 && *color == ColorRgba::rgb(241, 245, 249) =>
            {
                Some(rect.x)
            }
            _ => None,
        })
    };

    let caret_end_x = find_caret_x(&skia.display_list().ops).expect("caret op at end");
    assert!(caret_end_x > r2.x + 12.0);
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::StrokeRoundedRect { color, width, .. }
            if *color == FOCUS_RING_COLOR && *width >= 2.0
    )));

    let mut buf = PixelBuffer::new(320, 200);
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(
        buf.get_rgb((r2.x + r2.width * 0.5) as u32, r2.y as u32),
        Some((FOCUS_RING_COLOR.r, FOCUS_RING_COLOR.g, FOCUS_RING_COLOR.b))
    );
    assert_eq!(
        buf.get_rgb(caret_end_x.round() as u32, (r2.y + r2.height * 0.5) as u32),
        Some((241, 245, 249))
    );

    skia.set_caret_byte_index(0);
    let caret_start_x = find_caret_x(&skia.display_list().ops).expect("caret op at start");
    assert!(caret_start_x < caret_end_x);
    assert!((caret_start_x - (r2.x + 12.0)).abs() < 0.1);

    skia.select_all_text();
    assert_eq!(skia.selection_range(), Some((0, 5)));
    assert_eq!(skia.selected_text().as_deref(), Some("hello"));
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::FillRoundedRect { color, .. } if *color == FOCUS_SELECTION_COLOR
    )));

    // Sub-range selection renders a highlight strictly narrower than full-text selection.
    let full_sel_w = skia
        .display_list()
        .ops
        .iter()
        .find_map(|op| match op {
            PaintOp::FillRoundedRect { rect, color, .. } if *color == FOCUS_SELECTION_COLOR => {
                Some(rect.width)
            }
            _ => None,
        })
        .expect("full selection highlight");
    skia.set_selection_range(1, 4);
    assert_eq!(skia.selection_range(), Some((1, 4)));
    assert_eq!(skia.selected_text().as_deref(), Some("ell"));
    let partial_sel_rect = skia
        .display_list()
        .ops
        .iter()
        .find_map(|op| match op {
            PaintOp::FillRoundedRect { rect, color, .. } if *color == FOCUS_SELECTION_COLOR => {
                Some(*rect)
            }
            _ => None,
        })
        .expect("partial selection highlight");
    assert!(partial_sel_rect.x > r2.x + 12.0);
    assert!(partial_sel_rect.width < full_sel_w);

    // cursor_index_at_x maps left edge -> 0, far right -> len, and midpoints monotonically.
    assert_eq!(skia.cursor_index_at_x(2, r2.x), Some(0));
    assert_eq!(skia.cursor_index_at_x(2, r2.x + 250.0), Some(5));
    let mid_idx = skia.cursor_index_at_x(2, partial_sel_rect.x).unwrap();
    assert!((1..=2).contains(&mid_idx));

    skia.set_caret_byte_index(2);
    skia.set_ime_preedit(Some("日本".to_string()), Some((0, "日本".len())));
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::DrawText { text, .. } if text == "he日本llo"
    )));
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::FillRoundedRect { rect, color, .. }
            if *color == IME_PREEDIT_UNDERLINE_COLOR && (rect.height - 1.5).abs() < 0.01
    )));

    assert!(skia.set_focused_node(Some(3)));
    let secure_caret_end = find_caret_x(&skia.display_list().ops).expect("secure caret at end");
    skia.set_caret_byte_index(2);
    let secure_caret_mid = find_caret_x(&skia.display_list().ops).expect("secure caret at 2");
    assert!(secure_caret_mid > r3.x + 12.0 && secure_caret_mid < secure_caret_end);

    assert!(skia.set_focused_node(None));
    assert!(!skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::StrokeRoundedRect { color, .. } if *color == FOCUS_RING_COLOR
    )));
}

#[test]
fn z_order_hit_testing_and_display_list_work_without_clone_above_inline_threshold() {
    // 72 z-ordered overlapping siblings (> INLINE_Z_SORT_CAPACITY=64) via negative top margins
    // verify both inline and overflow paths in hit_test and scratch-reused sorting in display_list.
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 200.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    let mut ids = Vec::new();
    for i in 0u32..72 {
        let id = 10 + i;
        ids.push(id);
        b.create_node(id, ComponentKind::TextInput, None);
        b.set_layout(
            id,
            &LayoutProps {
                width: Dimension::Points(100.0),
                height: Dimension::Points(30.0),
                flex_shrink: 0.0,
                margin: EdgeInsets {
                    left: 10.0,
                    top: if i == 0 { 10.0 } else { -30.0 },
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let z = if id == 42 { 500 } else { (i % 7) as i32 };
        b.set_style(
            id,
            &StyleProps {
                background: ColorRgba::rgb(20, 30, 40),
                z_index: z,
                ..Default::default()
            },
        );
        b.set_event_mask(id, EventMask::PRESS);
    }
    b.set_children(ROOT_NODE_ID, &ids);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    assert_eq!(skia.hit_test(40.0, 20.0), Some(42));
    assert_eq!(skia.hit_test_focusable(40.0, 20.0), Some(42));
}

#[cfg(feature = "skia")]
#[test]
fn skia_gpu_or_cpu_canvas_renders_focus_ring_and_caret_pixels() {
    use crate::{GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig, GpuSwapchain};

    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 260.0,
            height: 80.0,
            scale_factor: 1.0,
        },
    );
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::TextInput, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(220.0),
            height: Dimension::Points(36.0),
            padding: EdgeInsets::symmetric(0.0, 12.0),
            margin: EdgeInsets {
                left: 16.0,
                top: 16.0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            border_radius: 6.0,
            text_color: ColorRgba::rgb(241, 245, 249),
            font_size: 14.0,
            ..Default::default()
        },
    );
    b.set_raw_prop(2, "value", &RawPropValue::String("Metal".into()));
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();
    skia.set_focused_node(Some(2));

    let mut sw = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
        width: 260,
        height: 80,
        scale_factor: 1.0,
        preferred_backend: GpuBackendKind::platform_preferred(),
        allow_cpu_fallback: true,
        ..Default::default()
    })
    .expect("offscreen Skia/Metal swapchain");
    sw.execute_frame(skia.display_list(), 1.0, ColorRgba::BLACK, None)
        .expect("execute frame");
    let mut readback = PixelBuffer::new(1, 1);
    sw.copy_to_pixel_buffer(&mut readback).expect("readback");

    let r2 = skia.layout_of(2).unwrap();
    let (r, g, b) = readback
        .get_rgb((r2.x + r2.width * 0.5) as u32, r2.y as u32)
        .unwrap();
    assert!(
        b > 180 && g > 140 && r < 100,
        "expected sky-blue focus ring pixel on Skia/GPU surface, got ({r}, {g}, {b})"
    );
    let caret_x = skia
        .display_list()
        .ops
        .iter()
        .find_map(|op| match op {
            PaintOp::FillRoundedRect { rect, color, .. }
                if (rect.width - 2.0).abs() < 0.01 && *color == ColorRgba::rgb(241, 245, 249) =>
            {
                Some(rect.x)
            }
            _ => None,
        })
        .expect("caret op in display list");
    let (cr, cg, cb) = readback
        .get_rgb(caret_x.round() as u32, (r2.y + r2.height * 0.5) as u32)
        .unwrap();
    assert!(
        cr > 200 && cg > 200 && cb > 200,
        "expected bright caret pixel on Skia/GPU surface at x={caret_x}, got ({cr}, {cg}, {cb})"
    );
}
