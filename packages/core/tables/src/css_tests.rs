//! NodeStore inline-style grammar: shorthands, `currentColor`, flex grammar, ingress range sealing,
//! case-insensitive keywords, and strict colors.

use nve_abi::{ColorRgba, Dimension};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

use super::*;
use crate::tables::{AlignItems, FlexDirection, JustifyContent, Layout, Typography, Visual};

fn apply(store: &mut NodeStore, n: NodeId, key: &str, value: &str) -> CssAttrResult {
    let mut tx = store.tx();
    let r = apply_css_attr_layers_status_to_tx(&mut tx, n, key, value, STYLE_LAYER_ALL);
    tx.commit_into(&mut ChangeSet::default());
    r
}

fn node(store: &mut NodeStore, parent: Option<NodeId>) -> NodeId {
    let mut tx = store.tx();
    let n = tx.create();
    if let Some(p) = parent {
        tx.append_child(p, n);
    }
    tx.commit_into(&mut ChangeSet::default());
    n
}

/// `[top, right, bottom, left]` of padding or margin in points.
fn edges(store: &NodeStore, n: NodeId, margin: bool) -> [f32; 4] {
    let l = store.try_row::<Layout>(n).expect("layout row");
    let e = if margin {
        [l.margin_top(), l.margin_right(), l.margin_bottom(), l.margin_left()]
    } else {
        [l.padding_top(), l.padding_right(), l.padding_bottom(), l.padding_left()]
    };
    e.map(Length::value)
}

fn flex(store: &NodeStore, n: NodeId) -> (f32, f32, Length) {
    let l = store.try_row::<Layout>(n).expect("layout row");
    (l.flex_grow(), l.flex_shrink(), l.flex_basis())
}

fn border(store: &NodeStore, n: NodeId) -> (f32, Rgba) {
    let v = store.try_row::<Visual>(n).expect("visual row");
    (v.border_width(), v.border_color())
}

#[test]
fn shorthands_fill_every_edge_and_current_color_is_order_independent() {
    let mut store = NodeStore::new();
    let n = node(&mut store, None);
    let slate = Rgba::rgb(0x1e, 0x29, 0x3b);
    apply(
        &mut store,
        n,
        "style",
        "padding: 14px 16px; margin: 1px 2px 3px; margin-left: -9px; paddingTop: 4PX; border: 1px solid currentColor; color: rgb(30, 41, 59); flex: 1 1 280px;",
    );
    assert_eq!(edges(&store, n, false), [4.0, 16.0, 14.0, 16.0]);
    assert_eq!(edges(&store, n, true), [1.0, 2.0, 3.0, -9.0], "negative margins are valid");
    assert_eq!(border(&store, n), (1.0, slate), "currentColor resolves after the whole list");
    assert_eq!(store.try_row::<Typography>(n).expect("typography").color(), slate);
    assert_eq!(flex(&store, n), (1.0, 1.0, Length::points(280.0)));

    // An explicit `border-color` after `border: … currentColor` wins; `border-color: currentColor`
    // before `color` tracks the final `color`.
    apply(&mut store, n, "style", "border: 2px solid; border-color: red; color: #ffffff;");
    assert_eq!(border(&store, n), (2.0, Rgba::rgb(239, 68, 68)));
    apply(&mut store, n, "style", "border-color: currentColor; color: rgb(10, 20, 30);");
    assert_eq!(border(&store, n).1, Rgba::rgb(10, 20, 30));
}

#[test]
fn current_color_inherits_the_nearest_explicit_ancestor_text_color() {
    let mut store = NodeStore::new();
    let root = node(&mut store, None);
    let mid = node(&mut store, Some(root));
    let leaf = node(&mut store, Some(mid));
    apply(&mut store, root, "color", "#00ff00");
    apply(&mut store, leaf, "border", "1px solid");
    assert_eq!(border(&store, leaf), (1.0, Rgba::rgb(0, 255, 0)));
    assert_eq!(effective_text_color(&store, mid), Rgba::rgb(0, 255, 0));
    apply(&mut store, mid, "color", "blue");
    assert_eq!(effective_text_color(&store, leaf), Rgba::rgb(59, 130, 246));
    let mut tx = store.tx();
    resolve_current_color_border(&mut tx, leaf);
    tx.commit_into(&mut ChangeSet::default());
    assert_eq!(border(&store, leaf).1, Rgba::rgb(59, 130, 246));
    assert_eq!(css_border_color_source("border", "1px solid"), Some(true));
    assert_eq!(css_border_color_source("borderColor", "currentcolor"), Some(true));
    assert_eq!(css_border_color_source("border-color", "#fff"), Some(false));
    assert_eq!(css_border_color_source("border-color", "bogus"), None);
    assert_eq!(css_border_color_source("color", "#fff"), None);
}

#[test]
fn flex_grammar_follows_the_css_omitted_value_rules() {
    let mut store = NodeStore::new();
    let n = node(&mut store, None);
    apply(&mut store, n, "flex", "2");
    assert_eq!(flex(&store, n), (2.0, 1.0, Length::percent(0.0)), "`flex: <n>` is `<n> 1 0%`");
    apply(&mut store, n, "style", "flex: none; flexBasis: 12px");
    assert_eq!(flex(&store, n), (0.0, 0.0, Length::points(12.0)));
    apply(&mut store, n, "flex-basis", "0");
    assert_eq!(flex(&store, n).2, Length::points(0.0));
    apply(&mut store, n, "flex", "auto");
    assert_eq!(flex(&store, n), (1.0, 1.0, Length::AUTO));
    apply(&mut store, n, "flex", "0 0 0");
    assert_eq!(flex(&store, n), (0.0, 0.0, Length::points(0.0)));
    apply(&mut store, n, "flex", "30%");
    assert_eq!(flex(&store, n), (1.0, 1.0, Length::percent(30.0)));
    apply(&mut store, n, "flex", "2 12px 3");
    assert_eq!(flex(&store, n), (1.0, 1.0, Length::percent(30.0)), "split factors reject");
    for bad in ["content", "12", "-1px"] {
        assert!(matches!(apply(&mut store, n, "flex-basis", bad), CssAttrResult::Invalid(_)), "{bad}");
    }
    assert_eq!(flex(&store, n).2, Length::percent(30.0));
}

#[test]
fn invalid_shorthands_are_all_or_nothing() {
    let mut store = NodeStore::new();
    let n = node(&mut store, None);
    apply(&mut store, n, "style", "padding: 1px; margin: 2px; border: 3px solid #fff; flex: 1 1 5px");
    let before = (edges(&store, n, false), edges(&store, n, true), border(&store, n), flex(&store, n));
    apply(
        &mut store,
        n,
        "style",
        "padding: 1px 2px 3px 4px 5px; padding: -1px 2px; paddingTop: -4px; margin: 0 auto; border: 1px wavy red; border: 2 solid red; flex: 1 280px 1; flex: 1 2 3;",
    );
    let after = (edges(&store, n, false), edges(&store, n, true), border(&store, n), flex(&store, n));
    assert_eq!(after, before);
}

#[test]
fn values_are_range_sealed_at_ingress_and_keywords_are_case_insensitive() {
    let mut store = NodeStore::new();
    let n = node(&mut store, None);
    for (key, value) in [
        ("width", " 120px "),
        ("flexDirection", " ROW "),
        ("justify-content", " CENTER "),
        ("alignItems", " FLEX-END "),
        ("flex-grow", " 2 "),
        ("flexShrink", " 0 "),
        ("opacity", " 1.5 "),
        ("z-index", " 10 "),
        ("border-width", "2PX"),
    ] {
        assert!(matches!(apply(&mut store, n, key, value), CssAttrResult::Applied(_)), "{key}: {value}");
    }
    let l = store.try_row::<Layout>(n).expect("layout");
    assert_eq!(l.width(), Length::points(120.0));
    assert_eq!(l.flex_direction(), FlexDirection::Row);
    assert_eq!(l.justify_content(), JustifyContent::Center);
    assert_eq!(l.align_items(), AlignItems::FlexEnd);
    assert_eq!((l.flex_grow(), l.flex_shrink()), (2.0, 0.0));
    let v = store.try_row::<Visual>(n).expect("visual");
    assert_eq!((v.opacity(), v.z_index(), v.border_width()), (1.0, 10, 2.0));

    let snapshot = |s: &NodeStore| {
        let l = s.try_row::<Layout>(n).expect("layout");
        let v = s.try_row::<Visual>(n).expect("visual");
        (l.width(), l.height(), l.flex_grow(), l.flex_shrink(), l.flex_basis(), l.row_gap(), v.border_width(), v.border_radius(), v.opacity(), v.z_index())
    };
    let before = snapshot(&store);
    for (key, bad) in [
        ("width", "-10px"),
        ("width", "1e8px"),
        ("width", "garbage"),
        ("width", ""),
        ("height", "1e8%"),
        ("padding", "20000000px"),
        ("margin-left", "-20000000px"),
        ("flex-grow", "nope"),
        ("flex-grow", "-1"),
        ("flex-grow", "99999999"),
        ("flex-shrink", "-2"),
        ("flex-shrink", "99999999"),
        ("flex", "99999999"),
        ("flex-basis", "1e8%"),
        ("gap", "-8px"),
        ("gap", "20000000px"),
        ("border-width", "20000000px"),
        ("border-width", "-1px"),
        ("border-width", "2"),
        ("border-width", "auto"),
        ("border-radius", "-4px"),
        ("border-radius", "1e8px"),
        ("opacity", "nan"),
        ("font-size", "0px"),
        ("font-size", "-4px"),
        ("font-size", "1e8px"),
        ("z-index", "nope"),
        ("flex-direction", "bogus"),
    ] {
        assert!(matches!(apply(&mut store, n, key, bad), CssAttrResult::Invalid(_)), "{key}: {bad:?} must be rejected");
    }
    assert_eq!(snapshot(&store), before);

    for (kw, width) in [("thin", 1.0), ("medium", 3.0), ("THICK", 5.0), ("0", 0.0)] {
        apply(&mut store, n, "borderWidth", kw);
        assert_eq!(border(&store, n).0, width, "{kw}");
    }
}

#[test]
fn numeric_parsers_are_strict_and_bounded() {
    assert_eq!(parse_px("12px"), Some(12.0));
    assert_eq!(parse_px(" 12Px "), Some(12.0));
    assert_eq!(parse_px(" 12.5 "), Some(12.5));
    assert_eq!(parse_px("-0px").map(f32::to_bits), Some((-0.0f32).to_bits()));
    assert_eq!(parse_px("1e2px"), Some(100.0));
    assert_eq!(parse_px(".5"), Some(0.5));
    for bad in ["", "px", "12 px", "12pt", "inf", "NaN", "1e39", "1e8", "0x10", "1,5px", "--1", "1é"] {
        assert_eq!(parse_px(bad), None, "{bad:?}");
    }
    assert_eq!(parse_dim("33.5%"), Dimension::Percent(33.5));
    assert_eq!(parse_dim(" 50 % "), Dimension::Percent(50.0));
    assert_eq!(parse_dim("AUTO"), Dimension::Auto);
    assert_eq!(parse_dim("abc%"), Dimension::Auto);
    assert_eq!(parse_dim("7px"), Dimension::Points(7.0));
    assert_eq!(try_parse_dim("-7px"), None);
    assert_eq!(try_parse_font_weight(" Bold "), Some(700));
}

#[test]
fn colors_parse_hex_shorthands_and_strict_rgb_functions() {
    assert_eq!(parse_color("#fff"), Some(ColorRgba::WHITE));
    assert_eq!(parse_color("#0F8"), Some(ColorRgba::rgb(0x00, 0xff, 0x88)));
    assert_eq!(parse_color("#0f8c"), Some(ColorRgba::rgba(0x00, 0xff, 0x88, 0xcc)));
    assert_eq!(parse_color("RED"), Some(ColorRgba::rgb(239, 68, 68)));
    for bad_hex in ["#aéabc", "#éé", "#ééé", "#aéabcdef", "#+f+f+f", "#+f+f+f+f", "#-12345", "#12345"] {
        assert_eq!(parse_color(bad_hex), None, "{bad_hex:?}");
    }
    assert_eq!(parse_color("rgb(30, 41, 59)"), Some(ColorRgba::rgb(30, 41, 59)));
    assert_eq!(parse_color("RGBA(0,0,0,0.5)"), Some(ColorRgba::rgba(0, 0, 0, 128)));
    assert_eq!(parse_color("rgb(300 -5 10.4 / 25%)"), Some(ColorRgba::rgba(255, 0, 10, 64)));
    assert_eq!(parse_color("rgba(1, 2, 3, 2)"), Some(ColorRgba::rgb(1, 2, 3)), "alpha clamps");
    assert_eq!(parse_color("rgb(100%, 50%, 0%)"), Some(ColorRgba::rgb(255, 128, 0)));
    assert_eq!(parse_color("rgb(100% 50% 0% / 50%)"), Some(ColorRgba::rgba(255, 128, 0, 128)));
    for bad in [
        "rgb(1, 2)",
        "rgb(1, 2, 3",
        "rgb(1, 2, 3, 4, 5)",
        "rgb(a, b, c)",
        "rgb()",
        "rgb(1 2, 3)",
        "rgb(1, , 2, 3)",
        "rgb(1, 2, 3,)",
        "rgb(1 / 2 / 3 / 0.5)",
        "rgb(1 2 3 0.5)",
        "rgb(1, 2, 3 / 0.5)",
        "rgb(100%, 128, 0)",
        "rgbé(1, 2, 3)",
    ] {
        assert_eq!(parse_color(bad), None, "{bad:?}");
    }
}
