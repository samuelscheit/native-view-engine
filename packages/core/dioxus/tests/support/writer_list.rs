//! Steady-state `WriteMutations` fixture shared by the `NveMutationWriter` zero-allocation guard
//! and Criterion bench: a keyed list of 32 template items, each `view > [text, inner view >
//! [placeholder]]`, under a column container bound to `ElementId(1)`. Every inner view is bound
//! through `assign_node_id` and carries a `TypedStyle` plus a `TableAttr`, so removal and
//! re-instantiation recycle every kind of per-node writer buffer.

use dioxus_core::{
    AttributeValue, ElementId, IntoAttributeValue, Template, TemplateNode, WriteMutations,
};
use nve_abi::ColorRgba;
use nve_dioxus::{NveMutationWriter, TypedStyle, table_attr_fn, NVE_TYPED_STYLE_ATTR};
use nve_store::Rgba;
use nve_tables::media::MediaQuery;
use nve_tables::tables::Visual;
use nve_tables::StylePatchExt;

pub const ITEMS: usize = 32;
const CONTAINER: ElementId = ElementId(1);
const FIRST_ITEM: usize = 2;

static CONTAINER_TEMPLATE: Template = Template {
    roots: &[TemplateNode::Element {
        tag: "view",
        namespace: None,
        attrs: &[],
        children: &[],
    }],
    node_paths: &[],
    attr_paths: &[],
};

static ITEM_TEMPLATE: Template = Template {
    roots: &[TemplateNode::Element {
        tag: "view",
        namespace: None,
        attrs: &[],
        children: &[
            TemplateNode::Text { text: "label" },
            TemplateNode::Element {
                tag: "view",
                namespace: None,
                attrs: &[],
                children: &[TemplateNode::Dynamic { id: 0 }],
            },
        ],
    }],
    node_paths: &[&[0, 1, 0]],
    attr_paths: &[],
};

#[inline]
pub fn item(k: usize) -> ElementId {
    ElementId(FIRST_ITEM + k)
}

/// The inner view of item `k`, bound through `assign_node_id`.
#[inline]
pub fn inner(k: usize) -> ElementId {
    ElementId(FIRST_ITEM + ITEMS + k)
}

/// Path of the inner view inside the item template.
const INNER_PATH: &[u8] = &[1];

/// Table-attr border colours, indexed like [`Values::table`].
pub const TABLE_BORDER: [Rgba; 2] = [Rgba::rgb(1, 2, 3), Rgba::rgb(4, 5, 6)];

/// Pre-built typed attribute values (owned by Dioxus in real use).
pub struct Values {
    pub padding: [AttributeValue; 2],
    pub opacity: [AttributeValue; 2],
    /// Different property sets, so switching drops `border-radius` (restores default) and adds
    /// `border-width`.
    pub typed_style: [AttributeValue; 2],
    pub inner_style: [AttributeValue; 2],
    pub z_index: [AttributeValue; 2],
    pub table: [AttributeValue; 2],
}

pub fn values() -> Values {
    Values {
        padding: [AttributeValue::Float(8.0), AttributeValue::Float(12.0)],
        opacity: [AttributeValue::Float(0.5), AttributeValue::Float(0.75)],
        typed_style: [
            TypedStyle::new()
                .flex_1()
                .width_px(120.0)
                .bg(ColorRgba::rgb(0x1e, 0x29, 0x3b))
                .radius(4.0)
                .on_hover_with(|p| p.opacity(0.9))
                .at_with(MediaQuery::compact(), |p| p.padding(6.0))
                .into_value(),
            TypedStyle::new()
                .flex_1()
                .width_px(160.0)
                .bg(ColorRgba::rgb(0x33, 0x41, 0x55))
                .border_width(2.0)
                .on_hover_with(|p| p.opacity(0.9))
                .at_with(MediaQuery::compact(), |p| p.padding(6.0))
                .into_value(),
        ],
        inner_style: [
            TypedStyle::new()
                .width_px(10.0)
                .opacity(0.25)
                .into_value(),
            TypedStyle::new()
                .width_px(20.0)
                .opacity(0.75)
                .radius(3.0)
                .into_value(),
        ],
        z_index: [AttributeValue::Int(1), AttributeValue::Int(2)],
        table: TABLE_BORDER.map(|c| {
            table_attr_fn(move |tx, node| {
                tx.edit::<Visual>(node).border_color(c);
            })
            .into_value()
        }),
    }
}

/// Instantiates item `k` on top of the stack and styles its inner view with `pick`.
fn instantiate_item(tx: &mut impl WriteMutations, v: &Values, k: usize, pick: usize) {
    tx.load_template(ITEM_TEMPLATE, 0, item(k));
    tx.assign_node_id(INNER_PATH, inner(k));
    tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &v.inner_style[pick], inner(k));
    tx.set_attribute("accent", None, &v.table[pick], inner(k));
}

pub fn mount_list(writer: &mut NveMutationWriter, v: &Values) {
    writer.with_transaction(|tx| {
        tx.load_template(CONTAINER_TEMPLATE, 0, CONTAINER);
        tx.append_children(ElementId(0), 1);
        for k in 0..ITEMS {
            instantiate_item(tx, v, k, 0);
        }
        tx.append_children(CONTAINER, ITEMS);
    });
}

/// Re-sets attributes to the values they already hold.
pub fn same_value_attributes(writer: &mut NveMutationWriter, v: &Values, frame: usize) {
    writer.with_transaction(|tx| {
        for k in 0..ITEMS {
            tx.set_attribute("padding", None, &v.padding[0], item(k));
            tx.set_attribute("opacity", None, &v.opacity[0], item(k));
            tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &v.typed_style[0], item(k));
        }
        let _ = frame;
    });
}

/// Alternates attribute values (real per-frame style churn).
pub fn changed_attributes(writer: &mut NveMutationWriter, v: &Values, frame: usize) {
    let pick = frame % 2;
    writer.with_transaction(|tx| {
        for k in 0..ITEMS {
            tx.set_attribute("padding", None, &v.padding[pick], item(k));
            tx.set_attribute("opacity", None, &v.opacity[pick], item(k));
            tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &v.typed_style[pick], item(k));
        }
    });
}

/// Alternates `nve_style` (`TypedStyle`), standalone numeric `padding` + `z-index`, and `accent`
/// (`TableAttr`) on all 32 items — exercising all three dense side tables every frame.
pub fn all_side_tables_attributes(writer: &mut NveMutationWriter, v: &Values, frame: usize) {
    let pick = frame % 2;
    writer.with_transaction(|tx| {
        for k in 0..ITEMS {
            tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &v.typed_style[pick], item(k));
            tx.set_attribute("padding", None, &v.padding[pick], item(k));
            tx.set_attribute("z-index", None, &v.z_index[pick], item(k));
            tx.set_attribute("accent", None, &v.table[pick], item(k));
        }
    });
}

/// Keyed reorder: moves one item before another, as Dioxus' keyed diff emits.
pub fn keyed_reorder(writer: &mut NveMutationWriter, frame: usize) {
    let moved = item((frame * 7) % ITEMS);
    let anchor = item((frame * 13 + 1) % ITEMS);
    if moved == anchor {
        return;
    }
    writer.with_transaction(|tx| {
        tx.push_root(moved);
        tx.insert_nodes_before(anchor, 1);
    });
}

/// Removes one item subtree, then re-instantiates it from the template into the freed slots
/// (Dioxus recycles the `ElementId`s), alternating the inner view's `TypedStyle` and table attr.
pub fn remove_and_reinstantiate(writer: &mut NveMutationWriter, v: &Values, frame: usize) {
    let k = frame % ITEMS;
    writer.with_transaction(|tx| {
        tx.remove_node(item(k));
        instantiate_item(tx, v, k, (frame / ITEMS) % 2);
        tx.append_children(CONTAINER, 1);
    });
}

/// Whether the last transaction changed the store at all (an elided frame measures nothing).
pub fn store_changed(writer: &NveMutationWriter) -> bool {
    let c = writer.last_changes();
    !(c.touched.is_empty() && c.children.is_empty() && c.created.is_empty() && c.removed.is_empty())
}
