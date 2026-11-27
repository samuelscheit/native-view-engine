//! Style precedence on `NveMutationWriter`, written straight into the `NodeStore` tables:
//! shorthand vs longhand order-independence, `currentColor` borders across attributes, dynamic
//! updates and ancestor inheritance, same-frame layer removal, invalid-attribute fallback, and
//! per-property text inheritance (explicit vs inherited `Typography` fields).

#[path = "support/style_scene.rs"]
mod style_scene;

use nve_dioxus::TypedStyle;
use nve_store::{Length, Rgba};
use style_scene::{Scene, txt, typed};

const RED: Rgba = Rgba::rgb(239, 68, 68); // CSS named `red`
const BLUE: Rgba = Rgba::rgb(0x00, 0x00, 0xff);
const GREEN: Rgba = Rgba::rgb(0x22, 0xc5, 0x5e);
const LIGHT_BLUE: Rgba = Rgba::rgb(96, 165, 250);
const ACCENT: Rgba = Rgba::rgb(59, 130, 246);
const AMBER: Rgba = Rgba::rgb(245, 158, 11);
const WHITE: Rgba = Rgba::rgb(255, 255, 255);

fn px(v: f32) -> Length {
    Length::points(v)
}

fn padding(top: f32, right: f32, bottom: f32, left: f32) -> [Length; 4] {
    [px(top), px(right), px(bottom), px(left)]
}

#[test]
fn shorthand_vs_longhand_precedence_survives_attribute_order_and_toggles() {
    let mut scene = Scene::new();
    for name in ["flex_split", "flex_split_rev", "pad_toggle", "border_split"] {
        scene.add(name, "view", None);
    }
    // Longhands win over shorthands whichever is set first (Dioxus applies static template
    // attributes before dynamic ones, so both orders occur in practice).
    for step in 0..3usize {
        let flex = if step % 2 == 0 {
            "1 0 200px"
        } else {
            "2 0 150px"
        };
        let pad = (step != 1).then_some("16px");
        let width = (step != 1).then_some("5px");
        scene.frame(&[
            (
                "flex_split",
                &[("flex-grow", txt(Some("3"))), ("flex", txt(Some(flex)))],
            ),
            (
                "flex_split_rev",
                &[("flex", txt(Some(flex))), ("flex-grow", txt(Some("3")))],
            ),
            (
                "pad_toggle",
                &[
                    ("padding", txt(pad)),
                    ("padding-horizontal", txt(Some("4px"))),
                ],
            ),
            (
                "border_split",
                &[
                    ("border", txt(Some("2px solid #ef4444"))),
                    ("border-width", txt(width)),
                ],
            ),
        ]);
        let basis = px(if step % 2 == 0 { 200.0 } else { 150.0 });
        for name in ["flex_split", "flex_split_rev"] {
            assert_eq!(scene.flex_factors(name), (3.0, 0.0), "{name} @ {step}");
            assert_eq!(scene.basis(name), basis, "{name} basis @ {step}");
        }
        let expected_pad = if step == 1 {
            padding(0.0, 4.0, 0.0, 4.0)
        } else {
            padding(16.0, 4.0, 16.0, 4.0)
        };
        assert_eq!(
            scene.padding("pad_toggle"),
            expected_pad,
            "padding @ {step}"
        );
        assert_eq!(
            scene.border_width("border_split"),
            if step == 1 { 2.0 } else { 5.0 }
        );
        assert_eq!(scene.border_color("border_split"), RED);
    }
}

#[test]
fn current_color_resolves_across_attributes_dynamic_updates_and_ancestor_inheritance() {
    let mut scene = Scene::new();
    scene.add("parent", "view", None);
    for name in [
        "cross_a",
        "cross_b",
        "dyn_color",
        "wrapper",
        "shielded",
        "explicit_border",
    ] {
        scene.add(name, "view", Some("parent"));
    }
    scene.add("inherited", "view", Some("wrapper"));

    let parent_colors = [
        Some("color: #ffffff"),
        Some("color: #22c55e"),
        None,
        Some("color: #3b82f6"),
    ];
    let dynamic = [
        "color: #ef4444",
        "color: #60a5fa",
        "color: #ef4444",
        "color: #60a5fa",
    ];
    for (step, (parent, dyn_style)) in parent_colors.into_iter().zip(dynamic).enumerate() {
        scene.frame(&[
            ("parent", &[("style", txt(parent))]),
            // Cross-attribute: inline `border` + standalone `color`, in both orders.
            (
                "cross_a",
                &[
                    ("style", txt(Some("border: 1px solid"))),
                    ("color", txt(Some("red"))),
                ],
            ),
            (
                "cross_b",
                &[
                    ("color", txt(Some("red"))),
                    ("style", txt(Some("border: 1px solid"))),
                ],
            ),
            // Dynamic inline `color` next to a standalone `border`.
            (
                "dyn_color",
                &[
                    ("border", txt(Some("2px solid"))),
                    ("style", txt(Some(dyn_style))),
                ],
            ),
            ("inherited", &[("style", txt(Some("border: 1px solid")))]),
            (
                "shielded",
                &[("style", txt(Some("color: #000000; border: 1px solid")))],
            ),
            (
                "explicit_border",
                &[("style", txt(Some("border: 1px solid #f59e0b")))],
            ),
        ]);
        let ancestor = match step {
            0 => WHITE,
            1 => GREEN,
            2 => Rgba::BLACK,
            _ => ACCENT,
        };
        assert_eq!(scene.border_color("cross_a"), RED, "cross_a @ {step}");
        assert_eq!(scene.border_color("cross_b"), RED, "cross_b @ {step}");
        let own = if step % 2 == 0 { RED } else { LIGHT_BLUE };
        assert_eq!(scene.border_color("dyn_color"), own, "dyn_color @ {step}");
        assert_eq!(
            scene.border_color("inherited"),
            ancestor,
            "inherited @ {step}"
        );
        assert_eq!(
            scene.border_color("shielded"),
            Rgba::BLACK,
            "shielded @ {step}"
        );
        assert_eq!(
            scene.border_color("explicit_border"),
            AMBER,
            "explicit_border @ {step}"
        );
    }
}

#[test]
fn unrelated_parent_animation_does_not_rewrite_current_color_child() {
    let mut scene = Scene::new();
    scene.add("anim_parent", "view", None);
    scene.add("anim_child", "view", Some("anim_parent"));
    let child = |extra: &'static str| -> [(&'static str, dioxus_core::AttributeValue); 1] {
        [("style", txt(Some(extra)))]
    };
    scene.frame(&[
        (
            "anim_parent",
            &[("style", txt(Some("color: #ffffff; opacity: 1")))],
        ),
        ("anim_child", &child("border: 1px solid")),
    ]);
    assert_eq!(scene.border_color("anim_child"), WHITE);

    scene.frame(&[
        (
            "anim_parent",
            &[("style", txt(Some("color: #ffffff; opacity: 0.5")))],
        ),
        ("anim_child", &child("border: 1px solid; padding: 12px")),
    ]);
    assert_eq!(scene.opacity("anim_parent"), 0.5);
    assert_eq!(
        scene.border_color("anim_child"),
        WHITE,
        "no revert to black"
    );

    // Only the parent's opacity changes: the child's tables are untouched.
    scene.frame(&[(
        "anim_parent",
        &[("style", txt(Some("color: #ffffff; opacity: 1")))],
    )]);
    assert!(
        !scene.touched("anim_child"),
        "opacity animation must not rewrite the child"
    );
    assert_eq!(scene.border_color("anim_child"), WHITE);
}

#[test]
fn same_frame_layer_removal_and_invalid_attr_fallback() {
    let mut scene = Scene::new();
    scene.set("root", &[("style", txt(Some("color: #ffffff")))]);
    for name in [
        "ov_to_w",
        "style_to_wb",
        "direct_invalid",
        "layered_invalid",
    ] {
        scene.add(name, "view", Some("root"));
    }
    let typed_pad = || typed(TypedStyle::new().padding(14.0));

    scene.frame(&[
        ("ov_to_w", &[("overflow", txt(Some("hidden")))]),
        (
            "style_to_wb",
            &[("style", txt(Some("background-color: #ef4444")))],
        ),
        ("direct_invalid", &[("padding", txt(Some("8px")))]),
        (
            "layered_invalid",
            &[typed_pad(), ("padding", txt(Some("8px")))],
        ),
    ]);
    assert_eq!(scene.padding("direct_invalid"), padding(8.0, 8.0, 8.0, 8.0));
    assert_eq!(
        scene.padding("layered_invalid"),
        padding(8.0, 8.0, 8.0, 8.0)
    );

    // Same frame: the last layer of each node is removed while a typed attr is inserted; an
    // invalid value clears the stale one (direct -> default, layered -> typed style).
    scene.frame(&[
        (
            "ov_to_w",
            &[("overflow", txt(None)), ("width", txt(Some("240px")))],
        ),
        (
            "style_to_wb",
            &[
                ("style", txt(None)),
                ("width", txt(Some("240px"))),
                ("border", txt(Some("2px solid"))),
            ],
        ),
        ("direct_invalid", &[("padding", txt(Some("garbage")))]),
        (
            "layered_invalid",
            &[typed_pad(), ("padding", txt(Some("garbage")))],
        ),
    ]);
    assert_eq!(scene.layout("ov_to_w", |l| l.width()), Some(px(240.0)));
    assert_eq!(scene.layout("style_to_wb", |l| l.width()), Some(px(240.0)));
    assert_eq!(scene.border_width("style_to_wb"), 2.0);
    assert_eq!(scene.border_color("style_to_wb"), WHITE);
    assert_eq!(
        scene
            .writer
            .store()
            .try_row::<nve_components::tables::Visual>(scene.node("style_to_wb"))
            .map(|v| v.background()),
        Some(Rgba::TRANSPARENT),
        "removed inline background must clear"
    );
    assert_eq!(scene.padding("direct_invalid"), padding(0.0, 0.0, 0.0, 0.0));
    assert_eq!(
        scene.padding("layered_invalid"),
        padding(14.0, 14.0, 14.0, 14.0)
    );
}

const TEXT_CHILDREN: [&str; 7] = [
    "dyn_opacity",
    "dyn_fs",
    "dyn_inline_fs",
    "dyn_inline_col",
    "dyn_color",
    "typed_op",
    "typed_fs_col",
];

#[test]
fn text_properties_inherit_per_property_and_reinherit_when_removed_or_invalid() {
    let mut scene = Scene::new();
    scene.add("parent", "text", None);
    for name in TEXT_CHILDREN {
        scene.add(name, "text", Some("parent"));
    }
    let red_typed = || {
        TypedStyle::new()
            .font_size(16.0)
            .color(nve_abi::ColorRgba::rgb(0xef, 0x44, 0x44))
    };

    let drive = |scene: &mut Scene, s: usize| {
        let (fs, col) = if s < 2 {
            ("24px", "#0000ff")
        } else {
            ("30px", "#22c55e")
        };
        let some_if = |on: bool, v: &'static str| on.then_some(v);
        let on = s == 1 || s == 2;
        let typed_fs_col = if on {
            red_typed()
        } else {
            TypedStyle::new().opacity(0.6)
        };
        scene.frame(&[
            (
                "parent",
                &[("font-size", txt(Some(fs))), ("color", txt(Some(col)))],
            ),
            (
                "dyn_opacity",
                &[(
                    "opacity",
                    txt(match s {
                        0 => None,
                        1 | 2 => Some("0.5"),
                        _ => Some("garbage"),
                    }),
                )],
            ),
            (
                "dyn_fs",
                &[(
                    "font-size",
                    txt(match s {
                        0 => None,
                        1 | 2 => Some("16px"),
                        _ => Some("bad"),
                    }),
                )],
            ),
            (
                "dyn_inline_fs",
                &[(
                    "style",
                    txt(match s {
                        0 => None,
                        1 | 2 => Some("font-size: 16px; opacity: 0.8"),
                        _ => Some("opacity: 0.8"),
                    }),
                )],
            ),
            (
                "dyn_inline_col",
                &[(
                    "style",
                    txt(match s {
                        0 => None,
                        1 | 2 => Some("color: #ef4444; opacity: 0.8"),
                        _ => Some("opacity: 0.8"),
                    }),
                )],
            ),
            ("dyn_color", &[("color", txt(some_if(on, "#ef4444")))]),
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
    assert_eq!(
        (
            scene.explicit_font_size("dyn_opacity"),
            scene.text_color("dyn_opacity")
        ),
        (None, BLUE)
    );
    assert_eq!(
        (
            scene.explicit_font_size("dyn_fs"),
            scene.text_color("dyn_fs")
        ),
        (Some(16.0), BLUE)
    );
    assert_eq!(scene.explicit_font_size("dyn_inline_fs"), Some(16.0));
    assert_eq!(scene.opacity("dyn_inline_fs"), 0.8);
    assert_eq!(scene.text_color("dyn_inline_fs"), BLUE);
    assert_eq!(
        (
            scene.explicit_color("dyn_inline_col"),
            scene.explicit_font_size("dyn_inline_col")
        ),
        (Some(RED), None)
    );
    assert_eq!(scene.opacity("dyn_inline_col"), 0.8);
    assert_eq!(scene.explicit_color("dyn_color"), Some(RED));
    assert_eq!(scene.explicit_font_size("dyn_color"), None);
    assert_eq!(
        (
            scene.explicit_font_size("typed_fs_col"),
            scene.explicit_color("typed_fs_col")
        ),
        (Some(16.0), Some(RED))
    );

    // s=2: parent -> (30px, green); only non-explicit properties follow.
    drive(&mut scene, 2);
    for id in ["dyn_opacity", "dyn_fs", "dyn_inline_fs", "typed_op"] {
        assert_eq!(scene.text_color(id), GREEN, "{id}");
    }
    for id in ["dyn_inline_col", "dyn_color", "typed_fs_col"] {
        assert_eq!(scene.text_color(id), RED, "{id}");
    }
    assert_eq!(scene.explicit_font_size("dyn_fs"), Some(16.0));

    // s=3: removing / invalidating explicit overrides re-inherits from the parent.
    drive(&mut scene, 3);
    assert_eq!(
        scene.opacity("dyn_opacity"),
        1.0,
        "invalid opacity falls back to default"
    );
    for id in ["dyn_fs", "dyn_inline_fs"] {
        assert_eq!(scene.explicit_font_size(id), None, "{id}");
    }
    assert_eq!(scene.opacity("dyn_inline_fs"), 0.8);
    assert_eq!(scene.opacity("dyn_inline_col"), 0.8);
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
    for name in ["d_fs", "d_inline_fs", "d_typed_fs"] {
        scene.add(name, "text", Some("size_parent"));
    }
    for name in ["d_col", "d_inline_col", "d_typed_col"] {
        scene.add(name, "text", Some("color_parent"));
    }
    scene.frame(&[
        ("size_parent", &[("font-size", txt(Some("24px")))]),
        ("color_parent", &[("color", txt(Some("#0000ff")))]),
        ("d_fs", &[("font-size", txt(Some("16px")))]),
        ("d_inline_fs", &[("style", txt(Some("font-size: 16px")))]),
        ("d_typed_fs", &[typed(TypedStyle::new().font_size(16.0))]),
        ("d_col", &[("color", txt(Some("#000000")))]),
        ("d_inline_col", &[("style", txt(Some("color: #000000")))]),
        (
            "d_typed_col",
            &[typed(TypedStyle::new().color(nve_abi::ColorRgba::BLACK))],
        ),
    ]);
    // A value equal to the default (16px / black) is still explicit: it must shield the node
    // from the parent's 24px / blue.
    for id in ["d_fs", "d_inline_fs", "d_typed_fs"] {
        assert_eq!(scene.explicit_font_size(id), Some(16.0), "{id}");
    }
    for id in ["d_col", "d_inline_col", "d_typed_col"] {
        assert_eq!(scene.explicit_color(id), Some(Rgba::BLACK), "{id}");
        assert_eq!(scene.text_color(id), Rgba::BLACK, "{id}");
    }
}

#[test]
fn current_color_border_follows_reparenting_and_survives_subtree_removal() {
    let mut scene = Scene::new();
    scene.add("red_parent", "view", None);
    scene.add("blue_parent", "view", None);
    scene.add("child", "view", Some("red_parent"));
    scene.set("red_parent", &[("color", txt(Some("#ef4444")))]);
    scene.set("blue_parent", &[("color", txt(Some("#3b82f6")))]);
    scene.set("child", &[("border", txt(Some("1px solid")))]);
    assert_eq!(scene.border_color("child"), RED);

    // Reparenting alone, with no style attribute touched, re-resolves `currentColor`.
    scene.reparent("child", "blue_parent");
    assert_eq!(scene.border_color("child"), ACCENT);
    scene.reparent("child", "red_parent");
    assert_eq!(scene.border_color("child"), RED);

    // Removing the subtree must not leave a dangling tracked node: later frames keep working.
    scene.reparent("child", "blue_parent");
    scene.remove("blue_parent");
    assert!(!scene.is_live("child"));
    scene.set("red_parent", &[("color", txt(Some("#22c55e")))]);
    assert_eq!(scene.text_color("red_parent"), GREEN);
}
