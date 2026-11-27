use super::*;
use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexDirection, LayoutProps, StyleProps};

/// Collects every [`ExtStyleDecl::FlexBasis`] and [`ExtStyleDecl::Raw`] emitted while applying `key: value`.
fn ext_decls(
    l: &mut LayoutProps,
    s: &mut StyleProps,
    key: &str,
    value: &str,
) -> Vec<(Option<Dimension>, String)> {
    let mut out = Vec::new();
    assert!(apply_typed_attr_with_ext(l, s, key, value, |d| match d {
        ExtStyleDecl::FlexBasis(b) => out.push((Some(b), String::new())),
        ExtStyleDecl::Raw(k, v) => out.push((None, format!("{k}:{v}"))),
        ExtStyleDecl::BorderCurrentColor(_)
        | ExtStyleDecl::ExplicitTextColor
        | ExtStyleDecl::ExplicitStyle => {}
    }));
    out
}

#[test]
fn inline_style_shorthands_fill_typed_props_and_typed_flex_basis() {
    let mut l = LayoutProps::default();
    let mut s = StyleProps::default();
    let decls = ext_decls(
        &mut l,
        &mut s,
        "style",
        "padding: 14px 16px; margin: 1px 2px 3px; margin-left: -9px; paddingTop: 4PX; border: 1px solid currentColor; color: rgb(30, 41, 59); flex: 1 1 280px;",
    );
    let edges = |top, right, bottom, left| EdgeInsets { top, right, bottom, left };
    let slate = ColorRgba::rgb(0x1e, 0x29, 0x3b);
    assert_eq!(l.padding, edges(4.0, 16.0, 14.0, 16.0));
    assert_eq!(l.margin, edges(1.0, 2.0, 3.0, -9.0), "negative margins are valid");
    // `currentColor` resolves after the full `style` list, regardless of declaration order.
    assert_eq!((s.border_width, s.border_color), (1.0, slate));
    assert_eq!(s.text_color, slate);

    // Explicit `border-color` after `border: ... currentColor` overrides `currentColor`;
    // `border-color: currentColor` before `color` also tracks the final `color`.
    let mut l2 = LayoutProps::default();
    let mut s2 = StyleProps::default();
    let _ = ext_decls(
        &mut l2,
        &mut s2,
        "style",
        "border: 2px solid; border-color: red; color: #ffffff;",
    );
    assert_eq!((s2.border_width, s2.border_color), (2.0, ColorRgba::rgb(239, 68, 68)));
    let _ = ext_decls(
        &mut l2,
        &mut s2,
        "style",
        "border-color: currentColor; color: rgb(10, 20, 30);",
    );
    assert_eq!(s2.border_color, ColorRgba::rgb(10, 20, 30));
    assert_eq!((l.flex_grow, l.flex_shrink), (1.0, 1.0));
    assert_eq!(decls, vec![(Some(Dimension::Points(280.0)), String::new())]);

    // `flex: <grow>` implies basis 0%; `flex: none` resets factors and basis; unitless zero.
    let decls = ext_decls(&mut l, &mut s, "flex", "2");
    assert_eq!((l.flex_grow, l.flex_shrink), (2.0, 1.0));
    assert_eq!(decls, vec![(Some(Dimension::Percent(0.0)), String::new())]);
    let decls = ext_decls(&mut l, &mut s, "style", "flex: none; flexBasis: 12px; flex-basis: 0");
    assert_eq!((l.flex_grow, l.flex_shrink), (0.0, 0.0));
    let bases: Vec<_> = decls.into_iter().map(|(b, _)| b.unwrap()).collect();
    assert_eq!(
        bases,
        [Dimension::Auto, Dimension::Points(12.0), Dimension::Points(0.0)]
    );
    let decls = ext_decls(&mut l, &mut s, "style", "flex: 0 0 0");
    assert_eq!((l.flex_grow, l.flex_shrink), (0.0, 0.0));
    assert_eq!(decls, vec![(Some(Dimension::Points(0.0)), String::new())]);
    // Unsupported bases (`content`, unitless non-zero, negative) emit nothing.
    assert!(ext_decls(&mut l, &mut s, "style", "flex-basis: content; flex-basis: 12; flex-basis: -1px").is_empty());

    // Invalid shorthands leave every target untouched (all-or-nothing).
    let before = (l, s);
    assert!(ext_decls(&mut l, &mut s, "style", "padding: 1px 2px 3px 4px 5px; padding: -1px 2px; paddingTop: -4px; margin: 0 auto; border: 1px wavy red; border: 2 solid red; flex: 1 280px 1; flex: 1 2 3;").is_empty());
    assert_eq!((l, s), before);
}

#[test]
fn parses_inline_style_and_colors() {
    let mut l = LayoutProps::default();
    let mut s = StyleProps::default();
    assert!(apply_typed_attr(
        &mut l,
        &mut s,
        "style",
        "width: 100%; height: 64px; flex-direction: row; gap: 12; padding: 16px; background-color: #0f172a; color: #ffffff; border-radius: 8px;"
    ));
    assert_eq!(l.width, Dimension::Percent(100.0));
    assert_eq!(l.height, Dimension::Points(64.0));
    assert_eq!(l.flex_direction, FlexDirection::Row);
    assert_eq!(l.gap, 12.0);
    assert_eq!(l.padding, EdgeInsets::all(16.0));
    assert_eq!(s.background, ColorRgba::rgb(15, 23, 42));
    assert_eq!(s.text_color, ColorRgba::WHITE);
    assert_eq!(s.border_radius, 8.0);
}

#[test]
fn numeric_style_values_use_the_fixed_point_parser() {
    assert_eq!(parse_px("12px"), Some(12.0));
    assert_eq!(parse_px("12PX"), Some(12.0));
    assert_eq!(parse_px(" 12Px "), Some(12.0));
    assert_eq!(parse_px(" 12.5 "), Some(12.5));
    assert_eq!(parse_px("-0px").map(f32::to_bits), Some((-0.0f32).to_bits()));
    assert_eq!(parse_px("1e2px"), Some(100.0));
    assert_eq!(parse_px(".5"), Some(0.5));
    for bad in ["", "px", "12 px", "12pt", "inf", "NaN", "1e39", "0x10", "1,5px", "--1"] {
        assert_eq!(parse_px(bad), None, "{bad:?}");
    }
    assert_eq!(parse_dim("33.5%"), Dimension::Percent(33.5));
    assert_eq!(parse_dim(" 50 % "), Dimension::Percent(50.0));
    assert_eq!(parse_dim("AUTO"), Dimension::Auto);
    assert_eq!(parse_dim("abc%"), Dimension::Auto);
    assert_eq!(parse_dim("7px"), Dimension::Points(7.0));

    let mut l = LayoutProps::default();
    let mut s = StyleProps::default();
    assert!(apply_typed_attr(&mut l, &mut s, "flex-grow", "2.5"));
    assert!(apply_typed_attr(&mut l, &mut s, "flexShrink", "0"));
    assert!(apply_typed_attr(&mut l, &mut s, "opacity", "1.5"));
    assert_eq!((l.flex_grow, l.flex_shrink, s.opacity), (2.5, 0.0, 1.0));
    // Malformed numbers leave the previous value untouched.
    assert!(apply_typed_attr(&mut l, &mut s, "flex-grow", "fast"));
    assert!(apply_typed_attr(&mut l, &mut s, "opacity", "nan"));
    assert_eq!((l.flex_grow, s.opacity), (2.5, 1.0));
}

#[test]
fn forwards_non_abi_inline_style_declarations_as_raw_props() {
    let mut l = LayoutProps::default();
    let mut s = StyleProps::default();
    let raw = |decls: Vec<(Option<Dimension>, String)>| -> Vec<String> {
        decls.into_iter().map(|(_, kv)| kv).collect()
    };
    let forwarded = raw(ext_decls(
        &mut l,
        &mut s,
        "style",
        "font-size: 18px; font-weight: 700; text-align: center; box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35); overflow: hidden; unknown-prop: 42;",
    ));
    assert_eq!(s.font_size, 18.0);
    assert_eq!(
        forwarded,
        [
            "font-weight:700",
            "text-align:center",
            "box-shadow:0 4px 12px rgba(0, 0, 0, 0.35)",
            "overflow:hidden",
        ]
    );

    // CamelCase variants inside inline style normalize to canonical kebab-case keys.
    let camel_forwarded = raw(ext_decls(
        &mut l,
        &mut s,
        "style",
        "fontWeight: semibold; textAlign: right; boxShadow: 2px 3px 6px #00000080;",
    ));
    assert_eq!(
        camel_forwarded,
        [
            "font-weight:semibold",
            "text-align:right",
            "box-shadow:2px 3px 6px #00000080",
        ]
    );
}

#[test]
fn rgb_functions_parse_legacy_and_modern_syntax() {
    assert_eq!(parse_color("rgb(30, 41, 59)"), Some(ColorRgba::rgb(30, 41, 59)));
    assert_eq!(parse_color("RGBA(0,0,0,0.5)"), Some(ColorRgba::rgba(0, 0, 0, 128)));
    assert_eq!(parse_color("rgb(300 -5 10.4 / 25%)"), Some(ColorRgba::rgba(255, 0, 10, 64)));
    assert_eq!(parse_color("rgba(1, 2, 3, 2)"), Some(ColorRgba::rgb(1, 2, 3)), "alpha clamps");
    assert_eq!(parse_color("rgb(100%, 50%, 0%)"), Some(ColorRgba::rgb(255, 128, 0)));
    assert_eq!(
        parse_color("rgb(100% 50% 0% / 50%)"),
        Some(ColorRgba::rgba(255, 128, 0, 128))
    );
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
    ] {
        assert_eq!(parse_color(bad), None, "{bad:?}");
    }
}

#[test]
fn hex_shorthands_non_ascii_safety_and_border_width_longhand_consistency() {
    // 3- and 4-digit hex shorthands.
    assert_eq!(parse_color("#fff"), Some(ColorRgba::WHITE));
    assert_eq!(parse_color("#0F8"), Some(ColorRgba::rgb(0x00, 0xff, 0x88)));
    assert_eq!(parse_color("#0f8c"), Some(ColorRgba::rgba(0x00, 0xff, 0x88, 0xcc)));
    // Non-ASCII and leading-sign hex inputs must return None without panicking.
    for bad_hex in ["#aéabc", "#éé", "#ééé", "#aéabcdef", "#+f+f+f", "#+f+f+f+f", "#-12345"] {
        assert_eq!(parse_color(bad_hex), None, "{bad_hex:?}");
    }
    assert_eq!(canonical_raw_style_key("object-fit"), Some("object-fit"));
    assert_eq!(canonical_raw_style_key("objectFit"), Some("object-fit"));
    // `border: 1px solid #fff` works with 3-digit hex.
    let mut l = LayoutProps::default();
    let mut s = StyleProps::default();
    assert!(apply_typed_attr(&mut l, &mut s, "border", "1px solid #fff"));
    assert_eq!((s.border_width, s.border_color), (1.0, ColorRgba::WHITE));

    // `border-width` longhand follows the exact same grammar as `border` shorthand.
    assert!(apply_typed_attr(&mut l, &mut s, "border-width", "thin"));
    assert_eq!(s.border_width, 1.0);
    assert!(apply_typed_attr(&mut l, &mut s, "borderWidth", "medium"));
    assert_eq!(s.border_width, 3.0);
    assert!(apply_typed_attr(&mut l, &mut s, "border-width", "thick"));
    assert_eq!(s.border_width, 5.0);
    assert!(apply_typed_attr(&mut l, &mut s, "border-width", "0"));
    assert_eq!(s.border_width, 0.0);
    assert!(apply_typed_attr(&mut l, &mut s, "border-width", "2PX"));
    assert_eq!(s.border_width, 2.0);
    // Negative and unitless non-zero widths are ignored (leave previous value intact).
    for bad in ["-1px", "2", "auto", ""] {
        assert!(apply_typed_attr(&mut l, &mut s, "border-width", bad));
        assert_eq!(s.border_width, 2.0, "{bad:?}");
    }

    // `TypedAttrKind::from_key` agrees with `apply_typed_attr`, rejects invalid values strictly,
    // and trims standalone keyword / integer attributes consistently with `style="..."`.
    for (raw, expected) in [
        ("flex", TypedAttrKind::Flex),
        ("flexBasis", TypedAttrKind::FlexBasis),
        ("paddingTop", TypedAttrKind::PaddingTop),
        ("marginHorizontal", TypedAttrKind::MarginHorizontal),
        ("background", TypedAttrKind::Background),
        ("borderColor", TypedAttrKind::BorderColor),
        ("borderWidth", TypedAttrKind::BorderWidth),
    ] {
        assert_eq!(TypedAttrKind::from_key(raw), Some(expected));
        assert!(apply_typed_attr(&mut l, &mut s, raw, ""));
    }
    assert_eq!(TypedAttrKind::from_key("style"), None);
    assert_eq!(TypedAttrKind::from_key("font-weight"), None);

    // Invalid dimension / keyword / z-index values return `false` and leave targets untouched.
    let mut l_strict = LayoutProps::default();
    let mut s_strict = StyleProps::default();
    assert!(apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::Width, " 120px ", |_| {}));
    assert!(apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::FlexDirection, " row ", |_| {}));
    assert!(apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::JustifyContent, " center ", |_| {}));
    assert!(apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::AlignItems, " flex-end ", |_| {}));
    assert!(apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::ZIndex, " 10 ", |_| {}));
    assert_eq!(l_strict.width, Dimension::Points(120.0));
    assert_eq!(l_strict.flex_direction, FlexDirection::Row);
    assert_eq!(s_strict.z_index, 10);
    let snap_l = l_strict;
    let snap_s = s_strict;
    assert!(!apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::Width, "garbage", |_| {}));
    assert!(!apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::Width, "", |_| {}));
    assert!(!apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::FlexDirection, "bogus", |_| {}));
    assert!(!apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::JustifyContent, "bogus", |_| {}));
    assert!(!apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::AlignItems, "bogus", |_| {}));
    assert!(!apply_typed_attr_kind(&mut l_strict, &mut s_strict, TypedAttrKind::ZIndex, "nope", |_| {}));
    assert_eq!((l_strict, s_strict), (snap_l, snap_s));

    let mut explicit_color = false;
    let mut explicit_style = false;
    let mut border_current = None;
    let mut s3 = StyleProps::default();
    apply_typed_attr_with_ext(&mut l, &mut s3, "style", "border: 1px solid; color: #fff", |d| {
        match d {
            ExtStyleDecl::ExplicitTextColor => explicit_color = true,
            ExtStyleDecl::ExplicitStyle => explicit_style = true,
            ExtStyleDecl::BorderCurrentColor(v) => border_current = Some(v),
            _ => {}
        }
    });
    assert!(explicit_color);
    assert!(explicit_style);
    assert_eq!(border_current, Some(true));

    // Default-valued `font-size: 16px` emits `ExplicitStyle`; layout-only `width: 100px` does not.
    let mut def_style_flag = false;
    apply_typed_attr_with_ext(&mut l, &mut s3, "style", "font-size: 16px", |d| {
        if matches!(d, ExtStyleDecl::ExplicitStyle) {
            def_style_flag = true;
        }
    });
    assert!(def_style_flag);
    let mut layout_only_flag = false;
    apply_typed_attr_with_ext(&mut l, &mut s3, "style", "width: 100px", |d| {
        if matches!(d, ExtStyleDecl::ExplicitStyle) {
            layout_only_flag = true;
        }
    });
    assert!(!layout_only_flag);
}
