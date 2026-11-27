//! Physics-invariant, convergence, variable-dt, retargeting, reduced-motion, easing, color
//! interpolation, and `MotionClock` unit tests.

use nve_abi::ColorRgba;

use crate::style_patch::VisualSnapshot as StyleProps;

use crate::media::MediaEnv;
use crate::motion::{
    lerp_color_rgba, lerp_style_property, lerp_style_props, resolve_transition_for, EasingCurve,
    MotionClock, MotionCompletion, MotionGroup, MotionKey, MotionPreference, TimedTransition,
    TransitionProperty, TransitionSpec,
};
use crate::spring::{SpringSpec, SpringState};

#[test]
fn spring_regimes_converge_across_variable_dt_without_nan_or_explosion() {
    let specs = [
        ("underdamped", SpringSpec::bouncy()),
        ("critically_damped", SpringSpec::from_duration_bounce(0.35, 0.0)),
        ("overdamped", SpringSpec::from_duration_bounce(0.35, -0.5)),
        ("snappy", SpringSpec::snappy()),
        ("gentle", SpringSpec::gentle()),
        ("stiff", SpringSpec::stiff()),
    ];
    let dts = [0.001_f32, 1.0 / 120.0, 1.0 / 60.0, 0.25];

    for (name, spec) in specs {
        // Verify damping ratio matches regime.
        match name {
            "underdamped" => assert!(spec.damping_ratio() < 1.0),
            "critically_damped" => assert!((spec.damping_ratio() - 1.0).abs() < 1e-4),
            "overdamped" => assert!(spec.damping_ratio() > 1.0),
            _ => {}
        }
        for dt in dts {
            let mut s = SpringState::new(0.0, 100.0);
            let mut steps = 0usize;
            let mut max_abs = 0.0_f32;
            while !s.is_settled(&spec) && steps < 20_000 {
                s.step(&spec, dt, MotionPreference::Full);
                assert!(s.value.is_finite(), "{name} dt={dt} produced NaN/Inf value");
                assert!(s.velocity.is_finite(), "{name} dt={dt} produced NaN/Inf velocity");
                max_abs = max_abs.max(s.value.abs());
                steps += 1;
            }
            assert!(
                s.is_settled(&spec),
                "{name} dt={dt} failed to settle in {steps} steps: {s:?}"
            );
            assert_eq!(s.value, 100.0);
            assert_eq!(s.velocity, 0.0);
            // Even bouncy springs must stay bounded well under 2x target (no numerical explosion).
            assert!(max_abs < 200.0, "{name} dt={dt} exploded to {max_abs}");
        }
    }
}

#[test]
fn closed_form_spring_is_step_size_invariant() {
    // Stepping 10x by 16ms must match 1 single analytic step of 160ms across all three regimes.
    let specs = [
        SpringSpec::new(250.0, 12.0, 1.0), // underdamped
        SpringSpec::new(400.0, 40.0, 1.0), // exact critically damped (zeta = 1.0)
        SpringSpec::new(200.0, 50.0, 1.0), // overdamped
    ];
    for spec in specs {
        let mut fine = SpringState::with_velocity(10.0, 45.0, 100.0);
        for _ in 0..10 {
            fine.step(&spec, 0.016, MotionPreference::Full);
        }
        let mut coarse = SpringState::with_velocity(10.0, 45.0, 100.0);
        coarse.step(&spec, 0.16, MotionPreference::Full);

        assert!(
            (fine.value - coarse.value).abs() < 1e-3,
            "value drift for zeta={}: fine={} coarse={}",
            spec.damping_ratio(),
            fine.value,
            coarse.value
        );
        assert!(
            (fine.velocity - coarse.velocity).abs() < 1e-2,
            "velocity drift for zeta={}: fine={} coarse={}",
            spec.damping_ratio(),
            fine.velocity,
            coarse.velocity
        );
    }
}

#[test]
fn spring_handles_extreme_hitches_and_invalid_inputs_safely() {
    let spec = SpringSpec::from_duration_bounce(0.3, -0.9);
    let mut s = SpringState::new(0.0, 50.0);
    // Negative and NaN dt are no-ops without corrupting state.
    assert!(!s.step(&spec, -0.1, MotionPreference::Full));
    assert!(!s.step(&spec, f32::NAN, MotionPreference::Full));
    assert_eq!(s.value, 0.0);
    // Large 10s hitch stays finite and monotonically approaches target (for zeta=10, slow mode
    // is ~0.0014 away at 10s), and a 20s hitch settles cleanly to exact target.
    s.step(&spec, 10.0, MotionPreference::Full);
    assert!(s.value.is_finite() && (s.value - 50.0).abs() < 0.01);
    assert!(s.step(&spec, 20.0, MotionPreference::Full));
    assert_eq!(s.value, 50.0);
    assert_eq!(s.velocity, 0.0);

    // High-magnitude coordinate + 1000Hz dt must not stall in an f32 ULP limit cycle.
    let crit = SpringSpec::from_duration_bounce(0.35, 0.0);
    let mut hi = SpringState::new(0.0, 10_000.0);
    for _ in 0..5_000 {
        if hi.step(&crit, 0.001, MotionPreference::Full) {
            break;
        }
    }
    assert!(hi.is_settled(&crit));
    assert_eq!(hi.value, 10_000.0);
    assert_eq!(hi.velocity, 0.0);
}

#[test]
fn mid_flight_retargeting_preserves_exact_live_velocity_without_teleporting() {
    let spec = SpringSpec::snappy();
    let mut s = SpringState::new(0.0, 100.0);
    // Advance 50ms so the spring is moving rapidly toward 100.
    for _ in 0..3 {
        s.step(&spec, 1.0 / 60.0, MotionPreference::Full);
    }
    let pos_before = s.value;
    let vel_before = s.velocity;
    assert!(pos_before > 5.0 && pos_before < 95.0);
    assert!(vel_before > 50.0);

    // Interrupt and retarget back to 0.0!
    s.retarget(0.0);
    assert_eq!(s.value, pos_before, "retarget must never teleport position");
    assert_eq!(s.velocity, vel_before, "retarget must preserve exact live velocity");
    assert_eq!(s.target, 0.0);

    // Over the next tiny 2ms step, positive momentum carries value slightly forward before turning.
    s.step(&spec, 0.002, MotionPreference::Full);
    assert!(
        s.value > pos_before,
        "positive momentum should carry position briefly past {pos_before}, got {}",
        s.value
    );
    assert!(s.velocity < vel_before, "restoring force should decelerate velocity");

    // Eventually converges cleanly to 0.0.
    for _ in 0..500 {
        if s.step(&spec, 1.0 / 60.0, MotionPreference::Full) {
            break;
        }
    }
    assert!(s.is_settled(&spec));
    assert_eq!(s.value, 0.0);
    assert_eq!(s.velocity, 0.0);
}

#[test]
fn reduced_motion_snaps_immediately_and_fires_completion_callbacks() {
    let env = MediaEnv {
        reduced_motion: true,
        ..MediaEnv::default()
    };
    let pref = MotionPreference::from_media_env(&env);
    assert_eq!(pref, MotionPreference::Reduce);

    let mut clock: MotionGroup = MotionClock::with_preference(pref);
    clock.animate_spring("opacity", SpringSpec::bouncy(), 0.0, 1.0);
    clock.animate_transition(
        (42, TransitionProperty::Opacity),
        TransitionSpec::new(TransitionProperty::Opacity, 300),
        0.2,
        0.9,
    );
    assert!(clock.has_active_animations());
    assert_eq!(clock.active_count(), 2);

    let mut fired: Vec<MotionCompletion> = Vec::new();
    clock.tick_with(0.001, |ev| fired.push(ev));

    assert!(!clock.has_active_animations());
    assert_eq!(clock.active_count(), 0);
    assert_eq!(clock.value_named("opacity"), Some(1.0));
    assert_eq!(clock.velocity(&MotionKey::from("opacity")), Some(0.0));
    assert_eq!(clock.value(&MotionKey::from((42, TransitionProperty::Opacity))), Some(0.9));
    assert_eq!(fired.len(), 2);
    assert_eq!(fired[0].key, MotionKey::from("opacity"));
    assert_eq!(fired[0].final_value, 1.0);
    assert_eq!(fired[1].key, MotionKey::from((42, TransitionProperty::Opacity)));
    assert_eq!(fired[1].final_value, 0.9);

    // Subsequent ticks emit zero duplicate completions and keep idle state false.
    assert!(clock.tick(0.016).is_empty());
    assert!(!clock.has_active_animations());
}

#[test]
fn motion_clock_retargets_active_spring_with_velocity_and_drops_to_zero_idle() {
    let mut clock = MotionClock::new();
    clock.animate_spring("x", SpringSpec::snappy(), 0.0, 100.0);
    assert!(clock.has_active_animations());

    let done = clock.tick(0.05);
    assert!(done.is_empty());
    let mid_val = clock.value_named("x").unwrap();
    let mid_vel = clock.velocity(&MotionKey::from("x")).unwrap();
    assert!(mid_val > 0.0 && mid_vel > 0.0);

    // Retarget mid-flight via animate_spring: keeps live value & velocity.
    clock.animate_spring("x", SpringSpec::snappy(), 0.0, 20.0);
    assert_eq!(clock.value_named("x"), Some(mid_val));
    assert_eq!(clock.velocity(&MotionKey::from("x")), Some(mid_vel));

    let mut completions = Vec::new();
    for _ in 0..300 {
        completions.extend(clock.tick(1.0 / 60.0));
        if !clock.has_active_animations() {
            break;
        }
    }
    assert!(!clock.has_active_animations());
    assert_eq!(clock.value_named("x"), Some(20.0));
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].final_value, 20.0);
}

#[test]
fn easing_curves_evaluate_accurately_and_format_canonical_css() {
    assert_eq!(EasingCurve::Linear.to_css(), "linear");
    assert_eq!(EasingCurve::Ease.to_css(), "ease");
    assert_eq!(EasingCurve::EaseIn.to_css(), "ease-in");
    assert_eq!(EasingCurve::EaseOut.to_css(), "ease-out");
    assert_eq!(EasingCurve::EaseInOut.to_css(), "ease-in-out");
    assert_eq!(
        EasingCurve::CubicBezier(0.34, 1.56, 0.64, 1.0).to_css(),
        "cubic-bezier(0.34,1.56,0.64,1)"
    );

    // Exact endpoints and midpoint properties.
    for curve in [
        EasingCurve::Linear,
        EasingCurve::Ease,
        EasingCurve::EaseIn,
        EasingCurve::EaseOut,
        EasingCurve::EaseInOut,
        EasingCurve::CubicBezier(0.0, 0.0, 0.0, 1.0), // vertical slope edge case
        EasingCurve::CubicBezier(1.0, 0.0, 1.0, 1.0), // vertical slope edge case
    ] {
        assert_eq!(curve.sample(0.0), 0.0);
        assert_eq!(curve.sample(1.0), 1.0);
        assert_eq!(curve.sample(-0.5), 0.0);
        assert_eq!(curve.sample(1.5), 1.0);
        let mid = curve.sample(0.5);
        assert!(mid.is_finite());
    }
    // EaseIn lags behind linear at midpoint; EaseOut leads linear; EaseInOut is symmetric.
    assert!(EasingCurve::EaseIn.sample(0.5) < 0.4);
    assert!(EasingCurve::EaseOut.sample(0.5) > 0.6);
    assert!((EasingCurve::EaseInOut.sample(0.5) - 0.5).abs() < 1e-4);
    // Overshoot bezier exceeds 1.0 cleanly.
    assert!(EasingCurve::CubicBezier(0.34, 1.56, 0.64, 1.0).sample(0.7) > 1.0);
}

#[test]
fn transition_spec_css_formatting_and_property_resolution() {
    let specs = [
        TransitionSpec::all(200).easing(EasingCurve::Ease),
        TransitionSpec::new(TransitionProperty::BackgroundColor, 150)
            .easing(EasingCurve::EaseOut)
            .delay_ms(25),
    ];
    let mut css = String::new();
    TransitionSpec::write_css_list_into(&specs, &mut css);
    assert_eq!(
        css,
        "transition:all 200ms ease,background-color 150ms ease-out 25ms;"
    );

    assert_eq!(
        resolve_transition_for(&specs, TransitionProperty::BackgroundColor)
            .unwrap()
            .duration_ms,
        150
    );
    assert_eq!(
        resolve_transition_for(&specs, TransitionProperty::Opacity)
            .unwrap()
            .duration_ms,
        200
    );
}

#[test]
fn premultiplied_color_and_style_props_interpolation() {
    // Opaque red -> transparent stays pure red at 50% alpha (no dark muddy fringe!).
    let red = ColorRgba::rgb(255, 0, 0);
    let mid = lerp_color_rgba(red, ColorRgba::TRANSPARENT, 0.5);
    assert_eq!(mid.r, 255);
    assert_eq!(mid.g, 0);
    assert_eq!(mid.b, 0);
    assert!((i16::from(mid.a) - 128).abs() <= 1);

    // Exact endpoints.
    assert_eq!(lerp_color_rgba(red, ColorRgba::WHITE, 0.0), red);
    assert_eq!(lerp_color_rgba(red, ColorRgba::WHITE, 1.0), ColorRgba::WHITE);

    let from = StyleProps {
        background: ColorRgba::rgb(0, 0, 0),
        text_color: ColorRgba::rgb(20, 40, 60),
        border_color: ColorRgba::rgb(100, 100, 100),
        border_width: 1.0,
        border_radius: 4.0,
        opacity: 0.2,
        font_size: 14.0,
        z_index: 1,
    };
    let to = StyleProps {
        background: ColorRgba::rgb(200, 100, 50),
        text_color: ColorRgba::rgb(220, 240, 255),
        border_color: ColorRgba::rgb(200, 200, 200),
        border_width: 3.0,
        border_radius: 12.0,
        opacity: 1.0,
        font_size: 18.0,
        z_index: 5,
    };
    let half = lerp_style_props(&from, &to, 0.5);
    assert_eq!(half.background, ColorRgba::rgb(100, 50, 25));
    assert!((half.border_radius - 8.0).abs() < 1e-5);
    assert!((half.opacity - 0.6).abs() < 1e-5);
    assert!((half.border_width - 2.0).abs() < 1e-5);
    assert_eq!(half.z_index, 5);

    let mut partial = from;
    lerp_style_property(&from, &to, TransitionProperty::Opacity, 0.5, &mut partial);
    assert!((partial.opacity - 0.6).abs() < 1e-5);
    assert_eq!(partial.background, from.background);
}

#[test]
fn timed_transition_respects_delay_and_non_instant_reduced_motion_flag() {
    let spec = TransitionSpec::new(TransitionProperty::Opacity, 100)
        .delay_ms(50)
        .easing(EasingCurve::Linear)
        .reduced_motion_instant(false);
    let mut tr = TimedTransition::new(0.0, 1.0, &spec);
    // During delay (first 50ms), value stays at 0.0 even under Reduce because opt-out flag is false.
    assert!(!tr.step(0.025, MotionPreference::Reduce));
    assert_eq!(tr.value, 0.0);
    // At 100ms total (50ms into 100ms duration), linear value is 0.5.
    assert!(!tr.step(0.075, MotionPreference::Reduce));
    assert!((tr.value - 0.5).abs() < 1e-4);
    // At 150ms total, settles cleanly to 1.0.
    assert!(tr.step(0.05, MotionPreference::Reduce));
    assert_eq!(tr.value, 1.0);
}
