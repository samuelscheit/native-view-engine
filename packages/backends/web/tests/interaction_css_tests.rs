//! Deep integration tests for zero-JS native CSS pseudo-class compilation (`:hover`, `:focus`,
//! `:focus-visible`, `:active`, `:disabled` / `[aria-disabled="true"]`, compound selectors,
//! `@media (hover:hover)` sticky-hover guard), base `cursor:<kw>` emission, 3-layer cascade
//! ordering, and SSR + hydration `data-nve-hashes` deduplication.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, LayoutProps, StyleProps,
    ROOT_NODE_ID,
};
use nve_components::interaction::{CursorKind, InteractionState, StateStyleRule};
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::style_patch::{Display, StylePatch};
use nve_engine::RenderBackend;

use nve_backend_web::{
    compile_node_state_rules, compile_node_state_rules_with_hover_guard, format_state_class,
    write_style_patch_decls, StyleSheetCache, WebBackend,
};
#[cfg(feature = "ssr")]
use nve_backend_web::SsrDocumentOptions;

#[test]
fn state_rule_compilation_covers_all_pseudos_compounds_and_hover_media_guard() {
    let rules = [
        // Deliberately reversed order; compiler must sort by cascade priority.
        StateStyleRule::new(
            InteractionState::DISABLED,
            StylePatch::new().opacity(0.45).cursor(CursorKind::NotAllowed),
        ),
        StateStyleRule::new(
            InteractionState::HOVER | InteractionState::DISABLED,
            StylePatch::new().border_color(ColorRgba::rgb(148, 163, 184)),
        ),
        StateStyleRule::new(
            InteractionState::ACTIVE,
            StylePatch::new().bg(ColorRgba::rgb(29, 78, 216)),
        ),
        StateStyleRule::new(
            InteractionState::HOVER | InteractionState::FOCUS_VISIBLE,
            StylePatch::new().border(3.0, ColorRgba::rgb(250, 204, 21)),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS_VISIBLE,
            StylePatch::new().border(2.0, ColorRgba::rgb(56, 189, 248)),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS,
            StylePatch::new().border_color(ColorRgba::rgb(99, 102, 241)),
        ),
        StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new().bg(ColorRgba::rgb(59, 130, 246)),
        ),
        // Empty / NONE rules must be skipped cleanly.
        StateStyleRule::new(InteractionState::NONE, StylePatch::new().opacity(0.5)),
        StateStyleRule::new(InteractionState::HOVER, StylePatch::new()),
    ];

    let mut guarded = Vec::new();
    compile_node_state_rules(&rules, &mut guarded);
    assert_eq!(guarded.len(), 7);

    // Exact deterministic cascade order:
    // Hover(1) < Focus(2) < FocusVisible(4) < Hover|FocusVisible(5) < Active(8) < Disabled(16) < Hover|Disabled(17)
    let states: Vec<InteractionState> = guarded.iter().map(|r| r.state).collect();
    assert_eq!(
        states,
        vec![
            InteractionState::HOVER,
            InteractionState::FOCUS,
            InteractionState::FOCUS_VISIBLE,
            InteractionState::HOVER | InteractionState::FOCUS_VISIBLE,
            InteractionState::ACTIVE,
            InteractionState::DISABLED,
            InteractionState::HOVER | InteractionState::DISABLED,
        ]
    );

    // Verify exact CSS syntax for every rule (guarded hover mode).
    let hover_css = guarded[0].to_css_rule();
    assert!(hover_css.starts_with("@media (hover:hover){.nve-s-"));
    assert!(hover_css.contains(":hover{background-color:#3b82f6;}}"));
    assert_eq!(
        guarded[0].class_name,
        format_state_class(InteractionState::HOVER, "background-color:#3b82f6;")
    );

    assert!(guarded[1].to_css_rule().starts_with(".nve-s-"));
    assert!(guarded[1].to_css_rule().ends_with(":focus{border-color:#6366f1;}"));

    assert!(guarded[2].to_css_rule().ends_with(":focus-visible{border:2px solid #38bdf8;}"));

    let compound_css = guarded[3].to_css_rule();
    assert!(compound_css.starts_with("@media (hover:hover){.nve-s-"));
    assert!(compound_css.ends_with(":hover:focus-visible{border:3px solid #facc15;}}"));

    assert!(guarded[4].to_css_rule().ends_with(":active{background-color:#1d4ed8;}"));

    let disabled_css = guarded[5].to_css_rule();
    let cls5 = &guarded[5].class_name;
    assert_eq!(
        disabled_css,
        format!(".{cls5}:disabled,.{cls5}[aria-disabled=\"true\"]{{opacity:0.45;cursor:not-allowed;}}")
    );

    let hover_disabled_css = guarded[6].to_css_rule();
    let cls6 = &guarded[6].class_name;
    assert_eq!(
        hover_disabled_css,
        format!(
            "@media (hover:hover){{.{cls6}:hover:disabled,.{cls6}:hover[aria-disabled=\"true\"]{{border-color:#94a3b8;}}}}"
        )
    );

    // Unwrapped hover mode omits `@media (hover:hover)` while keeping identical class hashes.
    let mut unwrapped = Vec::new();
    compile_node_state_rules_with_hover_guard(&rules, false, &mut unwrapped);
    assert_eq!(unwrapped[0].class_name, guarded[0].class_name);
    assert_eq!(
        unwrapped[0].to_css_rule(),
        format!(".{}:hover{{background-color:#3b82f6;}}", unwrapped[0].class_name)
    );
    assert_eq!(unwrapped[0].to_css_rule(), guarded[0].to_unwrapped_css_rule());

    // Unified StylePatch emits display + cursor declarations cleanly.
    let mut decls = String::new();
    write_style_patch_decls(
        &StylePatch::new().display(Display::None).cursor(CursorKind::ColResize),
        &mut decls,
    );
    assert_eq!(decls, "display:none;cursor:col-resize;");
}

#[test]
fn base_cursor_emits_for_non_auto_and_omits_auto() {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.create_node(3, ComponentKind::View, None);
    b.create_node(4, ComponentKind::View, None);
    b.set_children(ROOT_NODE_ID, &[2, 3, 4]);
    b.commit(true);

    let mut web = WebBackend::new();
    web.submit_frame(b.finish()).unwrap();

    web.set_node_cursor(2, Some(CursorKind::Auto));
    web.set_node_cursor(3, Some(CursorKind::Grab));
    web.set_node_cursor(4, Some(CursorKind::ColResize));

    // Auto emits no structural cursor rule.
    assert_eq!(web.node(2).unwrap().class_name, "nve");
    // Grab & ColResize emit distinct structural rules with exact CSS cursor keywords.
    let flushed = web.stylesheet().flushed_css_rules();
    assert!(flushed.iter().any(|r| r.contains("cursor:grab;")));
    assert!(flushed.iter().any(|r| r.contains("cursor:col-resize;")));
    assert!(!flushed.iter().any(|r| r.contains("cursor:auto;")));
    assert_ne!(web.node(3).unwrap().class_name, web.node(4).unwrap().class_name);

    // Consecutive-frame volatile node still emits cursor inline and retains `nve-s-*` classes.
    web.set_node_state_rules(
        3,
        vec![StateStyleRule::new(
            InteractionState::ACTIVE,
            StylePatch::new().cursor(CursorKind::Grabbing),
        )],
    );
    for frame_id in 2..=4 {
        let mut bf = CommandBufferBuilder::new(1, frame_id);
        bf.set_layout(
            3,
            &LayoutProps {
                width: Dimension::Points(100.0 + frame_id as f32),
                ..LayoutProps::default()
            },
        );
        bf.commit(false);
        web.submit_frame(bf.finish()).unwrap();
    }
    let n3 = web.node(3).unwrap();
    assert!(n3.is_consecutive_frame_volatile());
    assert!(n3.inline_style.contains("cursor:grab;"));
    assert!(n3.class_name.contains("nve-s-"));
}

#[cfg(feature = "ssr")]
#[test]
fn ssr_and_stylesheet_cache_enforce_three_layer_cascade_and_hydration_dedup() {
    // 1. Direct StyleSheetCache enforces Structural -> Media -> State(Hover < Focus < FocusVisible < Active < Disabled).
    let mut cache = StyleSheetCache::new();
    let mut compiled_states = Vec::new();
    compile_node_state_rules(
        &[
            StateStyleRule::new(InteractionState::DISABLED, StylePatch::new().opacity(0.4)),
            StateStyleRule::new(InteractionState::ACTIVE, StylePatch::new().opacity(0.7)),
            StateStyleRule::new(InteractionState::FOCUS_VISIBLE, StylePatch::new().opacity(0.8)),
            StateStyleRule::new(InteractionState::FOCUS, StylePatch::new().opacity(0.85)),
            StateStyleRule::new(InteractionState::HOVER, StylePatch::new().opacity(0.9)),
        ],
        &mut compiled_states,
    );
    // Insert in scrambled order: Disabled first, then Hover, then Active, then structural, then media.
    assert!(cache.ensure_state_rule(compiled_states[4].clone())); // Disabled
    assert!(cache.ensure_state_rule(compiled_states[0].clone())); // Hover
    assert!(cache.ensure_state_rule(compiled_states[3].clone())); // Active
    assert!(cache.ensure_state_rule(compiled_states[2].clone())); // FocusVisible
    assert!(cache.ensure_state_rule(compiled_states[1].clone())); // Focus
    let mut media = Vec::new();
    nve_backend_web::compile_node_media_rules(Some(MediaQuery::expanded()), &[], &mut media);
    assert!(cache.ensure_media_rule(media[0].clone()));
    assert!(cache.ensure_rule("sbase1".into(), "gap:8px;".into()));
    cache.flush_to_dom();

    let flushed = cache.flushed_css_rules();
    assert_eq!(flushed.len(), 7);
    assert!(flushed[0].starts_with(".sbase1{"));
    assert!(flushed[1].contains(".nve-m-"));
    assert!(flushed[2].contains(":hover{"));
    assert!(flushed[3].contains(":focus{"));
    assert!(flushed[4].contains(":focus-visible{"));
    assert!(flushed[5].contains(":active{"));
    assert!(flushed[6].contains(":disabled,"));

    // 2. Full SSR + hydration deduplication test.
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(37, 99, 235),
            ..StyleProps::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    let frame = b.finish().to_vec();

    let mut server = WebBackend::new();
    server.submit_frame(&frame).unwrap();
    let media_rules = [ResponsiveRule::new(
        MediaQuery::not_compact(),
        StylePatch::new().padding(16.0),
    )];
    let state_rules = [
        StateStyleRule::new(InteractionState::DISABLED, StylePatch::new().opacity(0.4).cursor(CursorKind::NotAllowed)),
        StateStyleRule::new(InteractionState::ACTIVE, StylePatch::new().bg(ColorRgba::rgb(30, 64, 175))),
        StateStyleRule::new(InteractionState::FOCUS_VISIBLE, StylePatch::new().border(2.0, ColorRgba::rgb(56, 189, 248))),
        StateStyleRule::new(InteractionState::FOCUS, StylePatch::new().border_color(ColorRgba::rgb(99, 102, 241))),
        StateStyleRule::new(InteractionState::HOVER, StylePatch::new().bg(ColorRgba::rgb(59, 130, 246))),
    ];
    server.set_node_responsive_rules(2, media_rules.to_vec());
    server.set_node_cursor(2, Some(CursorKind::Grab));
    server.set_node_state_rules(2, state_rules.to_vec());

    let ssr = server.render_ssr();
    let lines: Vec<&str> = ssr.style_css.lines().collect();
    let base_idx = lines.iter().position(|l| l.contains("cursor:grab;")).expect("base rule with cursor");
    let media_idx = lines.iter().position(|l| l.contains(".nve-m-")).expect("@media rule");
    let hover_idx = lines.iter().position(|l| l.contains(":hover{")).expect(":hover rule");
    let focus_idx = lines.iter().position(|l| l.contains(":focus{")).expect(":focus rule");
    let fv_idx = lines.iter().position(|l| l.contains(":focus-visible{")).expect(":focus-visible rule");
    let active_idx = lines.iter().position(|l| l.contains(":active{")).expect(":active rule");
    let disabled_idx = lines.iter().position(|l| l.contains(":disabled,")).expect(":disabled rule");

    assert!(base_idx < media_idx, "base before @media:\n{}", ssr.style_css);
    assert!(media_idx < hover_idx, "@media before :hover:\n{}", ssr.style_css);
    assert!(hover_idx < focus_idx && focus_idx < fv_idx && fv_idx < active_idx && active_idx < disabled_idx);

    let state_hashes: Vec<&String> = ssr
        .rule_hashes
        .iter()
        .filter(|h| h.starts_with("nve-s-"))
        .collect();
    assert_eq!(state_hashes.len(), 5);
    let doc = server.render_ssr_document(&SsrDocumentOptions::default());
    for h in &state_hashes {
        assert!(ssr.html.contains(h.as_str()));
        assert!(doc.contains(h.as_str()));
    }

    #[cfg(feature = "hydrate")]
    {
        let mut client = WebBackend::hydrate_from_ssr(&ssr);
        assert_eq!(client.hydration_report().seeded_rule_count, ssr.rule_hashes.len());
        client.submit_frame(&frame).unwrap();
        client.set_node_responsive_rules(2, media_rules.to_vec());
        client.set_node_cursor(2, Some(CursorKind::Grab));
        client.set_node_state_rules(2, state_rules.to_vec());
        assert_eq!(client.stylesheet().pending_count(), 0);
        assert!(
            client.stylesheet().flushed_css_rules().is_empty(),
            "hydrated client must not duplicate SSR state rules: {:?}",
            client.stylesheet().flushed_css_rules()
        );
        for h in &state_hashes {
            assert!(client.node(2).unwrap().class_name.contains(h.as_str()));
        }

        // Clearing state rules removes nve-s-* classes cleanly.
        client.set_node_cursor(2, None);
        client.set_node_state_rules(2, Vec::new());
        assert!(!client.node(2).unwrap().class_name.contains("nve-s-"));
    }
}
