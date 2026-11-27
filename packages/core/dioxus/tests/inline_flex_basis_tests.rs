//! `flex` / `flex-basis` precedence on `NveMutationWriter`: inline `style` (shorthand or
//! longhand) beats the standalone `flex` / `flex-basis` attributes, which beat an `nve_style`
//! typed basis, which beats the defaults. Changing or removing a declaration resets to the next
//! layer, and the result never depends on attribute order.

#[path = "support/style_scene.rs"]
mod style_scene;

use nve_dioxus::TypedStyle;
use nve_store::Length;
use style_scene::{Scene, txt, typed};

const AUTO: Length = Length::AUTO;

fn px(v: f32) -> Length {
    Length::points(v)
}

fn pct(v: f32) -> Length {
    Length::percent(v)
}

/// Inline styles cycled per step: shorthand, `none`, longhand, removal.
const INLINE: [Option<&str>; 5] = [
    Some("flex: 1 1 280px; padding: 4px"),
    Some("flex: none"),
    Some("flex-basis: 12%; flex-grow: 2"),
    Some("padding: 4px"),
    None,
];

fn typed_basis() -> TypedStyle {
    TypedStyle::new().flex_basis_pct(50.0).aspect_ratio(2.0)
}

#[test]
fn inline_flex_basis_resets_on_change_and_removal() {
    let mut scene = Scene::new();
    for name in ["plain", "typed", "attr"] {
        scene.add(name, "view", None);
    }

    // (plain basis, typed basis, attr basis) after each step.
    let expected = [
        (px(280.0), px(280.0), px(0.0)),
        (AUTO, AUTO, AUTO),
        (pct(12.0), pct(12.0), px(0.0)),
        (AUTO, pct(50.0), AUTO),
        (AUTO, pct(50.0), px(0.0)),
        (px(280.0), px(280.0), AUTO),
    ];
    for (step, (plain, typed_basis_exp, attr)) in expected.into_iter().enumerate() {
        let inline = INLINE[step % INLINE.len()];
        let flex = if step % 2 == 0 { "1 0 0" } else { "none" };
        let typed_attr = typed(typed_basis());
        scene.frame(&[
            ("plain", &[("style", txt(inline))]),
            ("typed", &[typed_attr, ("style", txt(inline))]),
            ("attr", &[("flex", txt(Some(flex)))]),
        ]);
        assert_eq!(scene.basis("plain"), plain, "plain @ step {step}");
        assert_eq!(scene.basis("typed"), typed_basis_exp, "typed @ step {step}");
        assert_eq!(scene.basis("attr"), attr, "attr @ step {step}");
    }
    assert_eq!(scene.flex_factors("plain"), (1.0, 1.0));
}

#[test]
fn mixed_flex_attr_and_style_precedence_is_order_independent() {
    let mut scene = Scene::new();
    for name in [
        "attr_first",
        "style_first",
        "all_three",
        "flex_before_typed",
    ] {
        scene.add(name, "view", None);
    }
    let typed_full = || {
        TypedStyle::new()
            .flex_grow(3.0)
            .flex_shrink(2.0)
            .flex_basis_pct(50.0)
            .aspect_ratio(2.0)
    };

    // Precedence: inline style > standalone flex attr > nve_style > defaults (0, 1, auto).
    // ((grow, shrink, basis) of plain nodes, (..) of typed nodes) per step.
    let expected = [
        ((1.0, 0.0, px(200.0)), (1.0, 0.0, px(200.0))),
        ((1.0, 0.0, pct(33.0)), (1.0, 0.0, pct(33.0))),
        ((1.0, 0.0, px(200.0)), (1.0, 0.0, px(200.0))),
        ((0.0, 1.0, pct(33.0)), (3.0, 2.0, pct(33.0))),
        ((0.0, 1.0, AUTO), (3.0, 2.0, pct(50.0))),
    ];
    for (step, ((pg, ps, pb), (tg, ts, tb))) in expected.into_iter().enumerate() {
        let flex = (step <= 2).then_some("1 0 200px");
        let inline = match step {
            0 => Some("padding: 4px"),
            1 | 3 => Some("flex-basis: 33%"),
            2 => None,
            _ => Some("padding: 8px"),
        };
        scene.frame(&[
            // Opposite attribute orders must produce identical basis + grow/shrink.
            ("attr_first", &[("flex", txt(flex)), ("style", txt(inline))]),
            (
                "style_first",
                &[("style", txt(inline)), ("flex", txt(flex))],
            ),
            (
                "all_three",
                &[
                    typed(typed_full()),
                    ("flex", txt(flex)),
                    ("style", txt(inline)),
                ],
            ),
            (
                "flex_before_typed",
                &[
                    ("flex", txt(flex)),
                    typed(typed_full()),
                    ("style", txt(inline)),
                ],
            ),
        ]);
        for name in ["attr_first", "style_first"] {
            assert_eq!(scene.basis(name), pb, "{name} basis @ step {step}");
            assert_eq!(
                scene.flex_factors(name),
                (pg, ps),
                "{name} factors @ step {step}"
            );
        }
        for name in ["all_three", "flex_before_typed"] {
            assert_eq!(scene.basis(name), tb, "{name} basis @ step {step}");
            assert_eq!(
                scene.flex_factors(name),
                (tg, ts),
                "{name} factors @ step {step}"
            );
        }
    }
}

#[test]
fn flex_shorthand_forms_set_grow_shrink_and_basis_and_invalid_basis_falls_back() {
    let mut scene = Scene::new();
    scene.add("n", "view", None);
    let cases: [(&str, (f32, f32), Length); 4] = [
        ("2", (2.0, 1.0), pct(0.0)),
        ("none", (0.0, 0.0), AUTO),
        ("auto", (1.0, 1.0), AUTO),
        ("1 0 120px", (1.0, 0.0), px(120.0)),
    ];
    for (value, factors, basis) in cases {
        scene.set("n", &[("flex", txt(Some(value)))]);
        assert_eq!(scene.flex_factors("n"), factors, "flex: {value}");
        assert_eq!(scene.basis("n"), basis, "flex: {value}");
    }
    // Strict ingress: a negative / out-of-range standalone basis is invalid, so it contributes
    // nothing and the basis falls back to the lower layer (the `1 0 120px` shorthand above).
    scene.set("n", &[("flex-basis", txt(Some("20px")))]);
    assert_eq!(scene.basis("n"), px(20.0), "longhand beats shorthand");
    scene.set("n", &[("flex-basis", txt(Some("-5px")))]);
    assert_eq!(
        scene.basis("n"),
        px(120.0),
        "negative basis falls back to the shorthand"
    );
    scene.set("n", &[("flex-basis", txt(Some("20px")))]);
    scene.set("n", &[("flex-basis", txt(Some("99999999px")))]);
    assert_eq!(
        scene.basis("n"),
        px(120.0),
        "basis above MAX_STYLE_SCALAR falls back"
    );
    assert_eq!(scene.flex_factors("n"), (1.0, 0.0), "grow/shrink untouched");
}
