use nve_abi::ColorRgba;
use nve_components::paint::{BackgroundFill, GradientStop};
use nve_components::tables::{
    BackgroundFillRes, BoxShadows, Layout, Scroll, TextContent, Typography, Visual,
};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

use nve_components::tables::parse_box_shadow;

use crate::{PaintOp, PixelBuffer, SkiaBackend};

#[test]
fn skia_commit_display_list_gradients_shadows_scroll_and_rasterize() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 320.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let mut tx = store.tx();
    let card = tx.create();
    let label = tx.create();
    let scroller = tx.create();
    let tall = tx.create();
    tx.set_children(NodeId::ROOT, &[card, scroller]);
    tx.set_children(card, &[label]);
    tx.set_children(scroller, &[tall]);

    tx.edit::<Layout>(card)
        .width(Length::points(280.0))
        .height(Length::points(80.0))
        .margin_left(Length::points(16.0))
        .margin_top(Length::points(12.0));
    tx.edit::<Visual>(card)
        .border_radius(10.0)
        .border_width(2.0)
        .border_color(Rgba::rgb(56, 189, 248))
        .box_shadows(Some(BoxShadows::new(parse_box_shadow(
            "0 4px 12px rgba(0,0,0,0.4)",
        ))))
        .background_fill(Some(BackgroundFillRes(BackgroundFill::linear(
            90.0,
            vec![
                GradientStop::new(0.0, ColorRgba::rgb(37, 99, 235)),
                GradientStop::new(1.0, ColorRgba::rgb(124, 58, 237)),
            ],
        ))));
    tx.edit::<TextContent>(label).text("Hello Skia NVE2");
    tx.edit::<Typography>(label)
        .color(Rgba::WHITE)
        .font_size(18.0)
        .font_weight(700);

    tx.edit::<Layout>(scroller)
        .width(Length::points(280.0))
        .height(Length::points(90.0));
    tx.edit::<Scroll>(scroller).axes(nve_components::tables::ScrollAxes::Vertical);
    tx.edit::<Layout>(tall)
        .flex_shrink(0.0)
        .height(Length::points(300.0));
    tx.edit::<Visual>(tall).background(Rgba::rgb(16, 185, 129));
    tx.commit_into(&mut changes);

    backend.commit(&store, &changes).unwrap();

    assert!(backend
        .display_list()
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::DrawShadow { .. })));
    assert!(backend
        .display_list()
        .ops
        .iter()
        .any(|op| matches!(op, PaintOp::FillRoundedRectGradient { .. })));

    let mut buf = PixelBuffer::new(320, 200);
    backend.rasterize_with_store(&store, &mut buf, ColorRgba::rgb(11, 17, 32));
    // Gradient card interior pixel at (40, 40) is non-background.
    assert_ne!(buf.get_rgb(40, 40), Some((11, 17, 32)));

    // Scroll clamping works on Scroll table nodes.
    let (_, sy) = backend.set_scroll_offset(&store, scroller, 0.0, 999.0);
    assert!((sy - 210.0).abs() < 1.0, "expected clamped ~210, got {sy}");
}

#[test]
fn skia_hit_test_recurses_into_visible_child_under_visibility_hidden_parent() {
    use nve_abi::EventMask;
    use nve_components::tables::{Interaction, ScrollAxes, Visibility};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 300.0,
            scale_factor: 1.0,
        },
    );

    let (hidden_parent, inherit_child, visible_child) = {
        let mut tx = store.tx();
        let p = tx.create();
        let inh = tx.create();
        let vis = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[inh, vis]);
        tx.edit::<Layout>(p)
            .width(Length::points(200.0))
            .height(Length::points(200.0));
        tx.edit::<Visual>(p)
            .visibility(Visibility::Hidden)
            .background(Rgba::rgb(255, 0, 0));
        tx.edit::<Interaction>(p).event_mask(EventMask::PRESS as u32);
        // Ordinary `Inherit` child under Hidden parent: neither painted nor hit-testable (including scroll & interaction).
        tx.edit::<Layout>(inh)
            .width(Length::points(80.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(inh).background(Rgba::rgb(0, 0, 255));
        tx.edit::<Scroll>(inh).axes(ScrollAxes::Vertical);
        tx.edit::<Interaction>(inh).event_mask(EventMask::PRESS as u32);
        // Explicit `Visible` child under Hidden parent: painted, hit-testable, interaction & scroll eligible.
        tx.edit::<Layout>(vis)
            .width(Length::points(80.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(vis)
            .visibility(Visibility::Visible)
            .background(Rgba::rgb(0, 255, 0));
        tx.edit::<Scroll>(vis).axes(ScrollAxes::Vertical);
        tx.edit::<Interaction>(vis).event_mask(EventMask::PRESS as u32);
        tx.commit_into(&mut changes);
        (p, inh, vis)
    };
    backend.commit(&store, &changes).unwrap();

    // DisplayList must NOT paint hidden_parent (red) or inherit_child (blue), and MUST paint visible_child (green).
    let painted_colors: Vec<ColorRgba> = backend
        .display_list()
        .ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::FillRoundedRect { color, .. } => Some(*color),
            _ => None,
        })
        .collect();
    assert!(!painted_colors.contains(&ColorRgba::rgb(255, 0, 0)), "hidden parent must not paint");
    assert!(!painted_colors.contains(&ColorRgba::rgb(0, 0, 255)), "inherit child under hidden parent must not paint");
    assert!(painted_colors.contains(&ColorRgba::rgb(0, 255, 0)), "explicit Visible child under hidden parent must paint");

    let inh_r = backend.layout_of(inherit_child.raw()).unwrap();
    let vis_r = backend.layout_of(visible_child.raw()).unwrap();
    assert_eq!(backend.hit_test(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    assert_eq!(backend.hit_test_interaction(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    assert_eq!(backend.hit_test_scroll_container(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    assert_eq!(backend.hit_test(&store, vis_r.x + 10.0, vis_r.y + 10.0), Some(visible_child.raw()));
    assert_eq!(
        backend.hit_test_interaction(&store, vis_r.x + 10.0, vis_r.y + 10.0),
        Some(visible_child.raw())
    );
    assert_eq!(
        backend.hit_test_scroll_container(&store, vis_r.x + 10.0, vis_r.y + 10.0),
        Some(visible_child.raw())
    );
    assert_eq!(backend.hit_test(&store, 190.0, 190.0), None);
    let _ = hidden_parent;
}
