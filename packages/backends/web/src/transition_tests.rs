//! Tests for zero-JS CSS `transition:` compilation, stylesheet deduplication,
//! `@media (prefers-reduced-motion:reduce)` generation, SSR output, and hydration hash seeding.

use nve_abi::{ColorRgba, CommandBufferBuilder, ComponentKind, StyleProps, ROOT_NODE_ID};
use nve_components::interaction::{InteractionState, StateStyleRule};
use nve_components::motion::{EasingCurve, TransitionProperty, TransitionSpec};
use nve_components::style_patch::StylePatch;
use nve_engine::RenderBackend;

use crate::{compile_node_transitions, WebBackend};

#[test]
fn transition_compilation_emits_compact_css_and_reduced_motion_media() {
    let specs = [
        TransitionSpec::new(TransitionProperty::BackgroundColor, 150)
            .easing(EasingCurve::EaseOut),
        TransitionSpec::new(TransitionProperty::Opacity, 220)
            .easing(EasingCurve::CubicBezier(0.2, 0.8, 0.2, 1.0))
            .delay_ms(30),
    ];
    let compiled = compile_node_transitions(&specs).expect("non-empty transition rule");
    assert!(compiled.class_name.starts_with("nve-t-"));
    assert_eq!(
        compiled.decls,
        "transition:background-color 150ms ease-out,opacity 220ms cubic-bezier(0.2,0.8,0.2,1) 30ms;"
    );
    let rm = compiled
        .reduced_motion_rule
        .expect("reduced motion media rule");
    assert_eq!(rm.condition, "(prefers-reduced-motion:reduce)");
    assert_eq!(
        rm.decls,
        "transition-duration:0ms!important;transition-delay:0ms!important;"
    );

    // Opting out of reduced-motion snapping suppresses the @media rule.
    let no_rm = compile_node_transitions(&[
        TransitionSpec::new(TransitionProperty::Opacity, 180).reduced_motion_instant(false),
    ])
    .unwrap();
    assert!(no_rm.reduced_motion_rule.is_none());

    // Mixed reduced_motion_instant emits per-slot durations/delays.
    let mixed = compile_node_transitions(&[
        TransitionSpec::new(TransitionProperty::BackgroundColor, 150).reduced_motion_instant(true),
        TransitionSpec::new(TransitionProperty::Opacity, 300)
            .delay_ms(40)
            .reduced_motion_instant(false),
    ])
    .unwrap();
    let mixed_rm = mixed.reduced_motion_rule.unwrap();
    assert_eq!(
        mixed_rm.decls,
        "transition-duration:0ms,300ms!important;transition-delay:0ms,40ms!important;"
    );
}

#[test]
fn web_backend_deduplicates_transitions_and_emits_reduced_motion_after_base() {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(10, ComponentKind::Pressable, None);
    b.create_node(11, ComponentKind::Pressable, None);
    b.set_children(ROOT_NODE_ID, &[10, 11]);
    b.set_style(
        10,
        &StyleProps {
            background: ColorRgba::rgb(30, 41, 59),
            ..StyleProps::default()
        },
    );
    b.set_style(
        11,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            ..StyleProps::default()
        },
    );
    let frame = b.finish().to_vec();

    let mut web = WebBackend::new();
    web.submit_frame(&frame).unwrap();

    let tr = [TransitionSpec::new(TransitionProperty::BackgroundColor, 160)
        .easing(EasingCurve::EaseOut)];
    web.set_node_transitions(10, tr.to_vec());
    web.set_node_transitions(11, tr.to_vec());
    web.set_node_state_rules(
        10,
        vec![StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new().bg(ColorRgba::rgb(59, 130, 246)),
        )],
    );

    let n10 = web.node(10).unwrap();
    let n11 = web.node(11).unwrap();
    assert!(n10.class_name.contains("nve-t-"));
    assert!(n11.class_name.contains("nve-t-"));
    assert!(n10.class_name.contains("nve-m-"));
    assert!(n11.class_name.contains("nve-m-"));

    // Both nodes share ONE deduplicated `nve-t-*` rule and ONE `@media (prefers-reduced-motion:reduce)` rule.
    let transition_rules: Vec<&String> = web
        .stylesheet()
        .flushed_css_rules()
        .iter()
        .filter(|r| r.contains("transition:background-color 160ms ease-out;"))
        .collect();
    assert_eq!(transition_rules.len(), 1);

    let rm_rules: Vec<&String> = web
        .stylesheet()
        .flushed_css_rules()
        .iter()
        .filter(|r| {
            r.contains("@media (prefers-reduced-motion:reduce)")
                && r.contains("transition-duration:0ms!important;")
        })
        .collect();
    assert_eq!(rm_rules.len(), 1);

    // Clearing transitions removes the transition class cleanly.
    web.set_node_transitions(10, Vec::new());
    web.set_node_transitions(11, Vec::new());
    assert!(!web.node(10).unwrap().class_name.contains("nve-t-"));
}

#[cfg(all(feature = "ssr", feature = "hydrate"))]
#[test]
fn ssr_and_hydration_emit_and_seed_transition_and_reduced_motion_rules() {
    use crate::SsrDocumentOptions;

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(10, ComponentKind::Pressable, None);
    b.set_children(ROOT_NODE_ID, &[10]);
    b.set_style(
        10,
        &StyleProps {
            background: ColorRgba::rgb(10, 20, 30),
            ..StyleProps::default()
        },
    );
    let frame = b.finish().to_vec();

    let tr = [
        TransitionSpec::new(TransitionProperty::BackgroundColor, 180)
            .easing(EasingCurve::EaseInOut),
        TransitionSpec::new(TransitionProperty::BorderRadius, 120).easing(EasingCurve::EaseOut),
    ];

    let mut server = WebBackend::new();
    server.submit_frame(&frame).unwrap();
    server.set_node_transitions(10, tr.to_vec());

    let ssr = server.render_ssr();
    assert!(ssr
        .style_css
        .contains("transition:background-color 180ms ease-in-out,border-radius 120ms ease-out;"));
    assert!(ssr.style_css.contains(
        "@media (prefers-reduced-motion:reduce){.nve-m-"
    ));
    assert!(ssr
        .style_css
        .contains("transition-duration:0ms!important;transition-delay:0ms!important;"));
    // Base transition rule must appear BEFORE the reduced-motion `@media` rule in SSR stylesheet.
    let tr_pos = ssr
        .style_css
        .find("transition:background-color 180ms ease-in-out")
        .unwrap();
    let rm_pos = ssr
        .style_css
        .find("@media (prefers-reduced-motion:reduce)")
        .unwrap();
    assert!(tr_pos < rm_pos);

    let doc = server.render_ssr_document(&SsrDocumentOptions {
        title: "Motion SSR".to_owned(),
        ..SsrDocumentOptions::default()
    });
    assert!(doc.contains("<style data-nve-ssr data-nve-hashes=\""));
    assert!(doc.contains("nve-t-"));

    // Hydrating from the SSR output seeds both `nve-t-*` and `nve-m-*` hashes so zero duplicate
    // rules are flushed on the client!
    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    client.submit_frame(&frame).unwrap();
    client.set_node_transitions(10, tr.to_vec());
    assert!(
        client.stylesheet().flushed_css_rules().is_empty(),
        "hydrated client should not re-emit already SSR-seeded transition/reduced-motion rules: {:?}",
        client.stylesheet().flushed_css_rules()
    );
    assert_eq!(client.hydration_report().mismatches.len(), 0);
}
