//! Const evaluator: every accepted real-API form and every rejection class.

use super::super::eval::{EvalError as E, TypedAttrValue as V};
use super::eval_src;

fn ok(src: &str, expected: V) {
    assert_eq!(eval_src(src), Ok(expected), "{src}");
}

fn err(src: &str, expected: E) {
    assert_eq!(eval_src(src), Err(expected), "{src}");
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> V {
    V::Rgba { r, g, b, a }
}

fn enum_v(ty: Option<&str>, variant: &str) -> V {
    V::Enum {
        ty: ty.map(Into::into),
        variant: variant.into(),
    }
}

#[test]
fn integer_literals_radixes_suffixes_and_ranges() {
    ok("12", V::Int(12));
    ok("1_000", V::Int(1000));
    ok("1__", V::Int(1));
    ok("-5", V::Int(-5));
    ok("0xff", V::Int(255));
    ok("0xF_F", V::Int(255));
    ok("0xFF_00_00_FF", V::Int(0xFF00_00FF));
    ok("0b101", V::Int(5));
    ok("0o17", V::Int(15));
    ok("12u8", V::Int(12));
    ok("-128i8", V::Int(-128));
    ok("0xffu8", V::Int(255));
    ok("9223372036854775807", V::Int(i64::MAX));
    ok("-9223372036854775808", V::Int(i64::MIN));
    err("9223372036854775808", E::IntOverflow);
    err("99999999999999999999999999999999999999999", E::IntOverflow);
    err("256u8", E::IntOverflow);
    err("-1u8", E::IntOverflow);
    err("-129i8", E::IntOverflow);
    err("12q", E::InvalidNumber);
    err("0x", E::InvalidNumber);
    err("0b102", E::InvalidNumber);
    // `0xfe-1` lexes as `0xfe`, `-`, `1`: not one literal.
    err("0xfe-1", E::TrailingTokens);
}

#[test]
fn float_literals_are_finite_in_f32() {
    ok("8.0", V::Float(8.0));
    ok("-8.0", V::Float(-8.0));
    ok("1_000.5", V::Float(1000.5));
    ok("1e3", V::Float(1000.0));
    ok("2.5e-1", V::Float(0.25));
    ok("1E+2", V::Float(100.0));
    ok("8.0f32", V::Float(8.0));
    ok("8.0_f32", V::Float(8.0));
    ok("8f64", V::Float(8.0));
    ok("1.", V::Float(1.0));
    ok("3.4e38", V::Float(3.4e38));
    err("1e39", E::NonFinite);
    err("-1e39", E::NonFinite);
    err("1e400", E::NonFinite);
    err("8.0u8", E::InvalidNumber);
    err("1e", E::InvalidNumber);
}

#[test]
fn signs_bools_and_strings() {
    err("+8.0", E::UnaryPlus);
    err("-Length::points(8.0)", E::UnsupportedNegation);
    err("--8", E::UnsupportedNegation);
    err("-x", E::UnsupportedNegation);
    ok("true", V::Bool(true));
    ok("false", V::Bool(false));
    ok("\"8px\"", V::String("8px".into()));
    ok("r#\"a \"quoted\" b\"#", V::String("a \"quoted\" b".into()));
    ok(
        "\"tab\\there\\x41\\u{1F600}\"",
        V::String("tab\there\u{41}\u{1F600}".into()),
    );
    ok("\"{{literal}}\"", V::String("{literal}".into()));
    ok("\"\"", V::String(String::new()));
    err("\"{x}px\"", E::Interpolated);
    err("\"a\" \"b\"", E::TrailingTokens);
    err("\"\\u{zz}\"", E::InvalidString);
    err("\"\\u{D800}\"", E::InvalidString);
    err("\"\\q\"", E::InvalidString);
    err("\"\\x80\"", E::InvalidString);
    err("\"\\xé0\"", E::InvalidString);
    err("\"\\x4\"", E::InvalidString);
}

#[test]
fn length_constructors_and_constants() {
    ok("Length::points(8.0)", V::LengthPx(8.0));
    ok("Length::points(-4.5)", V::LengthPx(-4.5));
    ok("Length::points(8.0,)", V::LengthPx(8.0));
    ok("nve_store::Length::points(8.0)", V::LengthPx(8.0));
    ok("Length::percent(25.0)", V::LengthPct(25.0));
    ok("Dimension::Points(3.0)", V::LengthPx(3.0));
    ok("nve_abi::Dimension::Percent(3.0)", V::LengthPct(3.0));
    ok("Length::AUTO", V::Auto);
    ok("Length::ZERO", V::LengthPx(0.0));
    ok("Dimension::Auto", V::Auto);
    // `Length::points` takes `f32`: integer literals do not compile.
    err("Length::points(8)", E::LengthArgument);
    err("Length::points(\"8\")", E::LengthArgument);
    err("Length::points()", E::Arity);
    err("Length::points(1.0, 2.0)", E::Arity);
    err("Length::points(,)", E::Arity);
    err("Length::points(1.0,,)", E::Arity);
    err("Length::points(x)", E::NotConstant);
    err("Length::points(1e39)", E::NonFinite);
    err("Length::points(8.0) * 2.0", E::TrailingTokens);
    err("Length::points(8.0).max(1.0)", E::TrailingTokens);
    // Invented helpers are not part of the API.
    for invented in [
        "px(8.0)",
        "pct(50.0)",
        "percent(5.0)",
        "Length::Px(8.0)",
        "Dimension::Px(1.0)",
    ] {
        err(invented, E::UnsupportedCall);
    }
    // `Length`/`Rgba` are structs: `Length::Auto` is not a variant (the constant is `AUTO`).
    err("Length::Auto", E::NotConstant);
    err("Rgba::Red", E::NotConstant);
}

#[test]
fn color_constructors_and_constants() {
    ok("Rgba::rgb(255, 0, 128)", rgba(255, 0, 128, 255));
    ok("Rgba::new(9, 8, 7, 6)", rgba(9, 8, 7, 6));
    ok("nve_store::Rgba::rgb(0xff, 0b1, 2u8)", rgba(255, 1, 2, 255));
    ok("ColorRgba::rgb(1, 2, 3)", rgba(1, 2, 3, 255));
    ok("ColorRgba::rgba(0, 0, 0, 0)", rgba(0, 0, 0, 0));
    ok("ColorRgba::WHITE", rgba(255, 255, 255, 255));
    ok("Rgba::BLACK", rgba(0, 0, 0, 255));
    ok("ColorRgba::TRANSPARENT", rgba(0, 0, 0, 0));
    err("ColorRgba::TEAL", E::NotConstant);
    for invented in [
        "rgb(1, 2, 3)",
        "rgba(1, 2, 3, 4)",
        "hex(0x112233)",
        "ColorRgba::hex(0x1)",
        "ColorRgba::from_hex(0x1)",
        "Rgba::rgba(1, 2, 3, 4)",
        "ColorRgba::new(1, 2, 3, 4)",
    ] {
        err(invented, E::UnsupportedCall);
    }
    err("Rgba::rgb(256, 0, 0)", E::ChannelRange);
    err("Rgba::rgb(-1, 0, 0)", E::ChannelRange);
    err("Rgba::rgb(1.0, 0, 0)", E::ChannelRange);
    err("Rgba::rgb(99999999999999999999999, 0, 0)", E::ChannelRange);
    err("Rgba::rgb(1, 2)", E::Arity);
    err("Rgba::new(1, 2, 3)", E::Arity);
    err("Rgba::new(1, 2, 3, 4, 5)", E::Arity);
    err("Rgba::rgb(r, 0, 0)", E::NotConstant);
}

#[test]
fn enum_paths_and_theme_tokens() {
    ok(
        "FlexDirection::Column",
        enum_v(Some("FlexDirection"), "Column"),
    );
    ok(
        "nve_components::tables::FlexDirection::Column",
        enum_v(Some("FlexDirection"), "Column"),
    );
    ok("Display::None", enum_v(Some("Display"), "None"));
    ok("Overflow::Auto", enum_v(Some("Overflow"), "Auto"));
    ok("Edge::X", enum_v(Some("Edge"), "X"));
    ok("Row", enum_v(None, "Row"));
    ok(
        "theme.surface.base",
        V::ThemeToken("theme.surface.base".into()),
    );
    ok("theme.radius.md", V::ThemeToken("theme.radius.md".into()));
    ok("Theme::LIGHT", V::ThemeToken("Theme::LIGHT".into()));
    ok("theme::ACCENT", V::ThemeToken("theme::ACCENT".into()));
    ok(
        "Theme::LIGHT.surface.base",
        V::ThemeToken("Theme::LIGHT.surface.base".into()),
    );
    err("theme", E::NotConstant);
    err("theme.surface.base.hex()", E::UnsupportedCall);
    err("theme.colors[0]", E::UnsupportedCall);
    err("Theme::dark()", E::UnsupportedCall);
    err("theme.surface.", E::TrailingTokens);
    err("theme.surface + 1", E::TrailingTokens);
}

#[test]
fn non_constant_expressions_are_rejected() {
    for src in [
        "x",
        "s.m.card_pad",
        "None",
        "Some",
        "Option::None",
        "Self",
        "Self::Row",
        "NaN",
        "X",
        "N",
        "MY_CONST",
        "Foo::BAR",
        "f32::NAN",
        "f32::INFINITY",
        "a::b::c::d::e::F",
        "{ 8.0 }",
        "(8.0)",
        "'a'",
        "'static",
        "b\"x\"",
        "&x",
    ] {
        err(src, E::NotConstant);
    }
    for src in [
        "compute_px()",
        "Some(8.0)",
        "Option::Some(8.0)",
        "nve::px(8.0)",
    ] {
        err(src, E::UnsupportedCall);
    }
    for src in [
        "8.0 * 2.0",
        "8.0 as f32",
        "8.0.max(1.0)",
        "FlexDirection::Row | FlexDirection::Column",
        "if a { 1 } else { 2 }",
        "format!(\"{}px\", 8)",
    ] {
        err(src, E::TrailingTokens);
    }
    err("", E::Empty);
}
