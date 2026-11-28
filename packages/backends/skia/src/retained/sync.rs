//! Incremental `COMPOSITE`, dirty-only `PAINT` (with `xxh64` typography/visibility stop-pruning
//! and `display: none` transition detection), and structural `LAYOUT` sync passes for
//! [`super::CompositorScene`].

use std::collections::HashSet;

use nve_layout::{is_overlay_closed, StoreLayoutEngine};
use nve_store::{ChangeSet, NodeId, NveTable, ResourceId, TableStorage};
use nve_tables::tables::{Image, Interaction, TextInput, Visual};

use super::chunks::CHUNK_LIVE;
use super::emit::{compute_inherit_hash, SceneSyncEnv};
use super::emit_helpers::{
    get_or_alloc_chunk, rebuild_ordered_children_for, sort_ordered_children_in_place,
};
use super::property_tree::ChunkIdx;
use super::{CompositorScene, SceneDirtyKind};
use crate::display_list::{
    is_node_display_none_effective, is_subtree_display_none_effective, resolve_node_text_style,
    should_clip_children,
};
use crate::slab_lru::FastBuildHasher;

impl CompositorScene {
    #[inline]
    pub(super) fn begin_dirty_queue(&mut self) {
        self.dirty_nodes_scratch.clear();
        self.dirty_epoch = self.dirty_epoch.wrapping_add(1).max(1);
    }

    #[inline]
    pub(super) fn enqueue_dirty(&mut self, id: NodeId) {
        let idx = id.index();
        if idx >= self.dirty_queued_marks.len() {
            self.dirty_queued_marks.resize(idx + 1, 0);
        }
        if self.dirty_queued_marks[idx] != self.dirty_epoch {
            self.dirty_queued_marks[idx] = self.dirty_epoch;
            self.dirty_nodes_scratch.push(id);
        }
    }

    #[inline]
    fn mark_dirty_done(&mut self, id: NodeId) {
        let idx = id.index();
        if idx >= self.dirty_done_marks.len() {
            self.dirty_done_marks.resize(idx + 1, 0);
        }
        self.dirty_done_marks[idx] = self.dirty_epoch;
    }

    /// Re-enqueues a child when its parent's inherited hash changed: skips if already pending
    /// in the queue, or re-queues if the child was already processed earlier in this pass before
    /// its parent updated `resolved_text`.
    #[inline]
    fn enqueue_dirty_child(&mut self, child_id: NodeId) {
        let idx = child_id.index();
        if idx >= self.dirty_queued_marks.len() {
            self.dirty_queued_marks.resize(idx + 1, 0);
        }
        let queued = self.dirty_queued_marks[idx] == self.dirty_epoch;
        let done = self
            .dirty_done_marks
            .get(idx)
            .copied()
            .unwrap_or(0)
            == self.dirty_epoch;
        if !queued || done {
            self.dirty_queued_marks[idx] = self.dirty_epoch;
            if idx < self.dirty_done_marks.len() {
                self.dirty_done_marks[idx] = 0;
            }
            self.dirty_nodes_scratch.push(child_id);
        }
    }

    #[inline]
    fn begin_z_parents(&mut self) {
        self.z_parents_scratch.clear();
        self.z_parent_epoch = self.z_parent_epoch.wrapping_add(1).max(1);
    }

    #[inline]
    fn enqueue_z_parent(&mut self, pid: NodeId) {
        let idx = pid.index();
        if idx >= self.z_parent_marks.len() {
            self.z_parent_marks.resize(idx + 1, 0);
        }
        if self.z_parent_marks[idx] != self.z_parent_epoch {
            self.z_parent_marks[idx] = self.z_parent_epoch;
            self.z_parents_scratch.push(pid);
        }
    }

    /// Returns `true` if any queued dirty node changed its effective inclusion state
    /// (`display: none` including ancestor subtrees & `StateRules` pseudo-states, or missing
    /// layout rect) relative to whether it currently holds a live retained chunk.
    pub(super) fn has_display_none_transition<L: StoreLayoutEngine>(
        &self,
        env: &SceneSyncEnv<'_, L>,
    ) -> bool {
        for &id in &self.dirty_nodes_scratch {
            let now_absent = is_subtree_display_none_effective(env.store, env.layout_tree, id)
                || is_overlay_closed(env.store, id)
                || env.layout_tree.layout_of(id).is_none();
            let prev_absent = self.live_chunk_idx(id).is_none();
            if now_absent != prev_absent {
                return true;
            }
        }
        false
    }

    pub(super) fn sync_composite_from_changes<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
        changes: &ChangeSet,
    ) {
        self.begin_z_parents();
        for &id in &changes.touched {
            let Some(cidx) = self.live_chunk_idx(id) else {
                continue;
            };
            let new_z = env.store.try_row::<Visual>(id).map_or(0, |v| v.z_index());
            if new_z != self.chunks[cidx.index()].z_index {
                self.chunks[cidx.index()].z_index = new_z;
                if let Some(pid) = env.store.parent(id) {
                    self.enqueue_z_parent(pid);
                }
            }
            let mut style = env.layout_tree.effective_visual(env.store, id);
            env.frame_hooks.apply_visual(id, &mut style);
            self.props.effects[cidx.index()].opacity = style.opacity;

            let eff_tf = env.frame_hooks.resolve_transform(
                id,
                env.layout_tree.effective_transform(env.store, id),
            );
            self.props.transforms[cidx.index()].set_transform(eff_tf);
        }
        // In-place zero-allocation z-order sort (zero `ordered_children` arena churn!).
        for i in 0..self.z_parents_scratch.len() {
            let pid = self.z_parents_scratch[i];
            if let Some(pcidx) = self.live_chunk_idx(pid) {
                sort_ordered_children_in_place(pcidx, &self.chunks, &mut self.arenas.ordered_children);
            }
        }
    }

    /// Propagates every scroll node whose offset changed during `LaidOutScene::reconcile_scroll_offsets`
    /// into [`PropertyTrees::scrolls`] on the zero-alloc `CompositeOnly` lane.
    pub(super) fn sync_reconciled_scrolls<L: StoreLayoutEngine>(&mut self, env: &SceneSyncEnv<'_, L>) {
        let changed = env.layout_tree.scene().last_reconciled_scroll_nodes();
        for &sid in changed {
            let (sx, sy) = env.layout_tree.scene().scroll_offset(sid);
            let _ = self.update_scroll(env, sid, sx, sy);
        }
    }

    pub(super) fn sync_paint_from_changes<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
        changes: &ChangeSet,
        plugin_dirty_nodes: &[NodeId],
    ) {
        self.begin_dirty_queue();

        for &id in &changes.touched {
            if env.store.is_live(id) {
                self.enqueue_dirty(id);
            }
        }
        for cmd in &changes.commands {
            if env.store.is_live(cmd.node) {
                self.enqueue_dirty(cmd.node);
            }
        }
        for &id in plugin_dirty_nodes {
            if env.store.is_live(id) {
                self.enqueue_dirty(id);
            }
        }

        let total_res_changed =
            changes.resources_defined.len() + changes.resources_released.len();
        if total_res_changed > 0 {
            let res_set = (total_res_changed > 4).then(|| {
                let mut set =
                    HashSet::with_capacity_and_hasher(total_res_changed, FastBuildHasher);
                set.extend(changes.resources_defined.iter().copied());
                set.extend(changes.resources_released.iter().copied());
                set
            });
            let res_changed = |rid: ResourceId| -> bool {
                if rid.is_none() {
                    return false;
                }
                if let Some(ref set) = res_set {
                    set.contains(&rid)
                } else {
                    changes.resources_defined.contains(&rid)
                        || changes.resources_released.contains(&rid)
                }
            };

            // Scan live chunks for Visual (shadows, gradient) and Image resources.
            for idx in 0..self.chunks.len() {
                let meta = self.chunks[idx];
                if !meta.is_live() {
                    continue;
                }
                let id = meta.node_id;
                let hits_visual = env.store.try_row::<Visual>(id).is_some_and(|v| {
                    res_changed(v.box_shadows()) || res_changed(v.background_fill())
                });
                let hits_image = env
                    .store
                    .try_row::<Image>(id)
                    .is_some_and(|im| res_changed(im.src()));
                if hits_visual || hits_image {
                    self.enqueue_dirty(id);
                }
            }

            // Scan `Interaction` sparse table owners directly so even currently `display: none`
            // nodes whose `StateRules` resource changed are caught and can become visible again.
            if let Some(TableStorage {
                owners, row_count, ..
            }) = env.store.table_storage(Interaction::slot())
            {
                for &id in owners.iter().take(row_count) {
                    if id.is_null() {
                        continue;
                    }
                    if env.store.is_live(id)
                        && env
                            .store
                            .try_row::<Interaction>(id)
                            .is_some_and(|inter| res_changed(inter.state_rules()))
                    {
                        self.enqueue_dirty(id);
                    }
                }
            }
        }

        // Catch `display: none` transitions triggered by `Interaction::disabled`, `StateRules`
        // edits, or `StateRules` resource updates without a `LAYOUT` effect.
        if self.has_display_none_transition(env) {
            self.stats.last_dirty_kind = Some(SceneDirtyKind::LayoutOrTree);
            self.stats.layout_tree_syncs += 1;
            self.sync_full_tree(env, changes, plugin_dirty_nodes, false, true);
            return;
        }

        self.stats.paint_incremental_paths += 1;
        if self.try_update_plugin_epochs_in_place(env, changes) {
            return;
        }
        self.reemit_dirty_nodes_with_inheritance(env);
    }

    pub(super) fn reemit_dirty_nodes_with_inheritance<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
    ) {
        self.begin_z_parents();
        let mut cursor = 0usize;
        while cursor < self.dirty_nodes_scratch.len() {
            let id = self.dirty_nodes_scratch[cursor];
            cursor += 1;

            let Some(cidx) = self.live_chunk_idx(id) else {
                continue;
            };
            let mut meta = self.chunks[cidx.index()];

            let (parent_hidden, parent_text) = if meta.parent_chunk.is_some() {
                let pmeta = self.chunks[meta.parent_chunk.index()];
                let peff = self.props.effects[meta.parent_chunk.index()];
                (peff.effective_hidden, Some(pmeta.resolved_text))
            } else {
                (false, None)
            };

            let mut style = env.layout_tree.effective_visual(env.store, id);
            env.frame_hooks.apply_visual(id, &mut style);

            let new_z = env.store.try_row::<Visual>(id).map_or(0, |v| v.z_index());
            if new_z != meta.z_index {
                meta.z_index = new_z;
                if let Some(pid) = env.store.parent(id) {
                    self.enqueue_z_parent(pid);
                }
            }

            let tf = &mut self.props.transforms[cidx.index()];
            let width = tf.width;
            let height = tf.height;
            let eff_tf = env.frame_hooks.resolve_transform(
                id,
                env.layout_tree.effective_transform(env.store, id),
            );
            tf.set_transform(eff_tf);

            let clip = &mut self.props.clips[cidx.index()];
            clip.radius = style.border_radius.max(0.0);
            clip.clips_children = should_clip_children(env.store, id);

            let effective_hidden = env
                .store
                .try_row::<Visual>(id)
                .map_or(parent_hidden, |v| v.visibility().resolve(parent_hidden));
            let eff = &mut self.props.effects[cidx.index()];
            eff.opacity = style.opacity;
            eff.effective_hidden = effective_hidden;

            let resolved_text = resolve_node_text_style(
                env.store,
                env.layout_tree,
                env.frame_hooks,
                id,
                &style,
                parent_text,
            );
            let new_inherit_hash = compute_inherit_hash(resolved_text, effective_hidden);
            let inherit_changed = new_inherit_hash != meta.child_inherit_hash;
            meta.resolved_text = resolved_text;
            meta.child_inherit_hash = new_inherit_hash;

            env.emit_chunk_local_ops(
                id,
                width,
                height,
                &style,
                resolved_text,
                effective_hidden,
                &mut self.arenas,
                &mut meta,
            );
            self.stats.chunks_emitted += 1;
            self.chunks[cidx.index()] = meta;
            self.mark_dirty_done(id);

            // Inherited typography & visibility stop-pruning: propagate to direct children ONLY
            // when the resolved inherited hash actually changed.
            if inherit_changed {
                let start = meta.children_start as usize;
                let end = start + meta.children_len as usize;
                for ci in start..end {
                    let child_cidx = self.arenas.ordered_children[ci];
                    let child_id = self.chunks[child_cidx.index()].node_id;
                    self.enqueue_dirty_child(child_id);
                }
            }
        }

        for i in 0..self.z_parents_scratch.len() {
            let pid = self.z_parents_scratch[i];
            if let Some(pcidx) = self.live_chunk_idx(pid) {
                sort_ordered_children_in_place(pcidx, &self.chunks, &mut self.arenas.ordered_children);
            }
        }

        self.arenas.maybe_compact(&mut self.chunks);
    }

    pub(super) fn sync_full_tree<L: StoreLayoutEngine>(
        &mut self,
        env: &SceneSyncEnv<'_, L>,
        changes: &ChangeSet,
        plugin_dirty_nodes: &[NodeId],
        force_reemit: bool,
        include_dirty_scratch: bool,
    ) {
        self.live_epoch = self.live_epoch.wrapping_add(1).max(1);
        let cur_epoch = self.live_epoch;
        self.touched_epoch = self.touched_epoch.wrapping_add(1).max(1);
        let cur_touched = self.touched_epoch;

        let mark_touched = |marks: &mut Vec<u64>, id: NodeId, epoch: u64| {
            let idx = id.index();
            if idx >= marks.len() {
                marks.resize(idx + 1, 0);
            }
            marks[idx] = epoch;
        };
        for &id in &changes.touched {
            mark_touched(&mut self.touched_epoch_marks, id, cur_touched);
        }
        for &id in &changes.created {
            mark_touched(&mut self.touched_epoch_marks, id, cur_touched);
        }
        for cmd in &changes.commands {
            mark_touched(&mut self.touched_epoch_marks, cmd.node, cur_touched);
        }
        for &id in plugin_dirty_nodes {
            mark_touched(&mut self.touched_epoch_marks, id, cur_touched);
        }
        if include_dirty_scratch {
            for i in 0..self.dirty_nodes_scratch.len() {
                let id = self.dirty_nodes_scratch[i];
                mark_touched(&mut self.touched_epoch_marks, id, cur_touched);
            }
        }

        self.props.sticky_chunks.clear();
        self.props.top_layer_chunks.clear();
        self.props.scrolls.clear();
        self.walk_scratch.clear();
        self.z_parents_scratch.clear();

        if !env.store.is_live(NodeId::ROOT)
            || is_node_display_none_effective(env.store, env.layout_tree, NodeId::ROOT)
            || env.layout_tree.layout_of(NodeId::ROOT).is_none()
        {
            self.clear();
            self.initialized = true;
            return;
        }

        let force_reemit_all = force_reemit
            || !self.initialized
            || !changes.resources_defined.is_empty()
            || !changes.resources_released.is_empty();

        // Always reset `ordered_children` because `sync_full_tree` rebuilds every parent's child
        // span cleanly; on full re-emit, reset the op/string/gradient arenas too so zero dead
        // space accumulates across viewport/resource rebuilds.
        if force_reemit_all {
            self.arenas.clear();
            for meta in &mut self.chunks {
                meta.shadow_start = 0;
                meta.shadow_len = 0;
                meta.op_start = 0;
                meta.op_len = 0;
                meta.children_start = 0;
                meta.children_len = 0;
            }
        } else {
            self.arenas.reset_children_arena();
            for meta in &mut self.chunks {
                meta.children_start = 0;
                meta.children_len = 0;
            }
        }

        self.walk_scratch
            .push((NodeId::ROOT, ChunkIdx::NONE, false, None));

        let mut root_chunk = ChunkIdx::NONE;

        while let Some((id, parent_chunk, inherited_hidden, parent_text)) = self.walk_scratch.pop()
        {
            if is_node_display_none_effective(env.store, env.layout_tree, id)
                || is_overlay_closed(env.store, id)
            {
                continue;
            }
            let Some(base_rect) = env.layout_tree.layout_of(id) else {
                continue;
            };

            let (cidx, is_new) = get_or_alloc_chunk(
                id,
                parent_chunk,
                &mut self.node_to_chunk,
                &mut self.chunks,
                &mut self.free_chunks,
                &mut self.props,
                &mut self.arenas,
            );
            if parent_chunk.is_none() {
                root_chunk = cidx;
            }

            if cidx.index() >= self.live_epoch_marks.len() {
                self.live_epoch_marks.resize(cidx.index() + 1, 0);
            }
            self.live_epoch_marks[cidx.index()] = cur_epoch;

            let mut meta = self.chunks[cidx.index()];
            let was_live = meta.is_live();
            meta.flags |= CHUNK_LIVE;
            meta.z_index = env.store.try_row::<Visual>(id).map_or(0, |v| v.z_index());

            let prev_tf = self.props.transforms[cidx.index()];
            let prev_hidden = self.props.effects[cidx.index()].effective_hidden;

            let mut style = env.layout_tree.effective_visual(env.store, id);
            env.frame_hooks.apply_visual(id, &mut style);

            let mut tf_node = prev_tf;
            let mut clip_node = self.props.clips[cidx.index()];
            let mut effect_node = self.props.effects[cidx.index()];
            env.fill_property_nodes(
                id,
                base_rect,
                inherited_hidden,
                &style,
                &mut tf_node,
                &mut clip_node,
                &mut effect_node,
                &mut self.props.scrolls,
            );
            let prev_size_eq = was_live
                && prev_tf.width.to_bits() == tf_node.width.to_bits()
                && prev_tf.height.to_bits() == tf_node.height.to_bits();
            if tf_node.is_sticky() {
                self.props.sticky_chunks.push((id, cidx));
            }
            if tf_node.is_top_layer() {
                self.props.top_layer_chunks.push(cidx);
            }
            self.props.transforms[cidx.index()] = tf_node;
            self.props.clips[cidx.index()] = clip_node;
            self.props.effects[cidx.index()] = effect_node;

            let resolved_text = resolve_node_text_style(
                env.store,
                env.layout_tree,
                env.frame_hooks,
                id,
                &style,
                parent_text,
            );
            let new_inherit_hash =
                compute_inherit_hash(resolved_text, effect_node.effective_hidden);
            let cur_plugin_epoch = env.plugin_epoch_of(id);
            let was_touched = self
                .touched_epoch_marks
                .get(id.index())
                .copied()
                .unwrap_or(0)
                == cur_touched;

            let node_needs_emit = force_reemit_all
                || is_new
                || !was_live
                || !prev_size_eq
                || prev_hidden != effect_node.effective_hidden
                || meta.child_inherit_hash != new_inherit_hash
                || meta.plugin_epoch != cur_plugin_epoch
                || env.focus.focused_node == Some(id)
                || env.store.has::<TextInput>(id)
                || (changes.is_empty() && !include_dirty_scratch)
                || was_touched;

            meta.resolved_text = resolved_text;
            meta.child_inherit_hash = new_inherit_hash;

            if node_needs_emit {
                env.emit_chunk_local_ops(
                    id,
                    tf_node.width,
                    tf_node.height,
                    &style,
                    resolved_text,
                    effect_node.effective_hidden,
                    &mut self.arenas,
                    &mut meta,
                );
                self.stats.chunks_emitted += 1;
            }
            self.chunks[cidx.index()] = meta;

            let raw_children = env.store.children(id);
            if !raw_children.is_empty() {
                self.z_parents_scratch.push(id);
                for &child_id in raw_children.iter().rev() {
                    self.walk_scratch.push((
                        child_id,
                        cidx,
                        effect_node.effective_hidden,
                        Some(resolved_text),
                    ));
                }
            }
        }

        // Retire chunks not visited in this epoch (removed or became `display: none`).
        for idx in 0..self.chunks.len() {
            let visited = self.live_epoch_marks.get(idx).copied().unwrap_or(0) == cur_epoch;
            if !visited && self.chunks[idx].is_live() {
                let mut meta = self.chunks[idx];
                self.arenas.retire_chunk_ops(&mut meta);
                meta.children_start = 0;
                meta.children_len = 0;
                meta.flags &= !CHUNK_LIVE;
                let nid = meta.node_id;
                if nid.index() < self.node_to_chunk.len()
                    && self.node_to_chunk[nid.index()] == ChunkIdx(idx as u32)
                {
                    self.node_to_chunk[nid.index()] = ChunkIdx::NONE;
                }
                self.chunks[idx] = meta;
                self.free_chunks.push(ChunkIdx(idx as u32));
            }
        }

        // Populate ordered children spans for all live visited parents.
        for i in 0..self.z_parents_scratch.len() {
            let pid = self.z_parents_scratch[i];
            if let Some(pcidx) = self.live_chunk_idx(pid) {
                rebuild_ordered_children_for(
                    env.store,
                    pid,
                    pcidx,
                    &self.node_to_chunk,
                    &mut self.chunks,
                    &mut self.arenas,
                );
            }
        }

        self.arenas.maybe_compact(&mut self.chunks);
        self.root = root_chunk.is_some().then_some(root_chunk);
        self.initialized = true;
    }
}
