//! Focused unit tests for `NveMutationWriter` numeric/bool attributes, child mutations,
//! `primary_slot` realization, dense side-table generation isolation, and writer reset.

use dioxus_core::{AttributeValue, ElementId, WriteMutations};
use nve_abi::SurfaceId;
use nve_store::{Length, NodeId, NveTable};
use nve_tables::tables::{Interaction, Layout, Link, TextContent, Visual};

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
fn table_attr_rerender_updates_in_place_and_typed_style_preserves_standalone_attrs() {
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

    // Re-rendering with the same TableWriter type must NOT invoke clear_from.
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

    // TypedStyle with `layout_mask == 0` and `layout_ext.is_some()` detaches Layout when cleared.
    let ext_only = TypedStyle::new().position_absolute();
    assert!(ext_only.layout_mask.is_empty() && ext_only.layout_ext.is_some());
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

    // Standalone `width: 240.0` must survive `nve_style` update/clear, while non-style numeric
    // attributes (`unknown_num`, `ariaValueNow`) are NOT recorded.
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
        .get(node3)
        .expect("standalone width recorded")
        .iter()
        .map(|&(k, _)| k)
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
fn typed_style_update_clears_dropped_fields_and_skips_identical_dirtying() {
    use std::rc::Rc;

    use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};
    use nve_abi::ColorRgba;

    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    let initial = TypedStyle::new()
        .width_px(300.0)
        .bg(ColorRgba::rgb(0, 255, 0));

    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(initial.clone())),
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
        ColorRgba::rgb(0, 255, 0).to_u32()
    );

    // Re-applying the exact same TypedStyle is a no-op (zero dirtying).
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(initial)),
            ElementId(1),
        );
    });
    assert!(
        writer.last_changes().table(Layout::slot()).is_none(),
        "identical TypedStyle must not dirty Layout"
    );
    assert!(
        writer.last_changes().table(Visual::slot()).is_none(),
        "identical TypedStyle must not dirty Visual"
    );

    // Updating to opacity-only must clear dropped width and background-color.
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().opacity(0.5))),
            ElementId(1),
        );
    });
    assert!(
        !writer.store().has::<Layout>(node_a),
        "dropped width must detach Layout when no standalone layout exists"
    );
    let vis_a = writer.store().try_row::<Visual>(node_a).unwrap();
    assert_eq!(vis_a.background(), nve_store::Rgba::TRANSPARENT);
    assert!((vis_a.opacity() - 0.5).abs() < 1e-6);
}

#[test]
fn recycled_node_and_element_slots_isolate_all_three_side_tables() {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::table_attr::{table_attr_fn_with_clear, NVE_TABLE_ATTR};
    use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};

    let clears = Rc::new(Cell::new(0u32));
    let clears_cb = Rc::clone(&clears);
    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);

    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().width_px(120.0).opacity(0.4))),
            ElementId(1),
        );
        mtx.set_attribute("padding", None, &AttributeValue::Float(10.0), ElementId(1));
        mtx.set_attribute(
            NVE_TABLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(table_attr_fn_with_clear(
                |tx, n| {
                    tx.edit::<Visual>(n).z_index(7);
                },
                move |tx, n| {
                    clears_cb.set(clears_cb.get() + 1);
                    tx.edit::<Visual>(n).z_index(0);
                },
            ))),
            ElementId(1),
        );
        mtx.append_children(ElementId(0), 1);
    });
    let gen0 = writer.resolve_el(ElementId(1)).unwrap();
    assert!(writer.attached_tables.contains_key(gen0));
    assert!(writer.typed_styles.contains_key(gen0));
    assert!(writer.standalone_style_attrs.contains_key(gen0));

    // Remove the node and re-allocate in the next transaction: NodeStore recycles the exact same
    // slot index with a bumped generation.
    writer.with_transaction(|mtx| {
        mtx.remove_node(ElementId(1));
    });
    assert!(writer.attached_tables.is_empty());
    assert!(writer.typed_styles.is_empty());
    assert!(writer.standalone_style_attrs.is_empty());

    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let gen1 = writer.resolve_el(ElementId(1)).unwrap();
    assert_eq!(gen1.node_index(), gen0.node_index(), "slot index recycled");
    assert_ne!(gen1.generation(), gen0.generation(), "generation bumped");

    // Even if stale gen0 entries existed in every side table, gen1 must never observe or clear them.
    writer
        .typed_styles
        .insert(gen0, super::StoredTypedStyle::from_style(TypedStyle::new().width_px(999.0)));
    writer.standalone_style_attrs.insert(
        gen0,
        vec![("padding", super::StandaloneStyleVal::Float(77.0))],
    );
    writer.attached_tables.insert(
        gen0,
        vec![(
            NVE_TABLE_ATTR,
            table_attr_fn_with_clear(
                |_, _| {},
                {
                    let c = Rc::clone(&clears);
                    move |_, _| c.set(c.get() + 100)
                },
            ),
        )],
    );

    writer.with_transaction(|mtx| {
        mtx.set_attribute(NVE_TABLE_ATTR, None, &AttributeValue::None, ElementId(1));
        mtx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::None, ElementId(1));
        mtx.set_attribute("padding", None, &AttributeValue::None, ElementId(1));
    });
    assert_eq!(clears.get(), 0, "clearing gen1 must not invoke gen0 table clear");
    assert_eq!(writer.store().try_row::<Layout>(gen1).unwrap().width(), Length::AUTO);
    assert_eq!(writer.store().try_row::<Layout>(gen1).unwrap().padding_left(), Length::points(0.0));
    assert!(!writer.store().has::<Visual>(gen1));

    // Writing fresh side-table entries onto gen1 cleanly overwrites stale gen0 slots in place.
    writer.with_transaction(|mtx| {
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().width_px(42.0).opacity(0.8))),
            ElementId(1),
        );
        mtx.set_attribute("padding", None, &AttributeValue::Float(6.0), ElementId(1));
        mtx.set_attribute(
            NVE_TABLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(crate::table_attr::table_attr_fn(|tx, n| {
                tx.edit::<Visual>(n).z_index(15);
            }))),
            ElementId(1),
        );
    });
    assert_eq!(clears.get(), 0, "overwriting stale gen0 slot must not invoke gen0 clear_from");
    assert_eq!(writer.attached_tables.len(), 1);
    assert_eq!(writer.typed_styles.len(), 1);
    assert_eq!(writer.standalone_style_attrs.len(), 1);
    assert_eq!(writer.store().try_row::<Layout>(gen1).unwrap().width(), Length::points(42.0));
    assert_eq!(writer.store().try_row::<Layout>(gen1).unwrap().padding_left(), Length::points(6.0));
    let vis = writer.store().try_row::<Visual>(gen1).unwrap();
    assert!((vis.opacity() - 0.8).abs() < 1e-6);
    assert_eq!(vis.z_index(), 15);
}

#[test]
fn writer_reset_clears_side_tables_and_same_frame_multi_mutations_converge() {
    use std::rc::Rc;

    use crate::table_attr::{table_attr_fn, NVE_TABLE_ATTR};
    use crate::typed_style::{TypedStyle, NVE_TYPED_STYLE_ATTR};

    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);

    // Multiple mutations on the same node in one transaction across all side tables.
    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().width_px(50.0).height_px(60.0).opacity(0.2))),
            ElementId(1),
        );
        mtx.set_attribute(
            NVE_TYPED_STYLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(TypedStyle::new().width_px(70.0).height_px(80.0).opacity(0.9))),
            ElementId(1),
        );
        mtx.set_attribute("width", None, &AttributeValue::Float(100.0), ElementId(1));
        mtx.set_attribute("width", None, &AttributeValue::Float(150.0), ElementId(1));
        mtx.set_attribute(
            NVE_TABLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(table_attr_fn(|tx, n| {
                tx.edit::<Visual>(n).z_index(1);
            }))),
            ElementId(1),
        );
        mtx.set_attribute(
            NVE_TABLE_ATTR,
            None,
            &AttributeValue::Any(Rc::new(table_attr_fn(|tx, n| {
                tx.edit::<Visual>(n).z_index(9);
            }))),
            ElementId(1),
        );
        mtx.set_attribute("width", None, &AttributeValue::None, ElementId(1));
        mtx.set_attribute("padding", None, &AttributeValue::Float(12.0), ElementId(1));
        mtx.create_event_listener("tableevent", ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let n1 = writer.resolve_el(ElementId(1)).unwrap();
    let lay = writer.store().try_row::<Layout>(n1).unwrap();
    assert_eq!(lay.width(), Length::points(70.0), "cleared standalone width falls back to latest TypedStyle");
    assert_eq!(lay.height(), Length::points(80.0));
    let vis = writer.store().try_row::<Visual>(n1).unwrap();
    assert!((vis.opacity() - 0.9).abs() < 1e-6);
    assert_eq!(vis.z_index(), 9);

    assert!(!writer.attached_tables.is_empty());
    assert!(!writer.typed_styles.is_empty());
    assert!(!writer.standalone_style_attrs.is_empty());
    assert!(!writer.aux_event_flags.is_empty());

    writer.reset();
    assert_eq!(writer.resolve_el(ElementId(0)), Some(NodeId::ROOT));
    assert_eq!(writer.element_id_of(NodeId::ROOT), Some(ElementId(0)));
    assert_eq!(writer.resolve_el(ElementId(1)), None);
    assert!(writer.attached_tables.is_empty() && writer.attached_tables.is_deallocated());
    assert!(writer.typed_styles.is_empty() && writer.typed_styles.is_deallocated());
    assert!(writer.standalone_style_attrs.is_empty() && writer.standalone_style_attrs.is_deallocated());
    assert!(writer.aux_event_flags.is_empty() && writer.aux_event_flags.capacity() == 0);
    assert!(writer.store().children(NodeId::ROOT).is_empty());

    writer.with_transaction(|mtx| {
        mtx.create_placeholder(ElementId(1));
        mtx.set_attribute("width", None, &AttributeValue::Float(33.0), ElementId(1));
        mtx.append_children(ElementId(0), 1);
    });
    let fresh = writer.resolve_el(ElementId(1)).unwrap();
    assert_eq!(writer.store().try_row::<Layout>(fresh).unwrap().width(), Length::points(33.0));
}
