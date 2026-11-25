//! Pure-Rust Flexbox [`LayoutEngine`] backed by `taffy`.

use std::sync::Arc;

use nve_abi::{
    ComponentKind, Dimension, FlexAlign, FlexDirection, FlexJustify, LayoutProps, NodeId,
    StyleProps,
};
use nve_engine::{HeuristicTextMeasurer, LayoutEngine, LayoutRect, TextMeasurer, Viewport};
use taffy::prelude as tf;
use taffy::TraversePartialTree;

use crate::slot_map::NodeSlotMap;

#[derive(Debug, Clone)]
struct TaffyNodeContext {
    nve_id: NodeId,
    kind: ComponentKind,
    text: Option<String>,
    font_size: f32,
}

#[derive(Debug, Clone, Copy)]
struct TaffySlot {
    taffy_id: tf::NodeId,
    layout: LayoutProps,
    computed_rect: LayoutRect,
    rect_gen: u64,
}

/// Pure-Rust `taffy` Flexbox implementation of [`LayoutEngine`].
///
/// Supports pluggable [`TextMeasurer`] callbacks via `taffy`'s `compute_layout_with_measure`
/// so text nodes measure and wrap identically to native layout engines such as React Native Yoga.
pub struct TaffyLayoutEngine {
    taffy: tf::TaffyTree<TaffyNodeContext>,
    slots: NodeSlotMap<TaffySlot>,
    default_taffy_style: tf::Style,
    layout_gen: u64,
    last_root_viewport: Option<(NodeId, f32, f32)>,
    scratch_tids: Vec<tf::NodeId>,
    scratch_stack: Vec<(tf::NodeId, f32, f32)>,
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
            slots: NodeSlotMap::new(),
            default_taffy_style: to_taffy_style(&LayoutProps::default()),
            layout_gen: 0,
            last_root_viewport: None,
            scratch_tids: Vec::new(),
            scratch_stack: Vec::new(),
            text_measurer,
        }
    }

    #[inline]
    fn sync_node_style(&mut self, id: NodeId, override_size: Option<(f32, f32)>) {
        let Some(slot) = self.slots.get(id) else {
            return;
        };
        let tid = slot.taffy_id;
        let mut style = to_taffy_style(&slot.layout);
        if let Some((w, h)) = override_size {
            style.size = tf::Size {
                width: tf::Dimension::length(w),
                height: tf::Dimension::length(h),
            };
        }
        let _ = self.taffy.set_style(tid, style);
    }

    fn collect_rects(&mut self, root_tid: tf::NodeId) {
        self.layout_gen = self.layout_gen.wrapping_add(1);
        if self.layout_gen == 0 {
            for slot in self.slots.values_mut() {
                slot.rect_gen = 0;
            }
            self.layout_gen = 1;
        }
        let current_gen = self.layout_gen;

        self.scratch_stack.clear();
        self.scratch_stack.push((root_tid, 0.0, 0.0));
        while let Some((tid, origin_x, origin_y)) = self.scratch_stack.pop() {
            let Ok(layout) = self.taffy.layout(tid) else {
                continue;
            };
            let Some(nve_id) = self.taffy.get_node_context(tid).map(|c| c.nve_id) else {
                continue;
            };
            let rect = LayoutRect {
                x: origin_x + layout.location.x,
                y: origin_y + layout.location.y,
                width: layout.size.width,
                height: layout.size.height,
            };
            if let Some(slot) = self.slots.get_mut(nve_id) {
                slot.computed_rect = rect;
                slot.rect_gen = current_gen;
            }
            let count = self.taffy.child_count(tid);
            for i in (0..count).rev() {
                let child_tid = self.taffy.get_child_id(tid, i);
                self.scratch_stack.push((child_tid, rect.x, rect.y));
            }
        }
    }
}

impl LayoutEngine for TaffyLayoutEngine {
    fn name(&self) -> &'static str {
        "taffy"
    }

    fn upsert_node(&mut self, id: NodeId, kind: ComponentKind) {
        if let Some(slot) = self.slots.get(id) {
            let tid = slot.taffy_id;
            if let Some(ctx) = self.taffy.get_node_context_mut(tid)
                && ctx.kind != kind
            {
                ctx.kind = kind;
                let _ = self.taffy.mark_dirty(tid);
            }
            return;
        }
        let default_layout = LayoutProps::default();
        let ctx = TaffyNodeContext {
            nve_id: id,
            kind,
            text: None,
            font_size: StyleProps::default().font_size,
        };
        let tid = self
            .taffy
            .new_leaf_with_context(self.default_taffy_style.clone(), ctx)
            .expect("taffy leaf allocation");
        self.slots.insert(
            id,
            TaffySlot {
                taffy_id: tid,
                layout: default_layout,
                computed_rect: LayoutRect::default(),
                rect_gen: 0,
            },
        );
    }

    fn remove_node(&mut self, id: NodeId) {
        if let Some(slot) = self.slots.remove(id) {
            let _ = self.taffy.remove(slot.taffy_id);
        }
        if self.last_root_viewport.is_some_and(|(rid, _, _)| rid == id) {
            self.last_root_viewport = None;
        }
    }

    fn set_layout(&mut self, id: NodeId, layout: &LayoutProps) {
        let Some(slot) = self.slots.get_mut(id) else {
            return;
        };
        if slot.layout == *layout {
            return;
        }
        slot.layout = *layout;
        let tid = slot.taffy_id;
        if self.last_root_viewport.is_some_and(|(rid, _, _)| rid == id) {
            self.last_root_viewport = None;
        }
        let _ = self.taffy.set_style(tid, to_taffy_style(layout));
    }

    fn set_style(&mut self, id: NodeId, style: &StyleProps) {
        let Some(slot) = self.slots.get(id) else {
            return;
        };
        let tid = slot.taffy_id;
        let font_size = if style.font_size.is_finite() && style.font_size > 0.0 {
            style.font_size
        } else {
            StyleProps::default().font_size
        };
        if let Some(ctx) = self.taffy.get_node_context_mut(tid)
            && (ctx.font_size - font_size).abs() > f32::EPSILON
        {
            ctx.font_size = font_size;
            if ctx.kind == ComponentKind::Text {
                let _ = self.taffy.mark_dirty(tid);
            }
        }
    }

    fn set_text(&mut self, id: NodeId, text: &str) {
        let Some(slot) = self.slots.get(id) else {
            return;
        };
        let tid = slot.taffy_id;
        if let Some(ctx) = self.taffy.get_node_context_mut(tid)
            && ctx.text.as_deref() != Some(text)
        {
            match &mut ctx.text {
                Some(existing) => {
                    existing.clear();
                    existing.push_str(text);
                }
                None => {
                    ctx.text = Some(text.to_owned());
                }
            }
            if ctx.kind == ComponentKind::Text {
                let _ = self.taffy.mark_dirty(tid);
            }
        }
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        let Some(parent_tid) = self.slots.get(parent).map(|s| s.taffy_id) else {
            return;
        };
        self.scratch_tids.clear();
        self.scratch_tids.reserve(children.len());
        for &cid in children {
            if let Some(child_slot) = self.slots.get(cid) {
                self.scratch_tids.push(child_slot.taffy_id);
            }
        }
        let _ = self.taffy.set_children(parent_tid, &self.scratch_tids);
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.text_measurer = measurer;
        self.scratch_tids.clear();
        for slot in self.slots.values() {
            if self
                .taffy
                .get_node_context(slot.taffy_id)
                .is_some_and(|ctx| ctx.kind == ComponentKind::Text)
            {
                self.scratch_tids.push(slot.taffy_id);
            }
        }
        for &tid in &self.scratch_tids {
            let _ = self.taffy.mark_dirty(tid);
        }
    }

    fn compute_layout(&mut self, root: NodeId, viewport: Viewport) {
        let Some(root_tid) = self.slots.get(root).map(|s| s.taffy_id) else {
            self.layout_gen = self.layout_gen.wrapping_add(1);
            return;
        };
        let desired_vp = (root, viewport.width, viewport.height);
        if self.last_root_viewport != Some(desired_vp) {
            self.sync_node_style(root, Some((viewport.width, viewport.height)));
            self.last_root_viewport = Some(desired_vp);
        }

        let avail = tf::Size {
            width: tf::AvailableSpace::Definite(viewport.width),
            height: tf::AvailableSpace::Definite(viewport.height),
        };
        let measurer = &*self.text_measurer;
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

        self.collect_rects(root_tid);
    }

    #[inline]
    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        let slot = self.slots.get(id)?;
        (slot.rect_gen == self.layout_gen && self.layout_gen != 0).then_some(slot.computed_rect)
    }

    fn reset(&mut self) {
        self.taffy.clear();
        self.slots.clear();
        self.layout_gen = 0;
        self.last_root_viewport = None;
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
