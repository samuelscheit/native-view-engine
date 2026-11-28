//! Parity guard: every `(attribute, value)` pair `to_template_patches` accepts is applied by
//! `nve_tables::css`, and every attribute's `css` flag matches `css_key_bits`.

use nve_tables::{
    CssAttrResult, STYLE_LAYER_ALL, apply_css_attr_layers_status_to_tx, css_key_bits,
};
use nve_store::{NodeId, NodeStore};

use super::super::attrs::{AttrKind, AttrSpec, PATCHABLE_ATTR_NAMES, lookup_patchable_attr};
use super::super::eval::TypedAttrValue as V;
use super::super::patch::css_value;

fn specs() -> impl Iterator<Item = &'static AttrSpec> {
    PATCHABLE_ATTR_NAMES
        .iter()
        .map(|name| lookup_patchable_attr(name).unwrap_or_else(|| panic!("{name}")))
}

/// Representative values of every class, including clamp-edge floats.
fn sample_values(spec: &AttrSpec) -> Vec<V> {
    let mut values = vec![
        V::Float(8.0),
        V::Float(-4.5),
        V::Float(0.25),
        V::Float(33.3),
        V::Float(1e30),
        V::Int(12),
        V::Int(-7),
        V::Int(i64::MAX),
        V::Bool(true),
        V::LengthPx(16.0),
        V::LengthPx(-2.0),
        V::LengthPct(50.0),
        V::Auto,
        V::Rgba {
            r: 1,
            g: 2,
            b: 3,
            a: 255,
        },
        V::Rgba {
            r: 1,
            g: 2,
            b: 3,
            a: 4,
        },
        V::ThemeToken("theme.surface.base".into()),
        V::Enum {
            ty: Some("Nope".into()),
            variant: "Nope".into(),
        },
    ];
    if let AttrKind::Keyword(kw) = spec.kind {
        for (variant, _) in kw.variants {
            values.push(V::Enum {
                ty: Some(kw.ty.into()),
                variant: (*variant).into(),
            });
            values.push(V::Enum {
                ty: None,
                variant: (*variant).into(),
            });
        }
    }
    values
}

fn apply(name: &str, css: &str) -> CssAttrResult {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let id = tx.create();
    tx.append_child(NodeId::ROOT, id);
    apply_css_attr_layers_status_to_tx(&mut tx, id, name, css, STYLE_LAYER_ALL)
}

#[test]
fn css_flag_matches_css_key_bits() {
    for spec in specs() {
        let known = spec.name == "style" || css_key_bits(spec.name).is_some();
        assert_eq!(spec.css, known, "{}", spec.name);
    }
}

#[test]
fn every_lowered_value_is_applied_by_the_css_parser() {
    let mut lowered = 0usize;
    for spec in specs() {
        for value in sample_values(spec) {
            let Ok(css) = css_value(&value, spec) else {
                continue;
            };
            lowered += 1;
            assert!(spec.css, "{} lowered {value} without a css form", spec.name);
            assert!(
                matches!(apply(spec.name, &css), CssAttrResult::Applied(_)),
                "{}: {value} -> {css:?} is not applied by nve_tables::css",
                spec.name
            );
        }
    }
    assert!(lowered > 150, "parity sweep covered only {lowered} pairs");
}

#[test]
fn lowering_refuses_values_the_string_path_would_misread() {
    let lookup = |name| lookup_patchable_attr(name).unwrap_or_else(|| panic!("{name}"));
    let refused = [
        ("padding", V::LengthPct(50.0)),
        ("padding", V::Auto),
        ("padding", V::LengthPx(-2.0)),
        ("width", V::LengthPx(1e30)),
        ("font-size", V::LengthPx(0.0)),
        ("gap", V::Int(4)),
        ("font-size", V::LengthPct(10.0)),
        ("border-radius", V::Auto),
        ("font-weight", V::Float(600.0)),
        ("font-weight", V::Int(600)),
        ("line-height", V::Float(1.5)),
        ("line-height", V::Auto),
        ("opacity", V::LengthPx(1.0)),
        ("z-index", V::LengthPx(1.0)),
        ("color", V::Float(1.0)),
        ("display", V::Bool(true)),
        (
            "white-space",
            V::Enum {
                ty: Some("WhiteSpace".into()),
                variant: "BreakSpaces".into(),
            },
        ),
        (
            "text-align",
            V::Enum {
                ty: Some("TextAlign".into()),
                variant: "Start".into(),
            },
        ),
        (
            "flex-direction",
            V::Enum {
                ty: Some("Display".into()),
                variant: "Row".into(),
            },
        ),
        ("background", V::ThemeToken("theme.surface.base".into())),
        ("padding-top", V::Float(1.0)),
        (
            "cursor",
            V::Enum {
                ty: Some("CursorKind".into()),
                variant: "Pointer".into(),
            },
        ),
    ];
    for (name, value) in refused {
        assert!(css_value(&value, lookup(name)).is_err(), "{name}: {value}");
    }
    let lowered = [
        ("padding", V::Float(-4.5), "0px"),
        ("margin", V::Float(-4.5), "-4.5px"),
        ("font-size", V::Float(0.25), "1px"),
        ("opacity", V::Float(1.5), "1"),
        ("flex-grow", V::Float(-1.0), "0"),
        ("z-index", V::Float(2.9), "2"),
        ("z-index", V::Int(i64::MAX), "2147483647"),
        ("width", V::Float(33.3), "33.3px"),
        ("width", V::Auto, "auto"),
        (
            "color",
            V::Rgba {
                r: 255,
                g: 0,
                b: 16,
                a: 255,
            },
            "#ff0010",
        ),
        ("font-weight", V::String("semibold".into()), "semibold"),
    ];
    for (name, value, css) in lowered {
        assert_eq!(
            css_value(&value, lookup(name)).as_deref(),
            Ok(css),
            "{name}: {value}"
        );
    }
}
