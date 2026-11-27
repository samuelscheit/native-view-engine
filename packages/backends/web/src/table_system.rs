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
use nve_components::tables::{
    Accessibility, AriaRole, Image, Link, TextContent, TextInput, Transform, Typography, Visual,
};
use nve_engine::{BackendApplyStats, BackendError, StoreBackend};
use nve_store::{ChangeSet, NodeId, NodeStore, TableEventBus, TableSlot};

#[path = "table_system_ctx.rs"]
mod ctx;
#[path = "table_system_ssr.rs"]
mod ssr;

#[cfg(test)]
pub(crate) use ctx::RetagElementSink;
pub use ctx::WebSystemCommitCtx;
pub(crate) use ssr::write_core_ssr_attrs;
pub use crate::sanitize::{is_safe_custom_tag, is_safe_web_attr, is_safe_web_url};

/// Uniform capability plugin trait for Web DOM + SSR (implemented by `nve-*-web` and 3P modules).
pub trait WebTableSystem: Send + Sync + 'static {
    /// Table slot handled by this plugin (e.g. `Svg::slot()`, `WebHtml::slot()`).
    fn slot(&self) -> TableSlot;

    /// Realization priority (higher wins when a node attaches multiple plugin tables).
    fn priority(&self) -> u8 {
        100
    }

    /// HTML/SVG element tag to create for `node` (`None` falls back to core tag derivation).
    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        None
    }

    /// Optional XML namespace URI (e.g. `Some("http://www.w3.org/2000/svg")`).
    fn namespace(&self) -> Option<&'static str> {
        None
    }

    /// Emits extra SSR opening-tag attributes into `out` (with leading space per attr).
    fn write_ssr_attrs(&self, _store: &NodeStore, _node: NodeId, _out: &mut String) {}

    /// Emits SSR inner HTML into `out`. Returning `true` suppresses normal child traversal.
    fn write_ssr_inner(&self, _store: &NodeStore, _node: NodeId, _out: &mut String) -> bool {
        false
    }

    /// Emits extra inline CSS declarations into `out` (e.g. `backdrop-filter:...;`).
    fn write_inline_css(&self, _store: &NodeStore, _node: NodeId, _out: &mut String) {}

    /// Synchronizes live/mirror DOM state when this table's row changes on `node`.
    fn commit_node(
        &self,
        _ctx: &mut WebSystemCommitCtx<'_>,
        _store: &NodeStore,
        _node: NodeId,
        _dirty_words: u32,
    ) {
    }

    /// Dispatches a typed imperative command addressed to this table on `node`.
    fn handle_command(
        &self,
        _ctx: &mut WebSystemCommitCtx<'_>,
        _store: &NodeStore,
        _node: NodeId,
        _op: u32,
        _payload: &[u8],
    ) {
    }

    /// Cleans up resources when `node` detaches this table or is removed.
    fn remove_node(&self, _ctx: &mut WebSystemCommitCtx<'_>, _node: NodeId) {}

    /// Re-synchronizes imperative live DOM state (e.g. repainting a freshly created `<canvas>`)
    /// after an element's tag was replaced without changing this table's row.
    #[cfg(target_arch = "wasm32")]
    fn sync_retagged_element(&self, _store: &NodeStore, _node: NodeId, _el: &web_sys::Element) {}
}

/// Ordered registry of [`WebTableSystem`] plugins installed on a web renderer.
#[derive(Default)]
pub struct WebTableRegistry {
    systems: Vec<Arc<dyn WebTableSystem>>,
}

impl WebTableRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a plugin system (replaces any existing plugin for the same `TableSlot`, sorted by priority descending).
    pub fn register<S: WebTableSystem>(&mut self, system: S) -> &mut Self {
        let slot = system.slot();
        self.systems.retain(|s| s.slot() != slot);
        self.systems.push(Arc::new(system));
        self.systems.sort_by_key(|a| std::cmp::Reverse(a.priority()));
        self
    }

    #[inline]
    pub fn get(&self, slot: TableSlot) -> Option<&Arc<dyn WebTableSystem>> {
        self.systems.iter().find(|s| s.slot() == slot)
    }

    #[inline]
    pub fn iter(&self) -> &[Arc<dyn WebTableSystem>] {
        &self.systems
    }

    /// Resolves the element tag & optional namespace for `node` across registered plugins first,
    /// then core tables (`Image > TextInput > Link > Semantics > Accessibility(Button/Heading) > TextContent > div`).
    pub fn resolve_tag<'a>(&self, store: &'a NodeStore, node: NodeId) -> (&'a str, Option<&'static str>) {
        for sys in &self.systems {
            if store.has_slot(node, sys.slot())
                && let Some(tag) = sys.tag_name(store, node)
            {
                return (tag, sys.namespace());
            }
        }
        if store.has::<Image>(node) {
            return ("img", None);
        }
        if let Some(ti) = store.try_row::<TextInput>(node) {
            return (if ti.multiline() { "textarea" } else { "input" }, None);
        }
        if store.has::<Link>(node) {
            return ("a", None);
        }
        if let Some(a11y) = store.try_row::<Accessibility>(node) {
            match a11y.role() {
                AriaRole::Button => return ("button", None),
                AriaRole::Heading => {
                    return (
                        match a11y.level() {
                            1 => "h1",
                            3 => "h3",
                            4 => "h4",
                            5 => "h5",
                            6 => "h6",
                            _ => "h2",
                        },
                        None,
                    );
                }
                AriaRole::Navigation => return ("nav", None),
                AriaRole::Main => return ("main", None),
                AriaRole::Article => return ("article", None),
                AriaRole::Complementary => return ("aside", None),
                AriaRole::Banner => return ("header", None),
                AriaRole::ContentInfo => return ("footer", None),
                AriaRole::Form => return ("form", None),
                AriaRole::Dialog => return ("dialog", None),
                AriaRole::Code => return ("code", None),
                AriaRole::CodeBlock => return ("pre", None),
                _ => {}
            }
        }
        if store.has::<TextContent>(node) && store.children(node).is_empty() {
            return ("span", None);
        }
        ("div", None)
    }
}

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
    inner_html_owner: HashMap<NodeId, TableSlot>,
    slot_inline_css: HashMap<(NodeId, TableSlot), ctx::OwnedInlineCss>,
    write_seq: u64,
    /// Reusable O(1) lookup of `(slot, nid)` committed during Step 2 (populated only when
    /// `retagged` is non-empty) so Step 2b skips already-committed slots in O(1).
    #[cfg(any(target_arch = "wasm32", test))]
    retag_committed_scratch: nve_abi::hash::HashSet<(TableSlot, NodeId)>,
    pub events: TableEventBus,
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

pub(crate) fn write_transform_css(tf: nve_components::tables::TransformRef<'_>, out: &mut String) {
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
            if let Some(sys) = self.registry.get(tc.slot).cloned() {
                for (nid, mask) in tc.iter() {
                    #[cfg(any(target_arch = "wasm32", test))]
                    if record_committed {
                        self.retag_committed_scratch.insert((tc.slot, nid));
                    }
                    let mut ctx = self.commit_ctx(
                        tc.slot,
                        #[cfg(target_arch = "wasm32")]
                        live_elements.and_then(|m| m.get(&nid)),
                    );
                    sys.commit_node(&mut ctx, store, nid, mask);
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

    /// Unified Step 1 -> 1b -> 2 -> 2b -> 3 plugin commit orchestrator shared by `wasm32` live DOM
    /// retag and native/wasm32 unit tests via [`ctx::RetagElementSink`].
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
        let mut mutations = self.detach_plugins_step1(
            changes,
            #[cfg(target_arch = "wasm32")] live_dom,
        );
        let has_retag = !retagged.is_empty() && retag_live.is_some();
        if let Some(live) = retag_live && !retagged.is_empty() {
            ctx::replay_retagged_mirrors(retagged, live, &self.plugin_attrs, &self.plugin_inline_css, &self.plugin_inner_html);
        }
        mutations += self.commit_plugins_step2(
            store,
            changes,
            has_retag,
            #[cfg(target_arch = "wasm32")] live_dom,
        );
        if let Some(live) = retag_live && !retagged.is_empty() {
            ctx::dispatch_retagged_sync(&self.registry, store, retagged, live, &self.retag_committed_scratch, on_sync);
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
        self.events.clear();
    }
}

