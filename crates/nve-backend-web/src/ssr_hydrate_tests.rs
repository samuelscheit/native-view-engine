//! Deep unit tests for Option 5 Hybrid CSS, keyed minimal-diff `SetChildren`,
//! Semantic HTML5 SSR, zero-flicker hydration, and root event delegation.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EventMask, LayoutProps, NodeId,
    RawPropValue, StyleProps, ROOT_NODE_ID,
};
#[cfg(feature = "ssr")]
use nve_abi::{EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_engine::RenderBackend;

#[cfg(all(feature = "ssr", feature = "hydrate"))]
use crate::HydrationMismatch;
#[cfg(feature = "ssr")]
use crate::{is_raw_style_prop, resolve_semantic_tag, SsrDocumentOptions, NVE_BASE_RESET_CSS};
use crate::{
    canonical_f32_bits, format_structural_class, hash_structural_style, is_volatile_dimension,
    is_volatile_opacity, plan_keyed_child_ops, resolve_delegated_node, ChildDiffOp, NodeEventMeta,
    RawStyleProps, StyleSheetCache, WebBackend, MAX_DYNAMIC_RULES,
};

fn apply_child_diff_ops(old: &[NodeId], ops: &[ChildDiffOp]) -> Vec<NodeId> {
    let mut list = old.to_vec();
    for &op in ops {
        match op {
            ChildDiffOp::Remove(id) => {
                if let Some(pos) = list.iter().position(|&x| x == id) {
                    list.remove(pos);
                }
            }
            ChildDiffOp::InsertBefore { node_id, before } => {
                if let Some(pos) = list.iter().position(|&x| x == node_id) {
                    list.remove(pos);
                }
                match before {
                    Some(anchor) => {
                        let idx = list
                            .iter()
                            .position(|&x| x == anchor)
                            .expect("anchor must exist in DOM list");
                        list.insert(idx, node_id);
                    }
                    None => list.push(node_id),
                }
            }
        }
    }
    list
}

#[cfg(feature = "ssr")]
#[test]
fn base_reset_css_and_compact_formatting_omit_defaults_and_avoid_font_shorthand() {
    assert!(NVE_BASE_RESET_CSS.contains(".nve{display:flex;box-sizing:border-box;"));
    assert!(NVE_BASE_RESET_CSS.contains(".nve-s{overflow:auto}"));
    assert!(NVE_BASE_RESET_CSS.contains(".nve-b{cursor:pointer;appearance:none;background:transparent;font-family:inherit}"));
    assert!(!NVE_BASE_RESET_CSS.contains("font:inherit"));
    assert!(NVE_BASE_RESET_CSS.contains(".nve-i{font-family:inherit;outline:none}"));
    assert!(NVE_BASE_RESET_CSS.contains(".nve-p{cursor:pointer}"));
    assert!(NVE_BASE_RESET_CSS.contains(".nve-tl{color:inherit;font-size:inherit;font-weight:inherit;text-align:inherit}"));

    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            flex_direction: FlexDirection::Row,
            padding: EdgeInsets::symmetric(8.0, 16.0),
            margin: EdgeInsets::all(4.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(37, 99, 235),
            text_color: ColorRgba::BLACK,
            border_width: 0.0,
            opacity: 1.0,
            font_size: 16.0,
            z_index: 0,
            ..Default::default()
        },
    );
    b.set_raw_prop(2, "font-weight", &RawPropValue::String("400".into()));
    b.set_raw_prop(2, "text-align", &RawPropValue::String("left".into()));
    b.set_raw_prop(2, "box-shadow", &RawPropValue::String("none".into()));
    b.set_raw_prop(2, "overflow", &RawPropValue::String("visible".into()));
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let ssr = web.render_ssr();
    assert!(ssr.style_css.contains("flex-direction:row;"));
    assert!(ssr.style_css.contains("padding:8px 16px;"));
    assert!(ssr.style_css.contains("margin:4px;"));
    assert!(ssr.style_css.contains("background-color:#2563eb;"));
    // Defaults are omitted.
    assert!(!ssr.style_css.contains("border:"));
    assert!(!ssr.style_css.contains("opacity:"));
    assert!(!ssr.style_css.contains("z-index:"));
    assert!(!ssr.style_css.contains("font-weight:"));
    assert!(!ssr.style_css.contains("text-align:"));
    assert!(!ssr.style_css.contains("box-shadow:"));
    assert!(!ssr.style_css.contains("overflow:"));
    assert!(!ssr.style_css.contains("font-size:"));
    // No inline style needed for non-volatile node.
    assert!(!ssr.html.contains("style=\""));
}

#[cfg(feature = "ssr")]
#[test]
fn raw_style_props_capture_typography_shadow_overflow_and_exclude_from_html_attrs() {
    for key in [
        "font-weight",
        "fontWeight",
        "text-align",
        "textAlign",
        "box-shadow",
        "boxShadow",
        "overflow",
        "resizeMode",
        "objectFit",
        "object-fit",
    ] {
        assert!(is_raw_style_prop(key), "{key} must route to RawStyleProps");
    }
    assert!(!is_raw_style_prop("testID"));
    assert!(!is_raw_style_prop("href"));

    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_raw_prop(
        2,
        "box-shadow",
        &RawPropValue::String("0 8px 24px rgba(0, 0, 0, 0.4)".into()),
    );
    b.set_raw_prop(2, "font-weight", &RawPropValue::String("700".into()));
    b.set_raw_prop(2, "text-align", &RawPropValue::String("center".into()));
    b.set_raw_prop(2, "overflow", &RawPropValue::String("hidden".into()));
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let node = web.node(2).unwrap();
    assert_eq!(node.raw_style.font_weight.as_deref(), Some("700"));
    assert_eq!(node.raw_style.text_align.as_deref(), Some("center"));
    assert_eq!(
        node.raw_style.box_shadow.as_deref(),
        Some("0 8px 24px rgba(0, 0, 0, 0.4)")
    );
    assert_eq!(node.raw_style.overflow.as_deref(), Some("hidden"));

    let ssr = web.render_ssr();
    assert!(ssr.style_css.contains("font-weight:700;"));
    assert!(ssr.style_css.contains("text-align:center;"));
    assert!(ssr
        .style_css
        .contains("box-shadow:0 8px 24px rgba(0, 0, 0, 0.4);"));
    assert!(ssr.style_css.contains("overflow:hidden;"));
    // Never emitted as invalid HTML attributes.
    assert!(!ssr.html.contains("font-weight="));
    assert!(!ssr.html.contains("text-align="));
    assert!(!ssr.html.contains("box-shadow="));
    assert!(!ssr.html.contains("overflow="));
}

#[test]
fn canonical_f32_hashing_and_volatile_inline_lane_and_rule_cap() {
    assert_eq!(canonical_f32_bits(0.0), 0);
    assert_eq!(canonical_f32_bits(-0.0), 0);
    assert_eq!(canonical_f32_bits(f32::NAN), 0);
    assert_eq!(canonical_f32_bits(f32::INFINITY), 0);

    let l_pos = LayoutProps {
        gap: 0.0,
        ..Default::default()
    };
    let l_neg = LayoutProps {
        gap: -0.0,
        ..Default::default()
    };
    let s = StyleProps::default();
    let r = RawStyleProps::default();
    assert_eq!(
        format_structural_class(hash_structural_style(&l_pos, &s, &r, false)),
        format_structural_class(hash_structural_style(&l_neg, &s, &r, false))
    );

    // Volatile dimension and opacity checks.
    assert!(is_volatile_dimension(Dimension::Points(4000.0)));
    assert!(is_volatile_dimension(Dimension::Points(12.5)));
    assert!(!is_volatile_dimension(Dimension::Points(2048.0)));
    assert!(!is_volatile_dimension(Dimension::Percent(50.0)));

    assert!(is_volatile_opacity(0.371));
    assert!(!is_volatile_opacity(0.75));
    assert!(!is_volatile_opacity(1.0));
    assert!(!is_volatile_opacity(0.5));

    // Volatile VirtualList spacer & non-discrete opacity route to inline lane, then clear cleanly.
    let mut web = WebBackend::new();
    let mut b1 = CommandBufferBuilder::new(1, 1);
    b1.create_node(2, ComponentKind::View, None);
    b1.set_layout(
        2,
        &LayoutProps {
            height: Dimension::Points(40000.0),
            width: Dimension::Points(120.0),
            ..Default::default()
        },
    );
    b1.set_style(
        2,
        &StyleProps {
            opacity: 0.371,
            background: ColorRgba::rgb(15, 23, 42),
            ..Default::default()
        },
    );
    b1.set_children(ROOT_NODE_ID, &[2]);
    b1.commit(true);
    web.submit_frame(b1.finish()).unwrap();

    let n2 = web.node(2).unwrap();
    assert!(n2.has_inline_style);
    assert!(n2.inline_style.contains("height:40000px;"));
    assert!(n2.inline_style.contains("opacity:0.371;"));
    assert!(!n2.inline_style.contains("width:120px;"));
    assert!(n2.class_name.contains(" s"));

    // Frame 2 (consecutive): triggers consecutive-frame volatility -> all non-defaults inline.
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_layout(
        2,
        &LayoutProps {
            height: Dimension::Points(100.0),
            width: Dimension::Points(120.0),
            ..Default::default()
        },
    );
    b2.set_style(
        2,
        &StyleProps {
            opacity: 1.0,
            background: ColorRgba::rgb(15, 23, 42),
            ..Default::default()
        },
    );
    b2.commit(true);
    web.submit_frame(b2.finish()).unwrap();
    assert!(web.node(2).unwrap().is_consecutive_frame_volatile());
    assert!(web.node(2).unwrap().has_inline_style);

    // Frame 3 (no style mutation on node 2), then Frame 4 (non-consecutive) -> returns to class-only
    // and clears `inline_style` (`has_inline_style == false`)!
    let mut b3 = CommandBufferBuilder::new(1, 3);
    b3.commit(true);
    web.submit_frame(b3.finish()).unwrap();

    let mut b4 = CommandBufferBuilder::new(1, 4);
    b4.set_style(
        2,
        &StyleProps {
            opacity: 1.0,
            background: ColorRgba::rgb(15, 23, 42),
            ..Default::default()
        },
    );
    b4.commit(true);
    web.submit_frame(b4.finish()).unwrap();
    let n2_settled = web.node(2).unwrap();
    assert!(!n2_settled.is_consecutive_frame_volatile());
    assert!(!n2_settled.has_inline_style);
    assert!(n2_settled.inline_style.is_empty());

    // Bounded MAX_DYNAMIC_RULES cap: 4097th unique rule falls back cleanly to inline lane.
    let mut cache = StyleSheetCache::new();
    for i in 0..MAX_DYNAMIC_RULES {
        assert!(cache.ensure_rule(format!("s{i}"), format!("gap:{i}px;")));
    }
    assert_eq!(cache.rule_count(), MAX_DYNAMIC_RULES);
    assert!(!cache.can_insert_new());
    assert!(!cache.ensure_rule("soverflow".into(), "gap:9999px;".into()));
}

#[test]
fn leaf_text_inheriting_parent_typography_gets_nve_tl_without_structural_rule() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Text, None);
    b.set_style(
        2,
        &StyleProps {
            text_color: ColorRgba::WHITE,
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.set_raw_prop(2, "font-weight", &RawPropValue::String("700".into()));
    b.create_node(3, ComponentKind::Text, None);
    b.set_style(
        3,
        &StyleProps {
            text_color: ColorRgba::WHITE,
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.set_raw_prop(3, "font-weight", &RawPropValue::String("700".into()));
    b.set_text(3, "Inherited Leaf");
    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    let leaf = web.node(3).unwrap();
    assert_eq!(leaf.class_name, "nve nve-tl");
    assert!(leaf.inline_style.is_empty());
    // Only the parent Text wrapper needed a structural rule (1 rule total, not 2).
    assert_eq!(web.stylesheet().rule_count(), 1);
}

#[test]
fn deferred_style_flush_skips_nodes_removed_in_same_frame() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(220, 38, 38),
            ..Default::default()
        },
    );
    b.remove_node(2);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    assert!(web.node(2).is_none());
    assert_eq!(web.stylesheet().rule_count(), 0);
}

#[test]
fn keyed_child_diff_planner_handles_edge_cases_with_minimal_ops() {
    // 1. Identical -> 0 ops.
    assert!(plan_keyed_child_ops(&[1, 2, 3, 4], &[1, 2, 3, 4]).is_empty());

    // 2. Append 1 to 100 -> 1 op.
    let old100: Vec<NodeId> = (1..=100).collect();
    let mut append101 = old100.clone();
    append101.push(101);
    let ops = plan_keyed_child_ops(&old100, &append101);
    assert_eq!(
        ops,
        vec![ChildDiffOp::InsertBefore {
            node_id: 101,
            before: None
        }]
    );
    assert_eq!(apply_child_diff_ops(&old100, &ops), append101);

    // 3. Prepend 1 to 100 -> 1 op.
    let mut prepend0 = vec![200];
    prepend0.extend_from_slice(&old100);
    let ops = plan_keyed_child_ops(&old100, &prepend0);
    assert_eq!(
        ops,
        vec![ChildDiffOp::InsertBefore {
            node_id: 200,
            before: Some(1)
        }]
    );
    assert_eq!(apply_child_diff_ops(&old100, &ops), prepend0);

    // 4. Delete middle item -> 1 Remove, 0 moves.
    let mut del_mid = old100.clone();
    del_mid.remove(50);
    let ops = plan_keyed_child_ops(&old100, &del_mid);
    assert_eq!(ops, vec![ChildDiffOp::Remove(51)]);
    assert_eq!(apply_child_diff_ops(&old100, &ops), del_mid);

    // 5. Move single item up by 1 slot -> 1 InsertBefore (focused sibling untouched).
    let ops = plan_keyed_child_ops(&[1, 2, 3, 4, 5], &[1, 2, 4, 3, 5]);
    assert_eq!(ops.len(), 1);
    assert_eq!(apply_child_diff_ops(&[1, 2, 3, 4, 5], &ops), vec![1, 2, 4, 3, 5]);

    // 6. Reverse, empty<->many, and mixed insert/remove/reorder cases.
    let cases: &[(&[NodeId], &[NodeId])] = &[
        (&[], &[1, 2, 3]),
        (&[1, 2, 3], &[]),
        (&[1, 2, 3, 4, 5], &[5, 4, 3, 2, 1]),
        (&[1, 2, 3, 4, 5, 6], &[6, 2, 9, 4, 1, 10]),
        (&[10, 20, 30, 40], &[20, 50, 40, 60, 10]),
        (&[1, 2, 3, 4, 5], &[2, 3, 4, 5, 1]),
        (&[1, 2, 3, 4, 5], &[5, 1, 2, 3, 4]),
    ];
    for &(old, new) in cases {
        let ops = plan_keyed_child_ops(old, new);
        assert_eq!(
            apply_child_diff_ops(old, &ops),
            new,
            "failed reconciling {old:?} -> {new:?} via {ops:?}"
        );
    }
}

#[cfg(feature = "ssr")]
#[test]
fn semantic_html5_ssr_and_document_emission_cover_all_tags_and_metadata() {
    let mut web = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);

    struct SemanticSpec {
        id: NodeId,
        kind: ComponentKind,
        role: &'static str,
        level: Option<i64>,
        href: Option<&'static str>,
        text: &'static str,
    }
    let semantic_specs = [
        SemanticSpec { id: 2, kind: ComponentKind::View, role: "banner", level: None, href: None, text: "" },
        SemanticSpec { id: 3, kind: ComponentKind::View, role: "navigation", level: None, href: None, text: "" },
        SemanticSpec { id: 4, kind: ComponentKind::View, role: "main", level: None, href: None, text: "" },
        SemanticSpec { id: 5, kind: ComponentKind::View, role: "article", level: None, href: None, text: "" },
        SemanticSpec { id: 6, kind: ComponentKind::View, role: "complementary", level: None, href: None, text: "" },
        SemanticSpec { id: 7, kind: ComponentKind::View, role: "contentinfo", level: None, href: None, text: "" },
        SemanticSpec { id: 8, kind: ComponentKind::Text, role: "heading", level: Some(1), href: None, text: "Docs & Guide" },
        SemanticSpec { id: 9, kind: ComponentKind::Text, role: "heading", level: Some(3), href: None, text: "Subheading" },
        SemanticSpec { id: 10, kind: ComponentKind::Pressable, role: "link", level: None, href: Some("/docs/intro"), text: "Read More" },
        SemanticSpec { id: 11, kind: ComponentKind::Text, role: "code", level: None, href: None, text: "x < y" },
        SemanticSpec { id: 12, kind: ComponentKind::Text, role: "codeblock", level: None, href: None, text: "fn main() {}" },
    ];

    for spec in &semantic_specs {
        b.create_node(spec.id, spec.kind, None);
        b.set_raw_prop(spec.id, "accessibilityRole", &RawPropValue::String(spec.role.into()));
        if let Some(lvl) = spec.level {
            b.set_raw_prop(spec.id, "ariaLevel", &RawPropValue::I64(lvl));
        }
        if let Some(url) = spec.href {
            b.set_raw_prop(spec.id, "href", &RawPropValue::String(url.into()));
        }
        if !spec.text.is_empty() {
            b.set_text(spec.id, spec.text);
        }
    }
    b.set_raw_prop(5, "data-nve-island", &RawPropValue::String("article-island".into()));
    b.set_raw_prop(5, "data-nve-hydrate", &RawPropValue::String("visible".into()));
    b.set_layout(
        4,
        &LayoutProps {
            justify_content: FlexJustify::SpaceBetween,
            align_items: FlexAlign::Center,
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();

    // Verify tag resolution.
    assert_eq!(resolve_semantic_tag(ComponentKind::Text, &web.node(8).unwrap().raw_props).tag, "h1");
    assert_eq!(resolve_semantic_tag(ComponentKind::Text, &web.node(9).unwrap().raw_props).tag, "h3");

    let ssr = web.render_ssr();
    assert!(ssr.html.contains("data-nve-ssr-hash=\""));
    assert!(ssr.html.contains("<header data-nve-id=\"2\""));
    assert!(ssr.html.contains("<nav data-nve-id=\"3\""));
    assert!(ssr.html.contains("<main data-nve-id=\"4\""));
    assert!(ssr.html.contains("<article data-nve-id=\"5\""));
    assert!(ssr.html.contains("data-nve-island=\"article-island\""));
    assert!(ssr.html.contains("data-nve-hydrate=\"visible\""));
    assert!(ssr.html.contains("<aside data-nve-id=\"6\""));
    assert!(ssr.html.contains("<footer data-nve-id=\"7\""));
    assert!(ssr.html.contains("<h1 data-nve-id=\"8\""));
    assert!(ssr.html.contains("Docs &amp; Guide</h1>"));
    assert!(ssr.html.contains("<h3 data-nve-id=\"9\""));
    assert!(ssr.html.contains("<a data-nve-id=\"10\""));
    assert!(ssr.html.contains("href=\"/docs/intro\""));
    assert!(ssr.html.contains("<code data-nve-id=\"11\""));
    assert!(ssr.html.contains("x &lt; y</code>"));
    assert!(ssr.html.contains("<pre data-nve-id=\"12\""));
    assert!(ssr.html.contains("<code>fn main() {}</code></pre>"));

    // Rules are sorted deterministically by hash.
    let mut sorted = ssr.rule_hashes.clone();
    sorted.sort();
    assert_eq!(ssr.rule_hashes, sorted);

    let doc = web.render_ssr_document(&SsrDocumentOptions {
        title: "NVE Docs".into(),
        description: Some("Fast native & web engine".into()),
        canonical_url: Some("https://nve.dev/docs".into()),
        og_image: Some("https://nve.dev/og.png".into()),
        lang: Some("en".into()),
        head_extra: Some("<link rel=\"icon\" href=\"/icon.svg\">".into()),
        body_scripts: Some("<script type=\"module\" src=\"/app.js\"></script>".into()),
    });
    assert!(doc.starts_with("<!doctype html>"));
    assert!(doc.contains("<title>NVE Docs</title>"));
    assert!(doc.contains("name=\"description\" content=\"Fast native &amp; web engine\""));
    assert!(doc.contains("rel=\"canonical\" href=\"https://nve.dev/docs\""));
    assert!(doc.contains("property=\"og:image\" content=\"https://nve.dev/og.png\""));
    assert!(doc.contains("<style data-nve-base>"));
    assert!(doc.contains("<style data-nve-ssr data-nve-hashes=\""));
    assert!(doc.contains("<div id=\"app\">"));
    assert!(doc.contains("<script type=\"module\" src=\"/app.js\"></script>"));
}

#[cfg(all(feature = "ssr", feature = "hydrate"))]
#[test]
fn zero_flicker_hydration_adopts_semantic_nodes_and_replaces_only_mismatched_subtree() {
    let build_frame = |mismatch_node3: bool| -> Vec<u8> {
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::Text, None);
        b.set_raw_prop(2, "accessibilityRole", &RawPropValue::String("heading".into()));
        b.set_raw_prop(2, "ariaLevel", &RawPropValue::I64(1));
        b.set_style(
            2,
            &StyleProps {
                text_color: ColorRgba::WHITE,
                font_size: 24.0,
                ..Default::default()
            },
        );
        b.set_text(2, "Hello SSR");
        b.create_node(
            3,
            if mismatch_node3 {
                ComponentKind::Pressable
            } else {
                ComponentKind::View
            },
            None,
        );
        b.set_children(ROOT_NODE_ID, &[2, 3]);
        b.commit(true);
        b.finish().to_vec()
    };

    // Server render.
    let mut server = WebBackend::new();
    let frame0 = build_frame(false);
    server.submit_frame(&frame0).unwrap();
    let ssr = server.render_ssr();

    // 1. Clean hydration: 0 mismatches, exact identity preserved, 0 duplicate CSS rules!
    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    let root_before = client.native_dom_instance_id(ROOT_NODE_ID).unwrap();
    let h1_before = client.native_dom_instance_id(2).unwrap();
    let div_before = client.native_dom_instance_id(3).unwrap();

    client.submit_frame(&frame0).unwrap();
    let rep = client.hydration_report();
    assert!(rep.hydrated);
    assert_eq!(rep.mismatches, Vec::<HydrationMismatch>::new());
    assert_eq!(rep.adopted_nodes, 3);
    assert_eq!(rep.replaced_nodes, 0);
    assert_eq!(rep.seeded_rule_count, ssr.rule_hashes.len());
    assert_eq!(client.stylesheet().pending_count(), 0);
    assert_eq!(client.native_dom_instance_id(ROOT_NODE_ID), Some(root_before));
    assert_eq!(client.native_dom_instance_id(2), Some(h1_before));
    assert_eq!(client.native_dom_instance_id(3), Some(div_before));

    // 2. Hydration with single node tag mismatch: ONLY node 3 replaced; node 1 & 2 preserved!
    let mut client_mismatch = WebBackend::hydrate_from_ssr(&ssr);
    let h1_id_before = client_mismatch.native_dom_instance_id(2).unwrap();
    let div_id_before = client_mismatch.native_dom_instance_id(3).unwrap();
    let mismatched_frame = build_frame(true);
    client_mismatch.submit_frame(&mismatched_frame).unwrap();

    let rep2 = client_mismatch.hydration_report();
    assert_eq!(rep2.adopted_nodes, 2);
    assert_eq!(rep2.replaced_nodes, 1);
    assert_eq!(
        rep2.mismatches,
        vec![HydrationMismatch {
            node_id: 3,
            expected_tag: "button",
            actual_tag: "div".into(),
        }]
    );
    assert_eq!(client_mismatch.native_dom_instance_id(2), Some(h1_id_before));
    assert_ne!(client_mismatch.native_dom_instance_id(3), Some(div_id_before));
}

#[test]
fn root_event_delegation_resolves_nearest_matching_ancestor() {
    let chain = [
        (
            12,
            NodeEventMeta {
                kind: ComponentKind::Text,
                mask: EventMask::NONE,
            },
        ),
        (
            11,
            NodeEventMeta {
                kind: ComponentKind::Text,
                mask: EventMask::NONE,
            },
        ),
        (
            5,
            NodeEventMeta {
                kind: ComponentKind::Pressable,
                mask: EventMask::PRESS,
            },
        ),
        (
            1,
            NodeEventMeta {
                kind: ComponentKind::Root,
                mask: EventMask::NONE,
            },
        ),
    ];

    assert_eq!(
        resolve_delegated_node(&chain, |m| (m.mask & EventMask::PRESS) != 0),
        Some(5)
    );
    assert_eq!(
        resolve_delegated_node(&chain, |m| (m.mask & EventMask::TEXT_CHANGE) != 0),
        None
    );
}

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    any(
        all(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
        ),
        feature = "camera",
    )
))]
fn nvnp_encode(fields: &[(u16, crate::NativeValueRef<'_>)]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&crate::native_view::NATIVE_PROPS_MAGIC);
    buf.push(crate::native_view::NATIVE_PROPS_VERSION);
    buf.extend_from_slice(&(fields.len() as u16).to_le_bytes());
    for &(id, val) in fields {
        buf.extend_from_slice(&id.to_le_bytes());
        match val {
            crate::NativeValueRef::Null => buf.push(0),
            crate::NativeValueRef::Bool(b) => {
                buf.push(1);
                buf.push(u8::from(b));
            }
            crate::NativeValueRef::U32(n) => {
                buf.push(2);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::NativeValueRef::I64(n) => {
                buf.push(3);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::NativeValueRef::U64(n) => {
                buf.push(4);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::NativeValueRef::F32(n) => {
                buf.push(5);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::NativeValueRef::F64(n) => {
                buf.push(6);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::NativeValueRef::Str(s) => {
                buf.push(7);
                buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
                buf.extend_from_slice(s.as_bytes());
            }
            crate::NativeValueRef::Bytes(b) => {
                buf.push(8);
                buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
                buf.extend_from_slice(b);
            }
        }
    }
    buf
}

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    any(
        all(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
        ),
        feature = "camera",
    )
))]
fn nvnc_encode(opcode: u16, args: &[crate::NativeValueRef<'_>]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&crate::native_view::NATIVE_CMD_MAGIC);
    buf.push(crate::native_view::NATIVE_CMD_VERSION);
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.push(args.len() as u8);
    for &val in args {
        match val {
            crate::NativeValueRef::U32(n) => {
                buf.push(2);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::NativeValueRef::U64(n) => {
                buf.push(4);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::NativeValueRef::F32(n) => {
                buf.push(5);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            crate::NativeValueRef::Str(s) => {
                buf.push(7);
                buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
                buf.extend_from_slice(s.as_bytes());
            }
            _ => buf.push(0),
        }
    }
    buf
}

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    feature = "nv-blur",
    feature = "nv-webview",
    feature = "nv-video",
    feature = "nv-web-element",
))]
fn pack_web_attrs(attrs: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for &(k, v) in attrs {
        out.extend_from_slice(&(k.len() as u32).to_le_bytes());
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&(v.len() as u32).to_le_bytes());
        out.extend_from_slice(v.as_bytes());
    }
    out
}

#[cfg(all(
    feature = "ssr",
    feature = "hydrate",
    feature = "nv-blur",
    feature = "nv-webview",
    feature = "nv-video",
    feature = "nv-web-element",
))]
#[test]
fn native_view_web_element_blur_webview_and_video_ssr_hydration_and_commands() {
    use crate::native_view::{
        is_safe_web_attr, is_safe_web_url, validate_web_element_tag, NATIVE_VIEW_KIND_PROP_KEY,
        NATIVE_VIEW_PROPS_PROP_KEY,
    };
    use crate::{BlurMaterial, NativeValueRef};

    // 1. Security allowlist & sanitization checks.
    for ok_tag in [
        "video", "audio", "svg", "iframe", "canvas", "input", "select", "textarea", "progress",
        "meter", "details", "summary", "my-chart", "x-video-hud",
    ] {
        assert!(validate_web_element_tag(ok_tag).is_some(), "expected {ok_tag} allowed");
    }
    for bad_tag in ["script", "style", "object", "embed", "My-Chart", "bad-", "font-face", "div"] {
        assert!(validate_web_element_tag(bad_tag).is_none(), "expected {bad_tag} rejected");
    }
    assert!(!is_safe_web_url("javascript:alert(1)"));
    assert!(!is_safe_web_url("  JaVa\tScRiPt:alert(1)"));
    assert!(!is_safe_web_url("data:text/html,<script>alert(1)</script>"));
    assert!(is_safe_web_url("https://nve.dev/docs"));
    assert!(is_safe_web_url("/assets/intro.mp4"));
    assert!(!is_safe_web_attr("onclick", "alert(1)"));
    assert!(!is_safe_web_attr("ONERROR", "alert(1)"));
    assert!(!is_safe_web_attr("srcdoc", "<script>alert(1)</script>"));
    assert!(!is_safe_web_attr("src", "javascript:alert(1)"));
    assert!(is_safe_web_attr("viewBox", "0 0 24 24"));

    // 2. Build a single frame containing BlurSurface, WebView, VideoPlayer, allowlisted WebElement,
    //    custom Web Component, and an adversarial <script> attempt.
    let blur_bytes = nvnp_encode(&[
        (1, NativeValueRef::U32(BlurMaterial::Thick as u32)),
        (2, NativeValueRef::F32(0.5)),
        (3, NativeValueRef::U32(0x10203080)),
    ]);
    let webview_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("https://nve.dev/embed")),
        (2, NativeValueRef::Bool(true)),
    ]);
    let video_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("https://cdn.nve.dev/hero.mp4")),
        (2, NativeValueRef::Bool(false)),
        (3, NativeValueRef::Bool(true)),
        (4, NativeValueRef::Bool(true)),
        (5, NativeValueRef::Bool(true)),
    ]);
    let svg_attrs = pack_web_attrs(&[
        ("viewBox", "0 0 32 32"),
        ("onclick", "alert(1)"),
        ("aria-hidden", "true"),
    ]);
    let svg_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("svg")),
        (2, NativeValueRef::U32(3)),
        (3, NativeValueRef::Bytes(&svg_attrs)),
    ]);
    let custom_attrs = pack_web_attrs(&[("data-mode", "pro"), ("srcdoc", "<b>bad</b>")]);
    let custom_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("my-chart")),
        (2, NativeValueRef::U32(2)),
        (3, NativeValueRef::Bytes(&custom_attrs)),
    ]);
    let bad_script_attrs = pack_web_attrs(&[("src", "https://evil.test/x.js")]);
    let bad_script_bytes = nvnp_encode(&[
        (1, NativeValueRef::Str("script")),
        (2, NativeValueRef::U32(1)),
        (3, NativeValueRef::Bytes(&bad_script_attrs)),
    ]);

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Custom, Some("nve.blur_surface"));
    b.set_raw_prop(2, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("nve.blur_surface".into()));
    b.set_raw_prop(2, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(blur_bytes));

    b.create_node(3, ComponentKind::Custom, Some("nve.web_view"));
    b.set_raw_prop(3, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("nve.web_view".into()));
    b.set_raw_prop(3, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(webview_bytes));

    b.create_node(4, ComponentKind::Custom, Some("nve.video_player"));
    b.set_raw_prop(4, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("nve.video_player".into()));
    b.set_raw_prop(4, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(video_bytes));

    b.create_node(5, ComponentKind::Custom, Some("web.svg"));
    b.set_raw_prop(5, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("web.svg".into()));
    b.set_raw_prop(5, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(svg_bytes));

    b.create_node(6, ComponentKind::Custom, Some("web.my-chart"));
    b.set_raw_prop(6, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("web.my-chart".into()));
    b.set_raw_prop(6, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(custom_bytes));

    b.create_node(7, ComponentKind::Custom, Some("web.script"));
    b.set_raw_prop(7, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String("web.script".into()));
    b.set_raw_prop(7, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(bad_script_bytes));

    b.set_children(ROOT_NODE_ID, &[2, 3, 4, 5, 6, 7]);
    b.commit(true);
    let frame0 = b.finish().to_vec();

    let mut backend = WebBackend::new();
    backend.submit_frame(&frame0).unwrap();
    let ssr = backend.render_ssr();
    let mirror_html = backend.render_html();

    for html in [&ssr.html, &mirror_html] {
        // BlurSurface emits backdrop-filter + -webkit-backdrop-filter + tint
        assert!(html.contains("backdrop-filter:blur(16px);-webkit-backdrop-filter:blur(16px);"));
        assert!(html.contains("background-color:rgba(16,32,48,0.502);"));
        // WebView emits sandboxed <iframe>
        assert!(html.contains("<iframe data-nve-id=\"3\""));
        assert!(html.contains("sandbox=\"allow-scripts allow-same-origin allow-forms allow-popups\""));
        assert!(html.contains("src=\"https://nve.dev/embed\""));
        // VideoPlayer emits <video> with muted/loop/controls
        assert!(html.contains("<video data-nve-id=\"4\""));
        assert!(html.contains("src=\"https://cdn.nve.dev/hero.mp4\""));
        assert!(html.contains(" muted loop controls playsinline"));
        // WebElement <svg> preserves safe attrs and strips onclick
        assert!(html.contains("<svg data-nve-id=\"5\""));
        assert!(html.contains("viewBox=\"0 0 32 32\""));
        assert!(html.contains("aria-hidden=\"true\""));
        assert!(!html.contains("onclick"));
        // Custom element <my-chart> mounted and srcdoc stripped
        assert!(html.contains("<my-chart data-nve-id=\"6\""));
        assert!(html.contains("data-mode=\"pro\""));
        assert!(!html.contains("srcdoc"));
        // <script> rejected and downgraded to <div> without src
        assert!(!html.contains("<script"));
        assert!(html.contains("<div data-nve-id=\"7\""));
        assert!(!html.contains("evil.test"));
    }

    // 3. Zero-flicker SSR hydration adopts built-in native views (blur <div>, <iframe>, <video>, <svg>).
    let mut b_hyd = CommandBufferBuilder::new(1, 1);
    b_hyd.create_node(2, ComponentKind::Custom, Some("nve.blur_surface"));
    b_hyd.create_node(3, ComponentKind::Custom, Some("nve.web_view"));
    b_hyd.create_node(4, ComponentKind::Custom, Some("nve.video_player"));
    b_hyd.create_node(5, ComponentKind::Custom, Some("web.svg"));
    b_hyd.set_children(ROOT_NODE_ID, &[2, 3, 4, 5]);
    b_hyd.commit(true);
    let hyd_frame = b_hyd.finish().to_vec();
    let mut srv_hyd = WebBackend::new();
    srv_hyd.submit_frame(&hyd_frame).unwrap();
    let ssr_hyd = srv_hyd.render_ssr();
    let mut cli_hyd = WebBackend::hydrate_from_ssr(&ssr_hyd);
    cli_hyd.submit_frame(&hyd_frame).unwrap();
    assert!(cli_hyd.hydration_report().mismatches.is_empty());
    assert_eq!(cli_hyd.hydration_report().adopted_nodes, 5);

    // 4. Imperative NVNC commands via Command::DispatchCommand (WebView + VideoPlayer).
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.dispatch_command(3, "navigate", &nvnc_encode(1, &[NativeValueRef::Str("javascript:alert(1)")]));
    b2.dispatch_command(3, "navigate", &nvnc_encode(1, &[NativeValueRef::Str("https://nve.dev/next")]));
    b2.dispatch_command(3, "reload", &nvnc_encode(2, &[]));
    b2.dispatch_command(3, "go_back", &nvnc_encode(3, &[]));
    b2.dispatch_command(3, "go_forward", &nvnc_encode(4, &[]));
    b2.dispatch_command(4, "play", &nvnc_encode(1, &[]));
    b2.dispatch_command(4, "seek_ms", &nvnc_encode(3, &[NativeValueRef::U64(12_345)]));
    b2.dispatch_command(4, "set_volume", &nvnc_encode(4, &[NativeValueRef::F32(0.42)]));
    b2.dispatch_command(4, "pause", &nvnc_encode(2, &[]));
    b2.commit(false);
    backend.submit_frame(b2.finish()).unwrap();

    let wv = backend.native_view(3).unwrap();
    assert_eq!(wv.current_url.as_deref(), Some("https://nve.dev/next"));
    assert_eq!(wv.reload_count, 1);
    assert_eq!(wv.go_back_count, 1);
    assert_eq!(wv.go_forward_count, 1);

    let vp = backend.native_view(4).unwrap();
    assert!(!vp.playing);
    assert_eq!(vp.position_ms, 12_345);
    assert!((vp.volume - 0.42).abs() < 1e-6);

    // 5. Shorthand aliases ("nve.blur", "nve.webview", "nve.video", "nve.native.kind") + corrupt buffer resilience.
    let mut b3 = CommandBufferBuilder::new(1, 3);
    b3.create_node(10, ComponentKind::Custom, None);
    b3.set_raw_prop(10, "nve.native.kind", &RawPropValue::String("nve.blur".into()));
    b3.set_raw_prop(10, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(vec![0xFF, 0x00]));
    b3.dispatch_command(3, "navigate", &[0x00, 0x01, 0x02]);
    b3.commit(true);
    backend.submit_frame(b3.finish()).unwrap();
    assert_eq!(backend.native_view(10).unwrap().resolved_tag, "div");
    assert!(backend.native_view(10).unwrap().extra_css.contains("backdrop-filter:blur("));
    assert_eq!(backend.native_view(3).unwrap().current_url.as_deref(), Some("https://nve.dev/next"));
}

#[cfg(all(feature = "ssr", feature = "hydrate", feature = "camera"))]
#[test]
fn native_view_camera_preview_ssr_hydration_commands_and_corrupt_resilience() {
    use crate::native_view::{
        decode_camera_preview_command, decode_camera_preview_props, CameraPreviewCommand,
        CameraPreviewFacing, NativePropDecoder, NativeViewCodecError, StabilizationMode, TorchMode,
        KIND_CAMERA_ALIAS, KIND_CAMERA_PREVIEW, NATIVE_VIEW_KIND_PROP_KEY,
        NATIVE_VIEW_PROPS_PROP_KEY,
    };
    use crate::NativeValueRef;

    let encode_cam_props = |dev: Option<&str>,
                            facing: u32,
                            torch: u32,
                            zoom: f32,
                            fps: u32,
                            hdr: bool,
                            stab: u32,
                            fp: bool,
                            audio: bool,
                            active: bool| {
        nvnp_encode(&[
            (1, dev.map_or(NativeValueRef::Null, NativeValueRef::Str)),
            (2, NativeValueRef::U32(facing)),
            (3, NativeValueRef::U32(torch)),
            (4, NativeValueRef::F32(zoom)),
            (5, NativeValueRef::U32(fps)),
            (6, NativeValueRef::Bool(hdr)),
            (7, NativeValueRef::U32(stab)),
            (8, NativeValueRef::Bool(fp)),
            (9, NativeValueRef::Bool(audio)),
            (10, NativeValueRef::Bool(active)),
        ])
    };

    // 1. SSR + mirror HTML emission for `"nve.camera.preview"` with full NVNP fields.
    let props1 = encode_cam_props(Some("usb-cam-1"), 2, 1, 2.5, 60, true, 2, true, true, true);
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Custom, Some(KIND_CAMERA_PREVIEW));
    b.set_raw_prop(2, NATIVE_VIEW_KIND_PROP_KEY, &RawPropValue::String(KIND_CAMERA_PREVIEW.into()));
    b.set_raw_prop(2, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(props1));
    b.create_node(3, ComponentKind::Custom, Some(KIND_CAMERA_ALIAS));
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    let frame0 = b.finish().to_vec();

    let mut backend = WebBackend::new();
    backend.submit_frame(&frame0).unwrap();
    let ssr = backend.render_ssr();
    let mirror = backend.render_html();

    for html in [&ssr.html, &mirror] {
        assert!(html.contains("<video data-nve-id=\"2\""));
        assert!(html.contains("data-nve-native-kind=\"nve.camera.preview\""));
        assert!(html.contains(" autoplay playsinline muted data-nve-camera=\"1\""));
        assert!(html.contains("data-nve-camera-facing=\"front\""));
        assert!(html.contains("data-nve-camera-torch=\"on\""));
        assert!(html.contains("data-nve-camera-zoom=\"2.5\""));
        assert!(html.contains("data-nve-camera-fps=\"60\""));
        assert!(html.contains("data-nve-camera-active=\"1\""));
        assert!(html.contains("data-nve-camera-audio=\"1\""));
        assert!(html.contains("data-nve-camera-device=\"usb-cam-1\""));
        // Default alias node without NVNP still renders `<video autoplay playsinline muted data-nve-camera="1">`.
        assert!(html.contains("<video data-nve-id=\"3\""));
        assert!(html.contains("data-nve-camera-facing=\"back\""));
    }

    // 2. Zero-flicker SSR hydration adopts camera `<video>` nodes with 0 mismatches & exact DOM identity.
    let mut cli = WebBackend::hydrate_from_ssr(&ssr);
    let cam_id_before = cli.native_dom_instance_id(2).unwrap();
    let alias_id_before = cli.native_dom_instance_id(3).unwrap();
    cli.submit_frame(&frame0).unwrap();
    assert!(cli.hydration_report().mismatches.is_empty());
    assert_eq!(cli.hydration_report().adopted_nodes, 3);
    assert_eq!(cli.native_dom_instance_id(2), Some(cam_id_before));
    assert_eq!(cli.native_dom_instance_id(3), Some(alias_id_before));

    // 3. Imperative NVNC commands (opcodes 1..=7) update state & reflected HTML attributes.
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.dispatch_command(2, "switch_facing", &nvnc_encode(1, &[NativeValueRef::U32(3)]));
    b2.dispatch_command(2, "set_torch", &nvnc_encode(2, &[NativeValueRef::U32(2)]));
    b2.dispatch_command(2, "set_zoom", &nvnc_encode(3, &[NativeValueRef::F32(3.25)]));
    b2.dispatch_command(2, "focus_point", &nvnc_encode(4, &[NativeValueRef::F32(0.25), NativeValueRef::F32(0.75)]));
    b2.dispatch_command(2, "capture_photo", &nvnc_encode(5, &[NativeValueRef::U32(88), NativeValueRef::U32(2)]));
    b2.dispatch_command(2, "start_recording", &nvnc_encode(6, &[NativeValueRef::Str("/tmp/web.webm")]));
    b2.commit(false);
    backend.submit_frame(b2.finish()).unwrap();

    let cam = &backend.native_view(2).unwrap().camera;
    assert_eq!(cam.facing, CameraPreviewFacing::External);
    assert_eq!(cam.torch, TorchMode::Auto);
    assert!((cam.zoom - 3.25).abs() < 1e-6);
    assert_eq!(cam.last_focus_point, Some((0.25, 0.75)));
    assert_eq!(cam.photo_capture_count, 1);
    assert_eq!(cam.last_photo_quality, Some(88));
    assert_eq!(cam.last_photo_flash, Some(2));
    assert!(cam.recording);
    assert_eq!(cam.last_recording_path.as_deref(), Some("/tmp/web.webm"));
    let ssr_rec = backend.render_ssr();
    assert!(ssr_rec.html.contains("data-nve-camera-facing=\"external\""));
    assert!(ssr_rec.html.contains("data-nve-camera-torch=\"auto\""));
    assert!(ssr_rec.html.contains("data-nve-camera-zoom=\"3.25\""));
    assert!(ssr_rec.html.contains("data-nve-camera-recording=\"1\""));

    // StopRecording + deactivate via NVNP prop update.
    let props_inactive = encode_cam_props(None, 1, 0, 1.0, 30, false, 0, false, false, false);
    let mut b3 = CommandBufferBuilder::new(1, 3);
    b3.dispatch_command(2, "stop_recording", &nvnc_encode(7, &[]));
    b3.set_raw_prop(2, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(props_inactive));
    b3.commit(true);
    backend.submit_frame(b3.finish()).unwrap();
    let cam_off = &backend.native_view(2).unwrap().camera;
    assert!(!cam_off.recording);
    assert!(!cam_off.active);
    assert_eq!(cam_off.device_id, None);
    assert_eq!(cam_off.stabilization, StabilizationMode::Off);
    let ssr_off = backend.render_ssr();
    assert!(ssr_off.html.contains("data-nve-camera-active=\"0\""));
    assert!(!ssr_off.html.contains("data-nve-camera-recording="));
    assert!(!ssr_off.html.contains("data-nve-camera-device="));

    // 4. Corrupt buffer, out-of-range enum, and invalid numeric argument resilience.
    let bad_facing = encode_cam_props(None, 0, 0, 1.0, 30, false, 0, false, false, true);
    assert!(matches!(
        decode_camera_preview_props(&NativePropDecoder::decode(&bad_facing).unwrap()),
        Err(NativeViewCodecError::InvalidEnum("CameraPreviewFacing", 0))
    ));
    let bad_torch = encode_cam_props(None, 1, 3, 1.0, 30, false, 0, false, false, true);
    assert!(matches!(
        decode_camera_preview_props(&NativePropDecoder::decode(&bad_torch).unwrap()),
        Err(NativeViewCodecError::InvalidEnum("TorchMode", 3))
    ));
    let bad_zoom = encode_cam_props(None, 1, 0, f32::NAN, 30, false, 0, false, false, true);
    assert!(matches!(
        decode_camera_preview_props(&NativePropDecoder::decode(&bad_zoom).unwrap()),
        Err(NativeViewCodecError::TypeMismatch(_))
    ));
    let bad_fps = encode_cam_props(None, 1, 0, 1.0, 241, false, 0, false, false, true);
    assert!(matches!(
        decode_camera_preview_props(&NativePropDecoder::decode(&bad_fps).unwrap()),
        Err(NativeViewCodecError::InvalidEnum("CameraFps", 241))
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(3, &[NativeValueRef::F32(-1.0)])),
        Err(NativeViewCodecError::TypeMismatch(_))
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(4, &[NativeValueRef::F32(1.5), NativeValueRef::F32(0.5)])),
        Err(NativeViewCodecError::TypeMismatch(_))
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(5, &[NativeValueRef::U32(101), NativeValueRef::U32(0)])),
        Err(NativeViewCodecError::InvalidEnum("PhotoQuality", 101))
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(5, &[NativeValueRef::U32(90), NativeValueRef::U32(3)])),
        Err(NativeViewCodecError::InvalidEnum("FlashMode", 3))
    ));
    assert!(matches!(
        decode_camera_preview_command(&nvnc_encode(99, &[])),
        Err(NativeViewCodecError::UnknownCommandOpcode(99))
    ));
    assert_eq!(
        decode_camera_preview_command(&nvnc_encode(7, &[])).unwrap(),
        CameraPreviewCommand::StopRecording
    );

    // Submitting corrupt NVNP / NVNC frames does not panic or overwrite valid state.
    let mut b4 = CommandBufferBuilder::new(1, 4);
    b4.set_raw_prop(2, NATIVE_VIEW_PROPS_PROP_KEY, &RawPropValue::Bytes(vec![0xDE, 0xAD]));
    b4.dispatch_command(2, "set_zoom", &[0x00, 0x01, 0x02]);
    b4.dispatch_command(2, "switch_facing", &nvnc_encode(1, &[NativeValueRef::U32(99)]));
    b4.commit(false);
    backend.submit_frame(b4.finish()).unwrap();
    assert_eq!(backend.native_view(2).unwrap().camera.facing, CameraPreviewFacing::Back);
    assert!((backend.native_view(2).unwrap().camera.zoom - 1.0).abs() < 1e-6);
}
