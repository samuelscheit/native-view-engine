//! Comprehensive Wave C2A integration tests:
//! 1. Zero-serialization `@media` CSS compilation (`show_on` + `ResponsiveRule` overrides,
//!    cascade order, hydration dedup, live updates, injection safety).
//! 2. Pre-hydration `<script data-nve-replay>` HTML emission, pure-Rust replay filter,
//!    end-to-end hydration replay, and post-hydration `<a href>` SPA interception matrix.
//! 3. Reusable 3-way `StateReconciler` dirty-control hydration preservation.

#[cfg(all(feature = "ssr", feature = "hydrate"))]
use std::cell::Cell;
#[cfg(all(feature = "ssr", feature = "hydrate"))]
use std::rc::Rc;

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, FlexAlign, FlexJustify,
    LayoutProps, ROOT_NODE_ID,
};
#[cfg(feature = "ssr")]
use nve_abi::{FlexDirection, StyleProps};
#[cfg(all(feature = "ssr", feature = "hydrate"))]
use nve_abi::{EventMask, EventPayload, NativeEventKind, RawPropValue};
use nve_components::media::{
    FlexWrap, MediaColorScheme, MediaPointer, MediaQuery, Overflow, ResponsiveRule,
    ResponsiveStylePatch, TextAlign,
};
use nve_engine::RenderBackend;

use nve_backend_web::{
    compile_node_media_rules, drain_valid_pre_hydration_events, format_media_class,
    should_intercept_anchor_click, write_responsive_patch_decls, CapturedPreHydrationEvent,
    StyleSheetCache, WebBackend, MAX_REPLAY_AGE_MS,
};
#[cfg(feature = "ssr")]
use nve_backend_web::{SsrDocumentOptions, PRE_HYDRATION_REPLAY_SCRIPT_TAG};

#[test]
fn responsive_patch_emits_all_typed_fields_including_zero_resets_and_sanitizes_non_finite() {
    let patch = ResponsiveStylePatch::new()
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
        .flex_grow(0.0)
        .flex_shrink(0.0)
        .gap(0.0)
        .width_auto()
        .height_pct(100.0)
        .min_width_px(240.0)
        .max_width_px(960.0)
        .min_height_px(48.0)
        .max_height_px(640.0)
        .padding(0.0)
        .margin_xy(12.0, 8.0)
        .bg(ColorRgba::rgba(15, 23, 42, 255))
        .color(ColorRgba::rgba(255, 255, 255, 255))
        .border(2.0, ColorRgba::rgba(99, 102, 241, 255))
        .radius(12.0)
        .opacity(0.75)
        .font_size(18.0)
        .z_index(10)
        .flex_wrap(FlexWrap::Wrap)
        .overflow(Overflow::Hidden)
        .text_align(TextAlign::Center)
        .font_weight(700);

    let mut decls = String::new();
    write_responsive_patch_decls(&patch, &mut decls);

    for expected in [
        "flex-direction:row;",
        "justify-content:space-between;",
        "align-items:center;",
        "flex-grow:0;",
        "flex-shrink:0;",
        "gap:0;",
        "width:auto;",
        "height:100%;",
        "min-width:240px;",
        "max-width:960px;",
        "min-height:48px;",
        "max-height:640px;",
        "padding:0;",
        "margin:8px 12px;",
        "background-color:#0f172a;",
        "color:#ffffff;",
        "border:2px solid #6366f1;",
        "border-radius:12px;",
        "opacity:0.75;",
        "font-size:18px;",
        "z-index:10;",
        "flex-wrap:wrap;",
        "overflow:hidden;",
        "text-align:center;",
        "font-weight:700;",
    ] {
        assert!(decls.contains(expected), "missing `{expected}` in `{decls}`");
    }

    // Separate border_width-only and border_color-only patches.
    let mut bw_only = String::new();
    write_responsive_patch_decls(&ResponsiveStylePatch::new().border_width(3.0), &mut bw_only);
    assert_eq!(bw_only, "border-width:3px;");

    let mut bc_only = String::new();
    write_responsive_patch_decls(
        &ResponsiveStylePatch::new().border_color(ColorRgba::rgba(255, 0, 0, 255)),
        &mut bc_only,
    );
    assert_eq!(bc_only, "border-color:#ff0000;");

    // Direct struct literal with non-finite floats must never inject invalid CSS tokens.
    let nasty = ResponsiveStylePatch {
        gap: Some(f32::NAN),
        opacity: Some(f32::INFINITY),
        font_size: Some(f32::NEG_INFINITY),
        border_radius: Some(-10.0),
        padding: Some(EdgeInsets::all(f32::NAN)),
        font_weight: Some(9999),
        ..ResponsiveStylePatch::new()
    };
    let mut safe = String::new();
    write_responsive_patch_decls(&nasty, &mut safe);
    assert!(!safe.contains("NaN") && !safe.contains("inf"));
    assert!(!safe.contains('{') && !safe.contains('}') && !safe.contains('<'));
    assert!(safe.contains("font-weight:1000;"));
    assert!(safe.contains("padding:0;"));
}

#[test]
fn media_show_on_and_responsive_rules_compile_deterministically_with_de_morgan_negation() {
    // 1. Empty show_on emits no rules; invalid range emits `@media all`.
    let mut empty_out = Vec::new();
    compile_node_media_rules(Some(MediaQuery::EMPTY), &[], &mut empty_out);
    assert!(empty_out.is_empty());

    let mut invalid_out = Vec::new();
    compile_node_media_rules(Some(MediaQuery::width_range(1000, 500)), &[], &mut invalid_out);
    assert_eq!(invalid_out.len(), 1);
    assert_eq!(invalid_out[0].condition, "all");
    assert_eq!(invalid_out[0].decls, "display:none!important;");
    assert!(invalid_out[0].class_name.starts_with("nve-m-"));
    assert_eq!(invalid_out[0].class_name.len(), 14);

    // 2. Compound query produces De Morgan negated rules for every predicate.
    let compound = MediaQuery::width_range(600, 1023)
        .and_color_scheme(MediaColorScheme::Dark)
        .and_pointer(MediaPointer::Fine);
    let mut rules = Vec::new();
    compile_node_media_rules(Some(compound), &[], &mut rules);
    let conds: Vec<&str> = rules.iter().map(|r| r.condition.as_str()).collect();
    assert_eq!(
        conds,
        vec![
            "(max-width:599.98px)",
            "(min-width:1024px)",
            "(prefers-color-scheme:light)",
            "(pointer:coarse)",
        ]
    );
    for r in &rules {
        assert_eq!(r.class_name, format_media_class(&r.condition, &r.decls));
        assert!(r.to_css_rule().starts_with("@media "));
        assert!(r.to_css_rule().ends_with("{display:none!important;}}"));
    }
}

#[cfg(feature = "ssr")]
#[test]
fn ssr_emits_media_rules_after_base_structural_rules_and_deduplicates_on_hydration() {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            flex_direction: FlexDirection::Column,
            gap: 8.0,
            padding: EdgeInsets::all(12.0),
            ..LayoutProps::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgba(240, 240, 240, 255),
            ..StyleProps::default()
        },
    );
    b.create_node(3, ComponentKind::View, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(280.0),
            ..LayoutProps::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);
    let frame = b.finish().to_vec();

    let mut backend = WebBackend::new();
    backend.submit_frame(&frame).unwrap();

    let responsive = [
        ResponsiveRule::new(
            MediaQuery::not_compact(),
            ResponsiveStylePatch::new().flex_row().gap(16.0).padding(24.0),
        ),
        ResponsiveRule::new(
            MediaQuery::dark(),
            ResponsiveStylePatch::new().bg(ColorRgba::rgba(15, 23, 42, 255)),
        ),
    ];
    backend.set_node_responsive_rules(2, responsive.to_vec());
    backend.set_node_show_on(3, Some(MediaQuery::expanded()));

    let ssr = backend.render_ssr();

    // All base structural `.s...{...}` rules must precede ALL `@media` rules in `style_css`.
    let last_base_pos = ssr
        .style_css
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with('.'))
        .map(|(idx, _)| idx)
        .max()
        .expect("expected base structural CSS rules");
    let first_media_pos = ssr
        .style_css
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with("@media "))
        .map(|(idx, _)| idx)
        .min()
        .expect("expected @media CSS rules");
    assert!(
        last_base_pos < first_media_pos,
        "base structural rules (line {last_base_pos}) must precede @media rules (line {first_media_pos}):\n{}",
        ssr.style_css
    );

    // Verify expected `@media` rules and `nve-m-` classes appear in both CSS and HTML.
    assert!(ssr.style_css.contains("@media (min-width:600px){.nve-m-"));
    assert!(ssr.style_css.contains("flex-direction:row;gap:16px;padding:24px;"));
    assert!(ssr.style_css.contains("@media (prefers-color-scheme:dark){.nve-m-"));
    assert!(ssr.style_css.contains("background-color:#0f172a;"));
    assert!(ssr.style_css.contains("@media (max-width:1023.98px){.nve-m-"));
    assert!(ssr.style_css.contains("display:none!important;"));

    let media_hashes: Vec<&String> = ssr
        .rule_hashes
        .iter()
        .filter(|h| h.starts_with("nve-m-"))
        .collect();
    assert_eq!(media_hashes.len(), 3);
    for mh in &media_hashes {
        assert!(ssr.html.contains(mh.as_str()), "missing class {mh} in HTML");
    }

    // Document embeds all `nve-m-*` keys in `data-nve-hashes`.
    let doc = backend.render_ssr_document(&SsrDocumentOptions::default());
    for mh in &media_hashes {
        assert!(doc.contains(mh.as_str()));
    }

    // Hydration seeds `nve-m-*` hashes so syncing the same responsive rules emits 0 duplicates.
    #[cfg(feature = "hydrate")]
    {
        let mut client = WebBackend::hydrate_from_ssr(&ssr);
        assert_eq!(client.hydration_report().seeded_rule_count, ssr.rule_hashes.len());
        client.submit_frame(&frame).unwrap();
        client.sync_responsive_from(
            [(3, MediaQuery::expanded())],
            [(2, responsive.as_slice())],
        );
        assert_eq!(client.stylesheet().pending_count(), 0);
        assert!(
            client.stylesheet().flushed_css_rules().is_empty(),
            "hydrated client must not re-emit already-seeded SSR structural or @media rules: {:?}",
            client.stylesheet().flushed_css_rules()
        );
        // Live DOM node class_name still receives the `nve-m-*` classes.
        let n2_cls = &client.node(2).unwrap().class_name;
        let n3_cls = &client.node(3).unwrap().class_name;
        assert!(n2_cls.contains("nve-m-"));
        assert!(n3_cls.contains("nve-m-"));
    }
}

#[test]
fn stylesheet_cache_flushes_structural_before_media_and_live_sync_updates_and_clears() {
    // 1. Direct StyleSheetCache cascade ordering even if media rule is queued before structural.
    let mut cache = StyleSheetCache::new();
    let mut compiled = Vec::new();
    compile_node_media_rules(Some(MediaQuery::expanded()), &[], &mut compiled);
    assert!(cache.ensure_media_rule(compiled[0].clone()));
    assert!(cache.ensure_rule("sabc123".into(), "gap:8px;".into()));
    cache.flush_to_dom();
    let flushed = cache.flushed_css_rules();
    assert_eq!(flushed.len(), 2);
    assert!(flushed[0].starts_with(".sabc123{"));
    assert!(flushed[1].starts_with("@media (max-width:1023.98px){.nve-m-"));

    // 2. Live WebBackend `sync_responsive_from` attaches, updates, and clears metadata.
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.create_node(3, ComponentKind::View, None);
    b.set_children(ROOT_NODE_ID, &[2, 3]);
    b.commit(true);

    let mut web = WebBackend::new();
    web.submit_frame(b.finish()).unwrap();

    let r1 = [ResponsiveRule::new(
        MediaQuery::compact(),
        ResponsiveStylePatch::new().gap(4.0),
    )];
    web.sync_responsive_from([(2, MediaQuery::expanded())], [(3, r1.as_slice())]);
    assert!(web.node(2).unwrap().class_name.contains("nve-m-"));
    assert!(web.node(3).unwrap().class_name.contains("nve-m-"));

    // Clearing via empty sync removes responsive classes from both nodes.
    web.sync_responsive_from(std::iter::empty(), std::iter::empty::<(u32, &[ResponsiveRule])>());
    assert!(!web.node(2).unwrap().class_name.contains("nve-m-"));
    assert!(!web.node(3).unwrap().class_name.contains("nve-m-"));

    // Consecutive-frame volatile node still retains its `nve-m-*` classes.
    web.set_node_show_on(2, Some(MediaQuery::not_compact()));
    for frame_id in 2..=4 {
        let mut bf = CommandBufferBuilder::new(1, frame_id);
        bf.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(100.0 + frame_id as f32),
                ..LayoutProps::default()
            },
        );
        bf.commit(false);
        web.submit_frame(bf.finish()).unwrap();
    }
    assert!(web.node(2).unwrap().is_consecutive_frame_volatile());
    assert!(web.node(2).unwrap().class_name.contains("nve-m-"));
}

#[cfg(feature = "ssr")]
#[test]
fn ssr_document_emits_compact_event_replay_script_only_when_opted_in() {
    assert!(
        PRE_HYDRATION_REPLAY_SCRIPT_TAG.len() < 1024,
        "replay script must stay < 1 KB, got {} bytes",
        PRE_HYDRATION_REPLAY_SCRIPT_TAG.len()
    );
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.starts_with("<script data-nve-replay>"));
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.ends_with("</script>"));
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.contains("a[href]"));
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.contains("__nveReplayDrain"));
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.contains("EventListener"));
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.contains("T('remove')"));
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.contains("q.length<64"));

    let web = WebBackend::new();

    // Default (`enable_event_replay: false`) omits the replay script.
    let doc_default = web.render_ssr_document(&SsrDocumentOptions {
        title: "Static".into(),
        body_scripts: Some("<script type=\"module\" src=\"/app.js\"></script>".into()),
        ..Default::default()
    });
    assert!(!doc_default.contains("data-nve-replay"));

    // Opt-in (`enable_event_replay: true`) places `<script data-nve-replay>` after `#app` and
    // strictly before `body_scripts`.
    let doc_replay = web.render_ssr_document(&SsrDocumentOptions {
        title: "Interactive".into(),
        body_scripts: Some("<script type=\"module\" src=\"/app.js\"></script>".into()),
        enable_event_replay: true,
        ..Default::default()
    });
    let app_end = doc_replay.find("</div>\n").expect("missing #app end");
    let replay_pos = doc_replay
        .find("<script data-nve-replay>")
        .expect("missing replay script");
    let app_script_pos = doc_replay
        .find("<script type=\"module\" src=\"/app.js\">")
        .expect("missing body_scripts");
    assert!(app_end < replay_pos && replay_pos < app_script_pos);
}

#[test]
fn pre_hydration_replay_filter_drops_stale_and_unmounted_preserving_order() {
    let input = vec![
        CapturedPreHydrationEvent::Press {
            node_id: 2,
            x: 10.0,
            y: 20.0,
            age_ms: 500,
        },
        CapturedPreHydrationEvent::Press {
            node_id: 2,
            x: 11.0,
            y: 21.0,
            age_ms: MAX_REPLAY_AGE_MS + 1, // stale -> drop
        },
        CapturedPreHydrationEvent::KeyDown {
            node_id: ROOT_NODE_ID,
            key: "k".into(),
            meta: true,
            ctrl: false,
            shift: false,
            alt: false,
            age_ms: MAX_REPLAY_AGE_MS, // exact boundary -> keep
        },
        CapturedPreHydrationEvent::Submit {
            node_id: 99, // absent from tree -> drop
            value: "ghost".into(),
            age_ms: 100,
        },
        CapturedPreHydrationEvent::Submit {
            node_id: 3,
            value: "real".into(),
            age_ms: 0,
        },
    ];

    let valid = drain_valid_pre_hydration_events(input, |id| matches!(id, 1..=3));
    assert_eq!(valid.len(), 3);
    assert!(matches!(valid[0], CapturedPreHydrationEvent::Press { node_id: 2, .. }));
    assert!(matches!(valid[1], CapturedPreHydrationEvent::KeyDown { node_id: 1, .. }));
    assert!(matches!(valid[2], CapturedPreHydrationEvent::Submit { node_id: 3, .. }));
}

#[test]
fn should_intercept_anchor_click_matrix_covers_plain_modifier_buttons_target_and_download() {
    // Plain left-click on internal link -> intercept for SPA navigation.
    assert!(should_intercept_anchor_click(0, false, false, false, false, None, false));
    assert!(should_intercept_anchor_click(0, false, false, false, false, Some(""), false));
    assert!(should_intercept_anchor_click(0, false, false, false, false, Some("_self"), false));
    assert!(should_intercept_anchor_click(0, false, false, false, false, Some(" _SELF "), false));

    // Modifier keys -> native browser handling.
    assert!(!should_intercept_anchor_click(0, true, false, false, false, None, false));
    assert!(!should_intercept_anchor_click(0, false, true, false, false, None, false));
    assert!(!should_intercept_anchor_click(0, false, false, true, false, None, false));
    assert!(!should_intercept_anchor_click(0, false, false, false, true, None, false));

    // Non-primary mouse buttons (middle/right click) -> native browser handling.
    assert!(!should_intercept_anchor_click(1, false, false, false, false, None, false));
    assert!(!should_intercept_anchor_click(2, false, false, false, false, None, false));

    // External / new-tab targets -> native browser handling.
    assert!(!should_intercept_anchor_click(0, false, false, false, false, Some("_blank"), false));
    assert!(!should_intercept_anchor_click(0, false, false, false, false, Some("_top"), false));
    assert!(!should_intercept_anchor_click(0, false, false, false, false, Some("docs"), false));

    // Download attribute -> native browser handling.
    assert!(!should_intercept_anchor_click(0, false, false, false, false, None, true));
}

#[cfg(all(feature = "ssr", feature = "hydrate"))]
#[test]
fn hydration_three_way_reconciler_preserves_dirty_controls_and_replays_events_in_order() {
    // Build SSR tree with 5 inputs (clean, incoming-only, local-dirty, conflict, converged + escaped)
    // plus an interactive button with a child text label and a disabled button.
    let build_frame = |incoming_vals: [&str; 6]| -> Vec<u8> {
        let mut b = CommandBufferBuilder::new(1, 1);
        for (idx, val) in incoming_vals.iter().enumerate() {
            let nid = (idx as u32) + 2;
            b.create_node(nid, ComponentKind::TextInput, None);
            b.set_raw_prop(nid, "value", &RawPropValue::String((*val).into()));
            b.set_event_mask(nid, EventMask::TEXT_CHANGE);
        }
        // Node 8: Pressable button wrapping child Text node 9.
        b.create_node(8, ComponentKind::Pressable, None);
        b.set_event_mask(8, EventMask::PRESS);
        b.create_node(9, ComponentKind::Text, None);
        b.set_text(9, "Search");
        b.set_children(8, &[9]);

        // Node 10: Disabled Pressable button.
        b.create_node(10, ComponentKind::Pressable, None);
        b.set_event_mask(10, EventMask::PRESS);
        b.set_raw_prop(10, "disabled", &RawPropValue::Bool(true));

        b.set_children(ROOT_NODE_ID, &[2, 3, 4, 5, 6, 7, 8, 10]);
        b.commit(true);
        b.finish().to_vec()
    };

    let ssr_frame = build_frame([
        "clean",          // node 2: Clean
        "ssr-v1",         // node 3: IncomingOnly (Frame 0 updates to "hydrated-v2")
        "",               // node 4: LocalOnlyDirty (user types "rust query")
        "base-draft",     // node 5: Conflict (user types "user-wins", Frame 0 has "server-v2")
        "start",          // node 6: Converged (both local & incoming become "same")
        "a & \"b\" <c>",  // node 7: HTML-escaped Clean baseline
    ]);

    let mut server = WebBackend::new();
    server.submit_frame(&ssr_frame).unwrap();
    let ssr = server.render_ssr();

    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    let callback_hits = Rc::new(Cell::new(0usize));
    let cb_hits = Rc::clone(&callback_hits);
    client.set_render_callback(move || cb_hits.set(cb_hits.get() + 1));

    // Simulate pre-hydration user typing on nodes 4, 5, and 6.
    client.set_hydrated_dom_control_value(4, "rust query");
    client.set_hydrated_dom_control_value(5, "user-wins");
    client.set_hydrated_dom_control_value(6, "same");

    // Simulate pre-hydration clicks & keyboard shortcuts.
    client.queue_pre_hydration_events(vec![
        // Clicking child Text node 9 should bubble up to Pressable node 8.
        CapturedPreHydrationEvent::Press {
            node_id: 9,
            x: 42.0,
            y: 18.0,
            age_ms: 120,
        },
        // Clicking disabled node 10 must be dropped.
        CapturedPreHydrationEvent::Press {
            node_id: 10,
            x: 1.0,
            y: 1.0,
            age_ms: 100,
        },
        // Stale event (> 10s) must be dropped.
        CapturedPreHydrationEvent::Submit {
            node_id: 4,
            value: "old".into(),
            age_ms: MAX_REPLAY_AGE_MS + 50,
        },
        // Valid Cmd+K shortcut on ROOT_NODE_ID.
        CapturedPreHydrationEvent::KeyDown {
            node_id: ROOT_NODE_ID,
            key: "k".into(),
            meta: true,
            ctrl: false,
            shift: false,
            alt: false,
            age_ms: 80,
        },
        // Valid Submit on node 4.
        CapturedPreHydrationEvent::Submit {
            node_id: 4,
            value: "rust query".into(),
            age_ms: 50,
        },
    ]);

    let client_frame0 = build_frame([
        "clean",          // node 2: Clean
        "hydrated-v2",    // node 3: IncomingOnly -> adopt
        "",               // node 4: LocalOnlyDirty -> preserve "rust query"
        "server-v2",      // node 5: Conflict -> PreferLocal preserves "user-wins"
        "same",           // node 6: Converged -> no upstream emit
        "a & \"b\" <c>",  // node 7: HTML-escaped Clean -> no false dirty
    ]);

    client.submit_frame(&client_frame0).unwrap();

    let report = client.hydration_report();
    assert!(report.mismatches.is_empty());
    assert_eq!(report.reconciled_dirty_controls, 2, "only nodes 4 & 5 are dirty");
    assert_eq!(report.replayed_events, 3, "Press(8), KeyDown(1), Submit(4)");
    assert_eq!(callback_hits.get(), 1, "on_event fires once for the hydration batch");

    // Verify simulated live DOM & mirror values for all 6 controls.
    assert_eq!(client.native_dom_control_value(2), Some("clean"));
    assert_eq!(client.native_dom_control_value(3), Some("hydrated-v2"));
    assert_eq!(client.native_dom_control_value(4), Some("rust query"));
    assert_eq!(client.node(4).unwrap().text.as_deref(), Some("rust query"));
    assert_eq!(
        client.node(4).unwrap().raw_props.get("value"),
        Some(&RawPropValue::String("rust query".into()))
    );
    assert_eq!(client.native_dom_control_value(5), Some("user-wins"));
    assert_eq!(client.node(5).unwrap().text.as_deref(), Some("user-wins"));
    assert_eq!(client.native_dom_control_value(6), Some("same"));
    assert_eq!(client.native_dom_control_value(7), Some("a & \"b\" <c>"));

    // Verify exact event ordering: reconciled dirty TextChanges FIRST, then replayed actions!
    let events = client.drain_events();
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].node_id, 4);
    assert_eq!(events[0].kind, NativeEventKind::TextChange);
    assert_eq!(events[0].payload, EventPayload::Text("rust query".into()));

    assert_eq!(events[1].node_id, 5);
    assert_eq!(events[1].kind, NativeEventKind::TextChange);
    assert_eq!(events[1].payload, EventPayload::Text("user-wins".into()));

    assert_eq!(events[2].node_id, 8);
    assert_eq!(events[2].kind, NativeEventKind::Press);
    assert_eq!(events[2].payload, EventPayload::Point { x: 42.0, y: 18.0 });

    assert_eq!(events[3].node_id, ROOT_NODE_ID);
    assert_eq!(events[3].kind, NativeEventKind::Custom);
    match &events[3].payload {
        EventPayload::Custom { name, data } => {
            assert_eq!(name, "keydown");
            let decoded = nve_abi::events::decode_keydown_bytes(data).expect("valid NK1 payload");
            assert_eq!(decoded.key, "k");
            assert!(decoded.meta && !decoded.ctrl && !decoded.shift && !decoded.alt);
        }
        other => panic!("expected Custom keydown, got {other:?}"),
    }

    assert_eq!(events[4].node_id, 4);
    assert_eq!(events[4].kind, NativeEventKind::Custom);
    assert_eq!(
        events[4].payload,
        EventPayload::Custom {
            name: "submit".into(),
            data: b"rust query".to_vec(),
        }
    );
}
