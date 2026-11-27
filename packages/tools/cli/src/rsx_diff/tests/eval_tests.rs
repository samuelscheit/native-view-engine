//! Const evaluator: every accepted form and every rejection class.

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

#[test]
fn integer_literals_radixes_suffixes_and_ranges() {
    ok("12", V::Int(12));
    ok("1_000", V::Int(1000));
    ok("-5", V::Int(-5));
    ok("0xff", V::Int(255));
    ok("0xF_F", V::Int(255));
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
    ok("1__", V::Int(1));
}

#[test]
fn float_literals_are_finite_f32_only() {
    ok("8.0", V::Float(8.0));
    ok("-8.0", V::Float(-8.0));
    ok("1_000.5", V::Float(1000.5));
    ok("1e3", V::Float(1000.0));
    ok("2.5e-1", V::Float(0.25));
    ok("1E+2", V::Float(100.0));
    ok("8.0f32", V::Float(8.0));
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
    err("-px(8.0)", E::UnsupportedNegation);
    err("--8", E::UnsupportedNegation);
    err("-x", E::UnsupportedNegation);
    ok("true", V::Bool(true));
    ok("false", V::Bool(false));
    ok("\"8px\"", V::String("8px".into()));
    ok("r#\"a \"quoted\" b\"#", V::String("a \"quoted\" b".into()));
    ok("\"tab\\there\"", V::String("tab\there".into()));
    ok("\"{{literal}}\"", V::String("{literal}".into()));
    ok("\"\"", V::String(String::new()));
    err("\"{x}px\"", E::Interpolated);
    err("\"a\" \"b\"", E::TrailingTokens);
    err("\"\\u{zz}\"", E::InvalidString);
}

#[test]
fn length_helpers() {
    ok("px(8)", V::LengthPx(8.0));
    ok("px(8.0)", V::LengthPx(8.0));
    ok("px(-4.5)", V::LengthPx(-4.5));
    ok("px(8.0,)", V::LengthPx(8.0));
    ok("pct(50)", V::LengthPct(50.0));
    ok("percent(12.5)", V::LengthPct(12.5));
    ok("Length::Px(8.0)", V::LengthPx(8.0));
    ok("Length::points(8.0)", V::LengthPx(8.0));
    ok("Length::Percent(25.0)", V::LengthPct(25.0));
    ok("Length::percent(25.0)", V::LengthPct(25.0));
    ok("Dimension::Points(3.0)", V::LengthPx(3.0));
    ok("Dimension::Percent(3.0)", V::LengthPct(3.0));
    ok("Length::Auto", V::Auto);
    ok("Dimension::Auto", V::Auto);
    err("px()", E::Arity);
    err("px(1.0, 2.0)", E::Arity);
    err("px(,)", E::Arity);
    err("px(1.0,,)", E::Arity);
    err("px(x)", E::NotConstant);
    err("px(\"8\")", E::InvalidNumber);
    err("px(1e39)", E::NonFinite);
    err("px(8.0) * 2.0", E::TrailingTokens);
    err("px(8.0).max(1.0)", E::TrailingTokens);
}

#[test]
fn color_helpers_and_constants() {
    ok("rgb(1, 2, 3)", rgba(1, 2, 3, 255));
    ok("rgba(1, 2, 3, 4)", rgba(1, 2, 3, 4));
    ok("Rgba::rgb(255, 0, 128)", rgba(255, 0, 128, 255));
    ok("ColorRgba::rgba(0, 0, 0, 0)", rgba(0, 0, 0, 0));
    ok("Rgba::new(9, 8, 7, 6)", rgba(9, 8, 7, 6));
    ok("rgb(0xff, 0b1, 2u8)", rgba(255, 1, 2, 255));
    ok("hex(0x112233)", rgba(0x11, 0x22, 0x33, 255));
    ok("hex(0x11223344)", rgba(0x11, 0x22, 0x33, 0x44));
    ok("hex(0x1122_33)", rgba(0x11, 0x22, 0x33, 255));
    ok("hex(0x000000)", rgba(0, 0, 0, 255));
    ok("hex(0x00000000)", rgba(0, 0, 0, 0));
    ok("hex(0xAABBCCu32)", rgba(0xaa, 0xbb, 0xcc, 255));
    ok("ColorRgba::from_hex(0xAABBCC)", rgba(0xaa, 0xbb, 0xcc, 255));
    ok("Rgba::hex(0xAABBCCDD)", rgba(0xaa, 0xbb, 0xcc, 0xdd));
    ok("ColorRgba::WHITE", rgba(255, 255, 255, 255));
    ok("Rgba::BLACK", rgba(0, 0, 0, 255));
    ok("ColorRgba::TRANSPARENT", rgba(0, 0, 0, 0));
    err("ColorRgba::TEAL", E::NotConstant);
    err("hex(0x123)", E::HexDigits);
    err("hex(0x1234567)", E::HexDigits);
    err("hex(0x112233445)", E::HexDigits);
    err("hex(1193046)", E::HexDigits);
    err("hex(-0x112233)", E::HexDigits);
    err("hex(0x11, 0x22)", E::Arity);
    err("rgb(256, 0, 0)", E::ChannelRange);
    err("rgb(-1, 0, 0)", E::ChannelRange);
    err("rgb(1.0, 0, 0)", E::ChannelRange);
    err("rgb(99999999999999999999999, 0, 0)", E::ChannelRange);
    err("rgb(1, 2)", E::Arity);
    err("rgba(1, 2, 3)", E::Arity);
    err("rgba(1, 2, 3, 4, 5)", E::Arity);
    err("rgb(r, 0, 0)", E::NotConstant);
}

#[test]
fn enum_paths_and_theme_tokens() {
    let enum_v = |ty: Option<&str>, variant: &str| V::Enum {
        ty: ty.map(Into::into),
        variant: variant.into(),
    };
    ok(
        "FlexDirection::Column",
        enum_v(Some("FlexDirection"), "Column"),
    );
    ok(
        "nve::FlexDirection::Column",
        enum_v(Some("FlexDirection"), "Column"),
    );
    ok("AlignItems::Center", enum_v(Some("AlignItems"), "Center"));
    ok("CursorKind::Pointer", enum_v(Some("CursorKind"), "Pointer"));
    ok("Row", enum_v(None, "Row"));
    ok("Edge::X", enum_v(Some("Edge"), "X"));
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
    err("", E::Empty);
    err("x", E::NotConstant);
    err("s.m.card_pad", E::NotConstant);
    err("None", E::NotConstant);
    err("Self", E::NotConstant);
    err("Self::Row", E::NotConstant);
    err("MY_CONST", E::NotConstant);
    err("Foo::BAR", E::NotConstant);
    err("f32::NAN", E::NotConstant);
    err("f32::INFINITY", E::NotConstant);
    err("a::b::c::d::E", E::NotConstant);
    err("{ 8.0 }", E::NotConstant);
    err("(8.0)", E::NotConstant);
    err("'a'", E::NotConstant);
    err("b\"x\"", E::NotConstant);
    err("&x", E::NotConstant);
    err("compute_px()", E::UnsupportedCall);
    err("Some(8.0)", E::UnsupportedCall);
    err("nve::px(8.0)", E::UnsupportedCall);
    err("8.0 * 2.0", E::TrailingTokens);
    err("8.0 as f32", E::TrailingTokens);
    err("8.0.max(1.0)", E::TrailingTokens);
    err(
        "FlexDirection::Row | FlexDirection::Column",
        E::TrailingTokens,
    );
    err("if a { 1 } else { 2 }", E::TrailingTokens);
    err("format!(\"{}px\", 8)", E::TrailingTokens);
}
