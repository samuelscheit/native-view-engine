//! Web DOM & SSR adapter for `nve-webview` (`WebTableSystem`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_backend_web::html::write_attr;
use nve_backend_web::{is_safe_web_url, WebSystemCommitCtx, WebTableSystem};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};
use nve_webview::{events, WebView, WebViewCommand};

/// Default iframe sandbox policy — intentionally omits `allow-same-origin` so sandboxed content
/// cannot escape the sandbox on same-origin URLs.
pub const WEBVIEW_SANDBOX_ATTR: &str = "allow-scripts allow-forms";

/// `WebTableSystem` plugin for [`WebView`] (`nve.web_view`).
#[derive(Debug, Default, Clone, Copy)]
pub struct WebViewWebSystem;

impl WebTableSystem for WebViewWebSystem {
    fn slot(&self) -> TableSlot {
        WebView::slot()
    }

    fn priority(&self) -> u8 {
        110
    }

    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        Some("iframe")
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        write_attr("sandbox", WEBVIEW_SANDBOX_ATTR, out);
        write_attr("referrerpolicy", "no-referrer", out);
        if let Some(row) = store.try_row::<WebView>(node) {
            let url = row.url();
            if is_safe_web_url(url) {
                write_attr("src", url, out);
            }
        }
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        ctx.set_attr(node, "sandbox", WEBVIEW_SANDBOX_ATTR);
        ctx.set_attr(node, "referrerpolicy", "no-referrer");
        if let Some(row) = store.try_row::<WebView>(node) {
            let url = row.url();
            if is_safe_web_url(url) {
                ctx.set_attr(node, "src", url);
            }
        }
    }

    fn handle_command(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        _store: &NodeStore,
        node: NodeId,
        op: u32,
        payload: &[u8],
    ) {
        if let Some(WebViewCommand::Navigate(url)) = WebViewCommand::decode(op, payload)
            && is_safe_web_url(&url)
        {
            ctx.set_attr(node, "src", &url);
            ctx.events
                .emit::<WebView, String>(0, node, 0, events::LOAD_START, url);
        }
    }
}
