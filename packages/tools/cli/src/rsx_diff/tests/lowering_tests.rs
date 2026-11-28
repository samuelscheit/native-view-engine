//! Parity guard: every `(attribute, value)` pair `encode_attr_into_row_patch` accepts serializes
//! to valid binary `RowPatch` words (`RowPatchView::from_words`) and applies cleanly via
//! `Transaction::apply_row_patch` onto a live `NodeStore` node, and rejects out-of-domain scalars
//! (`font-size < 1.0`, `line-height` ratio overflowing `MAX_STYLE_SCALAR` after `* 100.0`).

use nve_store::{ChangeSet, FlatResource, Length, NodeId, NodeStore, Rgba, RowPatch, RowPatchView};
use nve_tables::tables::{
    self, AlignItems, AlignSelf, CursorKind, JustifyContent, Layout, Position, TextAlign,
    TextDecoration, Typography, Visual, WhiteSpace,
};

use super::super::attrs::{
    AttrKind, AttrSpec, STANDALONE_ATTR_NAMES, lookup_standalone_attr, lookup_typed_style_arg,
};
use super::super::eval::TypedAttrValue as V;
use super::super::patch::encode_attr_into_row_patch;

const TYPED_STYLE_METHODS: &[(&str, usize)] = &[
    ("width", 0),
    ("width_px", 0),
    ("width_pct", 0),
    ("height", 0),
    ("height_pct", 0),
    ("min_width", 0),
    ("max_height", 0),
    ("flex_basis", 0),
    ("flex_basis_pct", 0),
    ("inset_top", 0),
    ("inset_top_px", 0),
    ("display", 0),
    ("position", 0),
    ("flex_direction", 0),
    ("flex_wrap", 0),
    ("justify", 0),
    ("align", 0),
    ("align_self", 0),
    ("flex", 0),
    ("flex_grow", 0),
    ("flex_shrink", 0),
    ("aspect_ratio", 0),
    ("gap", 0),
    ("row_gap", 0),
    ("column_gap", 0),
    ("padding", 0),
    ("padding_xy", 0),
    ("padding_xy", 1),
    ("padding_top", 0),
    ("margin", 0),
    ("margin_xy", 0),
    ("margin_xy", 1),
    ("bg", 0),
    ("border", 0),
    ("border", 1),
    ("radius", 0),
    ("opacity", 0),
    ("z_index", 0),
    ("overflow", 0),
    ("overflow_x", 0),
    ("overflow_y", 0),
    ("color", 0),
    ("font_size", 0),
    ("font_weight", 0),
    ("font_style", 0),
    ("line_height", 0),
    ("line_height_px", 0),
    ("line_height_ratio", 0),
    ("text_align", 0),
    ("white_space", 0),
    ("overflow_wrap", 0),
    ("text_decoration", 0),
    ("cursor", 0),
];

fn all_specs() -> Vec<&'static AttrSpec> {
    let mut out: Vec<&'static AttrSpec> = STANDALONE_ATTR_NAMES
        .iter()
        .map(|name| lookup_standalone_attr(name).unwrap_or_else(|| panic!("{name}")))
        .collect();
    for &(method, idx) in TYPED_STYLE_METHODS {
        out.push(lookup_typed_style_arg(method, idx).unwrap_or_else(|| panic!("{method}[{idx}]")));
    }
    out
}

/// Representative values of every class, including clamp-edge floats and keyword variants.
fn sample_values(spec: &AttrSpec) -> Vec<V> {
    let mut values = vec![
        V::Float(8.0),
        V::Float(-4.5),
        V::Float(0.25),
        V::Float(33.3),
        V::Float(1e30),
        V::Int(12),
        V::Int(-7),
        V::Int(600),
        V::Int(i64::MAX),
        V::Bool(true),
        V::LengthPx(16.0),
        V::LengthPx(-2.0),
        V::LengthPct(50.0),
        V::Auto,
        V::Rgba { r: 1, g: 2, b: 3, a: 255 },
        V::Rgba { r: 1, g: 2, b: 3, a: 4 },
        V::ThemeToken("theme.surface.base".into()),
        V::Enum {
            ty: Some("Nope".into()),
            variant: "Nope".into(),
        },
        V::Enum {
            ty: Some("TextDecoration".into()),
            variant: "UNDERLINE".into(),
        },
    ];
    if let AttrKind::Keyword(kw) = spec.kind {
        for &(variant, _) in kw.variants {
            values.push(V::Enum {
                ty: Some(kw.ty.into()),
                variant: variant.into(),
            });
            if let Some(alt) = kw.alt_ty {
                values.push(V::Enum {
                    ty: Some(alt.into()),
                    variant: variant.into(),
                });
            }
            values.push(V::Enum {
                ty: None,
                variant: variant.into(),
            });
        }
    }
    values
}

#[test]
fn every_encoded_row_patch_validates_and_applies_to_nodestore() {
    tables::register_core_tables();
    let mut lowered = 0usize;
    for spec in all_specs() {
        for value in sample_values(spec) {
            let mut rp = RowPatch::new();
            if encode_attr_into_row_patch(&mut rp, spec, &value).is_err() {
                continue;
            }
            lowered += 1;
            let mut words = Vec::new();
            assert!(
                rp.write_words(&mut words),
                "{}: {value} failed write_words",
                spec.name
            );
            let view = RowPatchView::from_words(&words)
                .unwrap_or_else(|| panic!("{}: {value} invalid RowPatchView", spec.name));
            assert!(!view.is_empty());

            let mut store = NodeStore::new();
            let mut tx = store.tx();
            let id = tx.create();
            tx.append_child(NodeId::ROOT, id);
            tx.apply_row_patch(id, view);
            let mut changes = ChangeSet::default();
            tx.commit_into(&mut changes);
            assert!(
                !changes.created.is_empty(),
                "{}: {value} must commit cleanly",
                spec.name
            );
        }
    }
    assert!(lowered > 220, "parity sweep covered only {lowered} pairs");
}

#[test]
fn binary_lowering_checks_domains_including_font_size_floor_and_line_height_overflow() {
    let ts_arg = |m, i| lookup_typed_style_arg(m, i).unwrap_or_else(|| panic!("{m}[{i}]"));
    let standalone = |n| lookup_standalone_attr(n).unwrap_or_else(|| panic!("{n}"));

    let refused: Vec<(&AttrSpec, V)> = vec![
        (standalone("padding"), V::LengthPct(50.0)),
        (standalone("padding"), V::Auto),
        (standalone("padding"), V::LengthPx(-2.0)),
        (standalone("gap"), V::Int(4)),
        (ts_arg("width", 0), V::LengthPx(1e30)),
        (ts_arg("width", 0), V::LengthPx(-8.0)),
        (
            ts_arg("position", 0),
            V::Enum {
                ty: Some("PositionMode".into()),
                variant: "Fixed".into(),
            },
        ),
        (
            ts_arg("position", 0),
            V::Enum {
                ty: Some("Position".into()),
                variant: "Relative".into(),
            },
        ),
        // `font-size` min is 1.0: `LengthPx(0.0)` and `LengthPx(0.5)` must BOTH be refused!
        (ts_arg("font_size", 0), V::LengthPx(0.0)),
        (ts_arg("font_size", 0), V::LengthPx(0.5)),
        (ts_arg("font_size", 0), V::LengthPct(10.0)),
        // `line-height` ratio * 100.0 exceeding MAX_STYLE_SCALAR must be refused!
        (ts_arg("line_height_ratio", 0), V::Float(1e6)),
        (ts_arg("line_height_ratio", 0), V::Float(0.0)),
        (ts_arg("line_height_ratio", 0), V::Float(-1.5)),
        (ts_arg("radius", 0), V::Auto),
        (ts_arg("opacity", 0), V::LengthPx(1.0)),
        (ts_arg("z_index", 0), V::LengthPx(1.0)),
        (ts_arg("color", 0), V::Float(1.0)),
        (ts_arg("display", 0), V::Bool(true)),
        (
            ts_arg("flex_direction", 0),
            V::Enum {
                ty: Some("Display".into()),
                variant: "Row".into(),
            },
        ),
        (ts_arg("bg", 0), V::ThemeToken("theme.surface.base".into())),
    ];
    for (spec, value) in refused {
        let mut rp = RowPatch::new();
        assert!(
            encode_attr_into_row_patch(&mut rp, spec, &value).is_err(),
            "{}: {value} should be refused",
            spec.name
        );
    }

    let mut rp = RowPatch::new();
    encode_attr_into_row_patch(&mut rp, ts_arg("inset_top_px", 0), &V::Float(-6.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("inset_top", 0), &V::LengthPx(-8.0)).unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("width_pct", 0), &V::Float(75.0)).unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("position", 0), &V::Enum {
        ty: Some("PositionMode".into()),
        variant: "Absolute".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("justify", 0), &V::Enum {
        ty: Some("FlexJustify".into()),
        variant: "SpaceBetween".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("align", 0), &V::Enum {
        ty: Some("FlexAlign".into()),
        variant: "Center".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("align_self", 0), &V::Enum {
        ty: Some("AlignSelf".into()),
        variant: "Center".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("aspect_ratio", 0), &V::Float(1.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("cursor", 0), &V::Enum {
        ty: Some("CursorKind".into()),
        variant: "Pointer".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("white_space", 0), &V::Enum {
        ty: Some("WhiteSpace".into()),
        variant: "BreakSpaces".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("text_align", 0), &V::Enum {
        ty: Some("TextAlign".into()),
        variant: "Start".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("text_decoration", 0), &V::Enum {
        ty: Some("TextDecoration".into()),
        variant: "UNDERLINE".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("font_weight", 0), &V::Int(600)).unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("line_height_ratio", 0), &V::Float(1.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, ts_arg("color", 0), &V::Rgba {
        r: 255,
        g: 0,
        b: 16,
        a: 255,
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, standalone("padding"), &V::Float(-4.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, standalone("opacity"), &V::Float(1.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, standalone("z-index"), &V::Int(i64::MAX)).unwrap();

    assert_eq!(rp.get_prop(Layout::WIDTH), Some(Length::percent(75.0)));
    assert_eq!(rp.get_prop(Layout::INSET_TOP), Some(Length::points(-8.0)));
    assert_eq!(rp.get_prop(Layout::POSITION), Some(Position::Absolute));
    assert_eq!(rp.get_prop(Layout::JUSTIFY_CONTENT), Some(JustifyContent::SpaceBetween));
    assert_eq!(rp.get_prop(Layout::ALIGN_ITEMS), Some(AlignItems::Center));
    assert_eq!(rp.get_prop(Layout::ALIGN_SELF), Some(AlignSelf::Center));
    assert_eq!(rp.get_prop(Layout::ASPECT_RATIO), Some(Some(1.5)));
    assert_eq!(rp.get_prop(tables::Interaction::CURSOR), Some(CursorKind::Pointer));
    assert_eq!(rp.get_prop(Typography::WHITE_SPACE), Some(WhiteSpace::BreakSpaces));
    assert_eq!(rp.get_prop(Typography::TEXT_ALIGN), Some(TextAlign::Start));
    assert_eq!(rp.get_prop(Typography::TEXT_DECORATION), Some(TextDecoration::UNDERLINE));
    assert_eq!(rp.get_prop(Typography::FONT_WEIGHT), Some(600));
    assert_eq!(rp.get_prop(Typography::LINE_HEIGHT), Some(Length::percent(150.0)));
    assert_eq!(rp.get_prop(Typography::COLOR), Some(Rgba::rgb(255, 0, 16)));
    assert_eq!(rp.get_prop(Layout::PADDING_TOP), Some(Length::points(0.0)));
    assert_eq!(rp.get_prop(Visual::OPACITY), Some(1.0));
    assert_eq!(rp.get_prop(Visual::Z_INDEX), Some(i32::MAX));
}
