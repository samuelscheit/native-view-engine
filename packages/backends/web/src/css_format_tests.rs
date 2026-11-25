//! CSS number/colour formatting on top of the `nve_abi::num` fixed-point spec (the same bytes a
//! JS CommandBuffer interpreter must produce).

use nve_abi::{ColorRgba, Dimension, EdgeInsets, LayoutProps, StyleProps};

use crate::css::{write_compact_dim, write_compact_num, write_compact_px, write_rgba};
use crate::{layout_to_css, style_to_css, write_compact_color, write_compact_edges};

fn px(v: f32) -> String {
    let mut s = String::new();
    write_compact_px(v, &mut s);
    s
}

#[test]
fn compact_lengths_drop_unit_only_when_formatted_as_zero() {
    for (v, want) in [
        (0.0, "0"),
        (-0.0, "0"),
        (0.0004, "0"),
        (-0.0004, "0"),
        (0.0005, "0.001px"),
        (f32::NAN, "0"),
        (f32::INFINITY, "0"),
        (12.0, "12px"),
        (-8.0, "-8px"),
        (1.0 / 3.0, "0.333px"),
        (0.1, "0.1px"),
        (1280.5, "1280.5px"),
    ] {
        assert_eq!(px(v), want, "{v}");
    }
    let mut s = String::new();
    write_compact_dim(Dimension::Percent(33.33333), &mut s);
    s.push(' ');
    write_compact_dim(Dimension::Percent(0.0001), &mut s);
    s.push(' ');
    write_compact_dim(Dimension::Auto, &mut s);
    s.push(' ');
    write_compact_num(0.75, &mut s);
    assert_eq!(s, "33.333% 0 auto 0.75");
}

#[test]
fn colors_use_hex_transparent_or_fixed_point_alpha() {
    let c = |r, g, b, a| {
        let mut s = String::new();
        write_compact_color(ColorRgba { r, g, b, a }, &mut s);
        s
    };
    assert_eq!(c(0, 0, 0, 255), "#000000");
    assert_eq!(c(17, 171, 255, 255), "#11abff");
    assert_eq!(c(1, 2, 3, 0), "transparent");
    assert_eq!(c(17, 24, 39, 128), "rgba(17,24,39,0.502)");
    assert_eq!(c(255, 255, 255, 1), "rgba(255,255,255,0.004)");
    assert_eq!(c(0, 0, 0, 254), "rgba(0,0,0,0.996)");
    for a in 1..=254u8 {
        let s = c(9, 9, 9, a);
        let alpha = s.trim_start_matches("rgba(9,9,9,").trim_end_matches(')');
        let parsed = nve_abi::num::parse_f32(alpha).expect(&s);
        assert!((parsed - f32::from(a) / 255.0).abs() <= 0.0005, "{s}");
    }
    let mut s = String::new();
    write_rgba(ColorRgba { r: 1, g: 2, b: 3, a: 255 }, &mut s);
    assert_eq!(s, "rgba(1,2,3,1)");
}

#[test]
fn edge_shorthand_collapses_on_formatted_values() {
    let e = |top, right, bottom, left| {
        let mut s = String::new();
        write_compact_edges("padding", EdgeInsets { top, right, bottom, left }, &mut s);
        s
    };
    assert_eq!(e(0.0, -0.0, 0.0, 0.0), "");
    assert_eq!(e(4.0, 4.0, 4.0, 4.0), "padding:4px;");
    assert_eq!(e(4.0, 8.0, 4.0, 8.0), "padding:4px 8px;");
    assert_eq!(e(1.5, 2.25, 3.125, 2.25), "padding:1.5px 2.25px 3.125px;");
    assert_eq!(e(1.0, 2.0, 3.0, 4.0), "padding:1px 2px 3px 4px;");
}

#[test]
fn full_css_has_no_float_noise_or_exponents() {
    let layout = LayoutProps {
        flex_grow: 1.0 / 3.0,
        flex_shrink: 0.1,
        gap: 1e-7,
        width: Dimension::Points(1e30),
        height: Dimension::Percent(-0.0),
        padding: EdgeInsets { top: 0.1, right: 0.2, bottom: 0.3, left: 0.7 },
        ..LayoutProps::default()
    };
    let css = layout_to_css(&layout);
    assert!(css.contains(";flex-grow:0.333;flex-shrink:0.1;gap:0px;"), "{css}");
    assert!(css.contains("width:9007199254740.992px;height:0%;"), "{css}");
    assert!(css.contains("padding:0.1px 0.2px 0.3px 0.7px;"), "{css}");
    let b = css.as_bytes();
    assert!(
        !b.windows(2).any(|w| w[0].is_ascii_digit() && (w[1] | 0x20) == b'e') && !css.contains('+'),
        "exponent in {css}"
    );
    let style = StyleProps { opacity: 0.3, z_index: -2, font_size: 13.5, ..StyleProps::default() };
    let css = style_to_css(&style);
    assert!(css.contains(";opacity:0.3;font-size:13.5px;z-index:-2;"), "{css}");
}
