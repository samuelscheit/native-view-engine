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
        enable_event_replay: false,
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

