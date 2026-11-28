//! Uniform plugin system (`WebTableSystem` + `WebTableRegistry`) and word-table `WebStoreRenderer`.
//!
//! `nve-backend-web` has **zero dependencies** on any capability module (`svg`, `canvas`, `blur`,
//! `webview`, `video`, `camera`, `web-element`, `web-html`, or 3P crates). Every capability
//! registers a [`WebTableSystem`] covering BOTH live DOM updates and SSR HTML emission uniformly.
//!
//! Key hot-path properties of [`WebStoreRenderer`]:
//! - **Deferred DOM creation**: elements are realized once at `commit`, after all tables for a
//!   newly created node are attached, so tags (`<svg>`, `<canvas>`, `<a>`, `<input>`, `<textarea>`,
//!   `<video>`, `<iframe>`, custom web elements) are chosen right on first insert with zero
//!   create-then-replace churn.
//! - **`COMPOSITE`-only fast lane**: when a node's changed effects are strictly `effect::COMPOSITE`
//!   (`opacity`, `z_index`, `Transform`), only the compositor style properties update — skipping
//!   layout CSS, structural class rehashing, attributes, and children reconciliation completely.

use std::sync::Arc;

use nve_abi::hash::HashMap;
use nve_abi::table::effect;
use nve_tables::tables::{Transform, Typography, Visual};
use nve_engine::{BackendApplyStats, BackendError, StoreBackend};
use nve_store::{ChangeSet, EventBatchBuf, NodeId, NodeStore, TableSlot};

#[path = "table_system_ctx.rs"]
mod ctx;
#[path = "table_system_registry.rs"]
mod registry;
#[path = "table_system_ssr.rs"]
mod ssr;

#[cfg(any(target_arch = "wasm32", test))]
pub(crate) use ctx::needs_connected_sync_on_place;
#[cfg(test)]
pub(crate) use ctx::RetagElementSink;
#[cfg(target_arch = "wasm32")]
pub(crate) use ctx::ClickInterceptors;
pub use ctx::WebSystemCommitCtx;
#[cfg(target_arch = "wasm32")]
pub use ctx::ClickInterceptor;
#[cfg(feature = "ssr")]
pub(crate) use ssr::write_core_ssr_attrs;
pub use registry::{WebTableRegistry, WebTableSystem};
pub use crate::sanitize::{is_safe_custom_tag, is_safe_web_attr, is_safe_web_url};

/// Per-node retained DOM/mirror state maintained by [`WebStoreRenderer`].
#[derive(Debug, Clone)]
pub struct RealizedWebNode {
    pub tag: String,
    pub namespace: Option<&'static str>,
    pub opacity: f32,
    pub z_index: i32,
    pub transform_css: String,
    pub full_style_flushes: u64,
    pub composite_fast_flushes: u64,
    pub(crate) last_flushed_frame: u64,
}

/// Word-table `NodeStore` web renderer with deferred DOM creation, `COMPOSITE`-only fast lane,
/// uniform [`WebTableRegistry`] plugin dispatch, and deterministic SSR.
#[derive(Default)]
pub struct WebStoreRenderer {
    registry: WebTableRegistry,
    realized: HashMap<NodeId, RealizedWebNode>,
    plugin_attrs: HashMap<NodeId, Vec<(String, String)>>,
    plugin_inner_html: HashMap<NodeId, String>,
    plugin_inline_css: HashMap<NodeId, String>,
    slot_attrs: HashMap<(NodeId, TableSlot), Vec<ctx::OwnedAttr>>,
    inner_html_owner: HashMap<NodeId, (TableSlot, u64)>,
    slot_inline_css: HashMap<(NodeId, TableSlot), ctx::OwnedInlineCss>,
    write_seq: u64,
    /// Reusable O(1) lookup of `(slot, nid)` committed during Step 2 (populated only when
    /// `retagged` is non-empty) so Step 2b skips already-committed slots in O(1).
    #[cfg(any(target_arch = "wasm32", test))]
    retag_committed_scratch: nve_abi::hash::HashSet<(TableSlot, NodeId)>,
    /// `(slot, node)` pairs queued by [`WebSystemCommitCtx::request_connected_sync`] or subtree sync.
    #[cfg(any(target_arch = "wasm32", test))]
    connected_sync: Vec<(TableSlot, NodeId)>,
    /// Reusable drain-time dedupe set for `connected_sync`.
    #[cfg(any(target_arch = "wasm32", test))]
    connected_seen: nve_abi::hash::HashSet<(TableSlot, NodeId)>,
    /// Reusable DFS scratch stack for `queue_connected_sync_subtree`.
    #[cfg(any(target_arch = "wasm32", test))]
    connected_stack: Vec<NodeId>,
    /// Reusable per-commit visited guard for `queue_connected_sync_subtree` (keeps top-down mounts
    /// $O(N)$ instead of $O(N \cdot D)$; cleared once per commit in `drain_connected_sync`).
    #[cfg(any(target_arch = "wasm32", test))]
    connected_visited: nve_abi::hash::HashSet<NodeId>,
    pub events: EventBatchBuf,
    pub composite_fast_lane_hits: u64,
    pub full_style_rebuilds: u64,
}

impl WebStoreRenderer {
    /// Creates an empty renderer with ZERO built-in module special-casing.
    /// Register capability plugins uniformly via [`Self::with_table_system`].
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_table_system<S: WebTableSystem>(mut self, system: S) -> Self {
        self.registry.register(system);
        self
    }

    pub fn register_table_system<S: WebTableSystem>(&mut self, system: S) -> &mut Self {
        self.registry.register(system);
        self
    }

    pub fn registry(&self) -> &WebTableRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut WebTableRegistry {
        &mut self.registry
    }

    pub fn realized(&self, id: NodeId) -> Option<&RealizedWebNode> {
        self.realized.get(&id)
    }

    pub fn plugin_attrs(&self, id: NodeId) -> &[(String, String)] {
        self.plugin_attrs.get(&id).map_or(&[], Vec::as_slice)
    }

    pub fn plugin_inner_html(&self, id: NodeId) -> Option<&str> {
        self.plugin_inner_html.get(&id).map(String::as_str)
    }

    pub fn plugin_inline_css(&self, id: NodeId) -> Option<&str> {
        self.plugin_inline_css.get(&id).map(String::as_str)
    }
}

pub(crate) fn write_transform_css(tf: nve_tables::tables::TransformRef<'_>, out: &mut String) {
    use core::fmt::Write as _;
    let (tx, ty) = (tf.translate_x(), tf.translate_y());
    let (sx, sy) = (tf.scale_x(), tf.scale_y());
    let rot = tf.rotate_deg();
    if tx != 0.0 || ty != 0.0 || sx != 1.0 || sy != 1.0 || rot != 0.0 {
        let _ = write!(
            out,
            "transform:translate({tx}px,{ty}px) scale({sx},{sy}) rotate({rot}deg);"
        );
    }
}

impl WebStoreRenderer {
    pub(crate) fn commit_core(&mut self, store: &NodeStore, changes: &ChangeSet) -> u32 {
        let mut mutations = 0u32;

        // 1. Remove destroyed nodes from mirror state.
        for &id in &changes.removed {
            self.realized.remove(&id);
            mutations += 1;
        }

        // 2. Deferred DOM creation: realize newly created nodes ONCE now that all initial tables exist.
        for &id in &changes.created {
            let (tag, ns) = self.registry.resolve_tag(store, id);
            let opacity = store.try_row::<Visual>(id).map_or(1.0, |v| v.opacity());
            let z_index = store.try_row::<Visual>(id).map_or(0, |v| v.z_index());
            let mut tf_css = String::new();
            if let Some(tf) = store.try_row::<Transform>(id) {
                write_transform_css(tf, &mut tf_css);
            }
            self.realized.insert(
                id,
                RealizedWebNode {
                    tag: tag.to_owned(),
                    namespace: ns,
                    opacity,
                    z_index,
                    transform_css: tf_css,
                    full_style_flushes: 1,
                    composite_fast_flushes: 0,
                    last_flushed_frame: changes.frame,
                },
            );
            self.full_style_rebuilds += 1;
            mutations += 1;
        }

        // 3. Process touched nodes: COMPOSITE-only fast lane vs full style/realize update.
        for (&id, &fx) in changes.touched.iter().zip(changes.effects.iter()) {
            if !store.is_live(id) {
                continue;
            }
            if let Some(node) = self.realized.get(&id)
                && node.last_flushed_frame == changes.frame
            {
                continue;
            }
            if fx == effect::COMPOSITE {
                if let Some(node) = self.realized.get_mut(&id) {
                    if let Some(vis) = store.try_row::<Visual>(id) {
                        node.opacity = vis.opacity();
                        node.z_index = vis.z_index();
                    }
                    node.transform_css.clear();
                    if let Some(tf) = store.try_row::<Transform>(id) {
                        write_transform_css(tf, &mut node.transform_css);
                    }
                    node.composite_fast_flushes += 1;
                    node.last_flushed_frame = changes.frame;
                    self.composite_fast_lane_hits += 1;
                    mutations += 1;
                }
                continue;
            }

            // Full update (including tag re-realization if REALIZE bit changed).
            let (tag, ns) = self.registry.resolve_tag(store, id);
            if let Some(node) = self.realized.get_mut(&id) {
                if fx & effect::REALIZE != 0 && (node.tag != tag || node.namespace != ns) {
                    node.tag.clear();
                    node.tag.push_str(tag);
                    node.namespace = ns;
                }
                if let Some(vis) = store.try_row::<Visual>(id) {
                    node.opacity = vis.opacity();
                    node.z_index = vis.z_index();
                }
                node.transform_css.clear();
                if let Some(tf) = store.try_row::<Transform>(id) {
                    write_transform_css(tf, &mut node.transform_css);
                }
                node.full_style_flushes += 1;
                node.last_flushed_frame = changes.frame;
                self.full_style_rebuilds += 1;
                mutations += 1;
            }
        }

        mutations
    }

    /// Notifies plugins of removed nodes while their live DOM elements are still mounted, then drops
    /// all retained plugin state for those nodes.
    pub(crate) fn remove_plugins(
        &mut self,
        changes: &ChangeSet,
        #[cfg(target_arch = "wasm32")] live_elements: Option<&HashMap<NodeId, web_sys::Element>>,
    ) {
        for &id in &changes.removed {
            for i in 0..self.registry.systems.len() {
                let sys = Arc::clone(&self.registry.systems[i]);
                let slot = sys.slot();
                let mut ctx = self.commit_ctx(
                    slot,
                    #[cfg(target_arch = "wasm32")]
                    live_elements.and_then(|m| m.get(&id)),
                );
                sys.remove_node(&mut ctx, id);
                self.slot_attrs.remove(&(id, slot));
                self.slot_inline_css.remove(&(id, slot));
            }
            self.plugin_attrs.remove(&id);
            self.plugin_inner_html.remove(&id);
            self.plugin_inline_css.remove(&id);
            self.inner_html_owner.remove(&id);
        }
    }

    #[inline]
    fn commit_ctx<'a>(
        &'a mut self,
        current_slot: TableSlot,
        #[cfg(target_arch = "wasm32")] live_element: Option<&'a web_sys::Element>,
    ) -> WebSystemCommitCtx<'a> {
        WebSystemCommitCtx {
            attrs: &mut self.plugin_attrs,
            inner_html: &mut self.plugin_inner_html,
            inline_css: &mut self.plugin_inline_css,
            events: &mut self.events,
            current_slot,
            registry: &self.registry,
            slot_attrs: &mut self.slot_attrs,
            inner_html_owner: &mut self.inner_html_owner,
            slot_inline_css: &mut self.slot_inline_css,
            write_seq: &mut self.write_seq,
            #[cfg(target_arch = "wasm32")]
            live_element,
            #[cfg(any(target_arch = "wasm32", test))]
            connected_sync: &mut self.connected_sync,
        }
    }

    /// Step 1: detach tables before retag mirror replay so detaching plugins are purged first.
    fn detach_plugins_step1(
        &mut self,
        changes: &ChangeSet,
        #[cfg(target_arch = "wasm32")] live_elements: Option<&HashMap<NodeId, web_sys::Element>>,
    ) -> u32 {
        let mut mutations = 0u32;
        for tc in changes.tables() {
            if tc.detached.is_empty() {
                continue;
            }
            if let Some(sys) = self.registry.get(tc.slot).cloned() {
                for &nid in &tc.detached {
                    let mut ctx = self.commit_ctx(
                        tc.slot,
                        #[cfg(target_arch = "wasm32")]
                        live_elements.and_then(|m| m.get(&nid)),
                    );
                    sys.remove_node(&mut ctx, nid);
                    ctx.cleanup_current_slot(nid);
                    mutations += 1;
                }
            }
        }
        mutations
    }

    /// Step 2: commit changed tables using real dirty masks, optionally recording committed pairs.
    fn commit_plugins_step2(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        #[cfg(any(target_arch = "wasm32", test))] record_committed: bool,
        #[cfg(target_arch = "wasm32")] live_elements: Option<&HashMap<NodeId, web_sys::Element>>,
    ) -> u32 {
        #[cfg(any(target_arch = "wasm32", test))]
        if record_committed {
            self.retag_committed_scratch.clear();
        }
        let mut mutations = 0u32;
        for tc in changes.tables() {
            let owner = self.registry.get(tc.slot).cloned();
            let has_watchers = self
                .registry
                .systems
                .iter()
                .any(|sys| sys.slot() != tc.slot && sys.watches_slot(tc.slot));
            if owner.is_none() && !has_watchers {
                continue;
            }
            // Candidate 0 is the owner; candidates 1.. are plugins watching `tc.slot`, which
            // re-commit their own rows (`dirty_words == 0`) on nodes whose watched table changed
            // OR detached (dropping `Accessibility` from an open modal `<dialog>` exits `:modal`).
            for candidate in 0..=self.registry.systems.len() {
                let sys = match candidate.checked_sub(1) {
                    None => match &owner {
                        Some(sys) => Arc::clone(sys),
                        None => continue,
                    },
                    Some(_) if !has_watchers => break,
                    Some(i) => {
                        let sys = &self.registry.systems[i];
                        if sys.slot() == tc.slot || !sys.watches_slot(tc.slot) {
                            continue;
                        }
                        Arc::clone(sys)
                    }
                };
                let slot = sys.slot();
                let own = slot == tc.slot;
                let detached: &[NodeId] = if own { &[] } else { &tc.detached };
                for (nid, mask) in tc.iter().chain(detached.iter().map(|&nid| (nid, 0))) {
                    if !own && !(store.is_live(nid) && store.has_slot(nid, slot)) {
                        continue;
                    }
                    #[cfg(any(target_arch = "wasm32", test))]
                    if record_committed {
                        self.retag_committed_scratch.insert((slot, nid));
                    }
                    let mut ctx = self.commit_ctx(
                        slot,
                        #[cfg(target_arch = "wasm32")]
                        live_elements.and_then(|m| m.get(&nid)),
                    );
                    sys.commit_node(&mut ctx, store, nid, if own { mask } else { 0 });
                    mutations += 1;
                }
            }
        }
        mutations
    }

    /// Step 3: dispatch typed imperative commands to registered `WebTableSystem`s.
    fn dispatch_commands_step3(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        #[cfg(target_arch = "wasm32")] live_elements: Option<&HashMap<NodeId, web_sys::Element>>,
    ) -> u32 {
        let mut mutations = 0u32;
        for cmd in &changes.commands {
            if let Some(sys) = self.registry.get(cmd.table).cloned() {
                let mut ctx = self.commit_ctx(
                    cmd.table,
                    #[cfg(target_arch = "wasm32")]
                    live_elements.and_then(|m| m.get(&cmd.node)),
                );
                sys.handle_command(&mut ctx, store, cmd.node, cmd.op, changes.command_payload(cmd));
                mutations += 1;
            }
        }
        mutations
    }

    /// Unified Step 1 -> 2 -> 2a -> 2b -> 3 plugin commit orchestrator shared by `wasm32` live DOM
    /// retag and native/wasm32 unit tests via [`ctx::RetagElementSink`]. Step 2a replays the
    /// FINAL mirrors onto retagged elements, so Step-2 retractions (`remove_attr`,
    /// `forget_attr`) are never undone by a replay of the previous frame's state, and values
    /// Steps 1-2 already wrote onto the fresh element are not written twice.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn commit_plugins_active_with<E: ctx::RetagElementSink>(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        #[cfg(target_arch = "wasm32")] live_dom: Option<&HashMap<NodeId, web_sys::Element>>,
        retag_live: Option<&HashMap<NodeId, E>>,
        retagged: &[NodeId],
        on_sync: impl FnMut(&dyn WebTableSystem, NodeId, &E),
    ) -> u32 {
        let written_after = self.write_seq;
        let mut mutations = self.detach_plugins_step1(
            changes,
            #[cfg(target_arch = "wasm32")] live_dom,
        );
        let has_retag = !retagged.is_empty() && retag_live.is_some();
        mutations += self.commit_plugins_step2(
            store,
            changes,
            has_retag,
            #[cfg(target_arch = "wasm32")] live_dom,
        );
        if let Some(live) = retag_live
            && !retagged.is_empty()
        {
            self.replay_retagged_mirrors(retagged, live, written_after);
            ctx::dispatch_retagged_sync(
                &self.registry,
                store,
                retagged,
                live,
                &self.retag_committed_scratch,
                on_sync,
            );
        }
        mutations + self.dispatch_commands_step3(store, changes, #[cfg(target_arch = "wasm32")] live_dom)
    }

    pub(crate) fn commit_plugins_active(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        #[cfg(target_arch = "wasm32")] live_elements: Option<&HashMap<NodeId, web_sys::Element>>,
        #[cfg(target_arch = "wasm32")] retagged: &[NodeId],
    ) -> u32 {
        #[cfg(target_arch = "wasm32")]
        {
            self.commit_plugins_active_with(store, changes, live_elements, live_elements, retagged, |sys, nid, el| {
                sys.sync_retagged_element(store, nid, el);
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut mutations = self.detach_plugins_step1(changes);
            mutations += self.commit_plugins_step2(store, changes, #[cfg(test)] false);
            mutations + self.dispatch_commands_step3(store, changes)
        }
    }

    pub(crate) fn commit_plugins(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        #[cfg(target_arch = "wasm32")] live_elements: Option<&HashMap<NodeId, web_sys::Element>>,
        #[cfg(target_arch = "wasm32")] retagged: &[NodeId],
    ) -> u32 {
        self.remove_plugins(
            changes,
            #[cfg(target_arch = "wasm32")]
            live_elements,
        );
        self.commit_plugins_active(
            store,
            changes,
            #[cfg(target_arch = "wasm32")]
            live_elements,
            #[cfg(target_arch = "wasm32")]
            retagged,
        )
    }
}

impl StoreBackend for WebStoreRenderer {
    fn name(&self) -> &'static str {
        "web-store"
    }

    fn set_viewport(&mut self, _surface_id: nve_abi::SurfaceId, _viewport: nve_engine::Viewport) {}

    fn commit(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
    ) -> Result<BackendApplyStats, BackendError> {
        let mut mutations = self.commit_core(store, changes);
        mutations += self.commit_plugins(
            store,
            changes,
            #[cfg(target_arch = "wasm32")]
            None,
            #[cfg(target_arch = "wasm32")]
            &[],
        );

        // Ensure Typography is referenced cleanly if needed.
        let _ = Typography::KEY;

        Ok(BackendApplyStats {
            op_count: mutations,
            mutation_count: mutations,
            ..Default::default()
        })
    }

    fn reset(&mut self) {
        self.registry.reset();
        self.realized.clear();
        self.plugin_attrs.clear();
        self.plugin_inner_html.clear();
        self.plugin_inline_css.clear();
        self.slot_attrs.clear();
        self.inner_html_owner.clear();
        self.slot_inline_css.clear();
        self.write_seq = 0;
        #[cfg(any(target_arch = "wasm32", test))]
        self.retag_committed_scratch.clear();
        #[cfg(any(target_arch = "wasm32", test))]
        self.connected_sync.clear();
        #[cfg(any(target_arch = "wasm32", test))]
        self.connected_seen.clear();
        #[cfg(any(target_arch = "wasm32", test))]
        self.connected_stack.clear();
        #[cfg(any(target_arch = "wasm32", test))]
        self.connected_visited.clear();
        self.events.clear();
    }
}

