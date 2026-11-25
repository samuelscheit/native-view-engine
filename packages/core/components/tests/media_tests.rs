//! Deep unit, repr(C) layout, CSS-parity, and randomized property tests for
//! `nve_components::media` (zero serialization).

use std::mem::{align_of, offset_of, size_of};

use nve_abi::{
    ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify, LayoutProps,
    StyleProps,
};
use nve_components::media::{
    FlexWrap, MediaColorScheme, MediaEnv, MediaPointer, MediaQuery, Overflow, ResponsiveRule,
    ResponsiveStylePatch, TextAlign, MEDIA_FLAG_HAS_MAX_WIDTH, MEDIA_FLAG_HAS_MIN_WIDTH,
    MEDIA_FLAG_HAS_REDUCED_MOTION, MEDIA_FLAG_REDUCED_MOTION_VAL,
};

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
fn media_query_repr_c_8_byte_abi_layout_and_raw_validation() {
    assert_eq!(size_of::<MediaQuery>(), 8);
    assert_eq!(align_of::<MediaQuery>(), 2);
    assert_eq!(offset_of!(MediaQuery, min_width_px), 0);
    assert_eq!(offset_of!(MediaQuery, max_width_px), 2);
    assert_eq!(offset_of!(MediaQuery, flags), 4);
    assert_eq!(offset_of!(MediaQuery, color_scheme), 5);
    assert_eq!(offset_of!(MediaQuery, pointer), 6);
    assert_eq!(offset_of!(MediaQuery, _pad), 7);

    let q = MediaQuery::width_range(600, 1023)
        .and_color_scheme(MediaColorScheme::Dark)
        .and_reduced_motion(true)
        .and_pointer(MediaPointer::Coarse);
    assert_eq!(q.min_width_opt(), Some(600));
    assert_eq!(q.max_width_opt(), Some(1023));
    assert_eq!(q.color_scheme_opt(), Some(MediaColorScheme::Dark));
    assert_eq!(q.reduced_motion_opt(), Some(true));
    assert_eq!(q.pointer_opt(), Some(MediaPointer::Coarse));
    assert_eq!(
        q.flags,
        MEDIA_FLAG_HAS_MIN_WIDTH
            | MEDIA_FLAG_HAS_MAX_WIDTH
            | MEDIA_FLAG_HAS_REDUCED_MOTION
            | MEDIA_FLAG_REDUCED_MOTION_VAL
    );

    // Invalid raw FFI discriminants are rejected cleanly without panicking.
    let bad_scheme = MediaQuery { color_scheme: 9, ..MediaQuery::EMPTY };
    assert!(!bad_scheme.is_valid());
    assert!(!bad_scheme.matches(&MediaEnv::default()));
    let mut s = String::new();
    assert!(!bad_scheme.write_css_condition(&mut s));

    let bad_ptr = MediaQuery { pointer: 3, ..MediaQuery::EMPTY };
    assert!(!bad_ptr.is_valid());
    assert!(!bad_ptr.matches(&MediaEnv::default()));
}

#[test]
fn css_conditions_and_complementary_negations() {
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
                .and_color_scheme(MediaColorScheme::Dark)
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
        assert!(q.write_css_condition(&mut css), "expected css for {q:?}");
        assert_eq!(css, *expected_css);

        let mut neg = Vec::new();
        q.write_negated_css_conditions(&mut neg);
        let neg_refs: Vec<&str> = neg.iter().map(String::as_str).collect();
        assert_eq!(neg_refs, *expected_neg);
    }

    // Edge cases: min_width(0), max_width(u16::MAX), empty, impossible range.
    let mut neg_zero = Vec::new();
    MediaQuery::min_width(0).write_negated_css_conditions(&mut neg_zero);
    assert!(neg_zero.is_empty());

    let mut neg_max = Vec::new();
    MediaQuery::max_width(u16::MAX).write_negated_css_conditions(&mut neg_max);
    assert_eq!(neg_max, vec!["(min-width:65536px)"]);

    let mut empty_css = String::new();
    assert!(!MediaQuery::EMPTY.write_css_condition(&mut empty_css));
    assert!(empty_css.is_empty());
    let mut empty_neg = Vec::new();
    MediaQuery::EMPTY.write_negated_css_conditions(&mut empty_neg);
    assert!(empty_neg.is_empty());

    let impossible = MediaQuery::width_range(1000, 500);
    assert!(!impossible.is_valid());
    let mut imp_css = String::new();
    assert!(!impossible.write_css_condition(&mut imp_css));
    assert!(imp_css.is_empty());
    let mut imp_neg = Vec::new();
    impossible.write_negated_css_conditions(&mut imp_neg);
    assert_eq!(imp_neg, vec!["all"]);
    assert!(!impossible.matches(&MediaEnv::default()));
}

#[test]
fn size_class_and_custom_breakpoints_partition_widths_with_zero_gap_or_overlap() {
    for quarter in 0..=6400 {
        let w = quarter as f32 * 0.25;
        let env = MediaEnv { width_pt: w, ..Default::default() };
        let hits = [
            MediaQuery::compact().matches(&env),
            MediaQuery::medium().matches(&env),
            MediaQuery::expanded().matches(&env),
        ]
        .into_iter()
        .filter(|&b| b)
        .count();
        assert_eq!(hits, 1, "expected exact partition at width={w}");

        for bp in [480u16, 768, 960, 1200] {
            let below = MediaQuery::max_width(bp - 1).matches(&env);
            let at_or_above = MediaQuery::min_width(bp).matches(&env);
            assert_ne!(below, at_or_above, "gap or overlap at bp={bp}, w={w}");
        }
    }

    let env_959_98 = MediaEnv { width_pt: 959.98, ..Default::default() };
    let env_960_0 = MediaEnv { width_pt: 960.0, ..Default::default() };
    assert!(MediaQuery::max_width(959).matches(&env_959_98));
    assert!(!MediaQuery::min_width(960).matches(&env_959_98));
    assert!(!MediaQuery::max_width(959).matches(&env_960_0));
    assert!(MediaQuery::min_width(960).matches(&env_960_0));

    for bad_w in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let env = MediaEnv { width_pt: bad_w, ..Default::default() };
        assert!(!MediaQuery::min_width(0).matches(&env));
        assert!(!MediaQuery::max_width(1000).matches(&env));
        assert!(MediaQuery::light().matches(&env));
    }
}

#[test]
fn randomized_negated_css_conditions_match_exact_de_morgan_complement() {
    let mut rng = XorShift64(0x517C_C1B7_2722_0A95);

    fn eval_single_css_clause(clause: &str, env: &MediaEnv) -> bool {
        if clause == "all" {
            return true;
        }
        if let Some(rest) = clause.strip_prefix("(min-width:").and_then(|s| s.strip_suffix("px)")) {
            let min: f32 = rest.parse().unwrap();
            return env.width_pt >= min;
        }
        if let Some(rest) = clause.strip_prefix("(max-width:").and_then(|s| s.strip_suffix(".98px)")) {
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
            1 => q = q.and_color_scheme(MediaColorScheme::Dark),
            2 => q = q.and_color_scheme(MediaColorScheme::Light),
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
        q.write_negated_css_conditions(&mut neg);
        let hidden_by_css = neg.iter().any(|c| eval_single_css_clause(c, &env));
        assert_eq!(
            !hidden_by_css,
            q.matches(&env),
            "De Morgan mismatch for q={q:?}, env={env:?}, neg={neg:?}"
        );
    }
}

#[test]
fn responsive_style_patch_applies_all_26_fields() {
    assert!(ResponsiveStylePatch::new().is_empty());

    let full_patch = ResponsiveStylePatch::new()
        .flex_direction(FlexDirection::RowReverse)
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Baseline)
        .flex_grow(2.5)
        .flex_shrink(0.5)
        .gap(14.0)
        .width_pct(75.0)
        .height_px(420.0)
        .min_width_px(320.0)
        .max_width_px(1200.0)
        .min_height(Dimension::Auto)
        .max_height(Dimension::Percent(90.0))
        .padding_insets(EdgeInsets { top: 4.0, right: 8.0, bottom: 12.0, left: 16.0 })
        .margin_xy(20.0, 10.0)
        .bg(ColorRgba::rgba(15, 23, 42, 240))
        .color(ColorRgba::rgb(248, 250, 252))
        .border(2.0, ColorRgba::rgb(59, 130, 246))
        .radius(12.0)
        .opacity(0.92)
        .font_size(18.5)
        .z_index(42)
        .flex_wrap(FlexWrap::Wrap)
        .overflow(Overflow::Auto)
        .text_align(TextAlign::Center)
        .font_weight(700);
    assert!(!full_patch.is_empty());

    let rule = ResponsiveRule::new(MediaQuery::expanded(), full_patch);
    assert_eq!(rule.query, MediaQuery::expanded());

    let mut layout = LayoutProps::default();
    let mut style = StyleProps::default();
    let mut raws = Vec::new();
    rule.patch.apply_to_with_raw(&mut layout, &mut style, |k, v| {
        raws.push((k.to_owned(), v.to_owned()));
    });

    assert_eq!(layout.flex_direction, FlexDirection::RowReverse);
    assert_eq!(layout.justify_content, FlexJustify::SpaceBetween);
    assert_eq!(layout.align_items, FlexAlign::Baseline);
    assert_eq!(layout.flex_grow, 2.5);
    assert_eq!(layout.flex_shrink, 0.5);
    assert_eq!(layout.gap, 14.0);
    assert_eq!(layout.width, Dimension::Percent(75.0));
    assert_eq!(layout.height, Dimension::Points(420.0));
    assert_eq!(layout.min_width, Dimension::Points(320.0));
    assert_eq!(layout.max_width, Dimension::Points(1200.0));
    assert_eq!(layout.min_height, Dimension::Auto);
    assert_eq!(layout.max_height, Dimension::Percent(90.0));
    assert_eq!(layout.padding, EdgeInsets { top: 4.0, right: 8.0, bottom: 12.0, left: 16.0 });
    assert_eq!(layout.margin, EdgeInsets::symmetric(10.0, 20.0));
    assert_eq!(style.background, ColorRgba::rgba(15, 23, 42, 240));
    assert_eq!(style.text_color, ColorRgba::rgb(248, 250, 252));
    assert_eq!(style.border_width, 2.0);
    assert_eq!(style.border_color, ColorRgba::rgb(59, 130, 246));
    assert_eq!(style.border_radius, 12.0);
    assert_eq!(style.opacity, 0.92);
    assert_eq!(style.font_size, 18.5);
    assert_eq!(style.z_index, 42);
    assert_eq!(
        raws,
        vec![
            ("flex-wrap".into(), "wrap".into()),
            ("overflow".into(), "auto".into()),
            ("text-align".into(), "center".into()),
            ("font-weight".into(), "700".into()),
        ]
    );
}
