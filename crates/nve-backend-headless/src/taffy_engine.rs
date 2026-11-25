//! Pure-Rust Flexbox [`LayoutEngine`] backed by `taffy`.

use std::collections::HashMap;
use std::sync::Arc;

use nve_abi::{
    ComponentKind, Dimension, FlexAlign, FlexDirection, FlexJustify, LayoutProps, NodeId,
    StyleProps,
};
use nve_engine::{HeuristicTextMeasurer, LayoutEngine, LayoutRect, TextMeasurer, Viewport};
use taffy::prelude as tf;

#[derive(Debug, Clone)]
struct TaffyNodeContext {
    kind: ComponentKind,
    text: Option<String>,
    font_size: f32,
}

/// Pure-Rust `taffy` Flexbox implementation of [`LayoutEngine`].
///
/// Supports pluggable [`TextMeasurer`] callbacks via `taffy`'s `compute_layout_with_measure`
/// so text nodes measure and wrap identically to native layout engines such as React Native Yoga.
pub struct TaffyLayoutEngine {
    taffy: tf::TaffyTree<TaffyNodeContext>,
    taffy_ids: HashMap<NodeId, tf::NodeId>,
    layouts: HashMap<NodeId, LayoutProps>,
    children: HashMap<NodeId, Vec<NodeId>>,
    computed_rects: HashMap<NodeId, LayoutRect>,
    text_measurer: Arc<dyn TextMeasurer>,
}

// SAFETY: `taffy::TaffyTree` stores `CompactLength` as a tagged `*const ()` solely to support
// CSS `calc()` handles. `TaffyLayoutEngine` only constructs numeric `length`/`percent`/`auto`
// dimensions (never `calc()` pointers) and owns its `TaffyTree` exclusively with `Send` fields.
unsafe impl Send for TaffyLayoutEngine {}

impl Default for TaffyLayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TaffyLayoutEngine {
    pub fn new() -> Self {
        Self::with_text_measurer(Arc::new(HeuristicTextMeasurer))
    }

    pub fn with_text_measurer(text_measurer: Arc<dyn TextMeasurer>) -> Self {
        Self {
            taffy: tf::TaffyTree::new(),
            taffy_ids: HashMap::new(),
            layouts: HashMap::new(),
            children: HashMap::new(),
            computed_rects: HashMap::new(),
            text_measurer,
        }
    }

    fn sync_node_style(&mut self, id: NodeId, override_size: Option<(f32, f32)>) {
        let Some(&tid) = self.taffy_ids.get(&id) else {
            return;
        };
        let props = self.layouts.get(&id).copied().unwrap_or_default();
        let mut style = to_taffy_style(&props);
        if let Some((w, h)) = override_size {
            style.size = tf::Size {
                width: tf::Dimension::length(w),
                height: tf::Dimension::length(h),
            };
        }
        let _ = self.taffy.set_style(tid, style);
    }

    fn collect_rects(&mut self, id: NodeId, origin_x: f32, origin_y: f32) {
        let Some(&tid) = self.taffy_ids.get(&id) else {
            return;
        };
        let Ok(layout) = self.taffy.layout(tid) else {
            return;
        };
        let rect = LayoutRect {
            x: origin_x + layout.location.x,
            y: origin_y + layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        };
        self.computed_rects.insert(id, rect);
        if let Some(child_ids) = self.children.get(&id).cloned() {
            for child in child_ids {
                self.collect_rects(child, rect.x, rect.y);
            }
        }
    }
}

impl LayoutEngine for TaffyLayoutEngine {
    fn name(&self) -> &'static str {
        "taffy"
    }

    fn upsert_node(&mut self, id: NodeId, kind: ComponentKind) {
        if let Some(&tid) = self.taffy_ids.get(&id) {
            if let Some(ctx) = self.taffy.get_node_context_mut(tid) {
                if ctx.kind != kind {
                    ctx.kind = kind;
                    let _ = self.taffy.mark_dirty(tid);
                }
            }
            return;
        }
        let default_layout = LayoutProps::default();
        let ctx = TaffyNodeContext {
            kind,
            text: None,
            font_size: StyleProps::default().font_size,
        };
        let tid = self
            .taffy
            .new_leaf_with_context(to_taffy_style(&default_layout), ctx)
            .expect("taffy leaf allocation");
        self.taffy_ids.insert(id, tid);
        self.layouts.insert(id, default_layout);
    }

    fn remove_node(&mut self, id: NodeId) {
        if let Some(tid) = self.taffy_ids.remove(&id) {
            let _ = self.taffy.remove(tid);
        }
        self.layouts.remove(&id);
        self.children.remove(&id);
        self.computed_rects.remove(&id);
    }

    fn set_layout(&mut self, id: NodeId, layout: &LayoutProps) {
        self.layouts.insert(id, *layout);
        self.sync_node_style(id, None);
    }

    fn set_style(&mut self, id: NodeId, style: &StyleProps) {
        let Some(&tid) = self.taffy_ids.get(&id) else {
            return;
        };
        let font_size = if style.font_size.is_finite() && style.font_size > 0.0 {
            style.font_size
        } else {
            StyleProps::default().font_size
        };
        if let Some(ctx) = self.taffy.get_node_context_mut(tid) {
            if (ctx.font_size - font_size).abs() > f32::EPSILON {
                ctx.font_size = font_size;
                if ctx.kind == ComponentKind::Text {
                    let _ = self.taffy.mark_dirty(tid);
                }
            }
        }
    }

    fn set_text(&mut self, id: NodeId, text: &str) {
        let Some(&tid) = self.taffy_ids.get(&id) else {
            return;
        };
        if let Some(ctx) = self.taffy.get_node_context_mut(tid) {
            let changed = ctx.text.as_deref() != Some(text);
            ctx.text = Some(text.to_owned());
            if changed && ctx.kind == ComponentKind::Text {
                let _ = self.taffy.mark_dirty(tid);
            }
        }
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        self.children.insert(parent, children.to_vec());
        if let Some(&parent_tid) = self.taffy_ids.get(&parent) {
            let child_tids: Vec<tf::NodeId> = children
                .iter()
                .filter_map(|cid| self.taffy_ids.get(cid).copied())
                .collect();
            let _ = self.taffy.set_children(parent_tid, &child_tids);
        }
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.text_measurer = measurer;
        let dirty_ids: Vec<tf::NodeId> = self
            .taffy_ids
            .values()
            .copied()
            .filter(|&tid| {
                self.taffy
                    .get_node_context(tid)
                    .is_some_and(|ctx| ctx.kind == ComponentKind::Text)
            })
            .collect();
        for tid in dirty_ids {
            let _ = self.taffy.mark_dirty(tid);
        }
    }

    fn compute_layout(&mut self, root: NodeId, viewport: Viewport) {
        let Some(&root_tid) = self.taffy_ids.get(&root) else {
            self.computed_rects.clear();
            return;
        };
        self.sync_node_style(root, Some((viewport.width, viewport.height)));

        let avail = tf::Size {
            width: tf::AvailableSpace::Definite(viewport.width),
            height: tf::AvailableSpace::Definite(viewport.height),
        };
        let measurer = Arc::clone(&self.text_measurer);
        let _ = self.taffy.compute_layout_with_measure(
            root_tid,
            avail,
            |inputs, _taffy_id, node_ctx, style| {
                taffy::compute_leaf_layout(
                    inputs,
                    style,
                    |_, _| 0.0,
                    |known_dimensions, available_space| {
                        let Some(ctx) = node_ctx else {
                            return tf::Size::ZERO;
                        };
                        if ctx.kind != ComponentKind::Text {
                            return tf::Size::ZERO;
                        }
                        let Some(text) = ctx.text.as_deref() else {
                            return tf::Size::ZERO;
                        };
                        if text.is_empty() {
                            return tf::Size::ZERO;
                        }
                        let max_width = known_dimensions.width.or(match available_space.width {
                            tf::AvailableSpace::Definite(w) => Some(w),
                            tf::AvailableSpace::MinContent => Some(0.0),
                            tf::AvailableSpace::MaxContent => None,
                        });
                        let (measured_w, measured_h) =
                            measurer.measure(text, ctx.font_size, max_width);
                        let safe_w = if measured_w.is_finite() && measured_w >= 0.0 {
                            measured_w
                        } else {
                            0.0
                        };
                        let safe_h = if measured_h.is_finite() && measured_h >= 0.0 {
                            measured_h
                        } else {
                            0.0
                        };
                        tf::Size {
                            width: known_dimensions.width.unwrap_or(safe_w),
                            height: known_dimensions.height.unwrap_or(safe_h),
                        }
                    },
                )
            },
        );

        self.computed_rects.clear();
        self.collect_rects(root, 0.0, 0.0);
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.computed_rects.get(&id).copied()
    }

    fn reset(&mut self) {
        self.taffy.clear();
        self.taffy_ids.clear();
        self.layouts.clear();
        self.children.clear();
        self.computed_rects.clear();
    }
}

pub fn to_taffy_dim(d: Dimension) -> tf::Dimension {
    match d {
        Dimension::Auto => tf::Dimension::auto(),
        Dimension::Points(v) => tf::Dimension::length(v),
        Dimension::Percent(v) => tf::Dimension::percent(v / 100.0),
    }
}

pub fn to_taffy_lpa(d: Dimension) -> tf::LengthPercentageAuto {
    match d {
        Dimension::Auto => tf::LengthPercentageAuto::auto(),
        Dimension::Points(v) => tf::LengthPercentageAuto::length(v),
        Dimension::Percent(v) => tf::LengthPercentageAuto::percent(v / 100.0),
    }
}

pub fn to_taffy_style(l: &LayoutProps) -> tf::Style {
    use taffy::style::{
        AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignmentSafety,
    };
    let justify_kw = match l.justify_content {
        FlexJustify::FlexStart => AlignContentKeyword::FlexStart,
        FlexJustify::Center => AlignContentKeyword::Center,
        FlexJustify::FlexEnd => AlignContentKeyword::FlexEnd,
        FlexJustify::SpaceBetween => AlignContentKeyword::SpaceBetween,
        FlexJustify::SpaceAround => AlignContentKeyword::SpaceAround,
        FlexJustify::SpaceEvenly => AlignContentKeyword::SpaceEvenly,
    };
    let align_kw = match l.align_items {
        FlexAlign::Stretch => AlignItemsKeyword::Stretch,
        FlexAlign::FlexStart => AlignItemsKeyword::FlexStart,
        FlexAlign::Center => AlignItemsKeyword::Center,
        FlexAlign::FlexEnd => AlignItemsKeyword::FlexEnd,
        FlexAlign::Baseline => AlignItemsKeyword::Baseline,
    };
    tf::Style {
        display: tf::Display::Flex,
        flex_direction: match l.flex_direction {
            FlexDirection::Column => tf::FlexDirection::Column,
            FlexDirection::Row => tf::FlexDirection::Row,
            FlexDirection::ColumnReverse => tf::FlexDirection::ColumnReverse,
            FlexDirection::RowReverse => tf::FlexDirection::RowReverse,
        },
        justify_content: AlignContent {
            keyword: justify_kw,
            safety: AlignmentSafety::Default,
        },
        align_items: AlignItems {
            keyword: align_kw,
            safety: AlignmentSafety::Default,
        },
        flex_grow: l.flex_grow,
        flex_shrink: l.flex_shrink,
        gap: tf::Size {
            width: tf::LengthPercentage::length(l.gap),
            height: tf::LengthPercentage::length(l.gap),
        },
        size: tf::Size {
            width: to_taffy_dim(l.width),
            height: to_taffy_dim(l.height),
        },
        min_size: tf::Size {
            width: to_taffy_lpa(l.min_width),
            height: to_taffy_lpa(l.min_height),
        },
        max_size: tf::Size {
            width: to_taffy_lpa(l.max_width),
            height: to_taffy_lpa(l.max_height),
        },
        padding: tf::Rect {
            top: tf::LengthPercentage::length(l.padding.top),
            right: tf::LengthPercentage::length(l.padding.right),
            bottom: tf::LengthPercentage::length(l.padding.bottom),
            left: tf::LengthPercentage::length(l.padding.left),
        },
        margin: tf::Rect {
            top: tf::LengthPercentageAuto::length(l.margin.top),
            right: tf::LengthPercentageAuto::length(l.margin.right),
            bottom: tf::LengthPercentageAuto::length(l.margin.bottom),
            left: tf::LengthPercentageAuto::length(l.margin.left),
        },
        ..Default::default()
    }
}
