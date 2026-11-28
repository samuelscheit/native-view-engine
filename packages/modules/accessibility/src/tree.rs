//! Stateful incremental [`AccessKitTreeBridge`] syncing [`NodeStore`] changes to [`TreeUpdate`].

use std::collections::HashMap;

use accesskit::{ActionRequest, Node, NodeId as AccessKitNodeId, TreeId, TreeInfo, TreeUpdate};
use nve_abi::table::effect;
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};
use nve_tables::tables::{
    Accessibility, DebugId, Image, Interaction, Link, Scroll, TextContent, TextInput,
};

use crate::action::{
    translate_action_request, ResolvedA11yAction, DEFAULT_SCROLL_STEP_ITEM,
    DEFAULT_SCROLL_STEP_PAGE,
};
use crate::geometry::{is_node_excluded, A11yGeometryProvider, A11yNodeHash};
use crate::id::{decode_store_node_id, encode_store_node_id, StoreEpoch};
use crate::node_build::build_a11y_node;

/// O(1) dense epoch mark counter replacing per-frame `HashSet` allocations.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MarkEpoch(u32);

impl MarkEpoch {
    #[inline]
    fn bump(&mut self, marks: &mut [MarkEpoch]) -> Self {
        let next = self.0.wrapping_add(1);
        if next == 0 {
            marks.fill(Self(0));
            self.0 = 1;
        } else {
            self.0 = next;
        }
        *self
    }
}

/// Platform-agnostic incremental AccessKit bridge over a [`NodeStore`].
///
/// - **Zero-cost when inactive**: until [`Self::activate`] is called (in response to
///   `InitialTreeRequested`), [`Self::sync_changes`] returns `None` with **0 heap allocations**.
/// - **Zero-cost on `COMPOSITE`-only frames**: opacity/transform animation frames return `None`
///   with **0 heap allocations**.
/// - **O(dirty) incremental updates**: single-node label/value/ARIA/focus edits emit only the
///   changed node(s) without walking the full tree; structural/layout/visibility changes diff
///   against dense per-node `XXH64` hashes and emit only changed/added nodes + updated parents.
/// - **Epoch-safe across hot reloads**: [`Self::advance_epoch`] bumps [`StoreEpoch`], rejecting
///   stale in-flight actions and forcing a fresh full tree on the next sync.
pub struct AccessKitTreeBridge {
    active: bool,
    has_full_tree: bool,
    epoch: StoreEpoch,
    /// Dense per-`NodeId::index()` hash of the last emitted node (`EMPTY` if not currently in tree).
    node_hashes: Vec<A11yNodeHash>,
    /// Dense per-`NodeId::index()` exact [`NodeId`] (generation-checked) currently in tree.
    live_ids: Vec<NodeId>,
    /// Reusable visit marks for structural diff walks.
    visit_marks: Vec<MarkEpoch>,
    visit_epoch: MarkEpoch,
    /// Reusable dirty marks for O(dirty) targeted updates.
    dirty_marks: Vec<MarkEpoch>,
    dirty_epoch: MarkEpoch,
    /// Reusable `DebugId::id -> NodeId` index for `AriaRelations` resolution.
    id_map: HashMap<String, NodeId>,
    /// Reusable traversal & scratch buffers.
    walk_stack: Vec<NodeId>,
    dirty_list: Vec<NodeId>,
    child_buf: Vec<AccessKitNodeId>,
    rel_buf: Vec<AccessKitNodeId>,
    last_focus_store: Option<NodeId>,
    last_focus_a11y: Option<AccessKitNodeId>,
    last_scale: f32,
    last_window_title: String,
}

impl Default for AccessKitTreeBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl AccessKitTreeBridge {
    /// Creates an inactive bridge (zero work until an assistive technology activates it).
    pub fn new() -> Self {
        Self {
            active: false,
            has_full_tree: false,
            epoch: StoreEpoch::INITIAL,
            node_hashes: Vec::new(),
            live_ids: Vec::new(),
            visit_marks: Vec::new(),
            visit_epoch: MarkEpoch(0),
            dirty_marks: Vec::new(),
            dirty_epoch: MarkEpoch(0),
            id_map: HashMap::new(),
            walk_stack: Vec::new(),
            dirty_list: Vec::new(),
            child_buf: Vec::new(),
            rel_buf: Vec::new(),
            last_focus_store: None,
            last_focus_a11y: None,
            last_scale: 1.0,
            last_window_title: String::new(),
        }
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[inline]
    pub fn epoch(&self) -> StoreEpoch {
        self.epoch
    }

    /// Marks the bridge active (called on `ActivationHandler::request_initial_tree` /
    /// `WindowEvent::InitialTreeRequested`).
    pub fn activate(&mut self) {
        self.active = true;
    }

    /// Deactivates the bridge and releases cached tree state so future frames do zero work.
    pub fn deactivate(&mut self) {
        self.active = false;
        self.has_full_tree = false;
        self.node_hashes.clear();
        self.live_ids.clear();
        self.id_map.clear();
        self.last_focus_store = None;
        self.last_focus_a11y = None;
    }

    /// Bumps [`StoreEpoch`] (e.g. on Tier-2 `DylibReload` or `frame == 1` keyframe reset),
    /// invalidating all prior-epoch action IDs and scheduling a full tree rebuild on the next sync.
    pub fn advance_epoch(&mut self) {
        self.epoch = self.epoch.next();
        self.has_full_tree = false;
        self.node_hashes.clear();
        self.live_ids.clear();
        self.id_map.clear();
        self.last_focus_store = None;
        self.last_focus_a11y = None;
    }

    /// Encodes `id` using this bridge's current [`StoreEpoch`].
    #[inline]
    pub fn encode_store_node_id(&self, id: NodeId) -> AccessKitNodeId {
        encode_store_node_id(self.epoch, id)
    }

    /// Decodes a primary [`NodeId`] if it belongs to this bridge's current [`StoreEpoch`].
    #[inline]
    pub fn decode_store_node_id(&self, id: AccessKitNodeId) -> Option<NodeId> {
        decode_store_node_id(id, self.epoch)
    }

    /// Translates an [`ActionRequest`] against this bridge's current [`StoreEpoch`].
    pub fn translate_action(&self, req: &ActionRequest) -> Option<ResolvedA11yAction> {
        translate_action_request(
            req,
            self.epoch,
            DEFAULT_SCROLL_STEP_ITEM,
            DEFAULT_SCROLL_STEP_PAGE,
        )
    }

    fn ensure_slot_len(&mut self, idx: usize) {
        if idx >= self.node_hashes.len() {
            let new_len = idx + 1;
            self.node_hashes.resize(new_len, A11yNodeHash::EMPTY);
            self.live_ids.resize(new_len, NodeId::NULL);
            self.visit_marks.resize(new_len, MarkEpoch(0));
            self.dirty_marks.resize(new_len, MarkEpoch(0));
        }
    }

    #[inline]
    fn is_tracked_live(&self, id: NodeId) -> bool {
        let idx = id.index();
        self.live_ids.get(idx).copied() == Some(id)
            && self.node_hashes.get(idx).copied() != Some(A11yNodeHash::EMPTY)
    }

    fn rebuild_id_map(&mut self, store: &NodeStore, geo: &dyn A11yGeometryProvider) {
        self.id_map.clear();
        let Some(storage) = store.table_storage(DebugId::slot()) else {
            return;
        };
        for &id in &storage.owners[..storage.row_count.min(storage.owners.len())] {
            if is_node_excluded(store, geo, id) {
                continue;
            }
            if let Some(dbg) = store.try_row::<DebugId>(id) {
                let dom_id = dbg.id();
                if !dom_id.is_empty() {
                    self.id_map.insert(dom_id.to_owned(), id);
                }
            }
        }
    }

    fn collect_visible_children(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        parent: NodeId,
    ) {
        self.child_buf.clear();
        for &child in store.children(parent) {
            if !is_node_excluded(store, geo, child) {
                self.child_buf.push(encode_store_node_id(self.epoch, child));
            }
        }
    }

    fn resolve_focus_a11y_id(
        &self,
        raw_focus: Option<NodeId>,
        root_a11y: AccessKitNodeId,
    ) -> AccessKitNodeId {
        raw_focus
            .filter(|&fid| self.is_tracked_live(fid))
            .map_or(root_a11y, |fid| encode_store_node_id(self.epoch, fid))
    }

    /// Builds a complete [`TreeUpdate`] (including `TreeInfo`) for the entire live accessible tree
    /// and resets the bridge's incremental diff cache to match.
    pub fn build_full_tree_update(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
    ) -> TreeUpdate {
        self.active = true;
        self.node_hashes.fill(A11yNodeHash::EMPTY);
        self.live_ids.fill(NodeId::NULL);
        self.rebuild_id_map(store, geo);

        let scale = geo.scale_factor();
        let root_a11y = encode_store_node_id(self.epoch, NodeId::ROOT);
        let mut nodes: Vec<(AccessKitNodeId, Node)> = Vec::new();

        self.walk_stack.clear();
        if store.is_live(NodeId::ROOT) {
            self.walk_stack.push(NodeId::ROOT);
        } else {
            let mut root = Node::new(accesskit::Role::Window);
            if !window_title.is_empty() {
                root.set_label(window_title);
            }
            nodes.push((root_a11y, root));
        }

        while let Some(id) = self.walk_stack.pop() {
            if is_node_excluded(store, geo, id) {
                continue;
            }
            self.ensure_slot_len(id.index());
            self.collect_visible_children(store, geo, id);
            let built = build_a11y_node(
                store,
                geo,
                self.epoch,
                id,
                window_title,
                scale,
                &self.id_map,
                &self.child_buf,
                &mut self.rel_buf,
            );
            let a11y_id = encode_store_node_id(self.epoch, id);
            self.node_hashes[id.index()] = built.hash;
            self.live_ids[id.index()] = id;
            nodes.push((a11y_id, built.node));
            if let Some((run_id, run_node)) = built.text_run {
                nodes.push((run_id, run_node));
            }
            for &child in store.children(id).iter().rev() {
                if !is_node_excluded(store, geo, child) {
                    self.walk_stack.push(child);
                }
            }
        }

        let raw_focus = geo.focused_node();
        let focus = self.resolve_focus_a11y_id(raw_focus, root_a11y);
        self.last_focus_store = raw_focus;
        self.last_focus_a11y = Some(focus);
        self.last_scale = scale;
        self.last_window_title.clear();
        self.last_window_title.push_str(window_title);
        self.has_full_tree = true;

        let mut tree = TreeInfo::new(root_a11y);
        tree.toolkit_name = Some("native-view-engine".to_owned());
        tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());

        TreeUpdate {
            nodes,
            tree: Some(tree),
            tree_id: TreeId::ROOT,
            focus,
        }
    }

    /// Computes a minimal incremental [`TreeUpdate`] for `changes` (or `None` when nothing changed
    /// or the bridge is inactive).
    ///
    /// Pass `force_geometry_or_scroll = true` after viewport resize, DPI scale change, scroll
    /// offset update, or local uncontrolled `TextInput`/selection change that bypassed `ChangeSet`.
    pub fn sync_changes(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        force_geometry_or_scroll: bool,
    ) -> Option<TreeUpdate> {
        if !self.active {
            return None;
        }
        if !self.has_full_tree {
            return Some(self.build_full_tree_update(store, geo, window_title));
        }

        let scale = geo.scale_factor();
        let raw_focus = geo.focused_node();
        let scale_changed = (scale - self.last_scale).abs() > 1e-6;
        let title_changed = window_title != self.last_window_title.as_str();
        let focus_changed = raw_focus != self.last_focus_store;

        if !force_geometry_or_scroll && !scale_changed && !title_changed && !focus_changed {
            if changes.is_empty() {
                return None;
            }
            if is_composite_only_for_a11y(changes) {
                return None;
            }
        }

        let can_use_targeted_dirty = !force_geometry_or_scroll
            && !scale_changed
            && !title_changed
            && changes.created.is_empty()
            && changes.removed.is_empty()
            && changes.children.is_empty()
            && changes.resources_defined.is_empty()
            && changes.resources_released.is_empty()
            && (changes.effects_union() & (effect::LAYOUT | effect::MEASURE)) == 0
            && changes.table(DebugId::slot()).is_none()
            && changes.table(Scroll::slot()).is_none();

        if can_use_targeted_dirty
            && let Some(targeted) =
                self.try_targeted_dirty_update(store, changes, geo, window_title, scale, raw_focus)
        {
            return targeted;
        }

        self.diff_full_tree(store, geo, window_title, scale, raw_focus)
    }

    fn try_targeted_dirty_update(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        scale: f32,
        raw_focus: Option<NodeId>,
    ) -> Option<Option<TreeUpdate>> {
        let mark = self.dirty_epoch.bump(&mut self.dirty_marks);
        self.dirty_list.clear();

        let relevant_slots = [
            Accessibility::slot(),
            TextContent::slot(),
            TextInput::slot(),
            Interaction::slot(),
            Image::slot(),
            Link::slot(),
        ];
        for slot in relevant_slots {
            if let Some(table) = changes.table(slot) {
                for &id in table.nodes.iter().chain(table.detached.iter()) {
                    let idx = id.index();
                    self.ensure_slot_len(idx);
                    if self.dirty_marks[idx] != mark {
                        self.dirty_marks[idx] = mark;
                        self.dirty_list.push(id);
                    }
                }
            }
        }
        for fid in [self.last_focus_store, raw_focus].into_iter().flatten() {
            let idx = fid.index();
            self.ensure_slot_len(idx);
            if self.dirty_marks[idx] != mark {
                self.dirty_marks[idx] = mark;
                self.dirty_list.push(fid);
            }
        }

        // If any dirty node toggled inclusion (`aria-hidden` or effective `display: none`), fall
        // back to the full tree diff so parent `children` lists and subtrees update atomically.
        for i in 0..self.dirty_list.len() {
            let id = self.dirty_list[i];
            let was_in_tree = self.is_tracked_live(id);
            let now_excluded = is_node_excluded(store, geo, id);
            if was_in_tree == now_excluded {
                return None;
            }
        }

        let mut nodes: Vec<(AccessKitNodeId, Node)> = Vec::new();
        for i in 0..self.dirty_list.len() {
            let id = self.dirty_list[i];
            if !self.is_tracked_live(id) {
                continue;
            }
            self.collect_visible_children(store, geo, id);
            let built = build_a11y_node(
                store,
                geo,
                self.epoch,
                id,
                window_title,
                scale,
                &self.id_map,
                &self.child_buf,
                &mut self.rel_buf,
            );
            if self.node_hashes[id.index()] != built.hash {
                self.node_hashes[id.index()] = built.hash;
                nodes.push((encode_store_node_id(self.epoch, id), built.node));
                if let Some((run_id, run_node)) = built.text_run {
                    nodes.push((run_id, run_node));
                }
            }
        }

        let root_a11y = encode_store_node_id(self.epoch, NodeId::ROOT);
        let focus = self.resolve_focus_a11y_id(raw_focus, root_a11y);
        let focus_a11y_changed = self.last_focus_a11y != Some(focus);
        self.last_focus_store = raw_focus;
        self.last_focus_a11y = Some(focus);

        if nodes.is_empty() && !focus_a11y_changed {
            Some(None)
        } else {
            Some(Some(TreeUpdate {
                nodes,
                tree: None,
                tree_id: TreeId::ROOT,
                focus,
            }))
        }
    }

    fn diff_full_tree(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        scale: f32,
        raw_focus: Option<NodeId>,
    ) -> Option<TreeUpdate> {
        self.rebuild_id_map(store, geo);
        let mark = self.visit_epoch.bump(&mut self.visit_marks);
        self.walk_stack.clear();
        if store.is_live(NodeId::ROOT) {
            self.walk_stack.push(NodeId::ROOT);
        }

        let mut nodes: Vec<(AccessKitNodeId, Node)> = Vec::new();
        while let Some(id) = self.walk_stack.pop() {
            if is_node_excluded(store, geo, id) {
                continue;
            }
            let idx = id.index();
            self.ensure_slot_len(idx);
            self.visit_marks[idx] = mark;
            self.collect_visible_children(store, geo, id);
            let built = build_a11y_node(
                store,
                geo,
                self.epoch,
                id,
                window_title,
                scale,
                &self.id_map,
                &self.child_buf,
                &mut self.rel_buf,
            );
            let changed = self.live_ids[idx] != id || self.node_hashes[idx] != built.hash;
            self.live_ids[idx] = id;
            self.node_hashes[idx] = built.hash;
            if changed {
                nodes.push((encode_store_node_id(self.epoch, id), built.node));
                if let Some((run_id, run_node)) = built.text_run {
                    nodes.push((run_id, run_node));
                }
            }
            for &child in store.children(id).iter().rev() {
                if !is_node_excluded(store, geo, child) {
                    self.walk_stack.push(child);
                }
            }
        }

        // Clear cached entries for nodes no longer reachable from root (their updated parent's
        // `children` list already removed them in AccessKit).
        for idx in 0..self.live_ids.len() {
            if self.visit_marks[idx] != mark && self.live_ids[idx] != NodeId::NULL {
                self.live_ids[idx] = NodeId::NULL;
                self.node_hashes[idx] = A11yNodeHash::EMPTY;
            }
        }

        let root_a11y = encode_store_node_id(self.epoch, NodeId::ROOT);
        let focus = self.resolve_focus_a11y_id(raw_focus, root_a11y);
        let focus_a11y_changed = self.last_focus_a11y != Some(focus);
        self.last_focus_store = raw_focus;
        self.last_focus_a11y = Some(focus);
        self.last_scale = scale;
        if self.last_window_title != window_title {
            self.last_window_title.clear();
            self.last_window_title.push_str(window_title);
        }

        if nodes.is_empty() && !focus_a11y_changed {
            None
        } else {
            Some(TreeUpdate {
                nodes,
                tree: None,
                tree_id: TreeId::ROOT,
                focus,
            })
        }
    }
}

/// Returns `true` when `changes` touches zero accessibility-relevant structure, layout, scroll,
/// text, or semantic tables (e.g. 120 Hz opacity/transform `COMPOSITE`-only frames).
#[inline]
fn is_composite_only_for_a11y(changes: &ChangeSet) -> bool {
    if !changes.created.is_empty()
        || !changes.removed.is_empty()
        || !changes.children.is_empty()
        || !changes.resources_defined.is_empty()
        || !changes.resources_released.is_empty()
    {
        return false;
    }
    let relevant_effects =
        effect::LAYOUT | effect::MEASURE | effect::SEMANTICS | effect::REALIZE | effect::EVENTS;
    if (changes.effects_union() & relevant_effects) != 0 {
        return false;
    }
    changes.table(Scroll::slot()).is_none()
        && changes.table(Accessibility::slot()).is_none()
        && changes.table(TextContent::slot()).is_none()
        && changes.table(TextInput::slot()).is_none()
        && changes.table(Interaction::slot()).is_none()
        && changes.table(DebugId::slot()).is_none()
        && changes.table(Image::slot()).is_none()
        && changes.table(Link::slot()).is_none()
}
