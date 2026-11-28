//! [`WebTableSystem`] plugin trait and the ordered [`WebTableRegistry`] (tag resolution across
//! plugins, then core tables).

use std::sync::Arc;

use nve_tables::tables::{Accessibility, AriaRole, Image, Link, TextContent, TextInput};
use nve_store::{NodeId, NodeStore, TableSlot};

use super::WebSystemCommitCtx;
use crate::stylesheet::ResolvedNodeStyle;
#[cfg(target_arch = "wasm32")]
use super::ctx::ClickInterceptor;

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

    /// Contributes content-addressed class and `@media` rules for `node` to the shared stylesheet
    /// (SSR `<style>` and live constructable sheets), e.g. compiled `transition` lists.
    fn resolve_style(&self, _store: &NodeStore, _node: NodeId, _out: &mut ResolvedNodeStyle) {}

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

    /// Synchronizes imperative state that needs a document-connected element (`showModal()`,
    /// `showPopover()`), run after child reconciliation (Step 6) for nodes queued through
    /// [`WebSystemCommitCtx::request_connected_sync`], and for every node carrying this table
    /// whose element was retagged or moved by the commit (a move drops top-layer state).
    #[cfg(target_arch = "wasm32")]
    fn sync_connected(&self, _store: &NodeStore, _node: NodeId, _el: &web_sys::Element) {}

    /// Returns `true` when changes to another table `slot` must re-run [`Self::commit_node`] on
    /// nodes carrying this plugin's table (e.g. `Accessibility::modal` driving `<dialog>`
    /// modality). Watched re-commits pass `dirty_words == 0`.
    fn watches_slot(&self, _slot: TableSlot) -> bool {
        false
    }

    /// Native DOM events bridged into the NVE event queue as `Custom` events (e.g. `<dialog>`
    /// `close`), attached on the mount root while this plugin is registered.
    #[cfg(target_arch = "wasm32")]
    fn native_event_bridges(&self) -> &'static [crate::native_events::NativeEventBridge] {
        &[]
    }

    /// Root-delegator click hook consulted before `Press` dispatch; `true` consumes the click.
    #[cfg(target_arch = "wasm32")]
    fn click_interceptor(&self) -> Option<ClickInterceptor> {
        None
    }
}

/// Ordered registry of [`WebTableSystem`] plugins installed on a web renderer.
#[derive(Default)]
pub struct WebTableRegistry {
    pub(super) systems: Vec<Arc<dyn WebTableSystem>>,
    /// Bumped on every [`Self::register`] so live mirrors (click hooks) refresh lazily.
    revision: u32,
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
        self.systems
            .sort_by_key(|a| std::cmp::Reverse(a.priority()));
        self.revision = self.revision.wrapping_add(1);
        self
    }

    #[inline]
    pub fn revision(&self) -> u32 {
        self.revision
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
    pub fn resolve_tag<'a>(
        &self,
        store: &'a NodeStore,
        node: NodeId,
    ) -> (&'a str, Option<&'static str>) {
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
        if let Some(a11y) = store.try_row::<Accessibility>(node)
            && let Some(role) = a11y.role()
        {
            match role {
                AriaRole::Button => return ("button", None),
                AriaRole::Heading => {
                    return (
                        match a11y.level() {
                            Some(1) => "h1",
                            Some(3) => "h3",
                            Some(4) => "h4",
                            Some(5) => "h5",
                            Some(6) => "h6",
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
