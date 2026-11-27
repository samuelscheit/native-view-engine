//! Steady-state `WriteMutations` fixture shared by the `NveMutationWriter` zero-allocation guard
//! and Criterion bench: a keyed list of 32 template items, each `view(padding) > [text, view >
//! [placeholder]]`, under a column container bound to `ElementId(1)`.

use dioxus_core::{
    AttributeValue, ElementId, Template, TemplateAttribute, TemplateNode, WriteMutations,
};
use nve_dioxus::NveMutationWriter;

pub const ITEMS: usize = 32;
const CONTAINER: ElementId = ElementId(1);
const FIRST_ITEM: usize = 2;

static CONTAINER_TEMPLATE: Template = Template {
    roots: &[TemplateNode::Element {
        tag: "view",
        namespace: None,
        attrs: &[TemplateAttribute::Static {
            name: "flex_direction",
            value: "column",
            namespace: None,
        }],
        children: &[],
    }],
    node_paths: &[],
    attr_paths: &[],
};

static ITEM_TEMPLATE: Template = Template {
    roots: &[TemplateNode::Element {
        tag: "view",
        namespace: None,
        attrs: &[TemplateAttribute::Static {
            name: "padding",
            value: "4px",
            namespace: None,
        }],
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

/// Pre-built attribute values: building an `AttributeValue::Text` allocates, Dioxus owns them.
pub struct Values {
    pub padding: [AttributeValue; 2],
    pub opacity: [AttributeValue; 2],
    pub style: [AttributeValue; 2],
}

pub fn values() -> Values {
    Values {
        padding: [
            AttributeValue::Text("8px".into()),
            AttributeValue::Text("12px".into()),
        ],
        opacity: [AttributeValue::Float(0.5), AttributeValue::Float(0.75)],
        style: [
            AttributeValue::Text("background-color: #1e293b; border-radius: 4px".into()),
            AttributeValue::Text("background-color: #334155; border-radius: 6px".into()),
        ],
    }
}

pub fn mount_list(writer: &mut NveMutationWriter) {
    writer.with_transaction(|tx| {
        tx.load_template(CONTAINER_TEMPLATE, 0, CONTAINER);
        tx.append_children(ElementId(0), 1);
        for k in 0..ITEMS {
            tx.load_template(ITEM_TEMPLATE, 0, item(k));
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
            tx.set_attribute("style", None, &v.style[0], item(k));
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
            tx.set_attribute("style", None, &v.style[pick], item(k));
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
/// (Dioxus recycles the `ElementId`).
pub fn remove_and_reinstantiate(writer: &mut NveMutationWriter, frame: usize) {
    let k = frame % ITEMS;
    writer.with_transaction(|tx| {
        tx.remove_node(item(k));
        tx.load_template(ITEM_TEMPLATE, 0, item(k));
        tx.append_children(CONTAINER, 1);
    });
}
