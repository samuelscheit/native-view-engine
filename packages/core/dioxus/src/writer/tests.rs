//! Focused unit tests for `NveMutationWriter` numeric/bool attributes, child mutations, and
//! `primary_slot` realization.

use dioxus_core::{AttributeValue, ElementId, WriteMutations};
use nve_abi::SurfaceId;
use nve_tables::tables::{Interaction, Layout, Link, TextContent, TextInput, Visual};
use nve_store::{Length, NodeId, NveTable};

use super::NveMutationWriter;

#[test]
fn direct_numeric_and_bool_attributes_avoid_string_roundtrip() {
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(1));
        mtx.set_attribute("height", None, &AttributeValue::Int(80), ElementId(1));
        mtx.set_attribute("opacity", None, &AttributeValue::Float(0.5), ElementId(1));
        mtx.set_attribute("disabled", None, &AttributeValue::Bool(true), ElementId(1));
        mtx.set_attribute("tabIndex", None, &AttributeValue::Int(3), ElementId(1));
        mtx.set_attribute(
            "href",
            None,
            &AttributeValue::Text("https://example.com".into()),
            ElementId(1),
        );
        mtx.append_children(ElementId(0), 1);
    });

    let node = writer.resolve_el(ElementId(1)).unwrap();
    let store = writer.store();
    assert_eq!(store.children(NodeId::ROOT), &[node]);
    assert_eq!(store.try_row::<Layout>(node).unwrap().width(), Length::points(240.0));
    assert_eq!(store.try_row::<Layout>(node).unwrap().height(), Length::points(80.0));
    assert_eq!(store.try_row::<Visual>(node).unwrap().opacity(), 0.5);
    let inter = store.try_row::<Interaction>(node).unwrap();
    assert!(inter.disabled());
    assert_eq!(inter.tab_index(), Some(3));
    assert_eq!(store.primary_slot(node), Link::slot());
}

#[test]
fn remove_node_cleans_up_subtree_and_id_maps() {
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.create_text_node("hello", ElementId(2));
        mtx.append_children(ElementId(1), 1);
        mtx.append_children(ElementId(0), 1);
    });

    let parent = writer.resolve_el(ElementId(1)).unwrap();
    let child = writer.resolve_el(ElementId(2)).unwrap();
    assert_eq!(writer.store().try_row::<TextContent>(child).unwrap().text(), "hello");
    assert_eq!(writer.store().children(parent), &[child]);

    writer.with_transaction(|mtx| {
        mtx.remove_node(ElementId(1));
    });
    assert!(writer.resolve_el(ElementId(1)).is_none());
    assert!(writer.resolve_el(ElementId(2)).is_none());
    assert!(writer.store().children(NodeId::ROOT).is_empty());
}

#[test]
fn table_attr_rerender_updates_in_place_and_clear_style_preserves_standalone_attrs() {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::table_attr::{TableAttr, TableWriter};
    use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};

    struct WriterA {
        val: u32,
        clears: Rc<Cell<u32>>,
    }
    impl TableWriter for WriterA {
        fn write_to(&self, tx: &mut nve_store::Transaction<'_>, node: NodeId) {
            tx.edit::<Visual>(node).z_index(self.val as i32);
        }
        fn clear_from(&self, tx: &mut nve_store::Transaction<'_>, node: NodeId) {
            self.clears.set(self.clears.get() + 1);
            tx.detach::<Visual>(node);
        }
    }

    struct WriterB;
    impl TableWriter for WriterB {
        fn write_to(&self, tx: &mut nve_store::Transaction<'_>, node: NodeId) {
            tx.edit::<Link>(node).href("https://example.com");
        }
    }

    let clears = Rc::new(Cell::new(0));
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            "nve_table",
            None,
            &AttributeValue::Any(Rc::new(TableAttr::new(WriterA {
                val: 1,
                clears: Rc::clone(&clears),
            }))),
            ElementId(1),
        );
        mtx.append_children(ElementId(0), 1);
    });
    assert_eq!(clears.get(), 0);

    // Re-rendering with the same TableWriter type must NOT invoke clear_from (no detach+reattach).
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "nve_table",
            None,
            &AttributeValue::Any(Rc::new(TableAttr::new(WriterA {
                val: 2,
                clears: Rc::clone(&clears),
            }))),
            ElementId(1),
        );
    });
    assert_eq!(clears.get(), 0, "same TableWriter type must not clear_from on re-render");
    let node = writer.resolve_el(ElementId(1)).unwrap();
    assert_eq!(writer.store().try_row::<Visual>(node).unwrap().z_index(), 2);

    // Switching to a different TableWriter type DOES call clear_from on the previous writer.
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "nve_table",
            None,
            &AttributeValue::Any(Rc::new(TableAttr::new(WriterB))),
            ElementId(1),
        );
    });
    assert_eq!(clears.get(), 1);

    // Clearing inline `style` must preserve standalone `width` attribute while dropping inline bg.
    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(1));
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("background-color: #ff0000;".into()),
            ElementId(1),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(node).unwrap().width(),
        Length::points(240.0)
    );
    assert_ne!(
        writer.store().try_row::<Visual>(node).unwrap().background(),
        nve_store::Rgba::TRANSPARENT
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute("style", None, &AttributeValue::None, ElementId(1));
    });
    assert_eq!(
        writer.store().try_row::<Layout>(node).unwrap().width(),
        Length::points(240.0),
        "clearing `style` must preserve standalone `width`"
    );
    assert!(
        !writer.store().has::<Visual>(node),
        "clearing `style` must drop inline-only Visual table"
    );

    // TypedStyle with `has_layout == false` and `layout_ext.is_some()` detaches Layout when cleared.
    let ext_only = TypedStyle::new().position_absolute();
    assert!(!ext_only.has_layout && ext_only.layout_ext.is_some());
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(2));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(ext_only)),
            ElementId(2),
        );
        mtx.append_children(ElementId(0), 1);
    });
    let node2 = writer.resolve_el(ElementId(2)).unwrap();
    assert!(writer.store().has::<Layout>(node2));
    writer.with_transaction(|mtx| {
        mtx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::None, ElementId(2));
    });
    assert!(
        !writer.store().has::<Layout>(node2),
        "clearing layout_ext-only TypedStyle must detach Layout"
    );

    // Standalone `width: 240.0` must survive `nve_style` update/clear AND inline `style` updates,
    // while non-style numeric attributes (`unknown_num`, `ariaValueNow`) are NOT recorded.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(3));
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(3));
        mtx.set_attribute("unknown_num", None, &AttributeValue::Float(99.0), ElementId(3));
        mtx.set_attribute("ariaValueNow", None, &AttributeValue::Float(50.0), ElementId(3));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().width_px(100.0).height_px(50.0))),
            ElementId(3),
        );
        mtx.append_children(ElementId(0), 1);
    });
    let node3 = writer.resolve_el(ElementId(3)).unwrap();
    let recorded_keys: Vec<&str> = writer
        .standalone_style_attrs
        .get(&node3)
        .expect("standalone width recorded")
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(
        recorded_keys,
        vec!["width"],
        "only handled style attributes must be recorded in standalone_style_attrs"
    );
    assert_eq!(
        writer.store().try_row::<Layout>(node3).unwrap().width(),
        Length::points(240.0),
        "standalone width must be replayed on nve_style apply/update"
    );
    // Inline `style` update must also replay standalone width over TypedStyle's 100px width.
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("background-color: #00ff00;".into()),
            ElementId(3),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(node3).unwrap().width(),
        Length::points(240.0),
        "inline `style` update must preserve standalone width over TypedStyle"
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().bg(nve_abi::ColorRgba::rgb(10, 20, 30)))),
            ElementId(3),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(node3).unwrap().width(),
        Length::points(240.0),
        "standalone width must survive nve_style update that drops typed layout"
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::None, ElementId(3));
    });
    assert_eq!(
        writer.store().try_row::<Layout>(node3).unwrap().width(),
        Length::points(240.0),
        "standalone width must survive nve_style clear"
    );
}

#[test]
fn inline_style_update_clears_dropped_declarations_and_skips_redundant_dirtying() {
    use std::rc::Rc;

    use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};
    use nve_store::NveTable as _;

    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);

    // Node A (no TypedStyle, no standalone): updating style clears dropped Layout/Visual props.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: 300px; background-color: #00ff00;".into()),
            ElementId(1),
        );
        mtx.append_children(ElementId(0), 1);
    });
    let node_a = writer.resolve_el(ElementId(1)).unwrap();
    assert_eq!(
        writer.store().try_row::<Layout>(node_a).unwrap().width(),
        Length::points(300.0)
    );
    assert_eq!(
        writer.store().try_row::<Visual>(node_a).unwrap().background().0,
        nve_abi::ColorRgba::rgb(0, 255, 0).to_u32()
    );

    // Re-applying the exact same inline style string is a no-op (zero dirtying).
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: 300px; background-color: #00ff00;".into()),
            ElementId(1),
        );
    });
    assert!(
        writer.last_changes().table(Layout::slot()).is_none(),
        "identical inline style must not dirty Layout"
    );
    assert!(
        writer.last_changes().table(Visual::slot()).is_none(),
        "identical inline style must not dirty Visual"
    );

    // Updating to opacity-only must clear dropped width and background-color.
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("opacity: 0.5;".into()),
            ElementId(1),
        );
    });
    assert!(
        !writer.store().has::<Layout>(node_a),
        "dropped width must detach Layout when no TypedStyle or standalone layout exists"
    );
    let vis_a = writer.store().try_row::<Visual>(node_a).unwrap();
    assert_eq!(
        vis_a.background(),
        nve_store::Rgba::TRANSPARENT,
        "dropped background-color must clear to default"
    );
    assert!((vis_a.opacity() - 0.5).abs() < 1e-6);

    // Node B (with TypedStyle + standalone width):
    // 1) Initial pure-Visual inline style must preserve standalone width WITHOUT dirtying Layout.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(2));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(
                TypedStyle::new()
                    .width_px(100.0)
                    .height_px(50.0)
                    .bg(nve_abi::ColorRgba::rgb(10, 20, 30)),
            )),
            ElementId(2),
        );
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(2));
        mtx.append_children(ElementId(0), 1);
    });
    let node_b = writer.resolve_el(ElementId(2)).unwrap();
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("opacity: 0.5;".into()),
            ElementId(2),
        );
    });
    assert!(
        writer.last_changes().table(Layout::slot()).is_none(),
        "initial pure-Visual inline style on node with standalone width must not dirty Layout"
    );
    assert_eq!(
        writer.store().try_row::<Layout>(node_b).unwrap().width(),
        Length::points(240.0)
    );
    assert!((writer.store().try_row::<Visual>(node_b).unwrap().opacity() - 0.5).abs() < 1e-6);

    // 2) Updating inline style to override width & bg, then dropping them restores TypedStyle + standalone.
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: 300px; background-color: #00ff00;".into()),
            ElementId(2),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(node_b).unwrap().width(),
        Length::points(300.0)
    );
    assert_eq!(
        writer.store().try_row::<Visual>(node_b).unwrap().background().0,
        nve_abi::ColorRgba::rgb(0, 255, 0).to_u32()
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("opacity: 0.25;".into()),
            ElementId(2),
        );
    });
    let lay_b = writer.store().try_row::<Layout>(node_b).unwrap();
    assert_eq!(
        lay_b.width(),
        Length::points(240.0),
        "dropped inline width must restore standalone width over TypedStyle"
    );
    assert_eq!(
        lay_b.height(),
        Length::points(50.0),
        "TypedStyle height must be preserved across inline style update"
    );
    let vis_b = writer.store().try_row::<Visual>(node_b).unwrap();
    assert_eq!(
        vis_b.background().0,
        nve_abi::ColorRgba::rgb(10, 20, 30).to_u32(),
        "dropped inline bg must restore TypedStyle bg"
    );
    assert!((vis_b.opacity() - 0.25).abs() < 1e-6);
}

#[test]
fn canonical_style_precedence_standalone_clear_composite_fast_lane_and_invalid_key_reset() {
    use std::rc::Rc;

    use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};
    use nve_abi::table::effect;
    use nve_tables::tables::Typography;
    use nve_store::NveTable as _;

    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);

    // 1. Canonical precedence: `style="width: 100px;"` then standalone `width="240"` -> inline 100px wins
    //    immediately and survives identical re-apply and additive `style` updates.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: 100px;".into()),
            ElementId(1),
        );
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let n1 = writer.resolve_el(ElementId(1)).unwrap();
    assert_eq!(
        writer.store().try_row::<Layout>(n1).unwrap().width(),
        Length::points(100.0),
        "inline style must beat standalone width even when standalone is set after style"
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: 100px;".into()),
            ElementId(1),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n1).unwrap().width(),
        Length::points(100.0)
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: 100px; opacity: 0.5;".into()),
            ElementId(1),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n1).unwrap().width(),
        Length::points(100.0)
    );
    assert!((writer.store().try_row::<Visual>(n1).unwrap().opacity() - 0.5).abs() < 1e-6);

    // 2. Updating `style="opacity: 0.5;"` -> `style="opacity: 0.25;"` preserves `effect::COMPOSITE`
    //    fast lane without dirtying `Layout` or `Typography`.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(2));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().width_px(80.0))),
            ElementId(2),
        );
        mtx.set_attribute("color", None, &AttributeValue::Text("#112233".into()), ElementId(2));
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("opacity: 0.5;".into()),
            ElementId(2),
        );
        mtx.append_children(ElementId(0), 1);
    });
    let n2 = writer.resolve_el(ElementId(2)).unwrap();
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("opacity: 0.25;".into()),
            ElementId(2),
        );
    });
    assert!((writer.store().try_row::<Visual>(n2).unwrap().opacity() - 0.25).abs() < 1e-6);
    assert!(
        writer.last_changes().table(Layout::slot()).is_none(),
        "opacity-only style update must not dirty Layout"
    );
    assert!(
        writer.last_changes().table(Typography::slot()).is_none(),
        "opacity-only style update must not dirty Typography"
    );
    let idx = writer
        .last_changes()
        .touched
        .iter()
        .position(|&id| id == n2)
        .expect("n2 must be in touched");
    assert_eq!(
        writer.last_changes().effects[idx],
        effect::COMPOSITE,
        "opacity-only inline style update must emit strictly effect::COMPOSITE"
    );

    // 3. Standalone `clear_attr("width")` clears stale 240px and falls back to inline / TypedStyle / default.
    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(2));
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n2).unwrap().width(),
        Length::points(240.0)
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::None, ElementId(2));
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n2).unwrap().width(),
        Length::points(80.0),
        "clearing standalone width must fall back to TypedStyle width"
    );
    // Clearing standalone width with no TypedStyle or inline width detaches Layout cleanly.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(3));
        mtx.set_attribute("width", None, &AttributeValue::Float(240.0), ElementId(3));
        mtx.append_children(ElementId(0), 1);
    });
    let n3 = writer.resolve_el(ElementId(3)).unwrap();
    assert_eq!(
        writer.store().try_row::<Layout>(n3).unwrap().width(),
        Length::points(240.0)
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::None, ElementId(3));
    });
    assert!(
        !writer.store().has::<Layout>(n3),
        "clearing sole standalone width must detach Layout"
    );

    // 4. Invalidated inline key (`width: 100px;` -> `width: invalid;`) resets Layout.width cleanly.
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: 100px;".into()),
            ElementId(3),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n3).unwrap().width(),
        Length::points(100.0)
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: invalid;".into()),
            ElementId(3),
        );
    });
    assert!(
        !writer.store().has::<Layout>(n3),
        "invalidated inline width must reset/detach Layout cleanly"
    );
    // And with TypedStyle underneath, `width: invalid;` restores TypedStyle's 80px instead of clobbering to Auto.
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: 150px;".into()),
            ElementId(2),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n2).unwrap().width(),
        Length::points(150.0)
    );
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("width: invalid;".into()),
            ElementId(2),
        );
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n2).unwrap().width(),
        Length::points(80.0),
        "invalidated inline width must restore underlying TypedStyle width"
    );
}

#[test]
fn standalone_invalid_reset_empty_style_clear_noop_and_shorthand_order_preservation() {
    use std::rc::Rc;

    use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};
    use nve_tables::tables::Typography;
    use nve_store::NveTable as _;

    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);

    // 1. Standalone invalid value on untouched node must NOT record junk in standalone_style_attrs.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().width_px(80.0))),
            ElementId(1),
        );
        mtx.set_attribute("width", None, &AttributeValue::Text("invalid".into()), ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let n1 = writer.resolve_el(ElementId(1)).unwrap();
    assert!(
        !writer.standalone_style_attrs.contains_key(&n1),
        "invalid standalone attr on untouched node must not insert junk into standalone_style_attrs"
    );
    assert_eq!(
        writer.store().try_row::<Layout>(n1).unwrap().width(),
        Length::points(80.0)
    );

    // 2. Standalone width="200px" -> width="invalid" restores TypedStyle (80px);
    //    standalone Float(200.0) -> width="invalid" also restores TypedStyle (80px).
    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::Text("200px".into()), ElementId(1));
    });
    assert_eq!(writer.store().try_row::<Layout>(n1).unwrap().width(), Length::points(200.0));
    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::Text("invalid".into()), ElementId(1));
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n1).unwrap().width(),
        Length::points(80.0),
        "standalone string width=\"200px\" -> \"invalid\" must restore TypedStyle width"
    );
    assert!(!writer.standalone_style_attrs.contains_key(&n1));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::Float(200.0), ElementId(1));
    });
    assert_eq!(writer.store().try_row::<Layout>(n1).unwrap().width(), Length::points(200.0));
    writer.with_transaction(|mtx| {
        mtx.set_attribute("width", None, &AttributeValue::Text("invalid".into()), ElementId(1));
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n1).unwrap().width(),
        Length::points(80.0),
        "standalone Float(200.0) -> \"invalid\" must restore TypedStyle width"
    );

    // 3. Clearing empty or invalid-only inline `style` emits zero dirtying on Layout/Visual/Typography.
    for empty_or_invalid in ["", "width: invalid;"] {
        writer.with_transaction(|mtx| {
            mtx.set_attribute(
                "style",
                None,
                &AttributeValue::Text(empty_or_invalid.into()),
                ElementId(1),
            );
        });
        writer.with_transaction(|mtx| {
            mtx.set_attribute("style", None, &AttributeValue::None, ElementId(1));
        });
        assert!(
            writer.last_changes().table(Layout::slot()).is_none(),
            "clearing `{empty_or_invalid}` must not dirty Layout"
        );
        assert!(
            writer.last_changes().table(Visual::slot()).is_none(),
            "clearing `{empty_or_invalid}` must not dirty Visual"
        );
        assert!(
            writer.last_changes().table(Typography::slot()).is_none(),
            "clearing `{empty_or_invalid}` must not dirty Typography"
        );
    }

    // 4. Shorthand-vs-longhand order preservation on in-place standalone update:
    //    padding={16.0}, paddingHorizontal={4.0}, then padding={24.0} -> top/bottom=24, left/right=4.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(2));
        mtx.set_attribute("padding", None, &AttributeValue::Float(16.0), ElementId(2));
        mtx.set_attribute("paddingHorizontal", None, &AttributeValue::Float(4.0), ElementId(2));
        mtx.append_children(ElementId(0), 1);
    });
    let n2 = writer.resolve_el(ElementId(2)).unwrap();
    writer.with_transaction(|mtx| {
        mtx.set_attribute("padding", None, &AttributeValue::Float(24.0), ElementId(2));
    });
    let lay = writer.store().try_row::<Layout>(n2).unwrap();
    assert_eq!(lay.padding_top(), Length::points(24.0));
    assert_eq!(lay.padding_bottom(), Length::points(24.0));
    assert_eq!(
        lay.padding_left(),
        Length::points(4.0),
        "updating earlier shorthand `padding` must preserve later longhand `paddingHorizontal`"
    );
    assert_eq!(lay.padding_right(), Length::points(4.0));

    // 5. History-independent shorthand/longhand rank:
    //    (a) Clear `padding` then re-set `padding=24` -> `paddingHorizontal=4` MUST still win on left/right!
    writer.with_transaction(|mtx| {
        mtx.set_attribute("padding", None, &AttributeValue::None, ElementId(2));
    });
    writer.with_transaction(|mtx| {
        mtx.set_attribute("padding", None, &AttributeValue::Float(24.0), ElementId(2));
    });
    let lay = writer.store().try_row::<Layout>(n2).unwrap();
    assert_eq!(lay.padding_top(), Length::points(24.0));
    assert_eq!(lay.padding_bottom(), Length::points(24.0));
    assert_eq!(
        lay.padding_left(),
        Length::points(4.0),
        "re-setting cleared shorthand `padding` must stay ranked before `paddingHorizontal`"
    );
    assert_eq!(lay.padding_right(), Length::points(4.0));

    //    (b) Invalidate `padding="invalid"` then re-set `padding="20px"` -> `paddingHorizontal=4` MUST still win!
    writer.with_transaction(|mtx| {
        mtx.set_attribute("padding", None, &AttributeValue::Text("invalid".into()), ElementId(2));
    });
    writer.with_transaction(|mtx| {
        mtx.set_attribute("padding", None, &AttributeValue::Text("20px".into()), ElementId(2));
    });
    let lay = writer.store().try_row::<Layout>(n2).unwrap();
    assert_eq!(lay.padding_top(), Length::points(20.0));
    assert_eq!(lay.padding_bottom(), Length::points(20.0));
    assert_eq!(
        lay.padding_left(),
        Length::points(4.0),
        "re-setting invalidated shorthand `padding` must stay ranked before `paddingHorizontal`"
    );
    assert_eq!(lay.padding_right(), Length::points(4.0));

    // 6. Non-overlapping standalone `opacity` update with trailing `backgroundColor` + `boxShadow`
    //    and non-overlapping inline `style="background-color: #112233;"` emits strictly effect::COMPOSITE.
    use nve_abi::table::effect;
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(3));
        mtx.set_attribute("opacity", None, &AttributeValue::Float(0.5), ElementId(3));
        mtx.set_attribute(
            "backgroundColor",
            None,
            &AttributeValue::Text("rgba(10, 20, 30, 0.5)".into()),
            ElementId(3),
        );
        mtx.set_attribute(
            "boxShadow",
            None,
            &AttributeValue::Text("0 2px 4px #000000".into()),
            ElementId(3),
        );
        mtx.set_attribute(
            "style",
            None,
            &AttributeValue::Text("background-color: #112233;".into()),
            ElementId(3),
        );
        mtx.append_children(ElementId(0), 1);
    });
    let n3 = writer.resolve_el(ElementId(3)).unwrap();
    writer.standalone_replay_count = 0;
    writer.inline_reapply_count = 0;
    writer.with_transaction(|mtx| {
        mtx.set_attribute("opacity", None, &AttributeValue::Float(0.25), ElementId(3));
    });
    assert_eq!(
        writer.standalone_replay_count, 0,
        "non-overlapping standalone opacity update must perform zero trailing standalone replays"
    );
    assert_eq!(
        writer.inline_reapply_count, 0,
        "non-overlapping standalone opacity update must perform zero inline style re-applies"
    );
    let vis3 = writer.store().try_row::<Visual>(n3).unwrap();
    assert!((vis3.opacity() - 0.25).abs() < 1e-6);
    assert_eq!(
        vis3.background().0,
        nve_abi::ColorRgba::rgb(0x11, 0x22, 0x33).to_u32(),
        "inline background-color must remain intact across standalone opacity update"
    );
    assert!(writer.last_changes().table(Layout::slot()).is_none());
    let idx = writer
        .last_changes()
        .touched
        .iter()
        .position(|&id| id == n3)
        .expect("n3 must be in touched");
    assert_eq!(
        writer.last_changes().effects[idx],
        effect::COMPOSITE,
        "non-overlapping standalone opacity update must emit strictly effect::COMPOSITE"
    );

    // 7. Single-source `css_key_rank` clear-then-reset parity for `flex`/`flexGrow` and `background`/`backgroundColor`.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(4));
        mtx.set_attribute("flex", None, &AttributeValue::Float(1.0), ElementId(4));
        mtx.set_attribute("flexGrow", None, &AttributeValue::Float(3.0), ElementId(4));
        mtx.set_attribute("background", None, &AttributeValue::Text("#ff0000".into()), ElementId(4));
        mtx.set_attribute("backgroundColor", None, &AttributeValue::Text("#00ff00".into()), ElementId(4));
        mtx.append_children(ElementId(0), 1);
    });
    let n4 = writer.resolve_el(ElementId(4)).unwrap();
    writer.with_transaction(|mtx| {
        mtx.set_attribute("flex", None, &AttributeValue::None, ElementId(4));
        mtx.set_attribute("background", None, &AttributeValue::None, ElementId(4));
    });
    writer.with_transaction(|mtx| {
        mtx.set_attribute("flex", None, &AttributeValue::Float(2.0), ElementId(4));
        mtx.set_attribute("background", None, &AttributeValue::Text("#0000ff".into()), ElementId(4));
    });
    assert_eq!(
        writer.store().try_row::<Layout>(n4).unwrap().flex_grow(),
        3.0,
        "re-setting cleared `flex` shorthand must stay ranked before `flexGrow`"
    );
    assert_eq!(
        writer.store().try_row::<Visual>(n4).unwrap().background().0,
        nve_abi::ColorRgba::rgb(0x00, 0xff, 0x00).to_u32(),
        "re-setting cleared `background` shorthand must stay ranked before `backgroundColor`"
    );
}

#[test]
fn text_input_minlength_and_maxlength_preserve_zero_and_map_negative_to_none() {
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let n1 = writer.resolve_el(ElementId(1)).unwrap();

    // Int attribute path: 0 -> Some(0), negative -> None, large positive -> clamped u16::MAX.
    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Int(0), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Int(0), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), Some(0));
    assert_eq!(ti.max_length(), Some(0));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Int(100_000), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Int(100_000), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), Some(u16::MAX));
    assert_eq!(ti.max_length(), Some(u16::MAX));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Int(-5), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Int(-1), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), None);
    assert_eq!(ti.max_length(), None);

    // String attribute path: "0" -> Some(0), "-3" -> None, "42" -> Some(42).
    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Text("0".into()), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Text("42".into()), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), Some(0));
    assert_eq!(ti.max_length(), Some(42));

    writer.with_transaction(|mtx| {
        mtx.set_attribute("minlength", None, &AttributeValue::Text("-3".into()), ElementId(1));
        mtx.set_attribute("maxlength", None, &AttributeValue::Text("-9".into()), ElementId(1));
    });
    let ti = writer.store().try_row::<TextInput>(n1).unwrap();
    assert_eq!(ti.min_length(), None);
    assert_eq!(ti.max_length(), None);
}
