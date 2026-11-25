//! Pure-Rust Flexbox & CSS Grid [`LayoutEngine`] backed by `taffy`.

use std::sync::Arc;

use nve_abi::{ComponentKind, LayoutProps, NodeId, StyleProps};
use nve_components::layout_ext::{LayoutDirection, LayoutExt};
use nve_engine::{HeuristicTextMeasurer, LayoutEngine, LayoutRect, TextMeasurer, Viewport};
use taffy::prelude as tf;
use taffy::util::ResolveOrZero;
use taffy::TraversePartialTree;

use crate::slot_map::NodeSlotMap;
pub use crate::taffy_style::{to_taffy_dim, to_taffy_lpa, to_taffy_style, to_taffy_style_with_ext};

#[derive(Debug, Clone)]
struct TaffyNodeContext {
    nve_id: NodeId,
    kind: ComponentKind,
    text: Option<String>,
    font_size: f32,
    font_weight: u16,
}

#[derive(Debug, Clone)]
struct TaffySlot {
    taffy_id: tf::NodeId,
    layout: LayoutProps,
    layout_ext: Option<Box<LayoutExt>>,
    resolved_direction: LayoutDirection,
    flex_wrap: tf::FlexWrap,
    computed_rect: LayoutRect,
    rect_gen: u64,
}

fn parse_raw_font_weight(raw: &str) -> u16 {
    let trimmed = raw.trim();
    if let Ok(n) = trimmed.parse::<u16>() {
        return n.clamp(100, 900);
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "thin" => 100,
        "extralight" | "ultralight" => 200,
        "light" => 300,
        "normal" | "regular" => 400,
        "medium" => 500,
        "semibold" | "demibold" => 600,
        "bold" => 700,
        "extrabold" | "ultrabold" => 800,
        "black" | "heavy" => 900,
        _ => 400,
    }
}

fn parse_raw_flex_wrap(raw: &str) -> tf::FlexWrap {
    match raw.trim().to_ascii_lowercase().as_str() {
        "wrap" => tf::FlexWrap::Wrap,
        "wrap-reverse" => tf::FlexWrap::WrapReverse,
        _ => tf::FlexWrap::NoWrap,
    }
}

/// Pure-Rust `taffy` Flexbox + CSS Grid implementation of [`LayoutEngine`].
///
/// Supports pluggable [`TextMeasurer`] callbacks via `taffy`'s `compute_layout_with_measure`,
/// CSS Grid tracks/placement, `flex-basis`, `aspect-ratio`, `position` + insets, and
/// inheritable RTL logical layout.
pub struct TaffyLayoutEngine {
    taffy: tf::TaffyTree<TaffyNodeContext>,
    slots: NodeSlotMap<TaffySlot>,
    default_taffy_style: tf::Style,
    layout_gen: u64,
    last_root_viewport: Option<(NodeId, f32, f32)>,
    direction_override_count: usize,
    direction_dirty: bool,
    scratch_tids: Vec<tf::NodeId>,
    scratch_stack: Vec<(tf::NodeId, f32, f32)>,
    scratch_dir_stack: Vec<(tf::NodeId, LayoutDirection)>,
    scratch_resync: Vec<NodeId>,
    text_measurer: Arc<dyn TextMeasurer>,
}

// SAFETY: `taffy::TaffyTree` stores `CompactLength` as a tagged `*const ()` solely to support
// CSS `calc()` handles. `TaffyLayoutEngine` only constructs numeric `length`/`percent`/`auto`/`fr`
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
            direction_override_count: 0,
            direction_dirty: false,
            scratch_tids: Vec::new(),
            scratch_stack: Vec::new(),
            scratch_dir_stack: Vec::new(),
            scratch_resync: Vec::new(),
            text_measurer,
        }
    }

    #[inline]
    fn sync_node_style(&mut self, id: NodeId, override_size: Option<(f32, f32)>) {
        let Some(slot) = self.slots.get(id) else {
            return;
        };
        let tid = slot.taffy_id;
        let mut style = to_taffy_style_with_ext(
            &slot.layout,
            slot.layout_ext.as_deref(),
            slot.resolved_direction,
        );
        style.flex_wrap = slot.flex_wrap;
        if let Some((w, h)) = override_size {
            style.size = tf::Size {
                width: tf::Dimension::length(w),
                height: tf::Dimension::length(h),
            };
        }
        let _ = self.taffy.set_style(tid, style);
    }

    /// Propagates inheritable `LayoutDirection` down from `root_tid` whenever any node declares an
    /// explicit direction override or tree structure changed while RTL overrides are active.
    fn propagate_directions(&mut self, root_id: NodeId, root_tid: tf::NodeId) {
        if !self.direction_dirty {
            return;
        }
        self.direction_dirty = false;
        self.scratch_dir_stack.clear();
        self.scratch_resync.clear();
        self.scratch_dir_stack.push((root_tid, LayoutDirection::Ltr));

        while let Some((tid, inherited_dir)) = self.scratch_dir_stack.pop() {
            let Some(nve_id) = self.taffy.get_node_context(tid).map(|c| c.nve_id) else {
                continue;
            };
            let effective = if let Some(slot) = self.slots.get_mut(nve_id) {
                let eff = slot
                    .layout_ext
                    .as_deref()
                    .and_then(|e| e.direction)
                    .unwrap_or(inherited_dir);
                if slot.resolved_direction != eff {
                    slot.resolved_direction = eff;
                    self.scratch_resync.push(nve_id);
                }
                eff
            } else {
                inherited_dir
            };
            let count = self.taffy.child_count(tid);
            for i in (0..count).rev() {
                let child_tid = self.taffy.get_child_id(tid, i);
                self.scratch_dir_stack.push((child_tid, effective));
            }
        }

        let vp = self.last_root_viewport;
        for i in 0..self.scratch_resync.len() {
            let nid = self.scratch_resync[i];
            let override_size = vp
                .filter(|&(rid, _, _)| rid == nid && nid == root_id)
                .map(|(_, w, h)| (w, h));
            self.sync_node_style(nid, override_size);
        }
        self.scratch_resync.clear();
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
            font_weight: 400,
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
                layout_ext: None,
                resolved_direction: LayoutDirection::Ltr,
                flex_wrap: tf::FlexWrap::NoWrap,
                computed_rect: LayoutRect::default(),
                rect_gen: 0,
            },
        );
    }

    fn remove_node(&mut self, id: NodeId) {
        if let Some(slot) = self.slots.remove(id) {
            if slot.layout_ext.as_deref().and_then(|e| e.direction).is_some() {
                self.direction_override_count = self.direction_override_count.saturating_sub(1);
                self.direction_dirty = true;
            }
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
        if self.last_root_viewport.is_some_and(|(rid, _, _)| rid == id) {
            self.last_root_viewport = None;
        }
        self.sync_node_style(id, None);
    }

    fn set_layout_ext(&mut self, id: NodeId, ext: Option<&LayoutExt>) {
        let next = ext
            .filter(|e| !e.is_default())
            .map(|e| Box::new(e.clone()));
        let Some(slot) = self.slots.get_mut(id) else {
            return;
        };
        if slot.layout_ext == next {
            return;
        }
        let had_dir = slot.layout_ext.as_deref().and_then(|e| e.direction);
        let next_dir = next.as_deref().and_then(|e| e.direction);
        if had_dir.is_some() && next_dir.is_none() {
            self.direction_override_count = self.direction_override_count.saturating_sub(1);
        } else if had_dir.is_none() && next_dir.is_some() {
            self.direction_override_count += 1;
        }
        if had_dir != next_dir {
            self.direction_dirty = true;
            if let Some(d) = next_dir {
                slot.resolved_direction = d;
            }
        }
        slot.layout_ext = next;
        if self.last_root_viewport.is_some_and(|(rid, _, _)| rid == id) {
            self.last_root_viewport = None;
        }
        self.sync_node_style(id, None);
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

    fn set_raw_layout_prop(&mut self, id: NodeId, key: &str, value: &str) {
        match key {
            "flex-wrap" | "flexWrap" => {
                let wrap = parse_raw_flex_wrap(value);
                let Some(slot) = self.slots.get_mut(id) else {
                    return;
                };
                if slot.flex_wrap != wrap {
                    slot.flex_wrap = wrap;
                    self.sync_node_style(id, None);
                }
            }
            "font-weight" | "fontWeight" => {
                let weight = parse_raw_font_weight(value);
                let Some(slot) = self.slots.get(id) else {
                    return;
                };
                let tid = slot.taffy_id;
                if let Some(ctx) = self.taffy.get_node_context_mut(tid)
                    && ctx.font_weight != weight
                {
                    ctx.font_weight = weight;
                    if ctx.kind == ComponentKind::Text {
                        let _ = self.taffy.mark_dirty(tid);
                    }
                }
            }
            _ => {}
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
        if self.direction_override_count > 0 {
            self.direction_dirty = true;
        }
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
        self.propagate_directions(root, root_tid);

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
                            if let Some(ratio) = style.aspect_ratio
                                && ratio.is_finite()
                                && ratio > 0.0
                            {
                                let pad = style
                                    .padding
                                    .resolve_or_zero(inputs.parent_size.width, |_, _| 0.0);
                                let border = style
                                    .border
                                    .resolve_or_zero(inputs.parent_size.width, |_, _| 0.0);
                                let pb_sum = (pad + border).sum_axes();
                                return match (known_dimensions.width, known_dimensions.height) {
                                    (Some(w), None) => tf::Size {
                                        width: (w - pb_sum.width).max(0.0),
                                        height: (w / ratio - pb_sum.height).max(0.0),
                                    },
                                    (None, Some(h)) => tf::Size {
                                        width: (h * ratio - pb_sum.width).max(0.0),
                                        height: (h - pb_sum.height).max(0.0),
                                    },
                                    _ => tf::Size::ZERO,
                                };
                            }
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
                        let (measured_w, measured_h) = measurer.measure_with_weight(
                            text,
                            ctx.font_size,
                            ctx.font_weight,
                            max_width,
                        );
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
        self.direction_override_count = 0;
        self.direction_dirty = false;
    }
}
