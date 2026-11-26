//! Tests for inline typography/display `RawStyleProps`, semantic inline/table HTML5 tags,
//! and base reset CSS rules for true inline flow and semantic tables.

#![cfg(feature = "ssr")]

use nve_abi::{CommandBufferBuilder, ComponentKind, RawPropValue, ROOT_NODE_ID};
use nve_backend_web::{is_raw_style_prop, WebBackend, NVE_BASE_RESET_CSS};
use nve_engine::RenderBackend;

#[test]
fn inline_typography_display_and_wrapping_raw_style_props_emit_css_and_never_attrs() {
    for key in [
        "display",
        "font-family",
        "fontFamily",
        "font-style",
        "fontStyle",
        "text-decoration",
        "textDecoration",
        "line-height",
        "lineHeight",
        "overflow-x",
        "overflowX",
        "overflow-wrap",
        "overflowWrap",
        "word-wrap",
        "word-break",
        "white-space",
        "vertical-align",
        "border-collapse",
    ] {
        assert!(is_raw_style_prop(key), "{key} must route to RawStyleProps");
    }

    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_raw_prop(2, "display", &RawPropValue::String("block".into()));
    b.set_raw_prop(2, "line-height", &RawPropValue::String("1.65".into()));
    b.set_raw_prop(2, "overflow-wrap", &RawPropValue::String("break-word".into()));
    b.set_raw_prop(2, "overflow-x", &RawPropValue::String("auto".into()));

    b.create_node(3, ComponentKind::Text, None);
    b.set_raw_prop(3, "semanticTag", &RawPropValue::String("em".into()));
    b.set_raw_prop(3, "display", &RawPropValue::String("inline".into()));
    b.set_raw_prop(3, "font-style", &RawPropValue::String("italic".into()));
    b.set_raw_prop(3, "font-family", &RawPropValue::String("monospace".into()));
    b.set_raw_prop(3, "text-decoration", &RawPropValue::String("underline".into()));
    b.set_text(3, "italic code");

    // Unsafe CSS injection attempts and invalid `display` keywords are rejected.
    b.create_node(4, ComponentKind::Text, None);
    b.set_raw_prop(4, "display", &RawPropValue::String("inline;color:red".into()));
    b.set_raw_prop(4, "font-family", &RawPropValue::String("mono;}</style>".into()));
    b.set_text(4, "safe");

    b.set_children(2, &[3, 4]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let ssr = web.render_ssr();
    assert!(ssr.style_css.contains("display:block;"), "{}", ssr.style_css);
    assert!(ssr.style_css.contains("line-height:1.65;"), "{}", ssr.style_css);
    assert!(ssr.style_css.contains("overflow-wrap:break-word;"), "{}", ssr.style_css);
    assert!(ssr.style_css.contains("overflow-x:auto;"), "{}", ssr.style_css);
    assert!(ssr.style_css.contains("display:inline;"), "{}", ssr.style_css);
    assert!(ssr.style_css.contains("font-style:italic;"), "{}", ssr.style_css);
    assert!(ssr.style_css.contains("font-family:monospace;"), "{}", ssr.style_css);
    assert!(ssr.style_css.contains("text-decoration:underline;"), "{}", ssr.style_css);
    assert!(!ssr.style_css.contains("color:red"));
    assert!(!ssr.style_css.contains("</style>"));
    assert!(ssr.html.contains("<em "), "expected semantic <em>: {}", ssr.html);
    assert!(!ssr.html.contains("display="));
    assert!(!ssr.html.contains("font-style="));
    assert!(!ssr.html.contains("overflow-wrap="));
}

#[test]
fn semantic_table_and_inline_tags_and_base_reset_rules_match_hydration() {
    assert!(NVE_BASE_RESET_CSS.contains(".nve-tl{display:inline;"));
    assert!(NVE_BASE_RESET_CSS.contains("pre.nve>code{display:flex;flex-direction:column;width:100%;min-width:0}"));
    assert!(NVE_BASE_RESET_CSS.contains("table.nve{display:table;border-collapse:collapse;border-style:hidden}"));
    assert!(NVE_BASE_RESET_CSS.contains("th.nve,td.nve{display:table-cell;vertical-align:top;width:auto}"));
    assert!(NVE_BASE_RESET_CSS.contains(":where(th.nve){text-align:left}"));

    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_raw_prop(2, "semanticTag", &RawPropValue::String("table".into()));
    b.create_node(3, ComponentKind::View, None);
    b.set_raw_prop(3, "semanticTag", &RawPropValue::String("thead".into()));
    b.create_node(4, ComponentKind::View, None);
    b.set_raw_prop(4, "semanticTag", &RawPropValue::String("tr".into()));
    b.create_node(5, ComponentKind::View, None);
    b.set_raw_prop(5, "semanticTag", &RawPropValue::String("th".into()));
    b.create_node(6, ComponentKind::Text, None);
    b.set_raw_prop(6, "semanticTag", &RawPropValue::String("strong".into()));
    b.set_text(6, "Col");
    b.set_children(5, &[6]);
    b.set_children(4, &[5]);
    b.set_children(3, &[4]);
    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    let bytes = b.finish().to_vec();
    web.submit_frame(&bytes).unwrap();

    let ssr = web.render_ssr();
    assert!(ssr.html.contains("<table "));
    assert!(ssr.html.contains("<thead "));
    assert!(ssr.html.contains("<tr "));
    assert!(ssr.html.contains("<th "));
    assert!(ssr.html.contains("<strong "));

    #[cfg(feature = "hydrate")]
    {
        let mut hydrated = WebBackend::hydrate_from_ssr(&ssr);
        hydrated.submit_frame(&bytes).unwrap();
        let report = hydrated.hydration_report();
        assert_eq!(report.mismatches.len(), 0);
        assert_eq!(report.replaced_nodes, 0);
        assert!(report.adopted_nodes >= 5);
    }
}
