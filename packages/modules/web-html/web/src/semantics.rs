//! [`HtmlSemanticsWebSystem`]: semantic element tags, `open` / `closedby` / `popover` attributes,
//! and the native `<dialog>` / `[popover]` top-layer state they drive (see [`crate::overlay`]).

use nve_backend_web::html::write_attr as push_attr;
use nve_backend_web::{WebSystemCommitCtx, WebTableSystem};
use nve_tables::tables::{Accessibility, Overlay, OverlayFlags};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};
use nve_web_html::HtmlSemantics;

#[cfg(target_arch = "wasm32")]
use crate::overlay;

/// `WebTableSystem` plugin for [`HtmlSemantics`] (`nve.html.semantics`).
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlSemanticsWebSystem;

impl WebTableSystem for HtmlSemanticsWebSystem {
    fn slot(&self) -> TableSlot {
        HtmlSemantics::slot()
    }

    fn priority(&self) -> u8 {
        60
    }

    fn tag_name<'a>(&self, store: &'a NodeStore, node: NodeId) -> Option<&'a str> {
        let row = store.try_row::<HtmlSemantics>(node)?;
        if let Some(el) = row.element() {
            return Some(el.as_html_tag());
        }
        row.popover().map(|_| "div")
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<HtmlSemantics>(node) else {
            return;
        };
        let is_dialog = self.tag_name(store, node) == Some("dialog");
        let overlay_open = store
            .try_row::<Overlay>(node)
            .map(|ov| ov.options().contains(OverlayFlags::OPEN));
        let effective_open = if is_dialog {
            overlay_open.unwrap_or(row.open())
        } else {
            row.open()
        };
        let effective_data_nve_open = if !is_dialog && row.popover().is_some() {
            overlay_open.unwrap_or(row.data_nve_open())
        } else {
            row.data_nve_open()
        };
        if effective_open {
            out.push_str(" open");
        }
        if let Some(cb) = row.closed_by() {
            push_attr("closedby", cb.as_html_attr(), out);
        }
        if let Some(pop) = row.popover() {
            push_attr("popover", pop.as_html_attr(), out);
        }
        if let Some(pa) = row.popover_action() {
            push_attr("popovertargetaction", pa.as_html_attr(), out);
        }
        if effective_data_nve_open {
            push_attr("data-nve-open", "true", out);
        }
    }

    /// `Accessibility::modal` decides `showModal()` vs `show()`, and `Overlay` drives top-layer
    /// open/modal state: editing or detaching either table re-syncs `HtmlSemantics`.
    fn watches_slot(&self, slot: TableSlot) -> bool {
        slot == Accessibility::slot() || slot == Overlay::slot()
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        let Some(row) = store.try_row::<HtmlSemantics>(node) else {
            return;
        };
        // A `<dialog>`'s `open` state is owned by `show()` / `showModal()` / `close()`: toggling
        // the attribute would bypass the top layer (no `:modal`, no backdrop, no inert), and a
        // stale mirrored `open` (e.g. from a retagged `<details open>`) must never be replayed.
        let is_dialog = ctx.resolved_tag(store, node) == "dialog";
        let overlay_open = store
            .try_row::<Overlay>(node)
            .map(|ov| ov.options().contains(OverlayFlags::OPEN));
        if is_dialog {
            ctx.forget_attr(node, "open");
        } else if row.open() {
            ctx.set_attr(node, "open", "");
        } else {
            ctx.remove_attr(node, "open");
        }
        if let Some(cb) = row.closed_by() {
            ctx.set_attr(node, "closedby", cb.as_html_attr());
        } else {
            ctx.remove_attr(node, "closedby");
        }
        if let Some(pop) = row.popover() {
            ctx.set_attr(node, "popover", pop.as_html_attr());
        } else {
            ctx.remove_attr(node, "popover");
        }
        if let Some(pa) = row.popover_action() {
            ctx.set_attr(node, "popovertargetaction", pa.as_html_attr());
        } else {
            ctx.remove_attr(node, "popovertargetaction");
        }
        let effective_data_nve_open = if !is_dialog && row.popover().is_some() {
            overlay_open.unwrap_or(row.data_nve_open())
        } else {
            row.data_nve_open()
        };
        if effective_data_nve_open {
            ctx.set_attr(node, "data-nve-open", "true");
        } else {
            ctx.remove_attr(node, "data-nve-open");
        }
        // Top-layer state is applied in Step 6, after child reconciliation placed (and possibly
        // moved) every element: a Step-5 move would evict a `:modal` applied any earlier.
        #[cfg(target_arch = "wasm32")]
        if is_dialog || row.popover().is_some() {
            ctx.request_connected_sync(node);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn sync_connected(&self, store: &NodeStore, node: NodeId, el: &web_sys::Element) {
        overlay::sync_overlay_state(store, node, el);
    }

    #[cfg(target_arch = "wasm32")]
    fn click_interceptor(&self) -> Option<nve_backend_web::ClickInterceptor> {
        Some(overlay::try_dialog_backdrop_fallback)
    }

    #[cfg(target_arch = "wasm32")]
    fn native_event_bridges(&self) -> &'static [nve_backend_web::NativeEventBridge] {
        &overlay::CLOSE_BRIDGES
    }
}
