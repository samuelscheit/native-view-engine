//! Stateful incremental [`AccessKitTreeBridge`] syncing [`NodeStore`] changes to [`TreeUpdate`].

use std::collections::HashMap;

use accesskit::{ActionRequest, Node, NodeId as AccessKitNodeId, TreeId, TreeInfo, TreeUpdate};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};
use nve_tables::tables::{DebugId, Layout, Position, Scroll};

use crate::action::{
    translate_action_request, ResolvedA11yAction, DEFAULT_SCROLL_STEP_ITEM,
    DEFAULT_SCROLL_STEP_PAGE,
};
use crate::dirty::{
    can_use_targeted_dirty, for_each_a11y_dirty_node, is_composite_only_for_a11y, MarkEpoch,
};
use crate::geometry::{
    is_node_display_contents_any, is_node_display_contents_transparent, is_node_excluded,
    A11yGeometryProvider, A11yNodeHash,
};
use crate::id::{decode_store_node_id, encode_store_node_id, StoreEpoch};
use crate::node_build::build_a11y_node;

const MAX_A11Y_DEPTH: usize = 256;

/// Platform-agnostic incremental AccessKit bridge over a [`NodeStore`].
///
/// - **Zero-cost when inactive**: until [`Self::activate`] is called (in response to
///   `InitialTreeRequested`), [`Self::sync_changes`] returns `None` with **0 heap allocations**.
/// - **Zero-cost on `Visual::opacity`-only frames**: pure opacity/z-index `COMPOSITE` animation
///   frames return `None` with **0 heap allocations**.
/// - **O(dirty) incremental updates**: single-node label/value/ARIA/focus/scroll/transform edits
///   emit only the changed node(s) without walking the full tree; structural/layout/visibility
///   changes diff against dense per-node `XXH64` hashes and emit only changed/added nodes +
///   updated parents.
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
    /// Reusable `DebugId::id -> NodeId` index populated exclusively from reachable emitted nodes.
    id_map: HashMap<String, NodeId>,
    /// Emitted `position: sticky` nodes tracked for O(1 + sticky_count) scroll updates.
    sticky_nodes: Vec<NodeId>,
    /// Reusable traversal & scratch buffers.
    walk_stack: Vec<NodeId>,
    reachable_order: Vec<NodeId>,
    dirty_list: Vec<NodeId>,
    child_buf: Vec<AccessKitNodeId>,
    visible_child_ids: Vec<NodeId>,
    rel_buf: Vec<AccessKitNodeId>,
    last_focus_store: Option<NodeId>,
    last_focus_a11y: Option<AccessKitNodeId>,
    last_scale: f32,
    last_window_title: String,
    /// Number of nodes projected by `build_a11y_node` during the most recent sync call.
    last_sync_nodes_built: usize,
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
            visit_epoch: MarkEpoch::ZERO,
            dirty_marks: Vec::new(),
            dirty_epoch: MarkEpoch::ZERO,
            id_map: HashMap::new(),
            sticky_nodes: Vec::new(),
            walk_stack: Vec::new(),
            reachable_order: Vec::new(),
            dirty_list: Vec::new(),
            child_buf: Vec::new(),
            visible_child_ids: Vec::new(),
            rel_buf: Vec::new(),
            last_focus_store: None,
            last_focus_a11y: None,
            last_scale: 1.0,
            last_window_title: String::new(),
            last_sync_nodes_built: 0,
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

    /// Number of nodes projected during the most recent `build_full_tree_update` / `sync_*` call.
    #[inline]
    pub fn last_sync_nodes_built(&self) -> usize {
        self.last_sync_nodes_built
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
        self.sticky_nodes.clear();
        self.last_focus_store = None;
        self.last_focus_a11y = None;
        self.last_sync_nodes_built = 0;
    }

    /// Bumps [`StoreEpoch`] (e.g. on Tier-2 `DylibReload` or keyframe reset), invalidating all
    /// prior-epoch action IDs and scheduling a full tree rebuild on the next sync.
    pub fn advance_epoch(&mut self) {
        self.epoch = self.epoch.next();
        self.has_full_tree = false;
        self.node_hashes.clear();
        self.live_ids.clear();
        self.id_map.clear();
        self.sticky_nodes.clear();
        self.last_focus_store = None;
        self.last_focus_a11y = None;
        self.last_sync_nodes_built = 0;
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
            self.visit_marks.resize(new_len, MarkEpoch::ZERO);
            self.dirty_marks.resize(new_len, MarkEpoch::ZERO);
        }
    }

    #[inline]
    fn is_tracked_live(&self, id: NodeId) -> bool {
        let idx = id.index();
        self.live_ids.get(idx).copied() == Some(id)
            && self.node_hashes.get(idx).copied() != Some(A11yNodeHash::EMPTY)
    }

    /// Collects visible (non-excluded) children of `parent`, flattening `display: contents`
    /// subtrees transparently so their visible descendants remain in the AccessKit tree.
    fn collect_visible_children(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        parent: NodeId,
    ) {
        self.child_buf.clear();
        self.visible_child_ids.clear();
        self.collect_visible_children_rec(store, geo, parent, 0);
    }

    fn collect_visible_children_rec(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        parent: NodeId,
        depth: usize,
    ) {
        if depth >= MAX_A11Y_DEPTH {
            return;
        }
        for &child in store.children(parent) {
            if is_node_excluded(store, geo, child) {
                continue;
            }
            if is_node_display_contents_transparent(store, geo, child) {
                self.collect_visible_children_rec(store, geo, child, depth + 1);
            } else {
                self.visible_child_ids.push(child);
                self.child_buf.push(encode_store_node_id(self.epoch, child));
            }
        }
    }

    /// Discovers all root-reachable, non-excluded, box-generating nodes in pre-order, populating
    /// `reachable_order`, `id_map` (exclusively from reachable emitted nodes so `AriaRelations`
    /// never emits dangling IDs), `sticky_nodes`, and `visit_marks`.
    fn collect_reachable_and_index(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
    ) -> MarkEpoch {
        let mark = self.visit_epoch.bump(&mut self.visit_marks);
        self.id_map.clear();
        self.sticky_nodes.clear();
        self.reachable_order.clear();
        self.walk_stack.clear();
        if store.is_live(NodeId::ROOT) && !is_node_excluded(store, geo, NodeId::ROOT) {
            self.walk_stack.push(NodeId::ROOT);
        }
        let has_debug_ids = store
            .table_storage(DebugId::slot())
            .is_some_and(|s| s.row_count > 0);
        while let Some(id) = self.walk_stack.pop() {
            let idx = id.index();
            self.ensure_slot_len(idx);
            if self.visit_marks[idx] == mark {
                continue;
            }
            self.visit_marks[idx] = mark;
            self.reachable_order.push(id);
            if has_debug_ids
                && let Some(dbg) = store.try_row::<DebugId>(id)
            {
                let dom_id = dbg.id();
                if !dom_id.is_empty() && !self.id_map.contains_key(dom_id) {
                    self.id_map.insert(dom_id.to_owned(), id);
                }
            }
            if store
                .try_row::<Layout>(id)
                .is_some_and(|l| l.position() == Position::Sticky)
            {
                self.sticky_nodes.push(id);
            }
            self.collect_visible_children(store, geo, id);
            for &child in self.visible_child_ids.iter().rev() {
                self.walk_stack.push(child);
            }
        }
        mark
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

    #[inline]
    fn push_dirty_unique(&mut self, id: NodeId, mark: MarkEpoch) {
        let idx = id.index();
        self.ensure_slot_len(idx);
        if self.dirty_marks[idx] != mark {
            self.dirty_marks[idx] = mark;
            self.dirty_list.push(id);
        }
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
        self.last_sync_nodes_built = 0;

        let scale = geo.scale_factor();
        let root_a11y = encode_store_node_id(self.epoch, NodeId::ROOT);
        let mut nodes: Vec<(AccessKitNodeId, Node)> = Vec::new();
        let _ = self.collect_reachable_and_index(store, geo);
        if self.reachable_order.is_empty() {
            let mut root = Node::new(accesskit::Role::Window);
            if !window_title.is_empty() {
                root.set_label(window_title);
            }
            nodes.push((root_a11y, root));
        } else {
            for i in 0..self.reachable_order.len() {
                let id = self.reachable_order[i];
                self.project_node_into(store, geo, id, window_title, scale, true, &mut nodes);
            }
        }

        let focus = self.update_sync_metadata(geo.focused_node(), scale, window_title).0;
        self.has_full_tree = true;

        let mut tree = TreeInfo::new(root_a11y);
        tree.toolkit_name = Some("native-view-engine".to_owned());
        tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());
        TreeUpdate { nodes, tree: Some(tree), tree_id: TreeId::ROOT, focus }
    }

    /// Projects `id`, updates `live_ids`/`node_hashes`, and appends to `nodes` if `always_emit`
    /// or its content hash / live identity changed.
    #[allow(clippy::too_many_arguments)]
    fn project_node_into(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        id: NodeId,
        window_title: &str,
        scale: f32,
        always_emit: bool,
        nodes: &mut Vec<(AccessKitNodeId, Node)>,
    ) {
        self.collect_visible_children(store, geo, id);
        let built = build_a11y_node(
            store, geo, self.epoch, id, window_title, scale, &self.id_map, &self.child_buf,
            &mut self.rel_buf,
        );
        self.last_sync_nodes_built += 1;
        let idx = id.index();
        let changed = always_emit || self.live_ids[idx] != id || self.node_hashes[idx] != built.hash;
        self.live_ids[idx] = id;
        self.node_hashes[idx] = built.hash;
        if changed {
            nodes.push((encode_store_node_id(self.epoch, id), built.node));
            if let Some((run_id, run_node)) = built.text_run {
                nodes.push((run_id, run_node));
            }
        }
    }

    fn update_sync_metadata(
        &mut self,
        raw_focus: Option<NodeId>,
        scale: f32,
        window_title: &str,
    ) -> (AccessKitNodeId, bool) {
        let root_a11y = encode_store_node_id(self.epoch, NodeId::ROOT);
        let focus = self.resolve_focus_a11y_id(raw_focus, root_a11y);
        let focus_changed = self.last_focus_a11y != Some(focus);
        self.last_focus_store = raw_focus;
        self.last_focus_a11y = Some(focus);
        self.last_scale = scale;
        if self.last_window_title != window_title {
            self.last_window_title.clear();
            self.last_window_title.push_str(window_title);
        }
        (focus, focus_changed)
    }

    /// O(1) targeted dirty-node update rebuilding ONLY `dirty_nodes` (plus focus transitions and
    /// any tracked `position: sticky` nodes when a `Scroll` container is dirty) without walking
    /// the full tree or rebuilding `id_map`.
    pub fn sync_dirty_nodes(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        dirty_nodes: &[NodeId],
    ) -> Option<TreeUpdate> {
        let empty = ChangeSet::default();
        self.sync_changes_with_dirty(store, &empty, geo, window_title, dirty_nodes, false)
    }

    /// O(1) targeted scroll-container sync (plus tracked sticky headers, if any).
    pub fn sync_scroll_node(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        scroll_node: NodeId,
    ) -> Option<TreeUpdate> {
        self.sync_dirty_nodes(store, geo, window_title, &[scroll_node])
    }

    /// O(1) targeted `TextInput` value/caret/selection sync.
    pub fn sync_text_input_node(
        &mut self,
        store: &NodeStore,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        input_node: NodeId,
    ) -> Option<TreeUpdate> {
        self.sync_dirty_nodes(store, geo, window_title, &[input_node])
    }

    /// Computes a minimal incremental [`TreeUpdate`] for `changes` (or `None` when nothing changed
    /// or the bridge is inactive).
    pub fn sync_changes(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        force_geometry_or_scroll: bool,
    ) -> Option<TreeUpdate> {
        self.sync_changes_with_dirty(store, changes, geo, window_title, &[], force_geometry_or_scroll)
    }

    /// Computes a minimal incremental [`TreeUpdate`] merging `changes` and host-supplied
    /// `extra_dirty` nodes (e.g. local scroll offsets, uncontrolled `TextInput` edits, drag
    /// selections, or transform animations) in O(dirty) time whenever no layout/structural change
    /// requires a full tree diff.
    pub fn sync_changes_with_dirty(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        extra_dirty: &[NodeId],
        force_full_diff: bool,
    ) -> Option<TreeUpdate> {
        self.last_sync_nodes_built = 0;
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

        if !force_full_diff
            && extra_dirty.is_empty()
            && !scale_changed
            && !title_changed
            && !focus_changed
            && (changes.is_empty() || is_composite_only_for_a11y(changes))
        {
            return None;
        }

        if can_use_targeted_dirty(changes, force_full_diff)
            && let Some(targeted) = self.try_targeted_dirty_update(
                store, changes, extra_dirty, geo, window_title, scale,
                scale_changed || title_changed, raw_focus,
            )
        {
            return targeted;
        }

        self.diff_full_tree(store, geo, window_title, scale, raw_focus)
    }

    #[allow(clippy::too_many_arguments)]
    fn try_targeted_dirty_update(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        extra_dirty: &[NodeId],
        geo: &dyn A11yGeometryProvider,
        window_title: &str,
        scale: f32,
        root_dirty: bool,
        raw_focus: Option<NodeId>,
    ) -> Option<Option<TreeUpdate>> {
        let mark = self.dirty_epoch.bump(&mut self.dirty_marks);
        self.dirty_list.clear();

        for_each_a11y_dirty_node(changes, |id| self.push_dirty_unique(id, mark));
        for &id in extra_dirty {
            self.push_dirty_unique(id, mark);
        }
        for fid in [self.last_focus_store, raw_focus].into_iter().flatten() {
            self.push_dirty_unique(fid, mark);
        }
        if root_dirty {
            self.push_dirty_unique(NodeId::ROOT, mark);
        }

        // Scrolling may alter `sticky_delta` on tracked `position: sticky` nodes: include them
        // without walking the full tree (O(sticky_nodes.len()), zero work when no sticky nodes).
        if self.dirty_list.iter().any(|&id| store.has::<Scroll>(id)) {
            for i in 0..self.sticky_nodes.len() {
                let sid = self.sticky_nodes[i];
                self.push_dirty_unique(sid, mark);
            }
        }

        // If any dirty node toggled inclusion (`aria-hidden` / effective `display: none`) or is a
        // transparent `display: contents` wrapper, fall back to a full diff so parent `children`
        // lists and `id_map` update atomically.
        for i in 0..self.dirty_list.len() {
            let id = self.dirty_list[i];
            if self.is_tracked_live(id) == is_node_excluded(store, geo, id)
                || is_node_display_contents_any(store, geo, id)
            {
                return None;
            }
        }

        let mut nodes: Vec<(AccessKitNodeId, Node)> = Vec::new();
        for i in 0..self.dirty_list.len() {
            let id = self.dirty_list[i];
            if self.is_tracked_live(id) {
                self.project_node_into(store, geo, id, window_title, scale, false, &mut nodes);
            }
        }

        let (focus, focus_changed) = self.update_sync_metadata(raw_focus, scale, window_title);
        if nodes.is_empty() && !focus_changed {
            Some(None)
        } else {
            Some(Some(TreeUpdate { nodes, tree: None, tree_id: TreeId::ROOT, focus }))
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
        let mark = self.collect_reachable_and_index(store, geo);
        let mut nodes: Vec<(AccessKitNodeId, Node)> = Vec::new();
        for i in 0..self.reachable_order.len() {
            let id = self.reachable_order[i];
            self.project_node_into(store, geo, id, window_title, scale, false, &mut nodes);
        }

        // Clear cached entries for nodes no longer reachable from root (their updated parent's
        // `children` list already removed them in AccessKit).
        for idx in 0..self.live_ids.len() {
            if self.visit_marks[idx] != mark && self.live_ids[idx] != NodeId::NULL {
                self.live_ids[idx] = NodeId::NULL;
                self.node_hashes[idx] = A11yNodeHash::EMPTY;
            }
        }

        let (focus, focus_changed) = self.update_sync_metadata(raw_focus, scale, window_title);
        if nodes.is_empty() && !focus_changed {
            None
        } else {
            Some(TreeUpdate { nodes, tree: None, tree_id: TreeId::ROOT, focus })
        }
    }
}

