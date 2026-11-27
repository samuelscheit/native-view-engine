//! Zero-allocation guard for the direct (non-layered) fast path of `NveMutationWriter`: warm
//! standalone typed attributes — named colors, `border` with `currentColor`, `border-width`
//! keywords, shorthands, keywords, and stack-formatted `Float` / `Int` values — must write
//! straight into the `NodeStore` word tables without touching the heap.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;

use counting_alloc::count_allocs;
use dioxus_core::{AttributeValue, ElementId, Template, TemplateNode, WriteMutations};
use nve_components::tables::{FlexDirection, Layout, Typography, Visual};
use nve_dioxus::NveMutationWriter;
use nve_store::{Length, Rgba};

static VIEW_TEMPLATE: Template = Template {
    roots: &[TemplateNode::Element {
        tag: "view",
        namespace: None,
        attrs: &[],
        children: &[],
    }],
    node_paths: &[],
    attr_paths: &[],
};

const EL1: ElementId = ElementId(2);
const EL2: ElementId = ElementId(3);

/// Pre-built attribute values (`AttributeValue::Text` allocates; Dioxus owns them in real use).
struct Frame {
    width: AttributeValue,
    padding: AttributeValue,
    color: AttributeValue,
    border: AttributeValue,
    direction: AttributeValue,
    opacity: AttributeValue,
    z_index: AttributeValue,
    border_width: AttributeValue,
}

fn text(s: &str) -> AttributeValue {
    AttributeValue::Text(s.to_owned())
}

impl Frame {
    fn apply(&self, writer: &mut NveMutationWriter) {
        writer.with_transaction(|tx| {
            tx.set_attribute("width", None, &self.width, EL1);
            tx.set_attribute("padding", None, &self.padding, EL1);
            tx.set_attribute("color", None, &self.color, EL1);
            tx.set_attribute("border", None, &self.border, EL1);
            tx.set_attribute("flex-direction", None, &self.direction, EL1);
            tx.set_attribute("opacity", None, &self.opacity, EL1);
            tx.set_attribute("z-index", None, &self.z_index, EL1);
            tx.set_attribute("border-width", None, &self.border_width, EL2);
        });
    }
}

#[test]
fn counting_alloc_probe_fires_on_this_thread_only() {
    counting_alloc::assert_counts_only_this_thread();
}

#[test]
fn direct_typed_attribute_updates_perform_zero_heap_allocations() {
    let (_, sanity) = count_allocs(|| std::hint::black_box(Vec::<u8>::with_capacity(64)));
    assert!(sanity > 0, "allocator probe must observe a Vec allocation");

    let mut writer = NveMutationWriter::new(1);
    writer.with_transaction(|tx| {
        tx.load_template(VIEW_TEMPLATE, 0, ElementId(1));
        tx.load_template(VIEW_TEMPLATE, 0, EL1);
        tx.load_template(VIEW_TEMPLATE, 0, EL2);
        tx.append_children(ElementId(0), 3);
    });

    // Named colors (case/space-insensitive), `border` with `currentColor`, keyword widths and
    // stack-formatted numerics, alternating with a second value set so every frame changes.
    let initial = Frame {
        width: text("100px"),
        padding: text("4px"),
        color: text("blue"),
        border: text("1px solid"),
        direction: text("column"),
        opacity: AttributeValue::Float(0.25),
        z_index: AttributeValue::Int(3),
        border_width: text("thin"),
    };
    let target = Frame {
        width: text("240px"),
        padding: text("8px 12px"),
        color: text("RED"),
        border: text("2px SOLID CurrentColor"),
        direction: text("ROW"),
        opacity: AttributeValue::Float(0.75),
        z_index: AttributeValue::Int(12),
        border_width: text("THICK"),
    };
    for _ in 0..3 {
        initial.apply(&mut writer);
        target.apply(&mut writer);
    }
    initial.apply(&mut writer);

    let (_, allocs) = count_allocs(|| target.apply(&mut writer));
    assert_eq!(
        allocs, 0,
        "warm direct-path standalone typed attrs must not allocate"
    );

    let red = Rgba::rgb(239, 68, 68);
    let el1 = writer.resolve_el(EL1).expect("el1 bound");
    let el2 = writer.resolve_el(EL2).expect("el2 bound");
    let store = writer.store();
    let layout = store.try_row::<Layout>(el1).expect("el1 layout");
    assert_eq!(layout.width(), Length::points(240.0));
    assert_eq!(layout.padding_top(), Length::points(8.0));
    assert_eq!(layout.padding_bottom(), Length::points(8.0));
    assert_eq!(layout.padding_left(), Length::points(12.0));
    assert_eq!(layout.padding_right(), Length::points(12.0));
    assert_eq!(layout.flex_direction(), FlexDirection::Row);
    let typography = store.try_row::<Typography>(el1).expect("el1 typography");
    assert_eq!(typography.color(), red);
    let visual = store.try_row::<Visual>(el1).expect("el1 visual");
    assert_eq!(visual.border_width(), 2.0);
    assert_eq!(
        visual.border_color(),
        red,
        "currentColor resolves to the named color"
    );
    assert_eq!(visual.opacity(), 0.75);
    assert_eq!(visual.z_index(), 12);
    let visual2 = store.try_row::<Visual>(el2).expect("el2 visual");
    assert_eq!(visual2.border_width(), 5.0);
}
