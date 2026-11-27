//! Deep unit, repr(C) layout, and breakpoint partition tests for
//! `nve_components::media` (zero serialization; CSS `@media` emission tests live in `nve-backend-web`).

use std::mem::{align_of, offset_of, size_of};

use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_components::media::{
    ColorScheme, FlexWrap, MediaEnv, MediaPointer, MediaQuery, Overflow, ResponsiveRule, RowPatch,
    TextAlign, MEDIA_FLAG_HAS_MAX_WIDTH, MEDIA_FLAG_HAS_MIN_WIDTH, MEDIA_FLAG_HAS_REDUCED_MOTION,
    MEDIA_FLAG_REDUCED_MOTION_VAL,
};
use nve_components::snapshots::{LayoutSnapshot, VisualSnapshot};
use nve_components::tables::{Layout, Typography, Visual};
use nve_components::StylePatchExt;

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
        .and_color_scheme(ColorScheme::Dark)
        .and_reduced_motion(true)
        .and_pointer(MediaPointer::Coarse);
    assert_eq!(q.min_width_opt(), Some(600));
    assert_eq!(q.max_width_opt(), Some(1023));
    assert_eq!(q.color_scheme_opt(), Some(ColorScheme::Dark));
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

    let bad_ptr = MediaQuery { pointer: 3, ..MediaQuery::EMPTY };
    assert!(!bad_ptr.is_valid());
    assert!(!bad_ptr.matches(&MediaEnv::default()));

    let impossible = MediaQuery::width_range(1000, 500);
    assert!(!impossible.is_valid());
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
fn responsive_row_patch_applies_all_26_fields() {
    assert!(RowPatch::new().is_empty());

    let full_patch = RowPatch::new()
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

    let mut layout = LayoutSnapshot::default();
    let mut style = VisualSnapshot::default();
    rule.patch.apply_to(&mut layout, &mut style);

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
    assert_eq!(rule.patch.get_prop(Layout::FLEX_WRAP), Some(FlexWrap::Wrap));
    assert_eq!(rule.patch.get_prop(Visual::OVERFLOW_X), Some(Overflow::Auto));
    assert_eq!(rule.patch.get_prop(Typography::TEXT_ALIGN), Some(TextAlign::Center));
    assert_eq!(rule.patch.get_prop(Typography::FONT_WEIGHT), Some(700));
}
