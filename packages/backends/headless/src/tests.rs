use super::*;
use nve_abi::{CommandBufferBuilder, Dimension, EdgeInsets, EventMask, FlexAlign, FlexDirection};
use nve_engine::DynLayoutEngine;

/// Minimal custom [`LayoutEngine`] stacking every child vertically in fixed-height bands.
struct BandStackLayout {
    band_height: f32,
    children: HashMap<NodeId, Vec<NodeId>>,
    texts: HashMap<NodeId, String>,
    font_sizes: HashMap<NodeId, f32>,
    kinds: HashMap<NodeId, ComponentKind>,
    rects: HashMap<NodeId, LayoutRect>,
    measurer: Arc<dyn TextMeasurer>,
}

impl BandStackLayout {
    fn new(band_height: f32) -> Self {
        Self {
            band_height,
            children: HashMap::new(),
            texts: HashMap::new(),
            font_sizes: HashMap::new(),
            kinds: HashMap::new(),
            rects: HashMap::new(),
            measurer: Arc::new(HeuristicTextMeasurer),
        }
    }

    fn layout_rec(&mut self, id: NodeId, x: f32, y: f32, width: f32) -> f32 {
        let is_text = self.kinds.get(&id) == Some(&ComponentKind::Text);
        let (node_w, node_h) = if is_text {
            let text = self.texts.get(&id).map(String::as_str).unwrap_or("");
            let fs = self.font_sizes.get(&id).copied().unwrap_or(16.0);
            self.measurer.measure(text, fs, Some(width))
        } else {
            (width, self.band_height)
        };

        let child_ids = self.children.get(&id).cloned().unwrap_or_default();
        let mut cursor_y = y;
        for child in child_ids {
            cursor_y += self.layout_rec(child, x + 10.0, cursor_y, (width - 20.0).max(0.0));
        }
        let total_h = if cursor_y > y { cursor_y - y } else { node_h };
        self.rects.insert(
            id,
            LayoutRect {
                x,
                y,
                width: node_w,
                height: total_h,
            },
        );
        total_h
    }
}

impl LayoutEngine for BandStackLayout {
    fn name(&self) -> &'static str {
        "band-stack"
    }

    fn upsert_node(&mut self, id: NodeId, kind: ComponentKind) {
        self.kinds.insert(id, kind);
    }

    fn remove_node(&mut self, id: NodeId) {
        self.kinds.remove(&id);
        self.children.remove(&id);
        self.texts.remove(&id);
        self.font_sizes.remove(&id);
        self.rects.remove(&id);
    }

    fn set_layout(&mut self, _id: NodeId, _layout: &LayoutProps) {}

    fn set_style(&mut self, id: NodeId, style: &StyleProps) {
        self.font_sizes.insert(id, style.font_size);
    }

    fn set_text(&mut self, id: NodeId, text: &str) {
        self.texts.insert(id, text.to_owned());
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        self.children.insert(parent, children.to_vec());
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.measurer = measurer;
    }

    fn compute_layout(&mut self, root: NodeId, viewport: Viewport) {
        self.rects.clear();
        let _ = self.layout_rec(root, 0.0, 0.0, viewport.width);
        if let Some(r) = self.rects.get_mut(&root) {
            r.width = viewport.width;
            r.height = viewport.height;
        }
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.rects.get(&id).copied()
    }

    fn reset(&mut self) {
        self.children.clear();
        self.texts.clear();
        self.font_sizes.clear();
        self.kinds.clear();
        self.rects.clear();
    }
}

fn build_sample_frame() -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Points(120.0),
            flex_direction: FlexDirection::Row,
            align_items: FlexAlign::FlexStart,
            padding: EdgeInsets::all(16.0),
            gap: 12.0,
            ..Default::default()
        },
    );
    b.create_node(3, ComponentKind::Pressable, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(44.0),
            ..Default::default()
        },
    );
    b.set_event_mask(3, EventMask::PRESS);
    b.create_node(4, ComponentKind::Text, None);
    b.set_text(4, "Tap me");
    b.set_style(
        4,
        &StyleProps {
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.set_children(3, &[4]);
    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    b.into_bytes()
}

#[test]
fn computes_flexbox_layout_and_hit_testing() {
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 400.0,
            height: 800.0,
            scale_factor: 2.0,
        },
    );

    let stats = backend.submit_frame(&build_sample_frame()).unwrap();
    assert!(stats.mutation_count >= 4);
    assert_eq!(backend.name(), "headless-taffy");

    let btn = backend.layout_of(3).unwrap();
    assert_eq!(btn.x, 16.0);
    assert_eq!(btn.y, 16.0);
    assert_eq!(btn.width, 100.0);
    assert_eq!(btn.height, 44.0);

    assert_eq!(backend.hit_test(50.0, 30.0, true), Some(3));
    assert_eq!(backend.hit_test(350.0, 30.0, true), None);
}

#[test]
fn supports_monomorphized_custom_layout_engine_and_live_swap() {
    let frame = build_sample_frame();
    let viewport = Viewport {
        width: 400.0,
        height: 800.0,
        scale_factor: 1.0,
    };

    // Start with Taffy
    let mut taffy_backend = HeadlessBackend::new();
    taffy_backend.set_viewport(1, viewport);
    taffy_backend.submit_frame(&frame).unwrap();
    assert_eq!(taffy_backend.layout_of(3).unwrap().x, 16.0);
    assert_eq!(taffy_backend.hit_test(50.0, 30.0, true), Some(3));

    // Live-swap to compile-time generic BandStackLayout: tree, styles, text, and hit-testing survive.
    let custom_backend: HeadlessBackend<BandStackLayout> =
        taffy_backend.swap_layout_engine(BandStackLayout::new(60.0));
    assert_eq!(custom_backend.name(), "headless-custom");
    assert_eq!(custom_backend.layout_engine().name(), "band-stack");
    assert_eq!(
        custom_backend.node(4).and_then(|n| n.text.as_deref()),
        Some("Tap me")
    );
    // BandStackLayout indents each nesting level by 10px: root(0) -> view2(10) -> btn3(20)
    let btn_custom = custom_backend.layout_of(3).unwrap();
    assert_eq!(btn_custom.x, 20.0);
    assert_eq!(btn_custom.y, 0.0);
    assert_eq!(custom_backend.hit_test(35.0, 10.0, true), Some(3));

    // Swap back to Taffy and verify original Flexbox geometry is restored identically.
    let restored: HeadlessBackend<TaffyLayoutEngine> =
        custom_backend.swap_layout_engine(TaffyLayoutEngine::new());
    let btn_restored = restored.layout_of(3).unwrap();
    assert_eq!(btn_restored.x, 16.0);
    assert_eq!(btn_restored.y, 16.0);
    assert_eq!(btn_restored.width, 100.0);
    assert_eq!(btn_restored.height, 44.0);
    assert_eq!(restored.hit_test(50.0, 30.0, true), Some(3));
}

#[test]
fn supports_runtime_boxed_dyn_layout_engine() {
    let frame = build_sample_frame();
    let viewport = Viewport {
        width: 400.0,
        height: 800.0,
        scale_factor: 1.0,
    };

    let boxed: DynLayoutEngine = Box::<TaffyLayoutEngine>::default();
    let mut backend: HeadlessBackend<DynLayoutEngine> = HeadlessBackend::with_layout(boxed);
    backend.set_viewport(1, viewport);
    backend.submit_frame(&frame).unwrap();
    assert_eq!(backend.layout_engine().name(), "taffy");
    assert_eq!(backend.layout_of(3).unwrap().x, 16.0);

    // Swap inside the same concrete type HeadlessBackend<DynLayoutEngine>!
    let swapped: HeadlessBackend<DynLayoutEngine> =
        backend.swap_layout_engine(Box::new(BandStackLayout::new(50.0)) as DynLayoutEngine);
    assert_eq!(swapped.layout_engine().name(), "band-stack");
    assert_eq!(swapped.layout_of(3).unwrap().x, 20.0);
}

#[test]
fn custom_text_measurer_propagates_into_taffy_and_survives_swap() {
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 400.0,
            scale_factor: 1.0,
        },
    );
    // Custom measurer returning fixed 123x45 for any non-empty text.
    backend.set_text_measurer(Arc::new(|text: &str, _fs: f32, _mw: Option<f32>| {
        if text.is_empty() {
            (0.0, 0.0)
        } else {
            (123.0, 45.0)
        }
    }));

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(260.0),
            flex_direction: FlexDirection::Column,
            align_items: FlexAlign::FlexStart,
            ..Default::default()
        },
    );
    b.create_node(3, ComponentKind::Text, None);
    b.set_text(3, "Measured");
    b.set_children(2, &[3]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);

    backend.submit_frame(b.finish()).unwrap();
    let rect = backend.layout_of(3).unwrap();
    assert_eq!(rect.width, 123.0);
    assert_eq!(rect.height, 45.0);

    // Swapping engines preserves the custom TextMeasurer!
    let swapped = backend.swap_layout_engine(BandStackLayout::new(40.0));
    let swapped_text_rect = swapped.layout_of(3).unwrap();
    assert_eq!(swapped_text_rect.width, 123.0);
    assert_eq!(swapped_text_rect.height, 45.0);
}

#[test]
fn rejects_cycles_duplicates_and_non_finite_scalars() {
    let mut backend = HeadlessBackend::new();

    // Self-cycle
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_children(2, &[2]);
    assert!(backend.submit_frame(b.finish()).is_err());

    // 2-node cycle
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.create_node(3, ComponentKind::View, None);
    b2.create_node(4, ComponentKind::View, None);
    b2.set_children(3, &[4]);
    b2.set_children(4, &[3]);
    assert!(backend.submit_frame(b2.finish()).is_err());

    // Root as child
    let mut b3 = CommandBufferBuilder::new(1, 3);
    b3.set_children(2, &[ROOT_NODE_ID]);
    assert!(backend.submit_frame(b3.finish()).is_err());

    // Duplicate child
    let mut b4 = CommandBufferBuilder::new(1, 4);
    b4.create_node(5, ComponentKind::View, None);
    b4.set_children(2, &[5, 5]);
    assert!(backend.submit_frame(b4.finish()).is_err());

    // NaN layout scalar
    let mut b5 = CommandBufferBuilder::new(1, 5);
    b5.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(f32::NAN),
            ..Default::default()
        },
    );
    assert!(backend.submit_frame(b5.finish()).is_err());
}

#[test]
fn inherits_text_styles_and_resets_when_unnested() {
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 400.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(100.0),
            ..Default::default()
        },
    );
    b.create_node(3, ComponentKind::Text, None);
    b.set_style(
        3,
        &StyleProps {
            text_color: ColorRgba::rgb(56, 189, 248),
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.create_node(4, ComponentKind::Text, None);
    b.set_text(4, "This is a long line that must wrap across multiple lines");
    b.set_children(ROOT_NODE_ID, &[2]);
    b.set_children(2, &[3]);
    b.set_children(3, &[4]);
    b.commit(true);

    backend.submit_frame(b.finish()).unwrap();
    let leaf = backend.node(4).unwrap();
    assert_eq!(leaf.style.text_color, ColorRgba::rgb(56, 189, 248));
    assert!((leaf.style.font_size - 20.0).abs() < 1e-3);
    // Wrapped in a 100pt box at 20pt font -> multiple lines (height > 2 * 27pt)
    assert!(leaf.computed_rect.width <= 100.5);
    assert!(leaf.computed_rect.height > 54.0);

    // Reparent leaf 4 directly under view 2 (no Text ancestor): inherited style resets to default.
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_children(3, &[]);
    b2.set_children(2, &[3, 4]);
    b2.commit(true);
    backend.submit_frame(b2.finish()).unwrap();
    let leaf_after = backend.node(4).unwrap();
    assert_eq!(leaf_after.style.text_color, ColorRgba::BLACK);
    assert!((leaf_after.style.font_size - 16.0).abs() < 1e-3);
}

#[test]
fn handles_sparse_high_node_ids_and_opt_out_mutation_tracking() {
    let mut backend = HeadlessBackend::new().with_mutation_tracking(false);
    backend.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );
    let high_parent = u32::MAX - 1;
    let high_child = u32::MAX;

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(high_parent, ComponentKind::View, None);
    b.set_layout(
        high_parent,
        &LayoutProps {
            width: Dimension::Points(120.0),
            height: Dimension::Points(60.0),
            padding: EdgeInsets::all(10.0),
            ..Default::default()
        },
    );
    b.create_node(high_child, ComponentKind::View, None);
    b.set_layout(
        high_child,
        &LayoutProps {
            width: Dimension::Points(40.0),
            height: Dimension::Points(20.0),
            ..Default::default()
        },
    );
    b.set_children(high_parent, &[high_child]);
    b.set_children(ROOT_NODE_ID, &[high_parent]);
    b.commit(true);

    let stats = backend.submit_frame(b.finish()).unwrap();
    assert_eq!(stats.mutation_count, 0);
    assert!(backend.last_mutations().is_empty());
    let child_rect = backend.layout_of(high_child).unwrap();
    assert_eq!(child_rect.x, 10.0);
    assert_eq!(child_rect.y, 10.0);
    assert_eq!(child_rect.width, 40.0);
    assert_eq!(child_rect.height, 20.0);

    // Cycle check must also work on sparse high IDs.
    let mut bad = CommandBufferBuilder::new(1, 2);
    bad.set_children(high_child, &[high_parent]);
    assert!(backend.submit_frame(bad.finish()).is_err());
}
