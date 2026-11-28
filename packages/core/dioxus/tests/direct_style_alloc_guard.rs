//! Zero-allocation guard for the direct typed fast path of `NveMutationWriter`: warm
//! `TypedStyle` and numeric standalone attributes (`Float` / `Int`) must write straight into the
//! `NodeStore` word tables without touching the heap.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;

use counting_alloc::count_allocs;
use dioxus_core::{
    AttributeValue, ElementId, IntoAttributeValue, Template, TemplateNode, WriteMutations,
};
use nve_abi::{ColorRgba, SurfaceId};
use nve_dioxus::{NveMutationWriter, TypedStyle, NVE_TYPED_STYLE_ATTR};
use nve_store::{Length, Rgba};
use nve_tables::tables::{FlexDirection, Layout, Typography, Visual};

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

/// Pre-built typed attribute values (owned by Dioxus in real use).
struct Frame {
    typed_style: AttributeValue,
    width: AttributeValue,
    padding: AttributeValue,
    opacity: AttributeValue,
    z_index: AttributeValue,
    border_width: AttributeValue,
    font_size: AttributeValue,
}

impl Frame {
    fn apply(&self, writer: &mut NveMutationWriter) {
        writer.with_transaction(|tx| {
            tx.set_attribute(NVE_TYPED_STYLE_ATTR, None, &self.typed_style, EL1);
            tx.set_attribute("width", None, &self.width, EL1);
            tx.set_attribute("padding", None, &self.padding, EL1);
            tx.set_attribute("opacity", None, &self.opacity, EL1);
            tx.set_attribute("z-index", None, &self.z_index, EL1);
            tx.set_attribute("font-size", None, &self.font_size, EL1);
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

    let mut writer = NveMutationWriter::new(SurfaceId::ROOT);
    writer.with_transaction(|tx| {
        tx.load_template(VIEW_TEMPLATE, 0, ElementId(1));
        tx.load_template(VIEW_TEMPLATE, 0, EL1);
        tx.load_template(VIEW_TEMPLATE, 0, EL2);
        tx.append_children(ElementId(0), 3);
    });

    let blue = ColorRgba::rgb(59, 130, 246);
    let red = ColorRgba::rgb(239, 68, 68);
    let initial = Frame {
        typed_style: TypedStyle::col()
            .color(blue)
            .border(1.0, blue)
            .padding_xy(4.0, 4.0)
            .into_value(),
        width: AttributeValue::Float(100.0),
        padding: AttributeValue::Float(4.0),
        opacity: AttributeValue::Float(0.25),
        z_index: AttributeValue::Int(3),
        border_width: AttributeValue::Float(1.0),
        font_size: AttributeValue::Float(13.0),
    };
    let target = Frame {
        typed_style: TypedStyle::row()
            .color(red)
            .border(2.0, red)
            .padding_xy(12.0, 8.0)
            .into_value(),
        width: AttributeValue::Float(240.0),
        padding: AttributeValue::Float(8.0),
        opacity: AttributeValue::Float(0.75),
        z_index: AttributeValue::Int(12),
        border_width: AttributeValue::Float(5.0),
        font_size: AttributeValue::Float(16.0),
    };
    for _ in 0..3 {
        initial.apply(&mut writer);
        target.apply(&mut writer);
    }
    initial.apply(&mut writer);

    let (_, allocs) = count_allocs(|| target.apply(&mut writer));
    assert_eq!(
        allocs, 0,
        "warm direct-path TypedStyle + numeric standalone attrs must not allocate"
    );

    let el1 = writer.resolve_el(EL1).expect("el1 bound");
    let el2 = writer.resolve_el(EL2).expect("el2 bound");
    let store = writer.store();
    let layout = store.try_row::<Layout>(el1).expect("el1 layout");
    assert_eq!(layout.width(), Length::points(240.0));
    assert_eq!(layout.padding_top(), Length::points(8.0));
    assert_eq!(layout.padding_left(), Length::points(8.0));
    assert_eq!(layout.flex_direction(), FlexDirection::Row);
    let typography = store.try_row::<Typography>(el1).expect("el1 typography");
    assert_eq!(typography.color(), Rgba::rgb(239, 68, 68));
    assert_eq!(typography.font_size(), 16.0);
    let visual = store.try_row::<Visual>(el1).expect("el1 visual");
    assert_eq!(visual.border_width(), 2.0);
    assert_eq!(visual.border_color(), Rgba::rgb(239, 68, 68));
    assert_eq!(visual.opacity(), 0.75);
    assert_eq!(visual.z_index(), 12);
    let visual2 = store.try_row::<Visual>(el2).expect("el2 visual");
    assert_eq!(visual2.border_width(), 5.0);
}
