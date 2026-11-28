//! Parity guard: every `(attribute, value)` pair `encode_attr_into_row_patch` accepts serializes
//! to valid binary `RowPatch` words (`RowPatchView::from_words`) and applies cleanly via
//! `Transaction::apply_row_patch` onto a live `NodeStore` node.

use nve_store::{ChangeSet, FlatResource, Length, NodeId, NodeStore, Rgba, RowPatch, RowPatchView};
use nve_tables::tables::{
    self, AlignSelf, BoxSizing, CursorKind, Layout, Position, TextAlign, Typography,
    VerticalAlign, Visual, WhiteSpace,
};

use super::super::attrs::{AttrKind, AttrSpec, PATCHABLE_ATTR_NAMES, lookup_patchable_attr};
use super::super::eval::TypedAttrValue as V;
use super::super::patch::encode_attr_into_row_patch;

fn specs() -> impl Iterator<Item = &'static AttrSpec> {
    PATCHABLE_ATTR_NAMES
        .iter()
        .map(|name| lookup_patchable_attr(name).unwrap_or_else(|| panic!("{name}")))
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
        for &(variant, _) in kw.variants {
            values.push(V::Enum {
                ty: Some(kw.ty.into()),
                variant: variant.into(),
            });
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
    for spec in specs() {
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
fn binary_lowering_handles_typed_only_attributes_and_refuses_invalid_domains() {
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
        ("opacity", V::LengthPx(1.0)),
        ("z-index", V::LengthPx(1.0)),
        ("color", V::Float(1.0)),
        ("display", V::Bool(true)),
        (
            "flex-direction",
            V::Enum {
                ty: Some("Display".into()),
                variant: "Row".into(),
            },
        ),
        ("background", V::ThemeToken("theme.surface.base".into())),
        ("padding-top", V::Float(1.0)),
        ("font-family", V::String("Inter".into())),
        ("box-shadow", V::String("0 2px 4px #000".into())),
        ("border", V::String("1px solid #fff".into())),
    ];
    for (name, value) in refused {
        let mut rp = RowPatch::new();
        assert!(
            encode_attr_into_row_patch(&mut rp, lookup(name), &value).is_err(),
            "{name}: {value} should be refused"
        );
    }

    // Previously `css: false` typed attributes now encode directly into binary `RowPatch` words!
    let mut rp = RowPatch::new();
    encode_attr_into_row_patch(&mut rp, lookup("top"), &V::Float(-6.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("position"), &V::Enum {
        ty: Some("Position".into()),
        variant: "Absolute".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("box-sizing"), &V::Enum {
        ty: Some("BoxSizing".into()),
        variant: "ContentBox".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("align-self"), &V::Enum {
        ty: Some("AlignSelf".into()),
        variant: "Center".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("aspect-ratio"), &V::Float(1.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("cursor"), &V::Enum {
        ty: Some("CursorKind".into()),
        variant: "Pointer".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("white-space"), &V::Enum {
        ty: Some("WhiteSpace".into()),
        variant: "BreakSpaces".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("text-align"), &V::Enum {
        ty: Some("TextAlign".into()),
        variant: "Start".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("vertical-align"), &V::Enum {
        ty: Some("VerticalAlign".into()),
        variant: "Middle".into(),
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("font-weight"), &V::Int(600)).unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("line-height"), &V::Float(1.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("color"), &V::Rgba {
        r: 255,
        g: 0,
        b: 16,
        a: 255,
    })
    .unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("padding"), &V::Float(-4.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("opacity"), &V::Float(1.5)).unwrap();
    encode_attr_into_row_patch(&mut rp, lookup("z-index"), &V::Int(i64::MAX)).unwrap();

    assert_eq!(rp.get_prop(Layout::INSET_TOP), Some(Length::points(-6.5)));
    assert_eq!(rp.get_prop(Layout::POSITION), Some(Position::Absolute));
    assert_eq!(rp.get_prop(Layout::BOX_SIZING), Some(BoxSizing::ContentBox));
    assert_eq!(rp.get_prop(Layout::ALIGN_SELF), Some(AlignSelf::Center));
    assert_eq!(rp.get_prop(Layout::ASPECT_RATIO), Some(Some(1.5)));
    assert_eq!(rp.get_prop(tables::Interaction::CURSOR), Some(CursorKind::Pointer));
    assert_eq!(rp.get_prop(Typography::WHITE_SPACE), Some(WhiteSpace::BreakSpaces));
    assert_eq!(rp.get_prop(Typography::TEXT_ALIGN), Some(TextAlign::Start));
    assert_eq!(rp.get_prop(Typography::VERTICAL_ALIGN), Some(VerticalAlign::Middle));
    assert_eq!(rp.get_prop(Typography::FONT_WEIGHT), Some(600));
    assert_eq!(rp.get_prop(Typography::LINE_HEIGHT), Some(Length::percent(150.0)));
    assert_eq!(rp.get_prop(Typography::COLOR), Some(Rgba::rgb(255, 0, 16)));
    assert_eq!(rp.get_prop(Layout::PADDING_TOP), Some(Length::points(0.0)));
    assert_eq!(rp.get_prop(Visual::OPACITY), Some(1.0));
    assert_eq!(rp.get_prop(Visual::Z_INDEX), Some(i32::MAX));
}
