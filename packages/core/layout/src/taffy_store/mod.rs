//! Direct Taffy 0.16 low-level layout (`LayoutPartialTree` / `CoreStyle`) over `NodeStore` columns.
//!
//! Eliminates `TaffyTree`, per-change 560 B `taffy::Style` rebuilds, `TaffySlot`, `TaffyNodeContext`,
//! and `unsafe impl Send`. Holds only dense `Vec<taffy::Cache>`, 16 B unrounded boxes, 16 B absolute
//! rounded rects, and resolved inherited direction/typography arrays.

mod contents;
mod layout_view;
mod node_lists;
mod style_view;

use std::sync::Arc;

use nve_abi::table::effect;
use nve_components::tables::{
    Direction as NveDirection, Layout as NveLayout, Position as NvePosition, TextContent,
    TextInput, Typography, is_display_contents, is_node_display_none,
};
use nve_engine::{LayoutRect, Viewport};

use crate::{
    HeuristicTextMeasurer, ResolvedTextStyle, StoreLayoutEngine, TextMeasurer,
    resolve_inherited_text,
};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};

use taffy::compute_root_layout;
use taffy::geometry::Size;
use taffy::style::AvailableSpace;
use taffy::tree::{Cache, ClearState};

use contents::BoxChildren;
use layout_view::LayoutView;
use node_lists::NodeLists;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct UnroundedBox {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
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
    /// Per slot: was `display: contents` at the last inheritance walk. A `display` / hidden-input
    /// change reshapes flattened box-children lists only when this flips.
    was_contents: Vec<bool>,
    /// Out-of-flow boxes per containing block, in one flat pooled-CSR buffer.
    hoisted: NodeLists,
    /// Taffy-visible children of parents with `display: contents` children.
    box_children: BoxChildren,
    layout_gen: u64,
    last_viewport: Option<(NodeId, f32, f32)>,
    measurer: Arc<dyn TextMeasurer>,
    /// Reusable stack for inheritance walk (`(node_id, parent_rtl, parent_text)`).
    inherit_stack: Vec<(NodeId, bool, ResolvedTextStyle)>,
    /// Reusable stack for fused rounding + absolute origin accumulation (`(id, px, py, hidden)`).
    round_stack: Vec<(NodeId, f32, f32, bool)>,
    /// Set when any structural / direction / typography change requires inheritance propagation.
    inherit_dirty: bool,
    /// Set when any cache entry was invalidated since the last full pass; when clear,
    /// `compute_layout` is a no-op (Taffy 0.16 re-runs the root positioning pass for unclaimed
    /// `position: fixed` boxes even on a full cache hit, allocating every frame).
    layout_dirty: bool,
    /// Root of the layout pass in flight; still set on entry only when that pass unwound.
    pass_root: Option<NodeId>,
}

#[inline]
fn is_out_of_flow(store: &NodeStore, id: NodeId) -> bool {
    // `display: contents` generates no box, so `position` does not take it out of flow: its
    // children flatten into the parent box in flow (CSS Display 3 §2.5).
    !is_node_display_none(store, id)
        && !is_display_contents(store, id)
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
            was_contents: Vec::new(),
            hoisted: NodeLists::default(),
            box_children: BoxChildren::default(),
            layout_gen: 0,
            last_viewport: None,
            measurer: Arc::new(HeuristicTextMeasurer),
            inherit_stack: Vec::new(),
            round_stack: Vec::new(),
            inherit_dirty: true,
            layout_dirty: true,
            pass_root: None,
        }
    }

    pub fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.measurer = measurer;
        for c in &mut self.caches {
            c.clear();
        }
        self.layout_dirty = true;
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
        self.was_contents.clear();
        self.pass_root = None;
        self.hoisted.reset();
        self.box_children.reset();
        self.layout_gen = 0;
        self.last_viewport = None;
        self.inherit_dirty = true;
        self.layout_dirty = true;
    }

    fn ensure_capacity(&mut self, cap: usize) {
        if self.caches.len() < cap {
            self.caches.resize_with(cap, Cache::new);
            self.boxes.resize(cap, UnroundedBox::default());
            self.rects.resize(cap, LayoutRect::default());
            self.rect_gen.resize(cap, 0);
            self.node_gens.resize(cap, 0);
            self.resolved_rtl.resize(cap, false);
            self.was_contents.resize(cap, false);
            self.resolved_text.resize(cap, ResolvedTextStyle::default());
        }
        self.hoisted.ensure_len(cap);
        self.box_children.ensure_len(cap);
    }

    /// Invalidates cached Taffy entries for all layout/measure/structural changes in `changes`.
    pub fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        self.ensure_capacity(store.capacity());

        // Direction is inherited; `display` / hidden-input changes reshape flattened box-children
        // lists only where `display: contents` is involved (a plain `display: none` toggle needs
        // no inheritance walk).
        // `direction` and `display` share one packed column word (one change-mask bit), so
        // the changed values decide.
        let layout_bits =
            (1u32 << NveLayout::DIRECTION.word()) | (1u32 << NveLayout::DISPLAY.word());
        let layout_reshaped = changes.table(NveLayout::slot()).is_some_and(|td| {
            // A detached `Layout` row may reset `direction` and `display` alike.
            !td.detached.is_empty()
                || td.nodes.iter().zip(&td.masks).any(|(&id, &m)| {
                    m & layout_bits != 0
                        && (self.direction_changed(store, id)
                            || self.display_change_reshapes_boxes(store, id))
                })
        });
        let input_kind_bit = 1u32 << TextInput::INPUT_KIND.word();
        let input_reshaped = changes.table(TextInput::slot()).is_some_and(|td| {
            td.nodes
                .iter()
                .zip(&td.masks)
                .filter(|&(_, &m)| m & input_kind_bit != 0)
                .map(|(&id, _)| id)
                .chain(td.detached.iter().copied())
                .any(|id| self.display_change_reshapes_boxes(store, id))
        });
        if !changes.created.is_empty()
            || !changes.removed.is_empty()
            || !changes.children.is_empty()
            || changes.table(Typography::slot()).is_some()
            || layout_reshaped
            || input_reshaped
        {
            self.inherit_dirty = true;
        }

        for &id in &changes.removed {
            let idx = id.index();
            if idx < self.caches.len() {
                self.layout_dirty = true;
                self.caches[idx].clear();
                self.boxes[idx] = UnroundedBox::default();
                self.rects[idx] = LayoutRect::default();
                self.rect_gen[idx] = 0;
                self.node_gens[idx] = 0;
                self.resolved_rtl[idx] = false;
                self.was_contents[idx] = false;
                self.resolved_text[idx] = ResolvedTextStyle::default();
                self.hoisted.release(idx);
                self.box_children.release(idx);
            }
        }
        for &id in &changes.created {
            if !store.is_live(id) {
                continue;
            }
            let idx = id.index();
            self.caches[idx].clear();
            self.boxes[idx] = UnroundedBox::default();
            self.rects[idx] = LayoutRect::default();
            self.rect_gen[idx] = 0;
            self.node_gens[idx] = id.generation();
            self.was_contents[idx] = false;
            self.hoisted.release(idx);
            self.box_children.release(idx);
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

    /// Whether `id`'s resolved direction differs from the one recorded by the last inheritance
    /// walk (its parent's recorded direction stands in for `inherit`; a parent whose own
    /// direction changed in the same frame triggers the walk itself).
    fn direction_changed(&self, store: &NodeStore, id: NodeId) -> bool {
        let parent_rtl = store
            .parent(id)
            .is_some_and(|p| self.resolved_rtl.get(p.index()).copied().unwrap_or(false));
        let rtl = match store.try_row::<NveLayout>(id).map(|l| l.direction()) {
            Some(NveDirection::Ltr) => false,
            Some(NveDirection::Rtl) => true,
            _ => parent_rtl,
        };
        self.resolved_rtl.get(id.index()).copied() != Some(rtl)
    }

    /// A `display` / hidden-input change on `id` reshapes box children only when it flips
    /// whether `id` is `display: contents` (any other value only changes how its own box lays
    /// out, which `mark_dirty` already covers).
    fn display_change_reshapes_boxes(&self, store: &NodeStore, id: NodeId) -> bool {
        is_display_contents(store, id)
            != self.was_contents.get(id.index()).copied().unwrap_or(false)
    }

    /// Marks every live node dirty (used after full replica load or measurer swap).
    pub fn invalidate_all(&mut self, store: &NodeStore) {
        // Full replica load: drop every per-node list so no dead span outlives its node; the
        // full pass below re-records hoisted lists and the inheritance walk rebuilds box children.
        self.hoisted.reset();
        self.box_children.reset();
        self.ensure_capacity(store.capacity());
        for c in &mut self.caches {
            c.clear();
        }
        // Inherited state restarts from defaults: the walk below the (possibly new) root
        // re-resolves its subtree, and slots outside it read like a fresh engine's.
        self.resolved_rtl.fill(false);
        self.resolved_text.fill(ResolvedTextStyle::default());
        self.was_contents.fill(false);
        // Every cache was dropped: nothing an unwound pass left behind can be hit.
        self.pass_root = None;
        self.inherit_dirty = true;
        self.layout_dirty = true;
    }

    #[inline]
    fn mark_dirty(&mut self, store: &NodeStore, mut id: NodeId) {
        self.layout_dirty = true;
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
            // An empty cache normally means every ancestor is already dirty, except on the
            // starting node (e.g. a former `display: contents` node, never laid out) and on
            // `display: contents` ancestors (never laid out, so always empty).
            if matches!(self.caches[idx].clear(), ClearState::AlreadyEmpty)
                && steps > 1
                && !is_display_contents(store, id)
            {
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

            let text = resolve_inherited_text(store, id, parent_text);

            let dir_changed = self.resolved_rtl[idx] != rtl;
            let text_measure_changed = !self.resolved_text[idx].measure_eq(&text);

            self.node_gens[idx] = id.generation();
            self.resolved_rtl[idx] = rtl;
            let contents = is_display_contents(store, id);
            self.was_contents[idx] = contents;
            self.resolved_text[idx] = text;

            if dir_changed || (text_measure_changed && store.has::<TextContent>(id)) {
                self.mark_dirty(store, id);
            }
            if id != root && contents {
                // Never laid out (Taffy only sees its nearest box ancestor's flattened list):
                // return any list recorded while it still generated a box.
                self.hoisted.release(idx);
                self.box_children.release(idx);
            } else {
                self.box_children.rebuild(store, id);
            }

            for &child in store.children(id).iter().rev() {
                self.inherit_stack.push((child, rtl, text));
            }
        }
        self.box_children.compact_if_fragmented();
    }

    /// Computes layout for `root` at `viewport` directly from `store` columns, self-gated per the
    /// [`StoreLayoutEngine::compute_layout`] contract: returns `false` without a pass when
    /// nothing was invalidated since the last one, `true` when rects may have changed.
    pub fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool {
        self.ensure_capacity(store.capacity());
        if !store.is_live(root) {
            // Every rect becomes stale; stay dirty so a live root recomputes.
            self.layout_gen = self.layout_gen.wrapping_add(1).max(1);
            self.layout_dirty = true;
            return true;
        }

        if let Some(unwound) = self.pass_root.take() {
            // The previous pass unwound (host `TextMeasurer` panic behind `ffi_catch_unwind`).
            // Taffy caches the root's in-flow layout *before* its positioning pass records the
            // root's hoisted `position: fixed` boxes, so a retry hitting that cache would skip
            // them for good: drop the unwound root's cache. (A `Drop` guard cannot do this:
            // `LayoutView` borrows `caches` for the whole pass.)
            if let Some(cache) = self.caches.get_mut(unwound.index()) {
                cache.clear();
            }
        }

        // Layout runs in logical points; `scale_factor` only maps the result to device pixels
        // downstream, so it is not part of the key.
        let vp_key = (root, viewport.width, viewport.height);
        if self.last_viewport != Some(vp_key) {
            if self.last_viewport.is_some_and(|(r, _, _)| r != root) {
                // A new root re-resolves inheritance from itself down and re-hoists fixed boxes
                // to itself. Caches (keyed by size inputs only) and hoisted lists recorded under
                // the old root would otherwise be hit or walked again: e.g. a node laid out with
                // root-pass inputs under the new root would keep the old root's fixed boxes.
                self.invalidate_all(store);
            }
            self.caches[root.index()].clear();
            self.last_viewport = Some(vp_key);
            self.layout_dirty = true;
        }

        self.propagate_inheritance(store, root);
        if !self.layout_dirty {
            // Nothing was invalidated since the last pass: every box and rounded rect is current.
            return false;
        }

        let mut view = LayoutView {
            store,
            root_index: root.index() as u32,
            viewport: (viewport.width, viewport.height),
            resolved_rtl: &self.resolved_rtl,
            resolved_text: &self.resolved_text,
            caches: &mut self.caches,
            boxes: &mut self.boxes,
            hoisted: &mut self.hoisted,
            box_children: &self.box_children,
            measurer: &*self.measurer,
        };

        self.pass_root = Some(root);
        compute_root_layout(
            &mut view,
            taffy::NodeId::from(root.index() as u64),
            Size {
                width: AvailableSpace::Definite(viewport.width),
                height: AvailableSpace::Definite(viewport.height),
            },
        );
        self.hoisted.compact_if_fragmented();

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
            // `display: contents` generates no box: a zero rect at the parent origin, which its
            // children (laid out by Taffy as the parent's) inherit unchanged.
            let b = if id != root && is_display_contents(store, id) {
                UnroundedBox::default()
            } else {
                self.boxes[idx]
            };
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

            let self_hidden = ancestor_hidden || is_node_display_none(store, id);

            if !self_hidden {
                for &child in self.hoisted.get(idx).iter().rev() {
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
        // Cleared only once the pass completed: a pass unwinding out of a host `TextMeasurer`
        // leaves the engine dirty (and `pass_root` armed) so the next frame retries cleanly.
        self.layout_dirty = false;
        self.pass_root = None;
        true
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

    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool {
        TaffyStoreLayout::compute_layout(self, store, root, viewport)
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
mod tests;

#[cfg(test)]
mod hoisted_tests;

#[cfg(test)]
mod contents_tests;

#[cfg(test)]
mod clean_frame_tests;
