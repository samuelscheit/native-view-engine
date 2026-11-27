//! CSS formatting, spring `linear()` sampling, reduced-motion overrides, and SSR integration.

use nve_backend_web::WebBackend;
use nve_engine::StoreBackend;
use nve_motion::{EasingCurve, Motion, SpringSpec, TransitionProperty, TransitionSpec, Transitions};
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::{compile_transitions, write_easing_css, MotionWebSystem};

fn easing(curve: EasingCurve) -> String {
    let mut s = String::new();
    write_easing_css(curve, &mut s);
    s
}

#[test]
fn easing_css_is_canonical_and_sanitized() {
    assert_eq!(easing(EasingCurve::Linear), "linear");
    assert_eq!(easing(EasingCurve::Ease), "ease");
    assert_eq!(easing(EasingCurve::EaseIn), "ease-in");
    assert_eq!(easing(EasingCurve::EaseOut), "ease-out");
    assert_eq!(easing(EasingCurve::EaseInOut), "ease-in-out");
    assert_eq!(
        easing(EasingCurve::CubicBezier(0.34, 1.56, 0.64, 1.0)),
        "cubic-bezier(0.34,1.56,0.64,1)"
    );
    assert_eq!(
        easing(EasingCurve::CubicBezier(f32::NAN, f32::INFINITY, 7.0, -2.0)),
        "cubic-bezier(0,0,1,-2)"
    );
}

#[test]
fn spring_easing_emits_linear_stops_matching_native_samples() {
    let spring = EasingCurve::Spring(SpringSpec::bouncy());
    let css = easing(spring);
    let body = css.strip_prefix("linear(").and_then(|s| s.strip_suffix(')')).expect("linear()");
    let stops: Vec<f32> = body.split(',').map(|v| v.parse().expect("number")).collect();
    assert_eq!(stops.len(), 25);
    assert_eq!((stops[0], stops[24]), (0.0, 1.0));
    assert!(stops.iter().any(|&v| v > 1.0), "bouncy spring overshoot survives in CSS");
    for (i, &v) in stops.iter().enumerate() {
        assert!((v - spring.sample(i as f32 / 24.0)).abs() <= 5e-4, "stop {i}");
    }
}

#[test]
fn compiled_rules_cover_all_instant_mixed_and_opt_out_reduced_motion() {
    let specs = [
        TransitionSpec::all(200).easing(EasingCurve::Ease),
        TransitionSpec::new(TransitionProperty::BackgroundColor, 150).delay_ms(25),
    ];
    let rule = compile_transitions(specs.into_iter()).expect("rule");
    assert_eq!(rule.decls, "transition:all 200ms ease,background-color 150ms ease-out 25ms;");
    assert!(rule.class_name.starts_with("nve-t-"));
    let rm = rule.reduced_motion_rule.expect("all instant");
    assert_eq!(rm.condition, "(prefers-reduced-motion:reduce)");
    assert_eq!(rm.decls, "transition-duration:0ms!important;transition-delay:0ms!important;");

    let mixed = [specs[0], specs[1].reduced_motion_instant(false)];
    let rm = compile_transitions(mixed.into_iter()).and_then(|r| r.reduced_motion_rule);
    assert_eq!(
        rm.expect("mixed").decls,
        "transition-duration:0ms,150ms!important;transition-delay:0ms,25ms!important;"
    );

    let opt_out = [specs[0].reduced_motion_instant(false)];
    assert!(compile_transitions(opt_out.into_iter()).expect("rule").reduced_motion_rule.is_none());
    assert!(compile_transitions(std::iter::empty()).is_none());
    // Content addressing: the base class depends only on the `transition` list (reduced-motion
    // flags live in the separate media rule); a different list gets a different class.
    let base = compile_transitions(specs.into_iter()).expect("rule").class_name;
    assert_eq!(base, compile_transitions(mixed.into_iter()).expect("rule").class_name);
    let slower = [specs[0], specs[1].delay_ms(30)];
    assert_ne!(base, compile_transitions(slower.into_iter()).expect("rule").class_name);
}

#[test]
fn registered_system_emits_transition_class_rule_through_ssr() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let n = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Motion>(n).transitions(Some(Transitions::new([TransitionSpec::spring(
            TransitionProperty::Opacity,
            SpringSpec::snappy(),
        )])));
        tx.commit_into(&mut changes);
        n
    };
    let mut web = WebBackend::new();
    web.register_table_system(MotionWebSystem);
    web.commit(&store, &changes).expect("commit");
    let ssr = web.render_ssr(&store);
    let rule = compile_transitions(
        store
            .resource::<Transitions>(store.row::<Motion>(n).transitions())
            .expect("transitions")
            .iter(),
    )
    .expect("rule");
    assert!(ssr.html.contains(&rule.class_name), "class applied: {}", ssr.html);
    assert!(ssr.style_css.contains(&rule.decls), "rule emitted: {}", ssr.style_css);
    assert!(ssr.style_css.contains("linear("), "spring easing emitted as CSS linear()");
    assert!(ssr.style_css.contains("(prefers-reduced-motion:reduce)"));

    // Without the plugin the core backend knows nothing about motion.
    let bare = WebBackend::new().render_ssr(&store);
    assert!(!bare.style_css.contains("transition:"));
}
