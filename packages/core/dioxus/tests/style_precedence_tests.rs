//! Style precedence on `NveMutationWriter`, written straight into the `NodeStore` tables:
//! numeric shorthand vs longhand order-independence, `TypedStyle` vs numeric standalone
//! precedence and layer removal, and per-property text inheritance (explicit vs inherited
//! `Typography` fields).

#[path = "support/style_scene.rs"]
mod style_scene;

use dioxus_core::AttributeValue;
use nve_abi::ColorRgba;
use nve_dioxus::TypedStyle;
use nve_store::{Length, Rgba};
use style_scene::{Scene, typed};

const RED: Rgba = Rgba::rgb(239, 68, 68);
const BLUE: Rgba = Rgba::rgb(0x00, 0x00, 0xff);
const GREEN: Rgba = Rgba::rgb(0x22, 0xc5, 0x5e);

fn px(v: f32) -> Length {
    Length::points(v)
}

fn num(v: Option<f64>) -> AttributeValue {
    v.map_or(AttributeValue::None, AttributeValue::Float)
}

fn padding(top: f32, right: f32, bottom: f32, left: f32) -> [Length; 4] {
    [px(top), px(right), px(bottom), px(left)]
}

#[test]
fn numeric_shorthand_vs_longhand_precedence_survives_attribute_order_and_toggles() {
    let mut scene = Scene::new();
    for name in ["flex_split", "flex_split_rev", "pad_toggle", "gap_split"] {
        scene.add(name, "view", None);
    }
    for step in 0..3usize {
        let flex = if step % 2 == 0 { 1.0 } else { 2.0 };
        let pad = (step != 1).then_some(16.0);
        let row_gap = (step != 1).then_some(20.0);
        scene.frame(&[
            (
                "flex_split",
                &[("flex-grow", num(Some(3.0))), ("flex", num(Some(flex)))],
            ),
            (
                "flex_split_rev",
                &[("flex", num(Some(flex))), ("flex-grow", num(Some(3.0)))],
            ),
            (
                "pad_toggle",
                &[
                    ("padding", num(pad)),
                    ("padding-horizontal", num(Some(4.0))),
                ],
            ),
            (
                "gap_split",
                &[("gap", num(Some(8.0))), ("row-gap", num(row_gap))],
            ),
        ]);
        for name in ["flex_split", "flex_split_rev"] {
            assert_eq!(scene.flex_factors(name).0, 3.0, "{name} @ {step}");
        }
        let expected_pad = if step == 1 {
            padding(0.0, 4.0, 0.0, 4.0)
        } else {
            padding(16.0, 4.0, 16.0, 4.0)
        };
        assert_eq!(scene.padding("pad_toggle"), expected_pad, "padding @ {step}");
        assert_eq!(
            scene.layout("gap_split", |l| (l.row_gap(), l.column_gap())),
            Some((px(if step == 1 { 8.0 } else { 20.0 }), px(8.0))),
            "gap @ {step}"
        );
    }
}

#[test]
fn typed_style_and_standalone_numeric_layer_removal_converge() {
    let mut scene = Scene::new();
    for name in ["typed_only", "layered_pad"] {
        scene.add(name, "view", Some("root"));
    }
    let typed_pad = || typed(TypedStyle::new().padding(14.0).bg(ColorRgba::rgb(0xef, 0x44, 0x44)));

    scene.frame(&[
        ("typed_only", &[typed(TypedStyle::new().bg(ColorRgba::rgb(0xef, 0x44, 0x44)).width_px(100.0))]),
        ("layered_pad", &[typed_pad(), ("padding", num(Some(8.0)))]),
    ]);
    assert_eq!(scene.padding("layered_pad"), padding(8.0, 8.0, 8.0, 8.0));

    // Same frame: clear `nve_style` on `typed_only` while setting standalone `width: 240.0`,
    // and clear standalone `padding` on `layered_pad` so `TypedStyle`'s 14px restores.
    scene.frame(&[
        ("typed_only", &[("nve_style", AttributeValue::None), ("width", num(Some(240.0)))]),
        ("layered_pad", &[("padding", AttributeValue::None)]),
    ]);
    assert_eq!(scene.layout("typed_only", |l| l.width()), Some(px(240.0)));
    assert!(
        !scene.writer.store().has::<nve_tables::tables::Visual>(scene.node("typed_only")),
        "cleared TypedStyle must drop Visual row"
    );
    assert_eq!(scene.padding("layered_pad"), padding(14.0, 14.0, 14.0, 14.0));
}

const TEXT_CHILDREN: [&str; 4] = ["dyn_opacity", "dyn_fs", "typed_op", "typed_fs_col"];

#[test]
fn text_properties_inherit_per_property_and_reinherit_when_removed() {
    let mut scene = Scene::new();
    scene.add("parent", "text", None);
    for name in TEXT_CHILDREN {
        scene.add(name, "text", Some("parent"));
    }
    let red_typed = || {
        TypedStyle::new()
            .font_size(16.0)
            .color(ColorRgba::rgb(0xef, 0x44, 0x44))
    };

    let drive = |scene: &mut Scene, s: usize| {
        let (fs, col) = if s < 2 {
            (24.0, ColorRgba::rgb(0x00, 0x00, 0xff))
        } else {
            (30.0, ColorRgba::rgb(0x22, 0xc5, 0x5e))
        };
        let on = s == 1 || s == 2;
        let typed_fs_col = if on {
            red_typed()
        } else {
            TypedStyle::new().opacity(0.6)
        };
        scene.frame(&[
            ("parent", &[typed(TypedStyle::new().font_size(fs).color(col))]),
            ("dyn_opacity", &[("opacity", num(on.then_some(0.5)))]),
            ("dyn_fs", &[("font-size", num(on.then_some(16.0)))]),
            ("typed_op", &[typed(TypedStyle::new().opacity(0.6))]),
            ("typed_fs_col", &[typed(typed_fs_col)]),
        ]);
    };

    // s=0: every child inherits (24px, blue) and carries no explicit text property.
    drive(&mut scene, 0);
    for id in TEXT_CHILDREN {
        assert_eq!(scene.explicit_font_size(id), None, "{id}");
        assert_eq!(scene.explicit_color(id), None, "{id}");
        assert_eq!(scene.text_color(id), BLUE, "{id}");
    }
    assert_eq!(scene.opacity("typed_op"), 0.6);

    // s=1: explicit overrides never clobber sibling inherited properties.
    drive(&mut scene, 1);
    assert_eq!(scene.opacity("dyn_opacity"), 0.5);
    assert_eq!((scene.explicit_font_size("dyn_opacity"), scene.text_color("dyn_opacity")), (None, BLUE));
    assert_eq!((scene.explicit_font_size("dyn_fs"), scene.text_color("dyn_fs")), (Some(16.0), BLUE));
    assert_eq!((scene.explicit_font_size("typed_fs_col"), scene.explicit_color("typed_fs_col")), (Some(16.0), Some(RED)));

    // s=2: parent -> (30px, green); only non-explicit properties follow.
    drive(&mut scene, 2);
    for id in ["dyn_opacity", "dyn_fs", "typed_op"] {
        assert_eq!(scene.text_color(id), GREEN, "{id}");
    }
    assert_eq!(scene.text_color("typed_fs_col"), RED);
    assert_eq!(scene.explicit_font_size("dyn_fs"), Some(16.0));

    // s=3: removing explicit overrides re-inherits from the parent.
    drive(&mut scene, 3);
    assert_eq!(scene.opacity("dyn_opacity"), 1.0);
    assert_eq!(scene.explicit_font_size("dyn_fs"), None);
    assert_eq!(scene.opacity("typed_fs_col"), 0.6);
    for id in TEXT_CHILDREN {
        assert_eq!(scene.explicit_color(id), None, "{id}");
        assert_eq!(scene.text_color(id), GREEN, "{id}");
    }
}

#[test]
fn explicit_default_valued_text_properties_stay_explicit() {
    let mut scene = Scene::new();
    scene.add("size_parent", "text", None);
    scene.add("color_parent", "text", None);
    for name in ["d_fs", "d_typed_fs"] {
        scene.add(name, "text", Some("size_parent"));
    }
    scene.add("d_typed_col", "text", Some("color_parent"));
    scene.frame(&[
        ("size_parent", &[typed(TypedStyle::new().font_size(24.0))]),
        ("color_parent", &[typed(TypedStyle::new().color(ColorRgba::rgb(0x00, 0x00, 0xff)))]),
        ("d_fs", &[("font-size", num(Some(16.0)))]),
        ("d_typed_fs", &[typed(TypedStyle::new().font_size(16.0))]),
        ("d_typed_col", &[typed(TypedStyle::new().color(ColorRgba::BLACK))]),
    ]);
    for id in ["d_fs", "d_typed_fs"] {
        assert_eq!(scene.explicit_font_size(id), Some(16.0), "{id}");
    }
    assert_eq!(scene.explicit_color("d_typed_col"), Some(Rgba::BLACK));
    assert_eq!(scene.text_color("d_typed_col"), Rgba::BLACK);
}
