//! Deep end-to-end Web CSS tests for CSS Grid, responsive `@media` grid column switches,
//! `flex-basis`, `aspect-ratio`, `position: sticky/absolute`, RTL `dir="rtl"` + logical inline
//! spacing, `Transform2D` (including `:hover` patches), SSR, `render_html`, and hydration parity.

use nve_abi::Dimension;
use nve_components::layout_ext::{LayoutDirection, PositionMode};
use nve_components::style_patch::StylePatch;
use nve_components::transform::Transform2D;
#[cfg(feature = "ssr")]
use {
    nve_abi::{ComponentKind, LayoutProps, ROOT_NODE_ID},
    nve_backend_web::WebBackend,
    nve_components::interaction::{InteractionState, StateStyleRule},
    nve_components::layout_ext::{GridItemPlacement, GridTrack, GridTrackPlacement, LayoutExt},
    nve_components::media::{MediaQuery, ResponsiveRule},
    nve_components::style_patch::Display,
    nve_engine::{BatchSink, RenderBackend, RenderSink},
};

#[cfg(feature = "ssr")]
fn emit_showcase_frame<S: RenderSink>(sink: &mut S) {
    sink.begin_frame(1, 1);
    sink.create_node(2, ComponentKind::View, None);
    sink.create_node(3, ComponentKind::View, None);
    sink.create_node(4, ComponentKind::Pressable, None);
    sink.create_node(5, ComponentKind::View, None);
    sink.set_children(ROOT_NODE_ID, &[2]);
    sink.set_children(2, &[3, 4, 5]);
    let layout = LayoutProps::default();
    sink.set_layout(2, &layout);
    sink.set_layout(3, &layout);
    sink.set_layout(4, &layout);
    sink.set_layout(5, &layout);

    // Node 2: CSS Grid container with responsive @media 1-col -> 3-col switch
    let mut grid_ext = LayoutExt {
        display: Display::Grid,
        ..LayoutExt::default()
    };
    let grid = grid_ext.grid_mut();
    grid.template_columns = vec![GridTrack::repeat_minmax_zero_fr(1)];
    grid.template_rows = vec![GridTrack::px(48.0), GridTrack::auto()];
    grid.column_gap = Some(16.0);
    grid.row_gap = Some(12.0);

    // Node 3: sticky header spanning 2 grid columns
    let mut sticky_ext = LayoutExt {
        position: PositionMode::Sticky,
        grid_item: GridItemPlacement {
            column: GridTrackPlacement::span(2),
            row: GridTrackPlacement::AUTO,
        },
        ..LayoutExt::default()
    };
    sticky_ext.insets.top = Dimension::Points(16.0);

    // Node 4: flex-basis 50%, 16:9 aspect-ratio, base transform + :hover transform patch
    let card_ext = LayoutExt {
        flex_basis: Dimension::Percent(50.0),
        aspect_ratio: Some(16.0 / 9.0),
        ..LayoutExt::default()
    };
    let card_tf = Transform2D::translate(0.0, 4.0).with_origin(0.0, 0.5);

    // Node 5: RTL logical inline spacing bar
    let mut rtl_ext = LayoutExt {
        direction: Some(LayoutDirection::Rtl),
        ..LayoutExt::default()
    };
    rtl_ext.padding_inline.start = Some(24.0);
    rtl_ext.padding_inline.end = Some(12.0);
    rtl_ext.margin_inline.start = Some(8.0);
    rtl_ext.margin_inline.end = Some(4.0);

    sink.set_layout_ext(2, Some(&grid_ext));
    sink.set_layout_ext(3, Some(&sticky_ext));
    sink.set_layout_ext(4, Some(&card_ext));
    sink.set_layout_ext(5, Some(&rtl_ext));
    sink.set_transform(4, Some(card_tf));

    let media_rules = [ResponsiveRule::new(
        MediaQuery::expanded(),
        StylePatch::new().grid_cols([GridTrack::repeat_minmax_zero_fr(3)]),
    )];
    sink.set_responsive_rules(2, &media_rules);

    let hover_rules = [StateStyleRule::new(
        InteractionState::HOVER,
        StylePatch::new().translate(0.0, -4.0).scale(1.03).rotate(1.5),
    )];
    sink.set_state_rules(4, &hover_rules);
    sink.commit_frame(true).unwrap();
}

#[cfg(feature = "ssr")]
#[test]
fn web_emits_native_css_grid_media_switch_sticky_rtl_and_hover_transform() {
    // 0. Encode via BatchSink and submit through the binary wire path
    let mut batch = BatchSink::new(1);
    emit_showcase_frame(&mut batch);
    let bytes = batch.into_bytes();

    let mut web = WebBackend::new();
    web.submit_frame(&bytes).unwrap();

    // 1. SSR output
    let ssr = web.render_ssr();
    assert!(
        ssr.style_css.contains("display:grid;"),
        "expected display:grid in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css
            .contains("grid-template-columns:repeat(1,minmax(0,1fr));"),
        "expected 1-col base grid in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("grid-template-rows:48px auto;"),
        "expected grid-template-rows in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("column-gap:16px;row-gap:12px;"),
        "expected column-gap/row-gap in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("@media (min-width:1024px){")
            && ssr
                .style_css
                .contains("grid-template-columns:repeat(3,minmax(0,1fr));"),
        "expected responsive @media 3-col switch in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("position:sticky;top:16px;")
            && ssr.style_css.contains("grid-column:span 2;"),
        "expected sticky header + grid-column span 2 in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("flex-basis:50%;")
            && ssr.style_css.contains("aspect-ratio:1.778;")
            && ssr.style_css.contains("transform:translate(0,4px);")
            && ssr.style_css.contains("transform-origin:0% 50%;"),
        "expected flex-basis/aspect-ratio/base transform in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("@media (hover:hover){")
            && ssr
                .style_css
                .contains("transform:translate(0,-4px) scale(1.03) rotate(1.5deg);"),
        "expected :hover transform patch in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains(
            "padding-inline-start:24px;padding-inline-end:12px;margin-inline-start:8px;margin-inline-end:4px;"
        ),
        "expected logical inline padding/margin in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.html.contains("dir=\"rtl\""),
        "expected dir=\"rtl\" attribute in SSR HTML:\n{}",
        ssr.html
    );

    // 2. render_html parity
    let html = web.render_html();
    assert!(html.contains("display:grid;"));
    assert!(html.contains("position:sticky;top:16px;"));
    assert!(html.contains("dir=\"rtl\""));
    assert!(html.contains("padding-inline-start:24px;"));
    assert!(html.contains("transform:translate(0,4px);"));

    // 3. Zero-mismatch hydration parity (both via BatchSink wire path and direct RenderSink)
    #[cfg(feature = "hydrate")]
    {
        let mut hydrated_batch = WebBackend::hydrate_from_ssr(&ssr);
        hydrated_batch.submit_frame(&bytes).unwrap();
        assert!(hydrated_batch.hydration_report().mismatches.is_empty());
        assert!(hydrated_batch.hydration_report().adopted_nodes >= 4);
        assert_eq!(hydrated_batch.stylesheet().flushed_css_rules().len(), 0);

        let mut hydrated_direct = WebBackend::hydrate_from_ssr(&ssr);
        emit_showcase_frame(&mut hydrated_direct);
        assert!(hydrated_direct.hydration_report().mismatches.is_empty());
        assert!(hydrated_direct.hydration_report().adopted_nodes >= 4);
        assert_eq!(hydrated_direct.stylesheet().flushed_css_rules().len(), 0);
    }
}

#[test]
fn web_style_patch_resets_transform_with_none_and_supports_absolute_insets() {
    let mut decls = String::new();
    let patch = StylePatch::new()
        .position(PositionMode::Absolute)
        .inset_right(Dimension::Points(12.0))
        .inset_bottom(Dimension::Percent(10.0))
        .direction(LayoutDirection::Rtl)
        .transform(Transform2D::IDENTITY);
    nve_backend_web::write_style_patch_decls(&patch, &mut decls);
    assert_eq!(
        decls,
        "position:absolute;right:12px;bottom:10%;direction:rtl;transform:none;"
    );
}
