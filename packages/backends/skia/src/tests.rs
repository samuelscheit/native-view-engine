use nve_abi::{ColorRgba, SurfaceId};
use nve_tables::paint::{BackgroundFill, GradientStop};
use nve_tables::tables::{
    BackgroundFillRes, BoxShadows, Layout, Scroll, TextContent, Typography, Visual,
};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

use nve_tables::tables::BoxShadow;

use crate::{PaintOp, PixelBuffer, SkiaBackend};

#[test]
fn skia_commit_display_list_gradients_shadows_scroll_and_rasterize() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
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
        .box_shadows(Some(BoxShadows::new(vec![BoxShadow::new(
            0.0,
            4.0,
            12.0,
            Rgba::new(0, 0, 0, 102),
        )])))
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
    tx.edit::<Scroll>(scroller).axes(nve_tables::tables::ScrollAxes::Vertical);
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
    use nve_abi::InteractionEvent;
    use nve_tables::tables::{Interaction, ScrollAxes, Visibility};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
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
        tx.edit::<Interaction>(p).listen(InteractionEvent::Press);
        // Ordinary `Inherit` child under Hidden parent: neither painted nor hit-testable (including scroll & interaction).
        tx.edit::<Layout>(inh)
            .width(Length::points(80.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(inh).background(Rgba::rgb(0, 0, 255));
        tx.edit::<Scroll>(inh).axes(ScrollAxes::Vertical);
        tx.edit::<Interaction>(inh).listen(InteractionEvent::Press);
        // Explicit `Visible` child under Hidden parent: painted, hit-testable, interaction & scroll eligible.
        tx.edit::<Layout>(vis)
            .width(Length::points(80.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(vis)
            .visibility(Visibility::Visible)
            .background(Rgba::rgb(0, 255, 0));
        tx.edit::<Scroll>(vis).axes(ScrollAxes::Vertical);
        tx.edit::<Interaction>(vis).listen(InteractionEvent::Press);
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

    let inh_r = backend.layout_of(inherit_child).unwrap();
    let vis_r = backend.layout_of(visible_child).unwrap();
    assert_eq!(backend.hit_test(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    assert_eq!(backend.hit_test_interaction(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    assert_eq!(backend.hit_test_scroll_container(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    assert_eq!(backend.hit_test(&store, vis_r.x + 10.0, vis_r.y + 10.0), Some(visible_child));
    assert_eq!(
        backend.hit_test_interaction(&store, vis_r.x + 10.0, vis_r.y + 10.0),
        Some(visible_child)
    );
    assert_eq!(
        backend.hit_test_scroll_container(&store, vis_r.x + 10.0, vis_r.y + 10.0),
        Some(visible_child)
    );
    assert_eq!(backend.hit_test(&store, 190.0, 190.0), None);
    let _ = hidden_parent;
}

#[test]
fn skia_inherited_typography_explicit_defaults_and_no_inherit_fields() {
    use core::num::NonZeroU16;
    use nve_tables::tables::VerticalAlign;
    use nve_store::ResolvedRow;

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let (parent, child_inherit, child_reset) = {
        let mut tx = store.tx();
        let p = tx.create();
        let c1 = tx.create();
        let c2 = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[c1, c2]);
        tx.edit::<Layout>(p)
            .width(Length::points(280.0))
            .height(Length::points(120.0));
        tx.edit::<Typography>(p)
            .color(Rgba::WHITE)
            .font_weight(700)
            .font_size(22.0)
            .max_lines(NonZeroU16::new(2))
            .vertical_align(VerticalAlign::Middle);

        tx.edit::<TextContent>(c1).text("inherits");
        tx.edit::<TextContent>(c2).text("resets");
        // Explicitly write table defaults BLACK + 400 to override WHITE + 700 ancestor.
        tx.edit::<Typography>(c2)
            .color(Rgba::BLACK)
            .font_weight(400);
        tx.commit_into(&mut changes);
        (p, c1, c2)
    };

    backend.commit(&store, &changes).unwrap();
    let mut seen_inherits = None;
    let mut seen_resets = None;
    for op in &backend.display_list().ops {
        if let PaintOp::DrawText {
            text,
            font_size,
            font_weight,
            color,
            ..
        } = op
        {
            if text == "inherits" {
                seen_inherits = Some((*font_size, *font_weight, *color));
            } else if text == "resets" {
                seen_resets = Some((*font_size, *font_weight, *color));
            }
        }
    }
    assert_eq!(
        seen_inherits,
        Some((22.0, 700, ColorRgba::from_u32(Rgba::WHITE.0)))
    );
    assert_eq!(
        seen_resets,
        Some((22.0, 400, ColorRgba::from_u32(Rgba::BLACK.0)))
    );

    // Non-inherited CSS fields (`max_lines`, `vertical_align`) apply to parent only, never children.
    let root_r = ResolvedRow::<Typography>::root(&store);
    let p_r = ResolvedRow::<Typography>::resolve(&store, parent, &root_r);
    let c1_r = ResolvedRow::<Typography>::resolve(&store, child_inherit, &p_r);
    let c2_r = ResolvedRow::<Typography>::resolve(&store, child_reset, &p_r);
    assert_eq!(p_r.get().max_lines(), NonZeroU16::new(2));
    assert_eq!(p_r.get().vertical_align(), VerticalAlign::Middle);
    assert_eq!(c1_r.get().max_lines(), None);
    assert_eq!(c1_r.get().vertical_align(), VerticalAlign::Baseline);
    assert!(!c1_r.get().is_max_lines_explicit());
    assert!(!c1_r.get().is_vertical_align_explicit());
    assert_eq!(c2_r.get().max_lines(), None);
    assert_eq!(c2_r.get().vertical_align(), VerticalAlign::Baseline);
    assert!(!c2_r.get().is_max_lines_explicit());
    assert!(!c2_r.get().is_vertical_align_explicit());
}
