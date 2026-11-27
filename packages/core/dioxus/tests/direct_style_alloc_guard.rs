//! `CountingAlloc` zero-allocation guard for standalone typed CSS parsing (`nve-components`) and
//! the direct fast path of `NveMutationWriter` (`nve-dioxus`), covering named colors, `border`,
//! `border-width`, `flex`, keywords, and stack-formatted numeric attributes.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use dioxus_core::{AttributeValue, ElementId, WriteMutations};
use nve_abi::{ComponentKind, LayoutProps, NodeId, RawPropValueRef, StyleProps};
use nve_components::{apply_typed_attr_kind, TypedAttrKind};
use nve_dioxus::NveMutationWriter;
use nve_engine::{BackendApplyStats, BackendError, RenderSink};

struct CountingAlloc;

static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_allocs<F: FnOnce()>(f: F) -> usize {
    ALLOCS.store(0, Ordering::SeqCst);
    TRACK.store(true, Ordering::SeqCst);
    f();
    TRACK.store(false, Ordering::SeqCst);
    ALLOCS.load(Ordering::SeqCst)
}

#[derive(Default)]
struct NoopSink {
    layout_calls: usize,
    style_calls: usize,
}

impl RenderSink for NoopSink {
    fn begin_frame(&mut self, _: u32, _: u64) {}
    fn create_node(&mut self, _: NodeId, _: ComponentKind, _: Option<&str>) {}
    fn remove_node(&mut self, _: NodeId) {}
    fn set_children(&mut self, _: NodeId, _: &[NodeId]) {}
    fn set_layout(&mut self, _: NodeId, _: &LayoutProps) {
        self.layout_calls += 1;
    }
    fn set_style(&mut self, _: NodeId, _: &StyleProps) {
        self.style_calls += 1;
    }
    fn set_text(&mut self, _: NodeId, _: &str) {}
    fn set_event_mask(&mut self, _: NodeId, _: u64) {}
    fn set_raw_prop(&mut self, _: NodeId, _: &str, _: RawPropValueRef<'_>) {}
    fn dispatch_command(&mut self, _: NodeId, _: &str, _: &[u8]) {}
    fn commit_frame(&mut self, _: bool) -> Result<BackendApplyStats, BackendError> {
        Ok(BackendApplyStats::default())
    }
}

#[test]
fn direct_typed_css_parsing_and_writer_updates_perform_zero_heap_allocations() {
    // 0. Sanity check: verify `CountingAlloc` observes real heap allocations.
    let sanity = count_allocs(|| {
        let v: Vec<u8> = std::hint::black_box(Vec::with_capacity(64));
        std::hint::black_box(&v);
    });
    assert!(sanity > 0, "CountingAlloc must observe Vec allocation");

    // 1. `nve-components` parsers: named colors, border, border-width, flex, keywords.
    let mut l = LayoutProps::default();
    let mut s = StyleProps::default();
    let parse_allocs = count_allocs(|| {
        assert!(apply_typed_attr_kind(&mut l, &mut s, TypedAttrKind::Color, " RED ", |_| {}));
        assert!(apply_typed_attr_kind(&mut l, &mut s, TypedAttrKind::Background, "White", |_| {}));
        assert!(apply_typed_attr_kind(
            &mut l,
            &mut s,
            TypedAttrKind::Border,
            "2px SOLID CurrentColor",
            |_| {},
        ));
        assert!(apply_typed_attr_kind(
            &mut l,
            &mut s,
            TypedAttrKind::BorderWidth,
            "THICK",
            |_| {},
        ));
        assert!(apply_typed_attr_kind(&mut l, &mut s, TypedAttrKind::Flex, "AUTO", |_| {}));
        assert!(apply_typed_attr_kind(&mut l, &mut s, TypedAttrKind::Flex, "NONE", |_| {}));
        assert!(apply_typed_attr_kind(&mut l, &mut s, TypedAttrKind::Flex, "1 0 120px", |_| {}));
        assert!(apply_typed_attr_kind(&mut l, &mut s, TypedAttrKind::FlexDirection, "ROW", |_| {}));
        assert!(apply_typed_attr_kind(
            &mut l,
            &mut s,
            TypedAttrKind::JustifyContent,
            "CENTER",
            |_| {},
        ));
        assert!(apply_typed_attr_kind(
            &mut l,
            &mut s,
            TypedAttrKind::AlignItems,
            "FLEX-END",
            |_| {},
        ));
    });
    assert_eq!(parse_allocs, 0, "CSS parsers must perform zero heap allocations");

    // 2. Warm `NveMutationWriter` direct-path updates on plain non-layered `View` nodes
    // (including named colors, `border`, `border-width`, keywords, and stack-formatted Float/Int).
    static VIEW_TMPL: dioxus_core::Template = dioxus_core::Template {
        roots: &[dioxus_core::TemplateNode::Element {
            tag: "view",
            namespace: None,
            attrs: &[],
            children: &[],
        }],
        node_paths: &[],
        attr_paths: &[&[0]],
    };
    let mut writer = NveMutationWriter::new(1);
    let mut sink = NoopSink::default();
    let el1 = ElementId(1);
    let el2 = ElementId(2);
    {
        let mut tx = writer.begin_tx(&mut sink, 1, 1);
        tx.load_template(VIEW_TMPL, 0, el1);
        tx.load_template(VIEW_TMPL, 0, el2);
        tx.set_attribute("width", None, &AttributeValue::Text("100px".into()), el1);
        tx.set_attribute("color", None, &AttributeValue::Text("blue".into()), el1);
        tx.set_attribute("border", None, &AttributeValue::Text("1px solid".into()), el1);
        tx.set_attribute("border-width", None, &AttributeValue::Text("thin".into()), el2);
        tx.finish_frame(true).unwrap();
    }

    let w_val = AttributeValue::Text("240px".into());
    let pad_val = AttributeValue::Text("8px 12px".into());
    let col_val = AttributeValue::Text("RED".into());
    let brd_val = AttributeValue::Text("2px SOLID CurrentColor".into());
    let bw_val = AttributeValue::Text("THICK".into());
    let dir_val = AttributeValue::Text("ROW".into());
    let op_val = AttributeValue::Float(0.75);
    let zi_val = AttributeValue::Int(12);

    let direct_allocs = count_allocs(|| {
        let mut tx = writer.begin_tx(&mut sink, 1, 2);
        tx.set_attribute("width", None, &w_val, el1);
        tx.set_attribute("padding", None, &pad_val, el1);
        tx.set_attribute("color", None, &col_val, el1);
        tx.set_attribute("border", None, &brd_val, el1);
        tx.set_attribute("flex-direction", None, &dir_val, el1);
        tx.set_attribute("opacity", None, &op_val, el1);
        tx.set_attribute("z-index", None, &zi_val, el1);
        tx.set_attribute("border-width", None, &bw_val, el2);
        tx.finish_frame(true).unwrap();
    });
    assert_eq!(
        direct_allocs, 0,
        "warm direct-path standalone typed attrs (including named colors, border, and numeric Float/Int) must allocate 0 bytes"
    );
    assert!(sink.layout_calls >= 2);
    assert!(sink.style_calls >= 2);
}
