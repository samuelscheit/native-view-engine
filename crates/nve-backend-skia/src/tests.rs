use std::sync::Arc;

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, FlexDirection,
    LayoutProps, StyleProps, ROOT_NODE_ID,
};
use nve_engine::{LayoutEngine, LayoutRect, RenderBackend, TextMeasurer, Viewport};

use crate::{
    rasterize_display_list, BitmapFontTextMeasurer, DisplayList, PaintOp, PixelBuffer, SkiaBackend,
    TaffyLayoutEngine,
};

#[test]
fn emits_retained_display_list_from_command_buffer() {
    let mut skia = SkiaBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(200.0),
            height: Dimension::Points(80.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(30, 41, 59),
            border_radius: 12.0,
            ..Default::default()
        },
    );
    b.create_node(3, ComponentKind::Text, None);
    b.set_text(3, "Skia Ready");
    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);

    skia.submit_frame(b.finish()).unwrap();
    assert_eq!(skia.display_list().ops.len(), 2);
}

#[test]
fn culls_zero_opacity_empty_opacity_groups_and_offscreen_nodes() {
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
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(80.0),
            height: Dimension::Points(40.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(255, 0, 0),
            opacity: 0.0,
            ..Default::default()
        },
    );
    b.create_node(3, ComponentKind::Text, None);
    b.set_text(3, "Hidden");
    b.set_children(2, &[3]);

    b.create_node(4, ComponentKind::View, None);
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(80.0),
            height: Dimension::Points(40.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.set_style(
        4,
        &StyleProps {
            background: ColorRgba::TRANSPARENT,
            opacity: 0.5,
            ..Default::default()
        },
    );

    b.create_node(5, ComponentKind::View, None);
    b.set_layout(
        5,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(50.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.set_style(
        5,
        &StyleProps {
            background: ColorRgba::rgb(16, 185, 129),
            ..Default::default()
        },
    );

    b.create_node(6, ComponentKind::View, None);
    b.set_layout(
        6,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(50.0),
            flex_shrink: 0.0,
            margin: EdgeInsets {
                top: 250.0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    b.set_style(
        6,
        &StyleProps {
            background: ColorRgba::rgb(239, 68, 68),
            ..Default::default()
        },
    );

    b.set_children(ROOT_NODE_ID, &[2, 4, 5, 6]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    assert_eq!(skia.display_list().ops.len(), 1);
    match &skia.display_list().ops[0] {
        PaintOp::FillRoundedRect { color, .. } => {
            assert_eq!(*color, ColorRgba::rgb(16, 185, 129));
        }
        other => panic!("unexpected op: {other:?}"),
    }
}

#[test]
fn rasterizes_rounded_rect_corners_stroke_and_opacity_stack() {
    let list = DisplayList {
        ops: vec![
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 20.0,
                    y: 20.0,
                    width: 80.0,
                    height: 80.0,
                },
                radius: 20.0,
                color: ColorRgba::rgb(37, 99, 235),
            },
            PaintOp::StrokeRoundedRect {
                rect: LayoutRect {
                    x: 120.0,
                    y: 20.0,
                    width: 60.0,
                    height: 60.0,
                },
                radius: 8.0,
                width: 2.0,
                color: ColorRgba::WHITE,
            },
            PaintOp::SaveOpacity(0.5),
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 20.0,
                    y: 120.0,
                    width: 40.0,
                    height: 40.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(255, 0, 0),
            },
            PaintOp::Restore,
            PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: 80.0,
                    y: 120.0,
                    width: 40.0,
                    height: 40.0,
                },
                radius: 0.0,
                color: ColorRgba::rgb(0, 255, 0),
            },
        ],
    };

    let mut buf = PixelBuffer::new(200, 180);
    rasterize_display_list(&list, &mut buf, 1.0, ColorRgba::BLACK);

    assert_eq!(buf.get_rgb(60, 60), Some((37, 99, 235)));
    assert_eq!(buf.get_rgb(20, 20), Some((0, 0, 0)));
    assert_eq!(buf.get_rgb(150, 20), Some((255, 255, 255)));
    assert_eq!(buf.get_rgb(150, 50), Some((0, 0, 0)));

    let (r, g, b) = buf.get_rgb(40, 140).unwrap();
    assert!((126..=130).contains(&r), "expected ~128 red, got {r}");
    assert_eq!((g, b), (0, 0));
    assert_eq!(buf.get_rgb(100, 140), Some((0, 255, 0)));
}

#[test]
fn rasterizes_full_ui_tree_with_text_and_expected_color_placement() {
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
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Percent(100.0),
            padding: EdgeInsets::all(16.0),
            gap: 12.0,
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

    b.create_node(3, ComponentKind::Text, None);
    b.set_style(
        3,
        &StyleProps {
            text_color: ColorRgba::rgb(56, 189, 248),
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.set_text(3, "HELLO NVE");

    b.create_node(4, ComponentKind::Pressable, None);
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(140.0),
            height: Dimension::Points(48.0),
            ..Default::default()
        },
    );
    b.set_style(
        4,
        &StyleProps {
            background: ColorRgba::rgb(37, 99, 235),
            border_radius: 10.0,
            ..Default::default()
        },
    );

    b.set_children(2, &[3, 4]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);

    skia.submit_frame(b.finish()).unwrap();

    let mut buf = PixelBuffer::new(320, 200);
    skia.rasterize(&mut buf, ColorRgba::BLACK);

    assert_eq!(buf.get_rgb(4, 4), Some((15, 23, 42)));

    let text_rect = skia.layout_of(3).unwrap();
    let mut cyan_pixels = 0usize;
    for y in (text_rect.y as u32)..((text_rect.y + text_rect.height) as u32).min(200) {
        for x in (text_rect.x as u32)..((text_rect.x + text_rect.width) as u32).min(320) {
            if buf.get_rgb(x, y) == Some((56, 189, 248)) {
                cyan_pixels += 1;
            }
        }
    }
    assert!(
        cyan_pixels > 40,
        "expected crisp glyph pixels for 'HELLO NVE', found {cyan_pixels}"
    );

    let btn_rect = skia.layout_of(4).unwrap();
    let bx = (btn_rect.x + btn_rect.width * 0.5) as u32;
    let by = (btn_rect.y + btn_rect.height * 0.5) as u32;
    assert_eq!(buf.get_rgb(bx, by), Some((37, 99, 235)));
}

#[test]
fn sorts_children_by_z_index_in_display_list() {
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
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(50.0),
            height: Dimension::Points(50.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(255, 0, 0),
            z_index: 10,
            ..Default::default()
        },
    );
    b.create_node(3, ComponentKind::View, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(50.0),
            height: Dimension::Points(50.0),
            ..Default::default()
        },
    );
    b.set_style(
        3,
        &StyleProps {
            background: ColorRgba::rgb(0, 0, 255),
            z_index: 1,
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    assert_eq!(skia.display_list().ops.len(), 2);
    match (&skia.display_list().ops[0], &skia.display_list().ops[1]) {
        (
            PaintOp::FillRoundedRect { color: c0, .. },
            PaintOp::FillRoundedRect { color: c1, .. },
        ) => {
            assert_eq!(*c0, ColorRgba::rgb(0, 0, 255));
            assert_eq!(*c1, ColorRgba::rgb(255, 0, 0));
        }
        other => panic!("unexpected ops: {other:?}"),
    }
}

struct FixedOffsetLayout {
    x_offset: f32,
    y_step: f32,
    children: std::collections::HashMap<nve_abi::NodeId, Vec<nve_abi::NodeId>>,
    kinds: std::collections::HashMap<nve_abi::NodeId, ComponentKind>,
    texts: std::collections::HashMap<nve_abi::NodeId, String>,
    font_sizes: std::collections::HashMap<nve_abi::NodeId, f32>,
    rects: std::collections::HashMap<nve_abi::NodeId, LayoutRect>,
    measurer: Arc<dyn TextMeasurer>,
}

impl FixedOffsetLayout {
    fn new(x_offset: f32, y_step: f32) -> Self {
        Self {
            x_offset,
            y_step,
            children: std::collections::HashMap::new(),
            kinds: std::collections::HashMap::new(),
            texts: std::collections::HashMap::new(),
            font_sizes: std::collections::HashMap::new(),
            rects: std::collections::HashMap::new(),
            measurer: Arc::new(BitmapFontTextMeasurer),
        }
    }

    fn place(&mut self, id: nve_abi::NodeId, depth: usize, slot: usize) {
        let is_text = self.kinds.get(&id) == Some(&ComponentKind::Text);
        let (w, h) = if is_text {
            let text = self.texts.get(&id).map(String::as_str).unwrap_or("");
            let fs = self.font_sizes.get(&id).copied().unwrap_or(16.0);
            self.measurer.measure(text, fs, None)
        } else {
            (90.0, 36.0)
        };
        self.rects.insert(
            id,
            LayoutRect {
                x: self.x_offset + (depth as f32) * 10.0,
                y: (slot as f32) * self.y_step,
                width: w,
                height: h,
            },
        );
        if let Some(kids) = self.children.get(&id).cloned() {
            for (i, child) in kids.into_iter().enumerate() {
                self.place(child, depth + 1, slot + i + 1);
            }
        }
    }
}

impl LayoutEngine for FixedOffsetLayout {
    fn name(&self) -> &'static str {
        "fixed-offset"
    }

    fn upsert_node(&mut self, id: nve_abi::NodeId, kind: ComponentKind) {
        self.kinds.insert(id, kind);
    }

    fn remove_node(&mut self, id: nve_abi::NodeId) {
        self.kinds.remove(&id);
        self.children.remove(&id);
        self.texts.remove(&id);
        self.font_sizes.remove(&id);
        self.rects.remove(&id);
    }

    fn set_layout(&mut self, _id: nve_abi::NodeId, _layout: &LayoutProps) {}

    fn set_style(&mut self, id: nve_abi::NodeId, style: &StyleProps) {
        self.font_sizes.insert(id, style.font_size);
    }

    fn set_text(&mut self, id: nve_abi::NodeId, text: &str) {
        self.texts.insert(id, text.to_owned());
    }

    fn set_children(&mut self, parent: nve_abi::NodeId, children: &[nve_abi::NodeId]) {
        self.children.insert(parent, children.to_vec());
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.measurer = measurer;
    }

    fn compute_layout(&mut self, root: nve_abi::NodeId, viewport: Viewport) {
        self.rects.clear();
        self.place(root, 0, 0);
        if let Some(r) = self.rects.get_mut(&root) {
            r.x = 0.0;
            r.y = 0.0;
            r.width = viewport.width;
            r.height = viewport.height;
        }
    }

    fn layout_of(&self, id: nve_abi::NodeId) -> Option<LayoutRect> {
        self.rects.get(&id).copied()
    }

    fn reset(&mut self) {
        self.children.clear();
        self.kinds.clear();
        self.texts.clear();
        self.font_sizes.clear();
        self.rects.clear();
    }
}

#[test]
fn skia_supports_custom_layout_dyn_box_swap_and_custom_text_measurer() {
    use nve_abi::{EventMask, FlexAlign};
    use nve_engine::DynLayoutEngine;

    let viewport = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(50.0),
            align_items: FlexAlign::FlexStart,
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
            background: ColorRgba::rgb(37, 99, 235),
            ..Default::default()
        },
    );
    b.set_event_mask(2, EventMask::PRESS);

    b.create_node(3, ComponentKind::Text, None);
    b.set_style(
        3,
        &StyleProps {
            text_color: ColorRgba::WHITE,
            font_size: 16.0,
            ..Default::default()
        },
    );
    b.set_text(3, "Swap");

    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    let cmd = b.into_bytes();

    let mut skia: SkiaBackend<DynLayoutEngine> =
        SkiaBackend::with_layout(Box::new(TaffyLayoutEngine::new()) as DynLayoutEngine);
    skia.set_viewport(1, viewport);
    skia.set_text_measurer(Arc::new(|_text: &str, _fs: f32, _mw: Option<f32>| {
        (77.0, 29.0)
    }));
    skia.submit_frame(&cmd).unwrap();

    assert_eq!(skia.layout_engine().name(), "taffy");
    assert_eq!(skia.layout_of(2).unwrap().x, 10.0);
    assert_eq!(skia.layout_of(3).unwrap().width, 77.0);
    assert_eq!(skia.layout_of(3).unwrap().height, 29.0);
    assert_eq!(skia.hit_test(50.0, 30.0), Some(2));

    let swapped = skia.swap_layout_engine(FixedOffsetLayout::new(120.0, 40.0));
    assert_eq!(swapped.layout_engine().name(), "fixed-offset");
    let r2 = swapped.layout_of(2).unwrap();
    assert_eq!(r2.x, 130.0);
    assert_eq!(r2.y, 40.0);
    assert_eq!(swapped.layout_of(3).unwrap().width, 77.0);
    assert_eq!(swapped.layout_of(3).unwrap().height, 29.0);
    assert_eq!(swapped.hit_test(50.0, 30.0), None);
    assert_eq!(swapped.hit_test(150.0, 50.0), Some(2));

    let mut buf = PixelBuffer::new(300, 200);
    swapped.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(50, 30), Some((0, 0, 0)));
    assert_eq!(buf.get_rgb(150, 55), Some((37, 99, 235)));
}

#[cfg(feature = "yoga")]
#[test]
fn skia_new_yoga_matches_availability_and_renders_with_real_rn_yoga() {
    use crate::YogaLayoutEngine;

    if !YogaLayoutEngine::is_available() {
        let err = SkiaBackend::new_yoga()
            .err()
            .expect("expected YogaUnavailableError when RN_REACT_COMMON_DIR is unset");
        assert!(err.to_string().contains("RN_REACT_COMMON_DIR"));
        return;
    }

    let viewport = Viewport {
        width: 320.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Percent(100.0),
            padding: EdgeInsets::all(16.0),
            gap: 12.0,
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
    b.create_node(3, ComponentKind::Pressable, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(140.0),
            height: Dimension::Points(48.0),
            ..Default::default()
        },
    );
    b.set_style(
        3,
        &StyleProps {
            background: ColorRgba::rgb(37, 99, 235),
            border_radius: 10.0,
            ..Default::default()
        },
    );
    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    let cmd = b.into_bytes();

    let mut taffy_skia = SkiaBackend::new();
    taffy_skia.set_viewport(1, viewport);
    taffy_skia.submit_frame(&cmd).unwrap();

    let mut yoga_skia = SkiaBackend::new_yoga().expect("real RN Yoga should initialize");
    assert_eq!(yoga_skia.layout_engine().name(), "yoga");
    yoga_skia.set_viewport(1, viewport);
    yoga_skia.submit_frame(&cmd).unwrap();

    for id in [ROOT_NODE_ID, 2, 3] {
        let tr = taffy_skia.layout_of(id).unwrap();
        let yr = yoga_skia.layout_of(id).unwrap();
        assert!((tr.x - yr.x).abs() <= 1.0, "node {id} x: {tr:?} vs {yr:?}");
        assert!((tr.y - yr.y).abs() <= 1.0, "node {id} y: {tr:?} vs {yr:?}");
        assert!((tr.width - yr.width).abs() <= 1.0, "node {id} w: {tr:?} vs {yr:?}");
        assert!((tr.height - yr.height).abs() <= 1.0, "node {id} h: {tr:?} vs {yr:?}");
    }

    let mut buf = PixelBuffer::new(320, 200);
    yoga_skia.rasterize(&mut buf, ColorRgba::BLACK);
    let btn = yoga_skia.layout_of(3).unwrap();
    assert_eq!(
        buf.get_rgb((btn.x + 20.0) as u32, (btn.y + 20.0) as u32),
        Some((37, 99, 235))
    );
}
