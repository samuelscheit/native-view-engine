use nve_abi::ColorRgba;
use nve_components::interaction::{CursorKind, InteractionState, StateStyleRule};
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::tables::{
    Accessibility, AriaRole, DebugId, FlexDirection, Interaction, Layout, Link, Responsive,
    ResponsiveRules, StateRules, TextContent, Transform, Typography, Visual,
};
use nve_components::{RowPatch, StylePatchExt};
use nve_engine::{HotPatchMsg, StoreBackend};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

use crate::{SsrDocumentOptions, WebBackend};

#[test]
fn web_backend_ssr_dedup_css_composite_fast_lane_hydrate_and_hot_patch() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut web = WebBackend::new();

    let mut tx = store.tx();
    let card1 = tx.create();
    let card2 = tx.create();
    let heading = tx.create();
    let link = tx.create();
    tx.set_children(NodeId::ROOT, &[card1, card2]);
    tx.set_children(card1, &[heading, link]);

    // Two cards with identical structural styles should deduplicate to the SAME `.n<hash>` class!
    for c in [card1, card2] {
        tx.edit::<Layout>(c)
            .flex_direction(FlexDirection::Row)
            .padding_top(Length::points(12.0))
            .padding_right(Length::points(12.0))
            .padding_bottom(Length::points(12.0))
            .padding_left(Length::points(12.0));
        tx.edit::<Visual>(c)
            .background(Rgba::rgb(15, 23, 42))
            .border_radius(8.0);
    }
    tx.edit::<Interaction>(card1)
        .cursor(CursorKind::Pointer)
        .state_rules(Some(StateRules(
            vec![StateStyleRule::new(
                InteractionState::HOVER,
                RowPatch::new().bg(ColorRgba::rgb(30, 64, 175)),
            )]
            .into(),
        )));
    tx.edit::<Responsive>(card1).rules(Some(ResponsiveRules(
        vec![ResponsiveRule::new(
            MediaQuery::expanded(),
            RowPatch::new().gap(24.0),
        )]
        .into(),
    )));
    tx.edit::<Accessibility>(heading)
        .role(Some(AriaRole::Heading))
        .level(Some(1));
    tx.edit::<TextContent>(heading).text("Hello NVE2 Web");
    tx.edit::<Typography>(heading).color(Rgba::WHITE).font_size(24.0);
    tx.edit::<DebugId>(heading).test_id("hero-title");
    tx.edit::<Link>(link).href("https://example.com");
    tx.edit::<TextContent>(link).text("Docs");
    tx.commit_into(&mut changes);

    web.commit(&store, &changes).unwrap();

    // Frame 2: COMPOSITE-only mutation on card1 hits the fast lane!
    let mut tx = store.tx();
    tx.edit::<Visual>(card1).opacity(0.8);
    tx.edit::<Transform>(card1).translate_x(12.0);
    tx.commit_into(&mut changes);
    web.commit(&store, &changes).unwrap();
    assert_eq!(web.composite_fast_lane_hits(), 1);

    // SSR emits semantic tags (`<h1>`, `<a href=...>`) + deduplicated CSS + `@media` + `:hover`.
    let ssr = web.render_ssr(&store);
    assert!(ssr.html.contains("<h1 "));
    assert!(ssr.html.contains("Hello NVE2 Web"));
    assert!(ssr.html.contains("<a "));
    assert!(ssr.html.contains("href=\"https://example.com\""));
    assert!(ssr.html.contains("data-testid=\"hero-title\""));
    assert!(ssr.style_css.contains("cursor:pointer;"));
    assert!(ssr.style_css.contains(":hover"));
    assert!(ssr.style_css.contains("@media"));

    let doc = web.render_ssr_document(
        &store,
        &SsrDocumentOptions {
            title: "NVE2 Test".into(),
            enable_event_replay: true,
            ..Default::default()
        },
    );
    assert!(doc.contains("<!doctype html>"));
    assert!(doc.contains("<style data-nve-base>"));
    assert!(doc.contains("<style data-nve-ssr"));

    // Hydration seeds rule hashes with zero duplicate CSS rules.
    let hydrated = WebBackend::hydrate_from_ssr(&ssr);
    assert!(hydrated.hydration_report().is_clean());
    assert_eq!(
        hydrated.hydration_report().reused_css_rules as usize,
        ssr.rule_hashes.len()
    );

    // Tier-1 hot-patch updates text directly in NodeStore!
    let patched = web.apply_template_patch(
        &mut store,
        &HotPatchMsg::TemplatePatch {
            generation: 1,
            template_id: "hero-title".into(),
            attr_updates: vec![("letter-spacing".into(), "1px".into())],
            text_updates: vec![],
            target_test_id: Some("hero-title".into()),
            node_path: vec![],
            text_replacements: vec![("Hello NVE2 Web".into(), "Patched NVE2 Web".into())],
        },
    );
    assert!(patched >= 1);
    let after = web.render_html(&store);
    assert!(after.contains("Patched NVE2 Web"));
    assert!(after.contains("letter-spacing:1px;"));
}

#[cfg(feature = "hydrate")]
#[test]
fn pre_hydration_press_on_child_text_bubbles_to_interactive_parent_and_drops_stale() {
    use nve_abi::{EventBatchBuf, InteractionEvent};
    use crate::CapturedPreHydrationEvent;

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let btn = tx.create();
    let label = tx.create();
    tx.set_children(NodeId::ROOT, &[btn]);
    tx.set_children(btn, &[label]);
    tx.edit::<Interaction>(btn).listen(InteractionEvent::Press);
    tx.edit::<TextContent>(label).text("Click Me");
    tx.commit_into(&mut changes);

    let mut ssr_web = WebBackend::new();
    ssr_web.commit(&store, &changes).unwrap();
    let ssr = ssr_web.render_ssr(&store);

    let mut hydrated = WebBackend::hydrate_from_ssr(&ssr);
    hydrated.queue_pre_hydration_events(vec![
        CapturedPreHydrationEvent::Press {
            node_id: label,
            x: 10.0,
            y: 12.0,
            age_ms: 25,
        },
        CapturedPreHydrationEvent::Press {
            node_id: label,
            x: 10.0,
            y: 12.0,
            age_ms: 60_000,
        },
    ]);
    hydrated.commit(&store, &changes).unwrap();
    let mut drained = EventBatchBuf::new();
    hydrated.drain_events(&mut drained);
    let validated = drained.as_validated();
    assert_eq!(hydrated.hydration_report().replayed_events, 1);
    assert_eq!(validated.len(), 1);
    let ev = validated.get(0).unwrap();
    assert_eq!(ev.interaction(), Some(InteractionEvent::Press));
    assert_eq!(ev.node(), btn);
}

#[test]
fn base_reset_css_keeps_element_defaults_zero_specificity() {
    let css = crate::NVE_BASE_RESET_CSS;
    assert!(css.contains(":where(.nve)"));
    assert!(css.contains(":where(span.nve){display:inline;"));
    assert!(css.contains(":where(a.nve)"));
    assert!(css.contains(":where(button.nve)"));
    assert!(!css.contains("}span.nve{"));
    assert!(!css.contains("}a.nve{"));
    assert!(!css.contains("}button.nve{"));
    assert!(css.starts_with("@layer nve-base,nve-s,nve-m,nve-st0,nve-st1,nve-st2,nve-st3,nve-st4;"));
}

#[test]
fn web_security_sanitizers_block_xss_and_css_breakout_vectors() {
    use nve_components::tables::{LinkTarget, TextInput};
    use crate::{is_safe_custom_tag, is_safe_web_attr, is_safe_web_url};

    // URL scheme & control-char smuggling vectors
    assert!(is_safe_web_url("https://example.com/docs"));
    assert!(is_safe_web_url("http://localhost:8080/a"));
    assert!(is_safe_web_url("mailto:hi@example.com"));
    assert!(is_safe_web_url("tel:+15551234567"));
    assert!(is_safe_web_url("/docs/start"));
    assert!(is_safe_web_url("./img.png"));
    assert!(is_safe_web_url("../img.png"));
    assert!(is_safe_web_url("#section"));
    assert!(is_safe_web_url("data:image/png;base64,iVBORw0KGgo="));
    assert!(!is_safe_web_url("javascript:alert(1)"));
    assert!(!is_safe_web_url("java\tscript:alert(1)"));
    assert!(!is_safe_web_url("java\nscript:alert(1)"));
    assert!(!is_safe_web_url("\x01javascript:alert(1)"));
    assert!(!is_safe_web_url("data:image/svg+xml,<svg onload=alert(1)>"));
    assert!(!is_safe_web_url("//evil.example/x.js"));
    assert!(!is_safe_web_url("/\\evil.example/x.js"));
    assert!(!is_safe_web_url("vbscript:msgbox(1)"));

    // Custom tag denylist
    assert!(is_safe_custom_tag("my-widget"));
    assert!(is_safe_custom_tag("article"));
    for bad_tag in [
        "script",
        "SCRIPT",
        "style",
        "iframe",
        "frame",
        "frameset",
        "object",
        "embed",
        "applet",
        "base",
        "meta",
        "link",
        "foreignObject",
        "animate",
        "set",
        "template",
        "noscript",
        "svg",
        "math",
        "1bad",
        "a b",
    ] {
        assert!(!is_safe_custom_tag(bad_tag), "tag {bad_tag} must be rejected");
    }

    // Attribute name & URL-bearing attribute checks
    assert!(is_safe_web_attr("data-state", "open"));
    assert!(is_safe_web_attr("data-nve-island", "ui-playground"));
    assert!(is_safe_web_attr("data-nve-hydrate", "visible"));
    assert!(is_safe_web_attr("aria-label", "Submit"));
    assert!(is_safe_web_attr("href", "https://example.com"));
    assert!(is_safe_web_attr("srcset", "/a.png 1x, https://cdn.example/b.webp 2x"));
    for (k, v) in [
        ("onclick", "alert(1)"),
        ("ONLOAD", "alert(1)"),
        ("is", "x-evil"),
        ("srcdoc", "<script>alert(1)</script>"),
        ("style", "background:url(javascript:alert(1))"),
        ("class", "injected"),
        ("data-nve-id", "999"),
        ("data-nve-ssr", "1"),
        ("data-nve-hydrated", "true"),
        ("attributeName", "href"),
        ("to", "javascript:alert(1)"),
        ("href", "java\tscript:alert(1)"),
        ("src", "data:image/svg+xml,<svg/>"),
        ("formaction", "//evil.com"),
        ("poster", "javascript:alert(1)"),
        ("data", "javascript:alert(1)"),
        ("srcset", "/ok.png 1x, javascript:alert(1) 2x"),
        ("1invalid", "x"),
    ] {
        assert!(
            !is_safe_web_attr(k, v),
            "attr {k}={v} must be rejected"
        );
    }

    // SSR: font-family CSS injection, </style> breakout, target="_blank" rel, and default maxlength
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut web = WebBackend::new();
    let mut tx = store.tx();
    let txt = tx.create();
    let link = tx.create();
    let inp = tx.create();
    let txt_clamped = tx.create();
    let inp_zero = tx.create();
    tx.set_children(NodeId::ROOT, &[txt, txt_clamped, link, inp, inp_zero]);
    tx.edit::<TextContent>(txt).text("safe");
    tx.edit::<Typography>(txt)
        .font_family("Inter;}</style><script>alert(1)</script>")
        .max_lines(core::num::NonZeroU16::new(0));
    tx.edit::<TextContent>(txt_clamped).text("clamped");
    tx.edit::<Typography>(txt_clamped).max_lines(core::num::NonZeroU16::new(3));
    tx.edit::<Link>(link)
        .href("https://example.com")
        .target(LinkTarget::Blank);
    tx.edit::<TextInput>(inp).max_length(None);
    tx.edit::<TextInput>(inp_zero).max_length(Some(0));
    tx.commit_into(&mut changes);
    web.commit(&store, &changes).unwrap();

    let doc = web.render_ssr_document(&store, &SsrDocumentOptions::default());
    assert!(!doc.contains("<script>alert(1)</script>"));
    assert!(doc.contains("target=\"_blank\""));
    assert!(doc.contains("rel=\"noopener noreferrer\""));
    assert!(!doc.contains("maxlength=\"65535\""));
    assert!(doc.contains("maxlength=\"0\""), "Some(0) max_length must be emitted: {doc}");
    assert!(!doc.contains("-webkit-line-clamp:0;"), "None max_lines must not emit broken CSS clamp: {doc}");
    assert_eq!(
        doc.matches("display:-webkit-box").count(),
        1,
        "companion -webkit-box layout CSS must emit only for the clamped node: {doc}"
    );
    assert!(doc.contains("-webkit-line-clamp:3;"), "Some(3) max_lines must emit clamp: {doc}");

    // Plain relative paths, query paths, data: srcset, multi-URL ping, data-nve-* prefix,
    // whitespace-padded tags/attrs, raw-text tags, font-family comments/quotes, and non-finite flex.
    assert!(is_safe_web_url("img.png"));
    assert!(is_safe_web_url("docs/guide"));
    assert!(is_safe_web_url("?q=foo:bar"));
    assert!(is_safe_web_attr("srcset", "data:image/png;base64,iVBORw0KGgo= 1x, img.png 2x"));
    assert!(!is_safe_web_attr("srcset", "data:image/png;base64,iVBORw0KGgo=, javascript:alert(1)"));
    assert!(is_safe_web_attr("ping", "https://a.example/p /b/p"));
    assert!(!is_safe_web_attr("ping", "https://a.example/p javascript:alert(1)"));
    assert!(!is_safe_web_attr("ping", "   "));
    assert!(!is_safe_web_attr("data-nve-internal-future", "1"));
    assert!(!is_safe_web_attr(" href", "https://example.com"));
    assert!(!is_safe_web_attr("href ", "https://example.com"));
    for bad in [" div", "div ", "plaintext", "xmp", "textarea", "title"] {
        assert!(!is_safe_custom_tag(bad), "tag {bad:?} must be rejected");
    }

    // Font-family comments / unbalanced quotes + non-finite flex_grow.
    let mut store2 = NodeStore::new();
    let mut changes2 = ChangeSet::default();
    let mut web2 = WebBackend::new();
    let n2 = {
        let mut tx = store2.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n).flex_grow(f32::NAN);
        tx.edit::<Typography>(n).font_family("Inter /* comment */");
        tx.commit_into(&mut changes2);
        n
    };
    web2.commit(&store2, &changes2).unwrap();
    let css2 = web2.render_ssr(&store2).style_css;
    assert!(!css2.contains("NaN"), "non-finite flex_grow leaked into CSS: {css2}");
    assert!(!css2.contains("/*"), "CSS comment in font_family leaked: {css2}");
    {
        let mut tx = store2.tx();
        tx.edit::<Typography>(n2).font_family("'Unclosed");
        tx.commit_into(&mut changes2);
    }
    web2.commit(&store2, &changes2).unwrap();
    let css3 = web2.render_ssr(&store2).style_css;
    assert!(!css3.contains("Unclosed"), "unbalanced quote in font_family leaked: {css3}");
}

#[test]
fn web_typography_explicit_default_color_and_empty_font_family_reset_emit_css() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut web = WebBackend::new();

    let mut tx = store.tx();
    let parent = tx.create();
    let child = tx.create();
    tx.set_children(NodeId::ROOT, &[parent]);
    tx.set_children(parent, &[child]);

    tx.edit::<Typography>(parent)
        .color(Rgba::WHITE)
        .font_family("Inter");
    tx.edit::<TextContent>(child).text("reset");
    // Explicitly write table-default BLACK color and empty font-family to override parent!
    tx.edit::<Typography>(child)
        .color(Rgba::BLACK)
        .font_family("");
    tx.commit_into(&mut changes);

    web.commit(&store, &changes).unwrap();
    let ssr = web.render_ssr(&store);
    assert!(
        ssr.style_css.contains("color:#000000;"),
        "explicit default BLACK color must emit CSS to override parent: {}",
        ssr.style_css
    );
    assert!(
        ssr.style_css.contains("font-family:initial;"),
        "explicit empty font_family must emit font-family:initial; to reset inherited parent font: {}",
        ssr.style_css
    );
}

#[test]
fn plugin_table_detach_automatically_clears_owned_attrs_inner_html_and_inline_css() {
    use nve_store::{NveTable, TableSlot};
    use crate::{WebSystemCommitCtx, WebTableSystem};

    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.plug", storage = "sparse")]
    struct TestPlug {
        #[effect(paint)]
        flag: u32,
    }

    struct PlugSys;
    impl WebTableSystem for PlugSys {
        fn slot(&self) -> TableSlot {
            TestPlug::slot()
        }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, _: &NodeStore, node: NodeId, _: u32) {
            ctx.set_attr(node, "data-plug", "on");
            ctx.set_inner_html(node, "<b>hi</b>".into());
            ctx.set_inline_css(node, "backdrop-filter:blur(4px);".into());
        }
    }

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut web = WebBackend::new();
    web.register_table_system(PlugSys);
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<TestPlug>(n).flag(1);
        tx.commit_into(&mut changes);
        n
    };
    web.commit(&store, &changes).unwrap();
    assert_eq!(web.renderer().plugin_attrs(n), &[("data-plug".to_string(), "on".to_string())]);
    assert_eq!(web.renderer().plugin_inner_html(n), Some("<b>hi</b>"));
    assert_eq!(web.renderer().plugin_inline_css(n), Some("backdrop-filter:blur(4px);"));

    // Detaching the plugin table without a custom remove_node impl must still clear all owned state.
    {
        let mut tx = store.tx();
        tx.detach::<TestPlug>(n);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert!(web.renderer().plugin_attrs(n).is_empty());
    assert!(web.renderer().plugin_inner_html(n).is_none());
    assert!(web.renderer().plugin_inline_css(n).is_none());
}

#[test]
fn css_visibility_emits_visible_hidden_and_omits_inherit() {
    use nve_components::tables::{Visibility, Visual};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut web = WebBackend::new();
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Visual>(n).visibility(Visibility::Inherit);
        tx.commit_into(&mut changes);
        n
    };
    web.commit(&store, &changes).unwrap();
    let ssr_inh = web.render_ssr(&store);
    assert!(!ssr_inh.style_css.contains("visibility:"), "Inherit must omit visibility: {}", ssr_inh.style_css);

    {
        let mut tx = store.tx();
        tx.edit::<Visual>(n).visibility(Visibility::Visible);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    let ssr_vis = web.render_ssr(&store);
    assert!(ssr_vis.style_css.contains("visibility:visible;"), "Visible must emit visibility:visible: {}", ssr_vis.style_css);

    {
        let mut tx = store.tx();
        tx.edit::<Visual>(n).visibility(Visibility::Hidden);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    let ssr_hid = web.render_ssr(&store);
    assert!(ssr_hid.style_css.contains("visibility:hidden;"), "Hidden must emit visibility:hidden: {}", ssr_hid.style_css);

    // Transitioning Hidden -> Inherit must drop `visibility:hidden;`.
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(n).visibility(Visibility::Inherit);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    let ssr_back = web.render_ssr(&store);
    assert!(
        !ssr_back.style_css.contains("visibility:"),
        "Hidden -> Inherit must clear visibility: {}",
        ssr_back.style_css
    );

    // CSS visibility keywords: collapse -> Hidden, initial -> Visible, inherit/unset/revert/revert-layer -> Inherit, invalid ignored.
    {
        let mut tx = store.tx();
        assert!(nve_components::apply_css_attr_to_tx(&mut tx, n, "visibility", "collapse"));
        tx.commit_into(&mut changes);
    }
    assert_eq!(store.row::<Visual>(n).visibility(), Visibility::Hidden);
    {
        let mut tx = store.tx();
        assert!(nve_components::apply_css_attr_to_tx(&mut tx, n, "visibility", "bogus"));
        tx.commit_into(&mut changes);
    }
    assert_eq!(store.row::<Visual>(n).visibility(), Visibility::Hidden, "invalid visibility value must not overwrite");
    {
        let mut tx = store.tx();
        assert!(nve_components::apply_css_attr_to_tx(&mut tx, n, "visibility", "initial"));
        tx.commit_into(&mut changes);
    }
    assert_eq!(store.row::<Visual>(n).visibility(), Visibility::Visible);
    for kw in ["unset", "revert", "revert-layer", "inherit"] {
        {
            let mut tx = store.tx();
            tx.edit::<Visual>(n).visibility(Visibility::Hidden);
            tx.commit_into(&mut changes);
        }
        {
            let mut tx = store.tx();
            assert!(nve_components::apply_css_attr_to_tx(&mut tx, n, "visibility", kw));
            tx.commit_into(&mut changes);
        }
        assert_eq!(
            store.row::<Visual>(n).visibility(),
            Visibility::Inherit,
            "CSS visibility keyword `{kw}` must resolve to Visibility::Inherit"
        );
    }
}

#[test]
fn cross_slot_plugin_ownership_protects_other_slot_attrs_and_inline_css() {
    use nve_store::{NveTable, TableSlot};
    use crate::{WebSystemCommitCtx, WebTableSystem};

    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.owner_a", storage = "sparse")]
    struct OwnerA {
        #[effect(paint)]
        v: u32,
    }
    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.owner_b", storage = "sparse")]
    struct OwnerB {
        #[effect(paint)]
        v: u32,
    }

    struct SysA;
    impl WebTableSystem for SysA {
        fn slot(&self) -> TableSlot { OwnerA::slot() }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, store: &NodeStore, node: NodeId, _: u32) {
            if store.row::<OwnerA>(node).v() == 1 {
                ctx.set_attr(node, "data-shared", "a");
                ctx.set_inline_css(node, "filter:blur(2px);".into());
            } else {
                ctx.remove_attr(node, "data-shared");
                ctx.set_inline_css(node, String::new());
            }
        }
    }
    struct SysB;
    impl WebTableSystem for SysB {
        fn slot(&self) -> TableSlot { OwnerB::slot() }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, store: &NodeStore, node: NodeId, _: u32) {
            if store.row::<OwnerB>(node).v() == 1 {
                ctx.set_attr(node, "data-shared", "b");
                ctx.set_inline_css(node, "backdrop-filter:blur(4px);".into());
            } else {
                ctx.remove_attr(node, "data-shared");
                ctx.set_inline_css(node, String::new());
            }
        }
    }

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut web = WebBackend::new();
    web.register_table_system(SysA);
    web.register_table_system(SysB);

    // Order 1: SysB commits first, then SysA overwrites both, then SysA removes/clears -> SysB's exact values restore!
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<OwnerB>(n).v(1);
        tx.commit_into(&mut changes);
        n
    };
    web.commit(&store, &changes).unwrap();
    {
        let mut tx = store.tx();
        tx.edit::<OwnerA>(n).v(1);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(web.renderer().plugin_attrs(n), &[("data-shared".to_string(), "a".to_string())]);
    assert_eq!(web.renderer().plugin_inline_css(n), Some("filter:blur(2px);"));

    {
        let mut tx = store.tx();
        tx.edit::<OwnerA>(n).v(2);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n),
        &[("data-shared".to_string(), "b".to_string())],
        "removing attr from SysA must restore SysB's exact value"
    );
    assert_eq!(
        web.renderer().plugin_inline_css(n),
        Some("backdrop-filter:blur(4px);"),
        "clearing inline_css from SysA must restore SysB's exact inline CSS"
    );

    // Order 2: SysA sets again, then SysB detaches -> SysA's exact values restore!
    {
        let mut tx = store.tx();
        tx.edit::<OwnerA>(n).v(1);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    {
        let mut tx = store.tx();
        tx.detach::<OwnerB>(n);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(web.renderer().plugin_attrs(n), &[("data-shared".to_string(), "a".to_string())]);
    assert_eq!(web.renderer().plugin_inline_css(n), Some("filter:blur(2px);"));

    // Detaching SysA too cleans everything.
    {
        let mut tx = store.tx();
        tx.detach::<OwnerA>(n);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert!(web.renderer().plugin_attrs(n).is_empty());
    assert!(web.renderer().plugin_inline_css(n).is_none());
}

#[test]
fn three_plugin_write_sequence_precedence_and_non_owner_guard() {
    use nve_store::{NveTable, TableSlot};
    use crate::{WebSystemCommitCtx, WebTableSystem};

    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.seq_a", storage = "sparse")]
    struct SeqA {
        #[effect(paint)]
        v: u32,
    }
    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.seq_b", storage = "sparse")]
    struct SeqB {
        #[effect(paint)]
        v: u32,
    }
    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.seq_c", storage = "sparse")]
    struct SeqC {
        #[effect(paint)]
        v: u32,
    }

    struct SysSeqA;
    impl WebTableSystem for SysSeqA {
        fn slot(&self) -> TableSlot { SeqA::slot() }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, store: &NodeStore, node: NodeId, _: u32) {
            match store.row::<SeqA>(node).v() {
                1 => {
                    ctx.set_attr(node, "data-shared", "a");
                    ctx.set_attr(node, "aria-label", "plugin-a-label");
                    ctx.set_inline_css(node, "filter:blur(1px);".into());
                }
                2 => {
                    ctx.set_attr(node, "data-shared", "a2");
                    ctx.set_inline_css(node, "filter:blur(10px);".into());
                }
                3 => {
                    ctx.set_attr(node, "data-k1", "a1");
                    ctx.set_attr(node, "data-k2", "a2");
                }
                _ => {
                    ctx.remove_attr(node, "data-shared");
                    ctx.remove_attr(node, "aria-label");
                    ctx.set_inline_css(node, String::new());
                }
            }
        }
    }
    struct SysSeqB;
    impl WebTableSystem for SysSeqB {
        fn slot(&self) -> TableSlot { SeqB::slot() }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, store: &NodeStore, node: NodeId, _: u32) {
            match store.row::<SeqB>(node).v() {
                1 => {
                    ctx.set_attr(node, "data-shared", "b");
                    ctx.set_inline_css(node, "filter:blur(2px);".into());
                }
                2 => {
                    // Non-owner removal attempts on another plugin's attrs and inline CSS.
                    ctx.remove_attr(node, "data-shared");
                    ctx.remove_attr(node, "aria-label");
                    ctx.remove_attr(node, "data-unowned");
                    ctx.set_inline_css(node, String::new());
                }
                3 => {
                    ctx.set_attr(node, "data-k1", "b1");
                    ctx.set_attr(node, "data-k2", "b2");
                }
                4 => {
                    ctx.retain_slot_attrs(node, |k| k == "data-k1");
                }
                5 => {
                    ctx.retain_slot_attrs(node, |_| false);
                }
                _ => {}
            }
        }
    }
    struct SysSeqC;
    impl WebTableSystem for SysSeqC {
        fn slot(&self) -> TableSlot { SeqC::slot() }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, store: &NodeStore, node: NodeId, _: u32) {
            match store.row::<SeqC>(node).v() {
                1 => {
                    ctx.set_attr(node, "data-shared", "c");
                    ctx.set_inline_css(node, "filter:blur(3px);".into());
                }
                2 => {
                    ctx.set_attr(node, "data-shared", "c2");
                    ctx.set_inline_css(node, "filter:blur(4px);".into());
                }
                3 => {
                    ctx.set_attr(node, "data-k1", "c1");
                }
                _ => {
                    ctx.remove_attr(node, "data-shared");
                    ctx.set_inline_css(node, String::new());
                }
            }
        }
    }

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut web = WebBackend::new();
    web.register_table_system(SysSeqA);
    web.register_table_system(SysSeqB);
    web.register_table_system(SysSeqC);

    // 1. A writes "a" + "aria-label", C writes "c"; non-owner B remove_attr / set_inline_css("")
    //    must leave both plugin-owned attrs and C's live values completely untouched.
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<SeqA>(n).v(1);
        tx.commit_into(&mut changes);
        n
    };
    web.commit(&store, &changes).unwrap();
    {
        let mut tx = store.tx();
        tx.edit::<SeqC>(n).v(1);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    {
        let mut tx = store.tx();
        tx.edit::<SeqB>(n).v(2);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n),
        &[
            ("data-shared".to_string(), "c".to_string()),
            ("aria-label".to_string(), "plugin-a-label".to_string()),
        ],
        "non-owner B remove_attr must leave C's data-shared and A's aria-label exact and leak-free"
    );
    assert_eq!(web.renderer().plugin_inline_css(n), Some("filter:blur(3px);"));

    // 2. Real 3-plugin max-seq tiebreak (all three live: A < B < C):
    //    detaching C must restore B (NOT registry-first A!), then detaching B restores A.
    {
        let mut tx = store.tx();
        tx.edit::<SeqB>(n).v(1);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    {
        let mut tx = store.tx();
        tx.edit::<SeqC>(n).v(2);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n),
        &[
            ("data-shared".to_string(), "c2".to_string()),
            ("aria-label".to_string(), "plugin-a-label".to_string()),
        ]
    );
    assert_eq!(web.renderer().plugin_inline_css(n), Some("filter:blur(4px);"));

    {
        let mut tx = store.tx();
        tx.detach::<SeqC>(n);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n),
        &[
            ("data-shared".to_string(), "b".to_string()),
            ("aria-label".to_string(), "plugin-a-label".to_string()),
        ],
        "detaching C with A < B < C must restore B (max seq), not registry-first A"
    );
    assert_eq!(
        web.renderer().plugin_inline_css(n),
        Some("filter:blur(2px);"),
        "detaching C with A < B < C must restore B's inline CSS"
    );

    // 3. Non-registry write order (B < A2 < C): detaching C must restore A2 (max seq), not B!
    {
        let mut tx = store.tx();
        tx.edit::<SeqA>(n).v(2);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    {
        let mut tx = store.tx();
        tx.edit::<SeqC>(n).v(1);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n),
        &[
            ("data-shared".to_string(), "c".to_string()),
            ("aria-label".to_string(), "plugin-a-label".to_string()),
        ]
    );
    {
        let mut tx = store.tx();
        tx.detach::<SeqC>(n);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n),
        &[
            ("data-shared".to_string(), "a2".to_string()),
            ("aria-label".to_string(), "plugin-a-label".to_string()),
        ],
        "detaching C with B < A2 < C must restore A2 (max seq), not B"
    );
    assert_eq!(web.renderer().plugin_inline_css(n), Some("filter:blur(10px);"));

    // Detaching older owner B must leave newer owner A2 intact; detaching A cleans everything.
    {
        let mut tx = store.tx();
        tx.detach::<SeqB>(n);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n),
        &[
            ("data-shared".to_string(), "a2".to_string()),
            ("aria-label".to_string(), "plugin-a-label".to_string()),
        ]
    );
    assert_eq!(web.renderer().plugin_inline_css(n), Some("filter:blur(10px);"));
    {
        let mut tx = store.tx();
        tx.detach::<SeqA>(n);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert!(web.renderer().plugin_attrs(n).is_empty());
    assert!(web.renderer().plugin_inline_css(n).is_none());

    // 4. `retain_slot_attrs` across 3 plugins (A < B < C on data-k1, A < B on data-k2):
    let n2 = {
        let mut tx = store.tx();
        let n2 = tx.create();
        tx.set_children(NodeId::ROOT, &[n, n2]);
        tx.edit::<SeqA>(n2).v(3);
        tx.commit_into(&mut changes);
        n2
    };
    web.commit(&store, &changes).unwrap();
    {
        let mut tx = store.tx();
        tx.edit::<SeqB>(n2).v(3);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    {
        let mut tx = store.tx();
        tx.edit::<SeqC>(n2).v(3);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n2),
        &[
            ("data-k1".to_string(), "c1".to_string()),
            ("data-k2".to_string(), "b2".to_string()),
        ]
    );

    // B retains only data-k1 -> drops data-k2 (restores A's "a2"), leaves data-k1 ("c1") intact.
    {
        let mut tx = store.tx();
        tx.edit::<SeqB>(n2).v(4);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n2),
        &[
            ("data-k1".to_string(), "c1".to_string()),
            ("data-k2".to_string(), "a2".to_string()),
        ]
    );

    // B drops data-k1 too -> C is newer than B, so data-k1 MUST stay "c1" (not regress to "a1")!
    {
        let mut tx = store.tx();
        tx.edit::<SeqB>(n2).v(5);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n2),
        &[
            ("data-k1".to_string(), "c1".to_string()),
            ("data-k2".to_string(), "a2".to_string()),
        ]
    );

    // Detaching C restores A's "a1".
    {
        let mut tx = store.tx();
        tx.detach::<SeqC>(n2);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes).unwrap();
    assert_eq!(
        web.renderer().plugin_attrs(n2),
        &[
            ("data-k1".to_string(), "a1".to_string()),
            ("data-k2".to_string(), "a2".to_string()),
        ]
    );
}

#[test]
fn retag_lifecycle_detaches_before_mirror_replay_and_gates_unchanged_sync_in_o1() {
    use std::cell::RefCell;

    use nve_abi::hash::HashMap;
    use nve_store::{NveTable, TableSlot};

    use crate::table_system::RetagElementSink;
    use crate::{WebStoreRenderer, WebSystemCommitCtx, WebTableSystem};

    #[derive(Default)]
    struct MockRetagEl {
        attrs: RefCell<Vec<(String, String)>>,
        inner_html: RefCell<Option<String>>,
    }
    impl RetagElementSink for MockRetagEl {
        fn set_attr(&self, name: &str, value: &str) {
            self.attrs.borrow_mut().push((name.to_owned(), value.to_owned()));
        }
        fn set_inner_html(&self, html: &str) {
            *self.inner_html.borrow_mut() = Some(html.to_owned());
        }
    }

    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.retag_detach", storage = "sparse")]
    struct RetagDetach {
        #[effect(paint)]
        v: u32,
    }
    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.retag_canvas", storage = "sparse")]
    struct RetagCanvas {
        #[effect(paint)]
        ops: u32,
    }
    #[allow(dead_code)]
    #[derive(NveTable)]
    #[nve_table(name = "web.test.retag_changed", storage = "sparse")]
    struct RetagChanged {
        #[effect(paint)]
        v: u32,
    }

    struct DetachSys;
    impl WebTableSystem for DetachSys {
        fn slot(&self) -> TableSlot { RetagDetach::slot() }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, _store: &NodeStore, node: NodeId, _: u32) {
            ctx.set_attr(node, "data-webview", "1");
            ctx.set_inline_css(node, "filter:blur(8px);".into());
            ctx.set_inner_html(node, "<p>stale</p>".into());
        }
    }
    struct CanvasSys;
    impl WebTableSystem for CanvasSys {
        fn slot(&self) -> TableSlot { RetagCanvas::slot() }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, store: &NodeStore, node: NodeId, _: u32) {
            let ops = store.row::<RetagCanvas>(node).ops();
            ctx.set_attr(node, "data-nve-draw-ops", &ops.to_string());
        }
    }
    struct ChangedSys;
    impl WebTableSystem for ChangedSys {
        fn slot(&self) -> TableSlot { RetagChanged::slot() }
        fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, store: &NodeStore, node: NodeId, _: u32) {
            let v = store.row::<RetagChanged>(node).v();
            ctx.set_attr(node, "data-changed", &format!("v{v}"));
        }
    }

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut renderer = WebStoreRenderer::new()
        .with_table_system(DetachSys)
        .with_table_system(CanvasSys)
        .with_table_system(ChangedSys);

    // Frame 1: attach all 3 plugins onto `n`.
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<RetagDetach>(n).v(1);
        tx.edit::<RetagCanvas>(n).ops(3);
        tx.edit::<RetagChanged>(n).v(1);
        tx.commit_into(&mut changes);
        n
    };
    renderer.commit(&store, &changes).unwrap();

    // Frame 2: same-frame retag where RetagDetach detaches, RetagChanged mutates, and RetagCanvas
    // remains attached & unchanged.
    {
        let mut tx = store.tx();
        tx.detach::<RetagDetach>(n);
        tx.edit::<RetagChanged>(n).v(2);
        tx.commit_into(&mut changes);
    }
    let mut live: HashMap<NodeId, MockRetagEl> = HashMap::default();
    live.insert(n, MockRetagEl::default());
    let mut synced_slots: Vec<(TableSlot, NodeId)> = Vec::new();
    renderer.commit_plugins_active_with(
        &store,
        &changes,
        #[cfg(target_arch = "wasm32")]
        None,
        Some(&live),
        &[n],
        |sys, nid, _| synced_slots.push((sys.slot(), nid)),
    );

    let el = live.get(&n).unwrap();
    let replayed = el.attrs.borrow();
    // 1) Detached plugin's attr, inline CSS, and innerHTML MUST NOT touch the fresh retagged element.
    assert!(
        !replayed.iter().any(|(k, _)| k == "data-webview" || k == "data-nve-mod-style"),
        "detached plugin must be purged before mirror replay: {replayed:?}"
    );
    assert_eq!(
        *el.inner_html.borrow(),
        None,
        "detached plugin innerHTML must not be replayed onto fresh retagged element"
    );
    // Remaining attached plugins' mirrors WERE replayed.
    assert!(replayed.iter().any(|(k, v)| k == "data-nve-draw-ops" && v == "3"));
    // Step 2 committed RetagChanged to v2 in the authoritative mirror.
    assert_eq!(
        renderer.plugin_attrs(n),
        &[
            ("data-nve-draw-ops".to_string(), "3".to_string()),
            ("data-changed".to_string(), "v2".to_string()),
        ]
    );
    // 2) Step 2b sync ran ONCE for unchanged RetagCanvas, and SKIPPED RetagChanged (committed in Step 2)
    //    and RetagDetach (detached in Step 1).
    assert_eq!(
        synced_slots,
        vec![(RetagCanvas::slot(), n)],
        "only unchanged attached plugins must run sync_retagged_element"
    );

    // Frame 3: consecutive retag frame where RetagCanvas mutates and RetagChanged (which was
    // committed in Frame 2) is unchanged -> proves `retag_committed_scratch.clear()` resets per frame!
    {
        let mut tx = store.tx();
        tx.edit::<RetagCanvas>(n).ops(7);
        tx.commit_into(&mut changes);
    }
    let mut live3: HashMap<NodeId, MockRetagEl> = HashMap::default();
    live3.insert(n, MockRetagEl::default());
    synced_slots.clear();
    renderer.commit_plugins_active_with(
        &store,
        &changes,
        #[cfg(target_arch = "wasm32")]
        None,
        Some(&live3),
        &[n],
        |sys, nid, _| synced_slots.push((sys.slot(), nid)),
    );
    assert_eq!(
        synced_slots,
        vec![(RetagChanged::slot(), n)],
        "consecutive retag frame must clear prior scratch so unchanged RetagChanged syncs once and changed RetagCanvas is skipped"
    );
    assert!(
        live3
            .get(&n)
            .unwrap()
            .attrs
            .borrow()
            .iter()
            .any(|(k, v)| k == "data-changed" && v == "v2"),
        "Frame 3 mirror replay must copy RetagChanged's v2 onto fresh retagged element"
    );
    assert_eq!(
        renderer.plugin_attrs(n),
        &[
            ("data-nve-draw-ops".to_string(), "7".to_string()),
            ("data-changed".to_string(), "v2".to_string()),
        ]
    );
}

#[test]
fn ssr_emits_aria_live_for_off_polite_assertive_and_omits_unset() {
    use nve_components::tables::{Accessibility, AriaLive, AriaRole};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (unset, off, polite, assertive) = {
        let mut tx = store.tx();
        let u = tx.create();
        let o = tx.create();
        let p = tx.create();
        let a = tx.create();
        tx.set_children(NodeId::ROOT, &[u, o, p, a]);
        tx.edit::<Accessibility>(u).live(None);
        tx.edit::<Accessibility>(o).live(Some(AriaLive::Off));
        tx.edit::<Accessibility>(p).live(Some(AriaLive::Polite));
        tx.edit::<Accessibility>(a)
            .role(Some(AriaRole::Alert))
            .live(Some(AriaLive::Assertive));
        tx.commit_into(&mut changes);
        (u, o, p, a)
    };

    let mut backend = WebBackend::new();
    backend.commit(&store, &changes).unwrap();
    let html = backend.render_html(&store);
    let tag_for = |id: NodeId| {
        let needle = format!("data-nve-id=\"{}\"", id.raw());
        html.split('<')
            .find(|seg| seg.contains(&needle))
            .and_then(|seg| seg.split('>').next())
            .unwrap_or_else(|| panic!("missing element for {needle}: {html}"))
            .to_owned()
    };

    assert!(!tag_for(unset).contains("aria-live"));
    assert!(tag_for(off).contains("aria-live=\"off\""));
    assert!(tag_for(polite).contains("aria-live=\"polite\""));
    let assertive_tag = tag_for(assertive);
    assert!(assertive_tag.contains("role=\"alert\""));
    assert!(assertive_tag.contains("aria-live=\"assertive\""));
}

#[test]
fn patch_css_emits_single_side_padding_margin_logical_combo_overflow_axes_and_nested_wire_resources() {
    use nve_components::interaction::{InteractionState, StateStyleRule};
    use nve_components::layout_ext::GridTrack;
    use nve_components::media::{MediaQuery, ResponsiveRule};
    use nve_components::paint::{BackgroundFill, GradientStop};
    use nve_components::tables::{
        BackgroundFillRes, GridLayout, GridTracks, Interaction, Layout, Overflow, Responsive,
        ResponsiveRules, StateRules, Visual,
    };
    use nve_store::wire::{WireDecoder, WireEncoder};
    use nve_store::{Length, NodeStore};

    let decls = |patch: &RowPatch, store: Option<&NodeStore>, css: &mut String| {
        let mut words = Vec::new();
        assert!(nve_store::FlatResource::write_words(patch, &mut words));
        let view = nve_store::RowPatchView::from_words(&words).expect("valid RowPatch words");
        crate::patch_css::write_row_patch_view_decls(view, store, css);
    };

    // 1. Single-side padding & margin are emitted (never silently dropped).
    let mut single_side = RowPatch::new();
    single_side
        .set(Layout::PADDING_TOP, Length::points(12.0))
        .set(Layout::MARGIN_LEFT, Length::points(-4.0));
    let mut css = String::new();
    decls(&single_side, None, &mut css);
    assert!(css.contains("padding-top:12px;"), "got: {css}");
    assert!(css.contains("margin-left:-4px;"), "got: {css}");

    // 2. Combining `.padding(8.0).padding_inline_start(16.0)` preserves top/bottom + logical inline.
    let combo = RowPatch::new()
        .padding(8.0)
        .padding_inline_start(16.0)
        .margin(6.0)
        .margin_inline_end(10.0);
    css.clear();
    decls(&combo, None, &mut css);
    assert!(css.contains("padding-top:8px;"), "got: {css}");
    assert!(css.contains("padding-bottom:8px;"), "got: {css}");
    assert!(css.contains("padding-inline-start:16px;"), "got: {css}");
    assert!(css.contains("margin-top:6px;"), "got: {css}");
    assert!(css.contains("margin-bottom:6px;"), "got: {css}");
    assert!(css.contains("margin-inline-end:10px;"), "got: {css}");

    // 3. Overflow X/Y distinct, single-axis Y, and equal shorthand.
    let mut ov_xy = RowPatch::new();
    ov_xy
        .set(Visual::OVERFLOW_X, Overflow::Hidden)
        .set(Visual::OVERFLOW_Y, Overflow::Auto);
    css.clear();
    decls(&ov_xy, None, &mut css);
    assert!(css.contains("overflow-x:hidden;overflow-y:auto;"), "got: {css}");

    let mut ov_y_only = RowPatch::new();
    ov_y_only.set(Visual::OVERFLOW_Y, Overflow::Scroll);
    css.clear();
    decls(&ov_y_only, None, &mut css);
    assert!(css.contains("overflow-y:scroll;"), "got: {css}");
    assert!(!css.contains("overflow-x:"), "got: {css}");

    // 4. Nested `BackgroundFillRes` inside `StateRules` + nested `GridTracks` inside `ResponsiveRules`
    //    round-tripped over `NVE2` wire into a replica `WebBackend` SSR stylesheet!
    let mut prod = NodeStore::new();
    let mut prod_changes = ChangeSet::default();
    let node = {
        let mut tx = prod.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.with_temp_resources(|scope| {
            let grad_id = scope.intern(&BackgroundFillRes(BackgroundFill::linear(
                45.0,
                vec![
                    GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0)),
                    GradientStop::new(1.0, ColorRgba::rgb(0, 0, 255)),
                ],
            )));
            let tracks_id = scope.intern(&GridTracks::new(vec![
                GridTrack::Fr(1.0),
                GridTrack::Px(240.0),
            ]));
            let hover_patch = RowPatch::new().background_fill_id(grad_id);
            let media_patch = RowPatch::new().with_res_id(GridLayout::TEMPLATE_COLUMNS, tracks_id);
            scope.tx().edit::<Interaction>(n).state_rules(Some(StateRules::new(vec![
                StateStyleRule::new(InteractionState::HOVER, hover_patch),
            ])));
            scope.tx().edit::<Responsive>(n).rules(Some(ResponsiveRules::new(vec![
                ResponsiveRule {
                    query: MediaQuery::min_width(900),
                    patch: media_patch,
                },
            ])));
        });
        tx.commit_into(&mut prod_changes);
        n
    };
    let mut wire = Vec::new();
    WireEncoder::default().encode(&prod, &prod_changes, &mut wire);

    let mut replica = NodeStore::replica();
    let mut rep_changes = ChangeSet::default();
    {
        let mut rtx = replica.tx();
        WireDecoder::default().apply(&mut rtx, &wire).unwrap();
        rtx.commit_into(&mut rep_changes);
    }
    let mut web = WebBackend::new();
    web.commit(&replica, &rep_changes).unwrap();
    let _html = web.render_html(&replica);
    let (css_out, _) = web.stylesheet().serialize_css();
    assert!(
        css_out.contains("linear-gradient(45deg"),
        "wire replica Web CSS must emit nested StateRules gradient: {css_out}"
    );
    assert!(
        css_out.contains("grid-template-columns:1fr 240px;"),
        "wire replica Web CSS must emit nested ResponsiveRules GridTracks: {css_out}"
    );
    let _ = node;
}

#[test]
fn synchronous_reentrant_commit_during_event_callback_has_no_refcell_borrow_conflict() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use nve_abi::{EventBatchBuf, InteractionEvent, SurfaceId, TimestampUs};
    use nve_components::tables::{Interaction, TextInput};
    use nve_store::{ChangeSet, NodeId, NodeStore};

    use crate::events::WebEventState;
    use crate::input_attrs::InputSyncTracker;

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (btn, inp) = {
        let mut tx = store.tx();
        let b = tx.create();
        let i = tx.create();
        tx.set_children(NodeId::ROOT, &[b, i]);
        tx.edit::<Interaction>(b).listen(InteractionEvent::Press);
        tx.edit::<TextInput>(i).value("init");
        tx.commit_into(&mut changes);
        (b, i)
    };

    let state = Rc::new(RefCell::new(WebEventState::new()));
    let sync = Rc::new(RefCell::new(InputSyncTracker::new()));
    let events = Rc::new(RefCell::new(EventBatchBuf::new()));
    state.borrow_mut().sync(&store, &changes, |_, _, _| {});

    // Simulate synchronous `on_event` pumping `WebBackend::commit` -> `state.borrow_mut()` &
    // `sync.borrow_mut()` & `events.borrow_mut().clear()` inside click / input / compositionend.
    let reentrant_commit = |st: &Rc<RefCell<WebEventState>>,
                            sy: &Rc<RefCell<InputSyncTracker>>,
                            ev: &Rc<RefCell<EventBatchBuf>>,
                            s: &NodeStore,
                            c: &ChangeSet| {
        st.borrow_mut().sync(s, c, |_, _, _| {});
        sy.borrow_mut().remove(inp);
        ev.borrow_mut().clear();
    };

    // 1. Click path: drop `st_ref` before `notify_event`.
    let st_ref = state.borrow();
    let click_id = st_ref.listens::<Interaction>(btn, InteractionEvent::Press).then_some(btn);
    drop(st_ref);
    assert_eq!(click_id, Some(btn));
    events.borrow_mut().push_press(SurfaceId::ROOT, btn, TimestampUs(1), 5.0, 5.0);
    reentrant_commit(&state, &sync, &events, &store, &changes);

    // 2. Input path: release `sync.borrow_mut()` before `notify_event`.
    let changed = sync.borrow_mut().on_input_event(inp, "abc".into(), false);
    assert_eq!(changed.as_deref(), Some("abc"));
    if let Some(val) = changed {
        events.borrow_mut().push_text_change(SurfaceId::ROOT, inp, TimestampUs(2), &val);
        reentrant_commit(&state, &sync, &events, &store, &changes);
    }

    // 3. CompositionEnd path: drop `guard` before `notify_event`.
    {
        let mut guard = sync.borrow_mut();
        let _ = guard.on_input_event(inp, "你好".into(), true);
        drop(guard);
        reentrant_commit(&state, &sync, &events, &store, &changes);
    }
}
