//! Real React Native Yoga C++20 [`StoreLayoutEngine`] implementation.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nve_tables::tables::{Layout, Scroll, TextContent, Typography, Visual};
use nve_engine::{LayoutRect, Viewport};
use nve_layout::{
    HeuristicTextMeasurer, MAX_TREE_DEPTH, ResolvedTextStyle, StoreLayoutEngine, TextMeasurer,
    resolve_inherited_text,
};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, TableSlot};

use crate::YogaUnavailableError;
use crate::ffi::{
    NveYogaLayoutExt, NveYogaLayoutProps, YGConfigRef, YGNodeRef, nve_yg_config_free,
    nve_yg_config_new, nve_yg_config_set_point_scale_factor, nve_yg_node_apply_layout,
    nve_yg_node_apply_layout_ext, nve_yg_node_calculate_layout, nve_yg_node_detach_and_clear,
    nve_yg_node_free, nve_yg_node_get_layout_rect, nve_yg_node_mark_dirty,
    nve_yg_node_new_with_config, nve_yg_node_set_children, nve_yg_node_set_fixed_size,
};
use crate::measure::{
    RawMeasureState, YogaNodeEntry, node_depth_within_limit, preserve_scroll_child_intrinsic_size,
    typography_measure_word_mask,
};

/// Real React Native Yoga C++20 implementation of [`StoreLayoutEngine`].
pub struct YogaLayoutEngine {
    config: YGConfigRef,
    nodes: HashMap<NodeId, YogaNodeEntry>,
    computed_rects: HashMap<NodeId, LayoutRect>,
    text_measurer: Arc<dyn TextMeasurer>,
    measurer_panicked: Arc<AtomicBool>,
    child_scratch: Vec<YGNodeRef>,
    /// Reusable stack for inherited typography propagation `(node_id, style, depth)`.
    typo_stack: Vec<(NodeId, ResolvedTextStyle, usize)>,
    /// Reusable stack for iterative bounded subtree syncing `(node_id, depth, children_pushed)`.
    sync_stack: Vec<(NodeId, usize, bool)>,
    /// Reusable stack for iterative bounded rect collection `(node_id, ox, oy, depth)`.
    rect_stack: Vec<(NodeId, f32, f32, usize)>,
    /// Monotonic per-`sync_changes` epoch used to avoid redundant FFI prop/child re-syncs.
    sync_epoch: u32,
    /// Whether the active root's full subtree has been synced after `new` / `reset`.
    tree_seeded: bool,
    /// Set when structure or layout/measure `Typography` changed so text nodes refresh metrics.
    typo_dirty: bool,
    /// `StoreLayoutEngine::compute_layout` self-gate: set by every layout-affecting change,
    /// measurer swap, caught panic, and reset; cleared only after a clean completed pass.
    layout_dirty: bool,
    /// `(root, width, height, scale)` of the last completed pass.
    last_pass: Option<(NodeId, f32, f32, f32)>,
}

// SAFETY: `YogaLayoutEngine` exclusively owns its `YGConfigRef`, all `YGNodeRef`s, and every
// `RawMeasureState` (`TextMeasurer: Send + Sync`). Yoga nodes have no thread affinity and
// are never shared across threads concurrently (`&mut self` required for all mutations).
unsafe impl Send for YogaLayoutEngine {}

impl YogaLayoutEngine {
    pub const fn is_available() -> bool {
        true
    }

    pub fn new() -> Result<Self, YogaUnavailableError> {
        Self::with_text_measurer(Arc::new(HeuristicTextMeasurer))
    }

    pub fn with_text_measurer(
        text_measurer: Arc<dyn TextMeasurer>,
    ) -> Result<Self, YogaUnavailableError> {
        // SAFETY: Allocates a fresh `YGConfigRef` freed in `Drop`.
        let config = unsafe { nve_yg_config_new() };
        if config.is_null() {
            return Err(YogaUnavailableError::with_message(
                "React Native Yoga YGConfigNew returned null",
            ));
        }
        Ok(Self {
            config,
            nodes: HashMap::new(),
            computed_rects: HashMap::new(),
            text_measurer,
            measurer_panicked: Arc::new(AtomicBool::new(false)),
            child_scratch: Vec::new(),
            typo_stack: Vec::new(),
            sync_stack: Vec::new(),
            rect_stack: Vec::new(),
            sync_epoch: 0,
            tree_seeded: false,
            typo_dirty: true,
            layout_dirty: true,
            last_pass: None,
        })
    }

    fn next_sync_epoch(&mut self) -> u32 {
        self.sync_epoch = self.sync_epoch.wrapping_add(1);
        if self.sync_epoch == 0 {
            for entry in self.nodes.values_mut() {
                entry.synced_epoch = 0;
                entry.children_synced_epoch = 0;
            }
            self.sync_epoch = 1;
        }
        self.sync_epoch
    }

    fn ensure_node(&mut self, store: &NodeStore, id: NodeId, epoch: u32) -> bool {
        if let Some(entry) = self.nodes.get(&id) {
            if epoch != 0 && entry.synced_epoch == epoch {
                return true;
            }
        } else {
            // SAFETY: `self.config` is a live `YGConfigRef` owned by `self`.
            let yg_node = unsafe { nve_yg_node_new_with_config(self.config) };
            if yg_node.is_null() {
                self.layout_dirty = true;
                return false;
            }
            self.nodes.insert(id, YogaNodeEntry::new(yg_node));
        }
        self.sync_node_props(store, id, epoch);
        true
    }

    fn sync_node_props(&mut self, store: &NodeStore, id: NodeId, epoch: u32) {
        let Some(entry) = self.nodes.get_mut(&id) else {
            return;
        };
        let hidden_input = nve_tables::tables::is_hidden_text_input(store, id);
        let layout = store.try_row::<Layout>(id);
        let visual = store.try_row::<Visual>(id);
        let mut props = if let Some(ref l) = layout {
            NveYogaLayoutProps::from_store_layout(l)
        } else {
            NveYogaLayoutProps {
                flex_shrink: 1.0,
                ..Default::default()
            }
        };
        let preserve = preserve_scroll_child_intrinsic_size(store, id);
        entry.shrink_preserved = preserve;
        if props.flex_shrink > 0.0 && preserve {
            props.flex_shrink = 0.0;
        }
        let mut ext = NveYogaLayoutExt::from_store(layout.as_ref(), visual.as_ref());
        if hidden_input {
            ext.display = 1;
        }
        // SAFETY: `entry.yg_node` is a valid Yoga node owned by `self.nodes`; `props` and
        // `ext` are valid stack references.
        unsafe {
            nve_yg_node_apply_layout(entry.yg_node, &props);
            nve_yg_node_apply_layout_ext(entry.yg_node, &ext);
        }

        let text_row = store.try_row::<TextContent>(id);
        let is_text = text_row.is_some();
        if is_text {
            entry.is_text = true;
            let state = entry.measure_state.get_or_insert_with(|| {
                RawMeasureState::new(
                    Arc::clone(&self.text_measurer),
                    Arc::clone(&self.measurer_panicked),
                )
            });
            let text = text_row.map(|t| t.text()).unwrap_or("");
            let text_changed = state.with_payload_mut(|payload| {
                if payload.text != text {
                    payload.text.clear();
                    payload.text.push_str(text);
                    true
                } else {
                    false
                }
            });
            if !entry.measure_attached && store.children(id).is_empty() {
                entry.attach_measure_hook(&self.text_measurer, &self.measurer_panicked);
            }
            if text_changed && entry.measure_attached {
                // SAFETY: `entry.yg_node` is a live owned Yoga node.
                unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
            }
        } else if entry.is_text || entry.measure_state.is_some() {
            entry.is_text = false;
            entry.detach_and_free_measure_state();
        }
        if epoch != 0 {
            entry.synced_epoch = epoch;
        }
    }

    /// Walks one table's delta in linear `O(changed rows)` time and re-syncs direct children
    /// whose cached `shrink_preserved` flag flipped.
    fn resync_table_delta_children(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        slot: TableSlot,
        word_mask: u32,
        epoch: u32,
    ) {
        let Some(td) = changes.table(slot) else {
            return;
        };
        for (&id, &m) in td.nodes.iter().zip(&td.masks) {
            if m & word_mask != 0 && store.is_live(id) && !store.children(id).is_empty() {
                self.resync_children_if_shrink_changed(store, id, epoch);
            }
        }
        for &id in &td.detached {
            if store.is_live(id) && !store.children(id).is_empty() {
                self.resync_children_if_shrink_changed(store, id, epoch);
            }
        }
    }

    /// Re-syncs direct children of `parent` ONLY when a child's cached `shrink_preserved` bit
    /// actually flips (avoiding O(children) FFI fan-out when appending to a large container).
    fn resync_children_if_shrink_changed(&mut self, store: &NodeStore, parent: NodeId, epoch: u32) {
        for &cid in store.children(parent) {
            let Some(entry) = self.nodes.get(&cid) else {
                continue;
            };
            if epoch != 0 && entry.synced_epoch == epoch {
                continue;
            }
            if preserve_scroll_child_intrinsic_size(store, cid) != entry.shrink_preserved {
                self.sync_node_props(store, cid, epoch);
            }
        }
    }

    /// Propagates inherited `Typography` (every measure-affecting field) from `root` down into
    /// every text node's Yoga measure payload via shared [`resolve_inherited_text`].
    fn propagate_typography(&mut self, store: &NodeStore, root: NodeId) {
        if !self.typo_dirty {
            return;
        }
        self.typo_dirty = false;
        self.typo_stack.clear();
        self.typo_stack
            .push((root, ResolvedTextStyle::default(), 0));

        while let Some((id, parent_text, depth)) = self.typo_stack.pop() {
            if depth > MAX_TREE_DEPTH || !store.is_live(id) {
                continue;
            }
            let text = resolve_inherited_text(store, id, parent_text);
            if let Some(entry) = self.nodes.get_mut(&id)
                && let Some(state) = entry.measure_state.as_mut()
            {
                let resolved = text.in_store(store);
                let changed = state.with_payload_mut(|payload| {
                    if payload.style.borrowed().measure_eq(&resolved) {
                        // Paint-only edits (`color`) never re-measure or relayout.
                        payload.style.color = resolved.color;
                        return false;
                    }
                    payload.style = resolved.map_family(|f| (*f).to_owned());
                    true
                });
                if changed {
                    self.layout_dirty = true;
                    // SAFETY: `entry.yg_node` is a live owned Yoga node.
                    unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                }
            }
            if depth < MAX_TREE_DEPTH {
                for &child in store.children(id).iter().rev() {
                    self.typo_stack.push((child, text, depth + 1));
                }
            }
        }
    }

    fn sync_children_of(&mut self, store: &NodeStore, parent: NodeId, epoch: u32) {
        let Some((parent_yg, already_synced)) = self
            .nodes
            .get(&parent)
            .map(|e| (e.yg_node, epoch != 0 && e.children_synced_epoch == epoch))
        else {
            return;
        };
        if already_synced {
            return;
        }
        self.child_scratch.clear();
        for &cid in store.children(parent) {
            if let Some(child_entry) = self.nodes.get(&cid) {
                self.child_scratch.push(child_entry.yg_node);
            }
        }
        let has_children = !self.child_scratch.is_empty();
        // SAFETY: `parent_yg` and every handle in `self.child_scratch` are live owned Yoga
        // nodes in `self.nodes`; the slice pointer is valid for `self.child_scratch.len()`.
        unsafe {
            nve_yg_node_set_children(
                parent_yg,
                self.child_scratch.as_ptr(),
                self.child_scratch.len(),
            );
        }
        if let Some(entry) = self.nodes.get_mut(&parent) {
            if epoch != 0 {
                entry.children_synced_epoch = epoch;
            }
            if has_children {
                // `nve_yg_node_set_children` cleared Yoga's measure hook so Yoga permits children.
                entry.measure_attached = false;
            } else if entry.is_text && !entry.measure_attached {
                // Re-attach the text measure hook when all temporary children are removed.
                entry.attach_measure_hook(&self.text_measurer, &self.measurer_panicked);
                // SAFETY: `entry.yg_node` is a live owned Yoga node with a freshly attached hook.
                unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                self.typo_dirty = true;
            }
        }
    }

    /// Iteratively seeds `root` and all live descendants up to [`MAX_TREE_DEPTH`].
    fn sync_subtree_if_unseen(&mut self, store: &NodeStore, root: NodeId) {
        let epoch = self.next_sync_epoch();
        self.sync_stack.clear();
        self.sync_stack.push((root, 0, false));
        while let Some((id, depth, children_pushed)) = self.sync_stack.pop() {
            if depth > MAX_TREE_DEPTH || !store.is_live(id) {
                continue;
            }
            if children_pushed {
                self.sync_children_of(store, id, epoch);
                continue;
            }
            if !self.ensure_node(store, id, epoch) {
                continue;
            }
            let children = store.children(id);
            if !children.is_empty() && depth < MAX_TREE_DEPTH {
                self.sync_stack.push((id, depth, true));
                for &child in children.iter().rev() {
                    self.sync_stack.push((child, depth + 1, false));
                }
            } else {
                self.sync_children_of(store, id, epoch);
            }
        }
    }

    fn clear_all_nodes(&mut self) {
        for entry in self.nodes.values() {
            // SAFETY: Detaches every live Yoga node from its parent/children and clears its
            // measure callback before any node is freed in the next loop.
            unsafe {
                nve_yg_node_detach_and_clear(entry.yg_node);
            }
        }
        for (_, mut entry) in self.nodes.drain() {
            // SAFETY: Every `yg_node` was detached above and is freed exactly once here,
            // before its `measure_state` is reclaimed.
            unsafe {
                nve_yg_node_free(entry.yg_node);
            }
            if let Some(state) = entry.measure_state.take() {
                // SAFETY: `yg_node` has been detached and freed above.
                unsafe { state.free_after_detach() };
            }
        }
        self.computed_rects.clear();
    }

    /// Iteratively collects absolute surface-space rects up to [`MAX_TREE_DEPTH`].
    fn collect_rects(&mut self, store: &NodeStore, root: NodeId) {
        self.rect_stack.clear();
        self.rect_stack.push((root, 0.0, 0.0, 0));
        while let Some((id, origin_x, origin_y, depth)) = self.rect_stack.pop() {
            if depth > MAX_TREE_DEPTH {
                continue;
            }
            let Some(entry) = self.nodes.get(&id) else {
                continue;
            };
            // SAFETY: `entry.yg_node` is a live owned Yoga node after `calculate_layout`.
            let raw = unsafe { nve_yg_node_get_layout_rect(entry.yg_node) };
            let rect = LayoutRect {
                x: origin_x + raw.x,
                y: origin_y + raw.y,
                width: raw.width,
                height: raw.height,
            };
            self.computed_rects.insert(id, rect);
            if depth < MAX_TREE_DEPTH {
                for &child in store.children(id).iter().rev() {
                    self.rect_stack.push((child, rect.x, rect.y, depth + 1));
                }
            }
        }
    }
}

impl Drop for YogaLayoutEngine {
    fn drop(&mut self) {
        self.clear_all_nodes();
        if !self.config.is_null() {
            // SAFETY: All nodes using `self.config` were freed by `clear_all_nodes()` above;
            // freeing `self.config` once here leaves no dangling references.
            unsafe {
                nve_yg_config_free(self.config);
            }
            self.config = std::ptr::null_mut();
        }
    }
}

impl StoreLayoutEngine for YogaLayoutEngine {
    fn name(&self) -> &'static str {
        "yoga"
    }

    fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        let layout_effect = nve_store::effect::LAYOUT | nve_store::effect::MEASURE;
        let typo_measure_mask = typography_measure_word_mask(layout_effect);
        let typo_changed = changes.table(Typography::slot()).is_some_and(|td| {
            !td.detached.is_empty() || td.masks.iter().any(|&m| m & typo_measure_mask != 0)
        });
        let scroll_axes_mask = 1u32 << Scroll::AXES.word();
        let scroll_axes_changed = changes.table(Scroll::slot()).is_some_and(|td| {
            !td.detached.is_empty() || td.masks.iter().any(|&m| m & scroll_axes_mask != 0)
        });
        if !changes.created.is_empty()
            || !changes.removed.is_empty()
            || !changes.children.is_empty()
            || typo_changed
        {
            self.typo_dirty = true;
        }
        let any_layout_touch =
            scroll_axes_changed || changes.effects.iter().any(|&fx| fx & layout_effect != 0);
        if !changes.created.is_empty()
            || !changes.removed.is_empty()
            || !changes.children.is_empty()
            || any_layout_touch
        {
            self.layout_dirty = true;
        }

        // Fast-path clean or paint-only frames with zero FFI work or epoch churn.
        if changes.removed.is_empty()
            && changes.created.is_empty()
            && changes.children.is_empty()
            && !any_layout_touch
        {
            return;
        }

        let epoch = self.next_sync_epoch();

        for &id in &changes.removed {
            if let Some(entry) = self.nodes.remove(&id) {
                entry.detach_and_free_node();
            }
            self.computed_rects.remove(&id);
        }

        for &id in &changes.created {
            if store.is_live(id) && node_depth_within_limit(store, id).is_some() {
                let _ = self.ensure_node(store, id, epoch);
            }
        }
        // Attach children already present on newly created nodes (e.g. live engine swap or
        // snapshot hydrate where `changes.created` lists the live tree without `changes.children`).
        for &id in &changes.created {
            if store.is_live(id)
                && !store.children(id).is_empty()
                && node_depth_within_limit(store, id).is_some_and(|d| d < MAX_TREE_DEPTH)
            {
                self.sync_children_of(store, id, epoch);
            }
        }

        for (&id, &fx) in changes.touched.iter().zip(&changes.effects) {
            if fx & layout_effect == 0
                || !store.is_live(id)
                || node_depth_within_limit(store, id).is_none()
            {
                continue;
            }
            let _ = self.ensure_node(store, id, epoch);
        }

        // Linear O(changed rows) check over the three tables that can flip a parent's
        // `preserve_scroll_child_intrinsic_size` result for its direct children.
        self.resync_table_delta_children(store, changes, Scroll::slot(), scroll_axes_mask, epoch);
        self.resync_table_delta_children(
            store,
            changes,
            Layout::slot(),
            1u32 << Layout::FLEX_DIRECTION.word(),
            epoch,
        );
        self.resync_table_delta_children(
            store,
            changes,
            Visual::slot(),
            (1u32 << Visual::OVERFLOW_X.word()) | (1u32 << Visual::OVERFLOW_Y.word()),
            epoch,
        );

        for &parent in &changes.children {
            if !store.is_live(parent) {
                continue;
            }
            let Some(depth) = node_depth_within_limit(store, parent) else {
                continue;
            };
            let _ = self.ensure_node(store, parent, epoch);
            if depth < MAX_TREE_DEPTH {
                if !store.children(parent).is_empty() {
                    self.resync_children_if_shrink_changed(store, parent, epoch);
                }
                self.sync_children_of(store, parent, epoch);
            }
        }
    }

    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool {
        if !store.is_live(root) {
            self.computed_rects.clear();
            self.layout_dirty = true;
            return true;
        }
        let pass_key = (root, viewport.width, viewport.height, viewport.scale_factor);
        if let Some((prev_root, _, _, _)) = self.last_pass
            && prev_root != root
        {
            // Restore the previous root's own `Layout` width/height (undoing fixed viewport size).
            if store.is_live(prev_root) && self.nodes.contains_key(&prev_root) {
                self.sync_node_props(store, prev_root, 0);
            }
            self.tree_seeded = false;
            self.typo_dirty = true;
            self.layout_dirty = true;
        }
        if !self.tree_seeded || !self.nodes.contains_key(&root) {
            self.sync_subtree_if_unseen(store, root);
            self.tree_seeded = true;
            self.typo_dirty = true;
            self.layout_dirty = true;
        }
        self.propagate_typography(store, root);
        if !self.layout_dirty && self.last_pass == Some(pass_key) {
            return false;
        }
        let Some(root_yg) = self.nodes.get(&root).map(|e| e.yg_node) else {
            self.computed_rects.clear();
            self.layout_dirty = true;
            return true;
        };
        self.measurer_panicked.store(false, Ordering::Release);
        // SAFETY: `self.config` and `root_yg` are live owned Yoga handles; all attached
        // child nodes and `RawMeasureState` payloads remain live throughout layout.
        unsafe {
            nve_yg_config_set_point_scale_factor(self.config, viewport.scale_factor);
            nve_yg_node_set_fixed_size(root_yg, viewport.width, viewport.height);
            nve_yg_node_calculate_layout(root_yg, viewport.width, viewport.height);
        }
        self.computed_rects.clear();
        self.collect_rects(store, root);
        self.last_pass = Some(pass_key);
        // Leave `layout_dirty = true` and dirty measured text nodes if a host `TextMeasurer`
        // panicked mid-pass so Yoga invalidates its cached `(0, 0)` fallback on retry.
        if self.measurer_panicked.swap(false, Ordering::AcqRel) {
            for entry in self.nodes.values() {
                if entry.measure_attached {
                    // SAFETY: `entry.yg_node` is a live owned Yoga node.
                    unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
                }
            }
            self.layout_dirty = true;
        } else {
            self.layout_dirty = false;
        }
        true
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.computed_rects.get(&id).copied()
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.text_measurer = Arc::clone(&measurer);
        for entry in self.nodes.values_mut() {
            if let Some(state) = entry.measure_state.as_mut() {
                state.with_payload_mut(|payload| {
                    payload.measurer = Arc::clone(&measurer);
                });
                // SAFETY: `entry.yg_node` is a live owned Yoga node.
                unsafe { nve_yg_node_mark_dirty(entry.yg_node) };
            }
        }
        self.layout_dirty = true;
    }

    fn reset(&mut self) {
        self.clear_all_nodes();
        self.tree_seeded = false;
        self.typo_dirty = true;
        self.layout_dirty = true;
        self.last_pass = None;
    }
}
