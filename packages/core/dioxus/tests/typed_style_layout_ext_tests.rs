//! Integration tests for Dioxus `TypedStyle` & `StylePatch` layout-extension, CSS Grid,
//! RTL logical edges, sticky positioning, and `Transform2D` propagation into `DioxusSurface`.

use nve_abi::Dimension;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;

fn layout_showcase_app() -> Element {
    let grid_style = TypedStyle::new()
        .display_grid()
        .grid_cols([GridTrack::repeat_minmax_zero_fr(1)])
        .grid_rows([GridTrack::px(48.0), GridTrack::auto()])
        .column_gap(16.0)
        .row_gap(12.0)
        .at(
            MediaQuery::expanded(),
            StylePatch::new().grid_cols([GridTrack::repeat_minmax_zero_fr(3)]),
        );

    let sticky_hdr = TypedStyle::new()
        .position_sticky()
        .inset_top_px(16.0)
        .grid_column_span(2);

    let card_style = TypedStyle::new()
        .flex_basis_pct(50.0)
        .aspect_ratio(16.0 / 9.0)
        .transform(Transform2D::translate(0.0, 4.0).with_origin(0.0, 0.5))
        .on_hover(StylePatch::new().translate(0.0, -4.0).scale(1.03).rotate(1.5));

    let rtl_bar = TypedStyle::new()
        .rtl()
        .flex_row()
        .padding_inline_start(24.0)
        .padding_inline_end(12.0)
        .margin_inline_start(8.0)
        .margin_inline_end(4.0);

    rsx! {
        view {
            nve_style: grid_style,
            view { nve_style: sticky_hdr, text { "Sticky Header" } }
            pressable { nve_style: card_style, text { "Interactive Card" } }
            view { nve_style: rtl_bar, text { "مرحبا" } }
        }
    }
}

#[test]
fn dioxus_surface_propagates_grid_sticky_rtl_and_transform_metadata() {
    let mut surface = DioxusSurface::new(1, layout_showcase_app);
    let _frame = surface.render_frame_slice(1);

    let exts: Vec<(u32, LayoutExt)> = surface
        .iter_layout_ext()
        .map(|(id, ext)| (id, ext.clone()))
        .collect();
    assert_eq!(exts.len(), 4, "expected 4 nodes with LayoutExt metadata");

    let has_grid = exts.iter().any(|(_, e)| {
        e.display == Display::Grid
            && e.grid
                .as_ref()
                .is_some_and(|g| g.column_gap == Some(16.0) && g.row_gap == Some(12.0))
    });
    assert!(has_grid, "grid container LayoutExt missing");

    let has_sticky = exts.iter().any(|(_, e)| {
        e.position == PositionMode::Sticky
            && e.insets.top == Dimension::Points(16.0)
            && e.grid_item.column == GridTrackPlacement::span(2)
    });
    assert!(has_sticky, "sticky header LayoutExt missing");

    let has_card_ext = exts.iter().any(|(_, e)| {
        e.flex_basis == Dimension::Percent(50.0)
            && e.aspect_ratio
                .is_some_and(|r| (r - 16.0 / 9.0).abs() < 1e-5)
    });
    assert!(has_card_ext, "card flex_basis/aspect_ratio LayoutExt missing");

    let has_rtl = exts.iter().any(|(_, e)| {
        e.direction == Some(LayoutDirection::Rtl)
            && e.padding_inline.start == Some(24.0)
            && e.padding_inline.end == Some(12.0)
            && e.margin_inline.start == Some(8.0)
            && e.margin_inline.end == Some(4.0)
    });
    assert!(has_rtl, "RTL logical edges LayoutExt missing");

    let transforms: Vec<(u32, Transform2D)> = surface.iter_transforms().collect();
    assert_eq!(transforms.len(), 1);
    assert_eq!(
        transforms[0].1,
        Transform2D::translate(0.0, 4.0).with_origin(0.0, 0.5)
    );

    let media_rules: Vec<_> = surface.iter_responsive_rules().collect();
    assert_eq!(media_rules.len(), 1);
    assert!(media_rules[0].1[0].patch.grid_template_columns.is_some());

    let hover_rules: Vec<_> = surface.iter_state_rules().collect();
    assert_eq!(hover_rules.len(), 1);
    let hover_tf = hover_rules[0].1[0].patch.transform.unwrap();
    assert_eq!(hover_tf.translate_y, -4.0);
    assert_eq!(hover_tf.scale_x, 1.03);
    assert_eq!(hover_tf.rotate_deg, 1.5);
}
