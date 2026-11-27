//! Physics-invariant, convergence, variable-dt, retargeting, reduced-motion, easing, color
//! interpolation, and `MotionClock` unit tests.

use nve_abi::{ColorRgba, NodeId};
use nve_tables::media::MediaEnv;
use nve_tables::snapshots::VisualSnapshot as StyleProps;
use nve_store::{FlatResource, ValidatedWords};

use crate::{
    lerp_color_rgba, lerp_f32, lerp_style_property, lerp_style_props, resolve_transition_for, EasingCurve,
    MotionClock, MotionCompletion, MotionKey, MotionPreference, SpringSpec, SpringState,
    TimedTransition, TransitionProperty, TransitionSpec, Transitions,
};

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

    let mut clock = MotionClock::with_preference(pref);
    clock.animate_spring("opacity", SpringSpec::bouncy(), 0.0, 1.0);
    clock.animate_transition(
        (NodeId::new(42, 0), TransitionProperty::Opacity),
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
    assert_eq!(clock.value(&MotionKey::named("opacity")), Some(1.0));
    assert_eq!(clock.velocity(&MotionKey::from("opacity")), Some(0.0));
    assert_eq!(
        clock.value(&MotionKey::from((NodeId::new(42, 0), TransitionProperty::Opacity))),
        Some(0.9)
    );
    assert_eq!(fired.len(), 2);
    assert_eq!(fired[0].key, MotionKey::from("opacity"));
    assert_eq!(fired[0].final_value, 1.0);
    assert_eq!(fired[1].key, MotionKey::from((NodeId::new(42, 0), TransitionProperty::Opacity)));
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
    let mid_val = clock.value(&MotionKey::named("x")).unwrap();
    let mid_vel = clock.velocity(&MotionKey::from("x")).unwrap();
    assert!(mid_val > 0.0 && mid_vel > 0.0);

    // Retarget mid-flight via animate_spring: keeps live value & velocity.
    clock.animate_spring("x", SpringSpec::snappy(), 0.0, 20.0);
    assert_eq!(clock.value(&MotionKey::named("x")), Some(mid_val));
    assert_eq!(clock.velocity(&MotionKey::from("x")), Some(mid_vel));

    let mut completions = Vec::new();
    for _ in 0..300 {
        completions.extend(clock.tick(1.0 / 60.0));
        if !clock.has_active_animations() {
            break;
        }
    }
    assert!(!clock.has_active_animations());
    assert_eq!(clock.value(&MotionKey::named("x")), Some(20.0));
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].final_value, 20.0);
}

#[test]
fn easing_curves_evaluate_accurately() {
    // Exact endpoints and midpoint properties.
    for curve in [
        EasingCurve::Linear,
        EasingCurve::Ease,
        EasingCurve::EaseIn,
        EasingCurve::EaseOut,
        EasingCurve::EaseInOut,
        EasingCurve::CubicBezier(0.0, 0.0, 0.0, 1.0), // vertical slope edge case
        EasingCurve::CubicBezier(1.0, 0.0, 1.0, 1.0), // vertical slope edge case
        EasingCurve::CubicBezier(f32::NAN, 2.0, f32::INFINITY, -1.0), // sanitized
        EasingCurve::Spring(SpringSpec::bouncy()),
        EasingCurve::Spring(SpringSpec::stiff()),
        EasingCurve::Spring(SpringSpec::from_duration_bounce(0.4, -0.5)),
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
fn spring_easing_overshoots_and_settles_within_its_duration() {
    let bouncy = EasingCurve::Spring(SpringSpec::bouncy());
    let peak = (1..100).map(|i| bouncy.sample(i as f32 / 100.0)).fold(0.0, f32::max);
    assert!(peak > 1.05, "underdamped spring easing must overshoot (peak {peak})");
    assert!((bouncy.sample(0.999) - 1.0).abs() < 2e-3, "settled at t -> 1");
    let stiff = EasingCurve::Spring(SpringSpec::stiff());
    let mut prev = 0.0;
    for i in 1..100 {
        let v = stiff.sample(i as f32 / 100.0);
        assert!(v + 1e-4 >= prev && v <= 1.0 + 1e-3, "near-critical spring is monotonic");
        prev = v;
    }
    // Settle duration matches the regime and stays finite for undamped springs.
    assert!(SpringSpec::stiff().settle_duration_s() < SpringSpec::bouncy().settle_duration_s());
    assert_eq!(SpringSpec::new(100.0, 0.0, 1.0).settle_duration_s(), 10.0);
    let spec = TransitionSpec::spring(TransitionProperty::Opacity, SpringSpec::gentle());
    assert_eq!(
        spec.duration_ms,
        (SpringSpec::gentle().settle_duration_s() * 1000.0).round() as u32
    );
}

#[test]
fn transitions_resource_roundtrips_and_rejects_hostile_words() {
    let specs = [
        TransitionSpec::all(200).easing(EasingCurve::Ease),
        TransitionSpec::new(TransitionProperty::BackgroundColor, 150)
            .easing(EasingCurve::CubicBezier(0.34, 1.56, 0.64, 1.0))
            .delay_ms(25)
            .reduced_motion_instant(false),
        TransitionSpec::spring(TransitionProperty::Transform, SpringSpec::bouncy()),
    ];
    let mut words = Vec::new();
    assert!(Transitions::new(specs).write_words(&mut words));
    let view = Transitions::view(ValidatedWords::try_new(&words).expect("valid"));
    assert_eq!(view.len(), 3);
    assert_eq!(view.iter().collect::<Vec<_>>(), specs.to_vec());

    let winner = |p| resolve_transition_for(view.iter(), p).map(|s| s.duration_ms);
    assert_eq!(winner(TransitionProperty::BackgroundColor), Some(150));
    assert_eq!(winner(TransitionProperty::Opacity), Some(200));

    // Every truncation, unknown property/easing tag, non-finite or out-of-range parameter fails.
    for cut in 0..words.len() {
        assert!(!Transitions::validate(&words[..cut]), "cut {cut}");
    }
    let hostile: [(usize, u32); 6] = [
        (1, 11),                         // unknown property
        (1, 7 << 8),                     // unknown easing tag
        (1, 2 << 16),                    // reduced-motion flag out of range
        (12, f32::NAN.to_bits()),        // cubic-bezier y1 = NaN
        (11, 2.0f32.to_bits()),          // cubic-bezier x1 > 1
        (18, 0.0f32.to_bits()),          // spring stiffness = 0
    ];
    for (i, w) in hostile {
        let mut bad = words.clone();
        bad[i] = w;
        assert!(!Transitions::validate(&bad), "word {i} = {w:#x} must fail");
    }
    // Producers write the canonical curve: non-finite spring parameters sanitize, custom rest
    // thresholds (a live-state property) drop out, and an undamped spring is rejected.
    let decode = |spec: TransitionSpec| {
        let mut w = Vec::new();
        Transitions::new([spec])
            .write_words(&mut w)
            .then(|| Transitions::view(ValidatedWords::try_new(&w).expect("valid")).get(0))
            .flatten()
    };
    let nan_mass = SpringSpec { mass: f32::NAN, ..SpringSpec::snappy() };
    let nan_spec = TransitionSpec::all(10).easing(EasingCurve::Spring(nan_mass));
    assert_eq!(decode(nan_spec).map(|s| s.easing), Some(EasingCurve::Spring(SpringSpec::snappy())));
    let custom_rest = SpringSpec::gentle().with_rest_thresholds(0.5, 0.5);
    let canonical = TransitionSpec::all(10).easing(EasingCurve::Spring(custom_rest));
    assert_eq!(decode(canonical).map(|s| s.easing), Some(canonical.easing.sanitized()));
    let undamped = TransitionSpec::all(10).easing(EasingCurve::Spring(SpringSpec::new(100.0, 0.0, 1.0)));
    assert_eq!(decode(undamped), None, "an undamped spring never settles");
}

#[test]
fn timed_transition_never_settles_mid_flight_and_noop_settles_immediately() {
    let overshoot = TransitionSpec::all(100).easing(EasingCurve::CubicBezier(0.34, 1.56, 0.64, 1.0));
    let mut tr = TimedTransition::new(0.0, 1.0, &overshoot);
    let mut crossed = false;
    for _ in 0..9 {
        assert!(!tr.step(0.01, MotionPreference::Full), "settled mid-flight at {}", tr.value);
        crossed |= tr.value > 1.0;
    }
    assert!(crossed, "the overshoot curve crosses the target before the end");
    assert!(tr.step(0.02, MotionPreference::Full));

    let noop = TimedTransition::new(0.5, 0.5, &TransitionSpec::all(100).delay_ms(500));
    assert!(noop.is_settled(), "from == to settles immediately even with a delay");
}

#[test]
fn interpolation_extrapolates_anticipation_and_overshoot() {
    assert_eq!(lerp_f32(10.0, 20.0, -0.1), 9.0);
    assert_eq!(lerp_f32(10.0, 20.0, 1.2), 22.0);
    assert_eq!(lerp_f32(10.0, 20.0, f32::NAN), 10.0);
    let back = EasingCurve::CubicBezier(0.36, -0.6, 0.5, 1.0);
    let t = back.sample(0.1);
    assert!(t < 0.0, "anticipation curve dips below 0 ({t})");
    assert!(lerp_f32(0.0, 100.0, t) < 0.0);
}

#[test]
fn motion_clock_slab_keeps_node_runs_contiguous_and_counts_exact() {
    let mut clock = MotionClock::new();
    let (a, b) = (NodeId::new(1, 0), NodeId::new(2, 0));
    for node in [b, a] {
        for prop in [TransitionProperty::Opacity, TransitionProperty::BackgroundColor] {
            clock.animate_transition((node, prop), TransitionSpec::all(100), 0.0, 1.0);
        }
    }
    clock.animate_spring(MotionKey::named("sheet"), SpringSpec::snappy(), 0.0, 1.0);
    assert_eq!((clock.len(), clock.active_count()), (5, 5));
    clock.remove_node(a);
    assert_eq!((clock.len(), clock.active_count()), (3, 3));
    assert!(clock.value(&MotionKey::from((a, TransitionProperty::Opacity))).is_none());
    assert!(clock.value(&MotionKey::from((b, TransitionProperty::Opacity))).is_some());
    clock.set_immediate(MotionKey::named("sheet"), 0.5);
    assert_eq!(clock.active_count(), 2);
    assert!(clock.remove(&MotionKey::from((b, TransitionProperty::Opacity))));
    assert!(!clock.remove(&MotionKey::from((b, TransitionProperty::Opacity))));
    assert_eq!(clock.active_count(), 1);
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
