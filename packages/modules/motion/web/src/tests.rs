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

fn linear_stops(css: &str) -> Vec<f32> {
    let body = css.strip_prefix("linear(").and_then(|s| s.strip_suffix(')')).expect("linear()");
    body.split(',').map(|v| v.parse().expect("number")).collect()
}

/// What a browser paints for CSS `linear(stops...)` (evenly spaced stops) at progress `t`.
fn css_linear_at(stops: &[f32], t: f32) -> f32 {
    let segments = (stops.len() - 1) as f32;
    let x = t * segments;
    let i = (x.floor() as usize).min(stops.len() - 2);
    stops[i] + (stops[i + 1] - stops[i]) * (x - i as f32)
}

#[test]
fn spring_linear_easing_tracks_the_native_curve_across_damping_regimes() {
    for (name, spec, max_err) in [
        ("bouncy", SpringSpec::bouncy(), 0.02),
        ("snappy", SpringSpec::snappy(), 0.02),
        ("critical", SpringSpec::from_duration_bounce(0.4, 0.0), 0.02),
        ("overdamped", SpringSpec::from_duration_bounce(0.4, -0.6), 0.02),
        ("wobbly", SpringSpec::new(900.0, 6.0, 1.0), 0.04),
    ] {
        let curve = EasingCurve::Spring(spec);
        let stops = linear_stops(&easing(curve));
        assert!((25..=129).contains(&stops.len()), "{name}: {} stops", stops.len());
        assert_eq!((stops[0], *stops.last().expect("stop")), (0.0, 1.0), "{name}");
        let worst = (0..=4000)
            .map(|i| i as f32 / 4000.0)
            .map(|t| (css_linear_at(&stops, t) - curve.sample(t)).abs())
            .fold(0.0, f32::max);
        assert!(worst <= max_err, "{name}: piecewise-linear error {worst} > {max_err}");
    }
    // Oscillating springs get more stops than critically damped ones.
    let bouncy = linear_stops(&easing(EasingCurve::Spring(SpringSpec::bouncy()))).len();
    let critical = linear_stops(&easing(EasingCurve::Spring(SpringSpec::from_duration_bounce(0.4, 0.0)))).len();
    assert!(bouncy > critical, "{bouncy} vs {critical}");
}

#[test]
fn springs_emit_a_cubic_bezier_fallback_declaration_first() {
    let spring = TransitionSpec::spring(TransitionProperty::Opacity, SpringSpec::bouncy());
    let rule = compile_transitions([spring].into_iter()).expect("rule");
    let (legacy, modern) = rule.decls.split_once(";transition:").expect("two declarations");
    assert!(legacy.starts_with("transition:opacity ") && legacy.contains("cubic-bezier(0.34,1.56,0.64,1)"), "{legacy}");
    assert!(modern.contains("linear(0,"), "{modern}");
    let plain = compile_transitions([TransitionSpec::all(100)].into_iter()).expect("rule");
    assert_eq!(plain.decls.matches("transition:").count(), 1, "no fallback without springs");
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
    web.register_table_system(MotionWebSystem::new());
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
