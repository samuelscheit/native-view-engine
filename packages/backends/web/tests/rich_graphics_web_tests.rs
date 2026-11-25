//! Deep integration tests for zero-serialization typed CSS gradients (base, `@media`, and
//! `:hover`/`:active`) and inline `<svg>` SSR, `render_html`, RTL mirror policy, and
//! zero-mismatch hydration parity in `nve-backend-web`.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, LayoutProps, StyleProps,
    ROOT_NODE_ID,
};
use nve_backend_web::WebBackend;
use nve_components::interaction::{InteractionState, StateStyleRule};
use nve_components::layout_ext::{LayoutDirection, LayoutExt};
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::paint::{BackgroundFill, GradientStop};
use nve_components::style_patch::StylePatch;
use nve_components::svg::SvgPicture;
use nve_engine::RenderBackend;

fn build_hero_card_frame() -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(320.0),
            height: Dimension::Points(180.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            border_radius: 16.0,
            ..Default::default()
        },
    );

    b.create_node(3, ComponentKind::Custom, Some("svg"));
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(24.0),
            height: Dimension::Points(24.0),
            ..Default::default()
        },
    );
    b.set_style(
        3,
        &StyleProps {
            text_color: ColorRgba::rgb(248, 250, 252),
            ..Default::default()
        },
    );
    b.set_raw_prop_str(3, "data-nve-mirror", "directional");

    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    b.finish().to_vec()
}

fn sync_hero_typed_metadata(web: &mut WebBackend) {
    let base_grad = BackgroundFill::linear(
        135.0,
        [
            GradientStop::new(0.0, ColorRgba::rgb(37, 99, 235)),
            GradientStop::new(1.0, ColorRgba::rgb(124, 58, 237)),
        ],
    );
    let hover_rule = StateStyleRule::new(
        InteractionState::HOVER,
        StylePatch::new().linear_gradient(
            90.0,
            [
                GradientStop::new(0.0, ColorRgba::rgb(14, 165, 233)),
                GradientStop::new(1.0, ColorRgba::rgb(99, 102, 241)),
            ],
        ),
    );
    let active_rule = StateStyleRule::new(
        InteractionState::ACTIVE,
        StylePatch::new().background_fill(BackgroundFill::Solid(ColorRgba::rgb(30, 41, 59))),
    );
    let media_rule = ResponsiveRule::new(
        MediaQuery::min_width(768),
        StylePatch::new().radial_gradient(
            0.5,
            0.25,
            1.0,
            [
                GradientStop::new(0.0, ColorRgba::rgb(56, 189, 248)),
                GradientStop::new(1.0, ColorRgba::rgb(15, 23, 42)),
            ],
        ),
    );
    let rtl_ext = LayoutExt {
        direction: Some(LayoutDirection::Rtl),
        ..Default::default()
    };

    let icon = SvgPicture::from_paths(24.0, 24.0, &["M4 12 L20 12 M14 6 L20 12 L14 18"]);

    web.set_node_layout_ext(3, Some(rtl_ext));
    web.set_node_responsive_rules(2, vec![media_rule]);
    web.set_node_state_rules(2, vec![hover_rule, active_rule]);
    web.sync_background_fills_from([(2, &base_grad)]);
    web.sync_svgs_from([(3, &icon)]);
}

#[test]
fn web_gradients_base_hover_active_media_and_inline_svg_ssr_hydration_zero_mismatches() {
    let frame_bytes = build_hero_card_frame();
    let mut ssr_web = WebBackend::new();
    ssr_web.submit_frame(&frame_bytes).unwrap();
    sync_hero_typed_metadata(&mut ssr_web);

    let ssr = ssr_web.render_ssr();

    // 1. Base structural rule contains native CSS linear-gradient.
    assert!(
        ssr.style_css
            .contains("background-image:linear-gradient(135deg,#2563eb 0%,#7c3aed 100%);"),
        "missing base linear-gradient in SSR CSS:\n{}",
        ssr.style_css
    );

    // 2. `@media (min-width:768px)` rule contains radial-gradient.
    assert!(
        ssr.style_css.contains("@media (min-width:768px)")
            && ssr.style_css.contains(
                "background-image:radial-gradient(50% 50% at 50% 25%,#38bdf8 0%,#0f172a 100%);"
            ),
        "missing @media radial-gradient in SSR CSS:\n{}",
        ssr.style_css
    );

    // 3. `:hover` rule contains linear-gradient and `:active` clears gradient with solid color.
    assert!(
        ssr.style_css.contains(":hover{")
            && ssr.style_css.contains(
                "background-image:linear-gradient(90deg,#0ea5e9 0%,#6366f1 100%);"
            ),
        "missing :hover linear-gradient in SSR CSS:\n{}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains(":active{")
            && ssr
                .style_css
                .contains("background-image:none;background-color:#1e293b;"),
        "missing :active solid override in SSR CSS:\n{}",
        ssr.style_css
    );

    // 4. Base reset stylesheet supports RTL directional icon mirroring.
    assert!(
        ssr.base_css
            .contains("[dir=\"rtl\"] [data-nve-mirror=\"directional\"]"),
        "missing RTL mirror rule in base reset CSS"
    );

    // 5. Inline `<svg viewBox="0 0 24 24">` emitted cleanly in BOTH SSR and render_html() with
    //    `dir="rtl"`, `data-nve-mirror="directional"`, and tinted `<path>`.
    assert!(
        ssr.html.contains("<svg ")
            && ssr.html.contains("viewBox=\"0 0 24 24\"")
            && ssr.html.contains("dir=\"rtl\"")
            && ssr.html.contains("data-nve-mirror=\"directional\"")
            && ssr
                .html
                .contains("<path d=\"M4 12 L20 12 M14 6 L20 12 L14 18\" fill=\"#f8fafc\"/>")
            && ssr.html.contains("</svg>"),
        "unexpected inline SVG in SSR HTML:\n{}",
        ssr.html
    );

    let mirror_html = ssr_web.render_html();
    assert!(
        mirror_html.contains("background-image:linear-gradient(135deg,#2563eb 0%,#7c3aed 100%);"),
        "missing gradient in render_html():\n{mirror_html}"
    );
    assert!(
        mirror_html.contains("<svg ")
            && mirror_html.contains("viewBox=\"0 0 24 24\"")
            && mirror_html
                .contains("<path d=\"M4 12 L20 12 M14 6 L20 12 L14 18\" fill=\"#f8fafc\"/>"),
        "missing inline SVG in render_html():\n{mirror_html}"
    );

    // 6. Hydration parity: 0 mismatches, 0 replaced nodes, all nodes adopted in-place!
    let mut hydrated = WebBackend::hydrate_from_ssr(&ssr);
    hydrated.submit_frame(&frame_bytes).unwrap();
    sync_hero_typed_metadata(&mut hydrated);

    let report = hydrated.hydration_report();
    assert!(report.hydrated);
    assert!(
        report.mismatches.is_empty(),
        "expected 0 hydration mismatches, got: {:?}",
        report.mismatches
    );
    assert_eq!(report.replaced_nodes, 0);
    assert_eq!(report.adopted_nodes, 3, "expected root + view + svg adopted");
}

#[test]
fn svg_attached_to_view_container_emits_nested_inline_svg_with_zero_hydration_mismatch() {
    let icon = SvgPicture::from_stroked_paths(24.0, 24.0, 2.0, &["M3 12 L21 12"]);
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    let frame = b.finish().to_vec();

    web.submit_frame(&frame).unwrap();
    web.set_node_svg(2, Some(icon.clone()));

    let ssr = web.render_ssr();
    assert!(
        ssr.html.contains("<div data-nve-id=\"2\"")
            && ssr
                .html
                .contains("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"")
            && ssr.html.contains("stroke-width=\"2\""),
        "unexpected container SVG SSR:\n{}",
        ssr.html
    );

    let mut hydrated = WebBackend::hydrate_from_ssr(&ssr);
    hydrated.submit_frame(&frame).unwrap();
    hydrated.set_node_svg(2, Some(icon));
    assert!(hydrated.hydration_report().mismatches.is_empty());
    assert_eq!(hydrated.hydration_report().replaced_nodes, 0);
}

#[test]
fn distinct_gradients_produce_distinct_structural_classes_and_clear_cleanly() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.create_node(3, ComponentKind::View, None);
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let g1 = BackgroundFill::linear(
        0.0,
        [
            GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0)),
            GradientStop::new(1.0, ColorRgba::rgb(0, 0, 255)),
        ],
    );
    let g2 = BackgroundFill::linear(
        90.0,
        [
            GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0)),
            GradientStop::new(1.0, ColorRgba::rgb(0, 0, 255)),
        ],
    );
    web.set_node_background_fill(2, Some(g1));
    web.set_node_background_fill(3, Some(g2));

    let c2 = web.node(2).unwrap().class_name.clone();
    let c3 = web.node(3).unwrap().class_name.clone();
    assert_ne!(c2, c3, "distinct gradient angles must hash to distinct structural classes");

    web.set_node_background_fill(2, None);
    let ssr = web.render_ssr();
    assert!(!ssr.style_css.contains("linear-gradient(0deg"));
    assert!(ssr.style_css.contains("linear-gradient(90deg"));
}
