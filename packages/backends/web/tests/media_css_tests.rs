//! CSS `@media` condition and De Morgan negation tests for `nve_backend_web::media_css`, and the
//! base reset's native overlay rules.

use nve_abi::ColorScheme;
use nve_backend_web::media_css::{
    color_scheme_as_css, color_scheme_negated_css, write_media_css_condition,
    write_negated_media_css_conditions,
};
use nve_backend_web::NVE_BASE_RESET_CSS;
use nve_components::media::{MediaEnv, MediaPointer, MediaQuery};

#[test]
fn base_reset_css_hides_closed_dialog_and_popover_without_hiding_open_states() {
    assert!(
        NVE_BASE_RESET_CSS
            .contains(".nve:is(dialog:not([open]),[popover]:not(:popover-open)){display:none}"),
        "base reset must keep UA display:none on closed <dialog> and [popover]: {NVE_BASE_RESET_CSS}"
    );
    assert!(NVE_BASE_RESET_CSS.contains("dialog.nve::backdrop{"), "dialog::backdrop styled");
    // Open dialogs and open popovers must never be hidden unconditionally.
    for forbidden in ["dialog[open]{display:none", ":popover-open{display:none", "dialog{display:none"] {
        assert!(!NVE_BASE_RESET_CSS.contains(forbidden), "{forbidden}");
    }
}

struct XorShift64(u64);
impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[test]
fn css_conditions_and_complementary_negations() {
    assert_eq!(color_scheme_as_css(ColorScheme::Light), "light");
    assert_eq!(color_scheme_as_css(ColorScheme::Dark), "dark");
    assert_eq!(color_scheme_negated_css(ColorScheme::Light), "dark");
    assert_eq!(color_scheme_negated_css(ColorScheme::Dark), "light");

    let bad_scheme = MediaQuery {
        color_scheme: 9,
        ..MediaQuery::EMPTY
    };
    let mut s = String::new();
    assert!(!write_media_css_condition(&bad_scheme, &mut s));

    let cases: &[(MediaQuery, &str, &[&str])] = &[
        (
            MediaQuery::min_width(960),
            "(min-width:960px)",
            &["(max-width:959.98px)"],
        ),
        (
            MediaQuery::max_width(959),
            "(max-width:959.98px)",
            &["(min-width:960px)"],
        ),
        (
            MediaQuery::width_range(600, 1023),
            "(min-width:600px) and (max-width:1023.98px)",
            &["(max-width:599.98px)", "(min-width:1024px)"],
        ),
        (
            MediaQuery::compact(),
            "(max-width:599.98px)",
            &["(min-width:600px)"],
        ),
        (
            MediaQuery::medium(),
            "(min-width:600px) and (max-width:1023.98px)",
            &["(max-width:599.98px)", "(min-width:1024px)"],
        ),
        (
            MediaQuery::expanded(),
            "(min-width:1024px)",
            &["(max-width:1023.98px)"],
        ),
        (
            MediaQuery::not_compact(),
            "(min-width:600px)",
            &["(max-width:599.98px)"],
        ),
        (
            MediaQuery::dark(),
            "(prefers-color-scheme:dark)",
            &["(prefers-color-scheme:light)"],
        ),
        (
            MediaQuery::light(),
            "(prefers-color-scheme:light)",
            &["(prefers-color-scheme:dark)"],
        ),
        (
            MediaQuery::reduced_motion(),
            "(prefers-reduced-motion:reduce)",
            &["(prefers-reduced-motion:no-preference)"],
        ),
        (
            MediaQuery::coarse_pointer(),
            "(pointer:coarse)",
            &["(pointer:fine)"],
        ),
        (
            MediaQuery::min_width(1200)
                .and_color_scheme(ColorScheme::Dark)
                .and_reduced_motion(false)
                .and_pointer(MediaPointer::Fine),
            "(min-width:1200px) and (prefers-color-scheme:dark) and (prefers-reduced-motion:no-preference) and (pointer:fine)",
            &[
                "(max-width:1199.98px)",
                "(prefers-color-scheme:light)",
                "(prefers-reduced-motion:reduce)",
                "(pointer:coarse)",
            ],
        ),
    ];

    for (q, expected_css, expected_neg) in cases {
        let mut css = String::new();
        assert!(write_media_css_condition(q, &mut css), "expected css for {q:?}");
        assert_eq!(css, *expected_css);

        let mut neg = Vec::new();
        write_negated_media_css_conditions(q, &mut neg);
        let neg_refs: Vec<&str> = neg.iter().map(String::as_str).collect();
        assert_eq!(neg_refs, *expected_neg);
    }

    let mut neg_zero = Vec::new();
    write_negated_media_css_conditions(&MediaQuery::min_width(0), &mut neg_zero);
    assert!(neg_zero.is_empty());

    let mut neg_max = Vec::new();
    write_negated_media_css_conditions(&MediaQuery::max_width(u16::MAX), &mut neg_max);
    assert_eq!(neg_max, vec!["(min-width:65536px)"]);

    let mut empty_css = String::new();
    assert!(!write_media_css_condition(&MediaQuery::EMPTY, &mut empty_css));
    assert!(empty_css.is_empty());
    let mut empty_neg = Vec::new();
    write_negated_media_css_conditions(&MediaQuery::EMPTY, &mut empty_neg);
    assert!(empty_neg.is_empty());

    let impossible = MediaQuery::width_range(1000, 500);
    assert!(!impossible.is_valid());
    let mut imp_css = String::new();
    assert!(!write_media_css_condition(&impossible, &mut imp_css));
    assert!(imp_css.is_empty());
    let mut imp_neg = Vec::new();
    write_negated_media_css_conditions(&impossible, &mut imp_neg);
    assert_eq!(imp_neg, vec!["all"]);
}

#[test]
fn randomized_negated_css_conditions_match_exact_de_morgan_complement() {
    let mut rng = XorShift64(0x517C_C1B7_2722_0A95);

    fn eval_single_css_clause(clause: &str, env: &MediaEnv) -> bool {
        if clause == "all" {
            return true;
        }
        if let Some(rest) = clause
            .strip_prefix("(min-width:")
            .and_then(|s| s.strip_suffix("px)"))
        {
            let min: f32 = rest.parse().unwrap();
            return env.width_pt >= min;
        }
        if let Some(rest) = clause
            .strip_prefix("(max-width:")
            .and_then(|s| s.strip_suffix(".98px)"))
        {
            let max: f32 = rest.parse().unwrap();
            return env.width_pt < max + 1.0;
        }
        match clause {
            "(prefers-color-scheme:dark)" => env.dark_mode,
            "(prefers-color-scheme:light)" => !env.dark_mode,
            "(prefers-reduced-motion:reduce)" => env.reduced_motion,
            "(prefers-reduced-motion:no-preference)" => !env.reduced_motion,
            "(pointer:coarse)" => env.coarse_pointer,
            "(pointer:fine)" => !env.coarse_pointer,
            other => panic!("unexpected clause: {other}"),
        }
    }

    for _ in 0..1_000 {
        let mut q = MediaQuery::EMPTY;
        if rng.next().is_multiple_of(2) {
            q = q.and_min_width((rng.next() % 1800) as u16);
        }
        if rng.next().is_multiple_of(2) {
            q = q.and_max_width((rng.next() % 1800) as u16);
        }
        match rng.next() % 3 {
            1 => q = q.and_color_scheme(ColorScheme::Light),
            2 => q = q.and_color_scheme(ColorScheme::Dark),
            _ => {}
        }
        match rng.next() % 3 {
            1 => q = q.and_reduced_motion(true),
            2 => q = q.and_reduced_motion(false),
            _ => {}
        }
        match rng.next() % 3 {
            1 => q = q.and_pointer(MediaPointer::Fine),
            2 => q = q.and_pointer(MediaPointer::Coarse),
            _ => {}
        }

        let env = MediaEnv {
            width_pt: (rng.next() % 2000) as f32 + ((rng.next() % 4) as f32) * 0.25,
            height_pt: 900.0,
            dark_mode: rng.next().is_multiple_of(2),
            reduced_motion: rng.next().is_multiple_of(2),
            coarse_pointer: rng.next().is_multiple_of(2),
        };

        let mut neg = Vec::new();
        write_negated_media_css_conditions(&q, &mut neg);
        let hidden_by_css = neg.iter().any(|c| eval_single_css_clause(c, &env));
        assert_eq!(
            !hidden_by_css,
            q.matches(&env),
            "De Morgan mismatch for q={q:?}, env={env:?}, neg={neg:?}"
        );
    }
}
