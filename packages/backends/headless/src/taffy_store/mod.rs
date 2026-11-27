//! Direct Taffy 0.16 low-level layout (`LayoutPartialTree` / `CoreStyle`) over `NodeStore` columns.
//!
//! Eliminates `TaffyTree`, per-change 560 B `taffy::Style` rebuilds, `TaffySlot`, `TaffyNodeContext`,
//! and `unsafe impl Send`. Holds only dense `Vec<taffy::Cache>`, 16 B unrounded boxes, 16 B absolute
//! rounded rects, and resolved inherited direction/typography arrays.

mod layout_view;
mod style_view;

use std::sync::Arc;

use nve_abi::table::effect;
use nve_components::tables::{
    Direction as NveDirection, Layout as NveLayout, Position as NvePosition, TextContent,
    Typography,
};
use nve_engine::{HeuristicTextMeasurer, LayoutRect, StoreLayoutEngine, TextMeasurer, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba};

use taffy::geometry::Size;
use taffy::style::AvailableSpace;
use taffy::tree::{Cache, ClearState};
use taffy::compute_root_layout;

use layout_view::LayoutView;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct UnroundedBox {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
}

/// Inherited typography resolved once per layout pass for every node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedTextStyle {
    pub color: Rgba,
    pub font_size: f32,
    pub font_weight: u16,
    pub line_height: Length,
}

impl Default for ResolvedTextStyle {
    fn default() -> Self {
        Self {
            color: Rgba::BLACK,
            font_size: 16.0,
            font_weight: 400,
            line_height: Length::AUTO,
        }
    }
}

/// Direct Taffy 0.16 layout engine reading `NodeStore` word columns zero-copy.
pub struct TaffyStoreLayout {
    caches: Vec<Cache>,
    boxes: Vec<UnroundedBox>,
    rects: Vec<LayoutRect>,
    rect_gen: Vec<u64>,
    node_gens: Vec<u8>,
    resolved_rtl: Vec<bool>,
    resolved_text: Vec<ResolvedTextStyle>,
    hoisted: Vec<Vec<NodeId>>,
    layout_gen: u64,
    last_viewport: Option<(NodeId, f32, f32)>,
    measurer: Arc<dyn TextMeasurer>,
    /// Reusable stack for inheritance walk (`(node_id, parent_rtl, parent_text)`).
    inherit_stack: Vec<(NodeId, bool, ResolvedTextStyle)>,
    /// Reusable stack for fused rounding + absolute origin accumulation (`(id, px, py, hidden)`).
    round_stack: Vec<(NodeId, f32, f32, bool)>,
    /// Set when any structural / direction / typography change requires inheritance propagation.
    inherit_dirty: bool,
}

#[inline]
fn is_out_of_flow(store: &NodeStore, id: NodeId) -> bool {
    !crate::is_display_none(store, id)
        && store
            .try_row::<NveLayout>(id)
            .is_some_and(|l| matches!(l.position(), NvePosition::Absolute | NvePosition::Fixed))
}

// Compile-time guarantee: zero `unsafe impl Send`/`Sync` needed because no `CompactLength` is stored.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TaffyStoreLayout>();
};

impl Default for TaffyStoreLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl TaffyStoreLayout {
    pub fn new() -> Self {
        Self {
            caches: Vec::new(),
            boxes: Vec::new(),
            rects: Vec::new(),
            rect_gen: Vec::new(),
            node_gens: Vec::new(),
            resolved_rtl: Vec::new(),
            resolved_text: Vec::new(),
            hoisted: Vec::new(),
            layout_gen: 0,
            last_viewport: None,
            measurer: Arc::new(HeuristicTextMeasurer),
            inherit_stack: Vec::new(),
            round_stack: Vec::new(),
            inherit_dirty: true,
        }
    }

    pub fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.measurer = measurer;
        for c in &mut self.caches {
            c.clear();
        }
    }

    pub fn text_measurer(&self) -> &Arc<dyn TextMeasurer> {
        &self.measurer
    }

    pub fn reset(&mut self) {
        self.caches.clear();
        self.boxes.clear();
        self.rects.clear();
        self.rect_gen.clear();
        self.node_gens.clear();
        self.resolved_rtl.clear();
        self.resolved_text.clear();
        self.hoisted.clear();
        self.layout_gen = 0;
        self.last_viewport = None;
        self.inherit_dirty = true;
    }

    fn ensure_capacity(&mut self, cap: usize) {
        if self.caches.len() < cap {
            self.caches.resize_with(cap, Cache::new);
            self.boxes.resize(cap, UnroundedBox::default());
            self.rects.resize(cap, LayoutRect::default());
            self.rect_gen.resize(cap, 0);
            self.node_gens.resize(cap, 0);
            self.resolved_rtl.resize(cap, false);
            self.resolved_text.resize(cap, ResolvedTextStyle::default());
            self.hoisted.resize_with(cap, Vec::new);
        }
    }

    /// Invalidates cached Taffy entries for all layout/measure/structural changes in `changes`.
    pub fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        self.ensure_capacity(store.capacity());

        let layout_dir_changed = changes.table(NveLayout::slot()).is_some_and(|td| {
            let dir_bit = 1u32 << NveLayout::DIRECTION.word();
            !td.detached.is_empty() || td.masks.iter().any(|&m| m & dir_bit != 0)
        });
        if !changes.created.is_empty()
            || !changes.removed.is_empty()
            || !changes.children.is_empty()
            || changes.table(Typography::slot()).is_some()
            || layout_dir_changed
        {
            self.inherit_dirty = true;
        }

        for &id in &changes.removed {
            let idx = id.index();
            if idx < self.caches.len() {
                self.caches[idx].clear();
                self.boxes[idx] = UnroundedBox::default();
                self.rects[idx] = LayoutRect::default();
                self.rect_gen[idx] = 0;
                self.node_gens[idx] = 0;
                self.resolved_rtl[idx] = false;
                self.resolved_text[idx] = ResolvedTextStyle::default();
                self.hoisted[idx].clear();
            }
        }
        for &id in &changes.created {
            let idx = id.index();
            self.caches[idx].clear();
            self.boxes[idx] = UnroundedBox::default();
            self.rects[idx] = LayoutRect::default();
            self.rect_gen[idx] = 0;
            self.node_gens[idx] = id.generation();
            self.hoisted[idx].clear();
            self.mark_dirty(store, id);
        }
        for &id in &changes.children {
            self.mark_dirty(store, id);
        }
        for (&id, &fx) in changes.touched.iter().zip(changes.effects.iter()) {
            if fx & (effect::LAYOUT | effect::MEASURE) != 0 {
                self.mark_dirty(store, id);
            }
        }
    }

    /// Marks every live node dirty (used after full replica load or measurer swap).
    pub fn invalidate_all(&mut self, store: &NodeStore) {
        self.ensure_capacity(store.capacity());
        for c in &mut self.caches {
            c.clear();
        }
        self.inherit_dirty = true;
    }

    #[inline]
    fn mark_dirty(&mut self, store: &NodeStore, mut id: NodeId) {
        let mut steps = 0usize;
        while store.is_live(id) {
            steps += 1;
            debug_assert!(
                steps <= crate::MAX_TREE_DEPTH + 1,
                "parent cycle detected in mark_dirty at node {:?}",
                id
            );
            if steps > crate::MAX_TREE_DEPTH + 1 {
                break;
            }
            let idx = id.index();
            if idx >= self.caches.len() {
                break;
            }
            if matches!(self.caches[idx].clear(), ClearState::AlreadyEmpty) {
                break;
            }
            let Some(p) = store.parent(id) else {
                break;
            };
            id = p;
        }
    }

    fn propagate_inheritance(&mut self, store: &NodeStore, root: NodeId) {
        if !self.inherit_dirty {
            return;
        }
        self.inherit_dirty = false;
        self.inherit_stack.clear();
        self.inherit_stack
            .push((root, false, ResolvedTextStyle::default()));

        while let Some((id, parent_rtl, parent_text)) = self.inherit_stack.pop() {
            let idx = id.index();
            let rtl = if let Some(l) = store.try_row::<NveLayout>(id) {
                match l.direction() {
                    NveDirection::Inherit => parent_rtl,
                    NveDirection::Ltr => false,
                    NveDirection::Rtl => true,
                }
            } else {
                parent_rtl
            };

            let mut text = parent_text;
            if let Some(t) = store.try_row::<Typography>(id) {
                let set = t.set();
                if set & Typography::SET_COLOR != 0 || (set == 0 && t.color() != Rgba::BLACK) {
                    text.color = t.color();
                }
                if set & Typography::SET_FONT_SIZE != 0
                    || (set == 0 && t.font_size().to_bits() != 16.0f32.to_bits())
                {
                    text.font_size = t.font_size();
                }
                if set & Typography::SET_FONT_WEIGHT != 0 || (set == 0 && t.font_weight() != 400) {
                    text.font_weight = t.font_weight();
                }
                if set & Typography::SET_LINE_HEIGHT != 0
                    || (set == 0 && t.line_height() != Length::AUTO)
                {
                    text.line_height = t.line_height();
                }
            }

            let dir_changed = self.resolved_rtl[idx] != rtl;
            let text_measure_changed = self.resolved_text[idx].font_size.to_bits()
                != text.font_size.to_bits()
                || self.resolved_text[idx].font_weight != text.font_weight;

            self.node_gens[idx] = id.generation();
            self.resolved_rtl[idx] = rtl;
            self.resolved_text[idx] = text;

            if dir_changed || (text_measure_changed && store.has::<TextContent>(id)) {
                self.mark_dirty(store, id);
            }

            for &child in store.children(id).iter().rev() {
                self.inherit_stack.push((child, rtl, text));
            }
        }
    }

    /// Computes layout for `root` at `viewport` directly from `store` columns.
    pub fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) {
        self.ensure_capacity(store.capacity());
        if !store.is_live(root) {
            self.layout_gen = self.layout_gen.wrapping_add(1).max(1);
            return;
        }

        let vp_key = (root, viewport.width, viewport.height);
        if self.last_viewport != Some(vp_key) {
            self.caches[root.index()].clear();
            self.last_viewport = Some(vp_key);
        }

        self.propagate_inheritance(store, root);

        let mut view = LayoutView {
            store,
            root_index: root.index() as u32,
            viewport: (viewport.width, viewport.height),
            resolved_rtl: &self.resolved_rtl,
            resolved_text: &self.resolved_text,
            caches: &mut self.caches,
            boxes: &mut self.boxes,
            hoisted: &mut self.hoisted,
            measurer: &*self.measurer,
        };

        compute_root_layout(
            &mut view,
            taffy::NodeId::from(root.index() as u64),
            Size {
                width: AvailableSpace::Definite(viewport.width),
                height: AvailableSpace::Definite(viewport.height),
            },
        );

        // Fused cumulative rounding + absolute origin accumulation (honoring containing-block
        // hoisting for `position: absolute` and `position: fixed`).
        self.layout_gen = self.layout_gen.wrapping_add(1).max(1);
        let cur_gen = self.layout_gen;
        self.round_stack.clear();
        self.round_stack.push((root, 0.0, 0.0, false));
        while let Some((id, px, py, ancestor_hidden)) = self.round_stack.pop() {
            if !store.is_live(id) {
                continue;
            }
            let idx = id.index();
            let b = self.boxes[idx];
            let cx = px + b.x;
            let cy = py + b.y;
            let rx = cx.round();
            let ry = cy.round();
            let rw = (cx + b.w).round() - rx;
            let rh = (cy + b.h).round() - ry;
            self.rects[idx] = LayoutRect {
                x: rx,
                y: ry,
                width: rw,
                height: rh,
            };
            self.rect_gen[idx] = cur_gen;
            self.node_gens[idx] = id.generation();

            let self_hidden = ancestor_hidden || crate::is_display_none(store, id);

            if !self_hidden {
                for &child in self.hoisted[idx].iter().rev() {
                    if store.is_live(child) {
                        self.round_stack.push((child, cx, cy, false));
                    }
                }
            }
            for &child in store.children(id).iter().rev() {
                if !self_hidden && is_out_of_flow(store, child) {
                    continue;
                }
                self.round_stack.push((child, cx, cy, self_hidden));
            }
        }
    }

    #[inline]
    pub fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        let idx = id.index();
        (idx < self.rect_gen.len()
            && self.layout_gen != 0
            && self.rect_gen[idx] == self.layout_gen
            && self.node_gens[idx] == id.generation())
            .then(|| self.rects[idx])
    }

    #[inline]
    pub fn resolved_rtl(&self, id: NodeId) -> bool {
        let idx = id.index();
        self.node_gens.get(idx).copied() == Some(id.generation())
            && self.resolved_rtl.get(idx).copied().unwrap_or(false)
    }

    #[inline]
    pub fn resolved_text(&self, id: NodeId) -> ResolvedTextStyle {
        let idx = id.index();
        if self.node_gens.get(idx).copied() == Some(id.generation()) {
            self.resolved_text.get(idx).copied().unwrap_or_default()
        } else {
            ResolvedTextStyle::default()
        }
    }
}

impl StoreLayoutEngine for TaffyStoreLayout {
    fn name(&self) -> &'static str {
        "taffy"
    }

    fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        TaffyStoreLayout::sync_changes(self, store, changes);
    }

    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) {
        TaffyStoreLayout::compute_layout(self, store, root, viewport);
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        TaffyStoreLayout::layout_of(self, id)
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        TaffyStoreLayout::set_text_measurer(self, measurer);
    }

    fn reset(&mut self) {
        TaffyStoreLayout::reset(self);
    }
}

#[cfg(test)]
#[path = "../taffy_store_tests.rs"]
mod tests;
