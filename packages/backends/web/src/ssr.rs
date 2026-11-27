//! Semantic HTML5 + deduplicated XXH64 structural/`@media`/state CSS Server-Side Rendering
//! (`render_ssr`, `render_ssr_document`, `render_html`) directly over `NodeStore` + `WebTableRegistry`,
//! sharing one recursive emitter with streaming shells and slot deltas (`crate::ssr_stream`).

use core::fmt::Write as _;

use nve_abi::Xxh64Hasher;
use nve_components::tables::{TextContent, TextInput};
use nve_store::{NodeId, NodeStore};

use crate::css::{write_compositor_css, NVE_BASE_RESET_CSS};
use crate::event_replay::PRE_HYDRATION_REPLAY_SCRIPT_TAG;
use crate::html::{
    close_html_element, is_void_html_tag, push_attr_escaped, push_style_css_escaped,
    push_text_escaped, write_attr, write_style_nonce_attr,
};
use crate::ssr_stream::StreamShellState;
use crate::streaming::StreamingError;
use crate::stylesheet::StyleSheetCache;
use crate::table_system::write_core_ssr_attrs;
use crate::WebBackend;

/// Output produced by [`WebBackend::render_ssr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsrOutput {
    pub base_css: &'static str,
    pub style_css: String,
    pub rule_hashes: Vec<String>,
    pub html: String,
    pub root_hash: u64,
}

/// Options for [`WebBackend::render_ssr_document`] standalone HTML5 document emission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SsrDocumentOptions {
    pub title: String,
    pub description: Option<String>,
    pub canonical_url: Option<String>,
    pub og_image: Option<String>,
    pub lang: Option<String>,
    pub head_extra: Option<String>,
    pub body_scripts: Option<String>,
    pub enable_event_replay: bool,
    /// Optional per-request CSP `nonce` stamped onto emitted `<style>` tags.
    pub style_nonce: Option<String>,
}

impl SsrDocumentOptions {
    /// Sets an opt-in per-request CSP `nonce` stamped onto emitted `<style>` elements.
    pub fn with_style_nonce(mut self, nonce: impl Into<String>) -> Self {
        let n = nonce.into();
        self.style_nonce = (!n.is_empty()).then_some(n);
        self
    }
}

/// Appends `hashes` comma-joined and attribute-escaped (`data-nve-hashes="..."` payload).
pub(crate) fn push_hashes_csv(hashes: &[String], out: &mut String) {
    for (i, h) in hashes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_attr_escaped(h, out);
    }
}

impl WebBackend {
    /// Computes a deterministic XXH64 structural hash of `store` for `data-nve-ssr-hash`.
    pub fn compute_ssr_tree_hash(&self, store: &NodeStore) -> u64 {
        let mut hash = Xxh64Hasher::new(0);
        let mut stack: Vec<(NodeId, bool)> = vec![(NodeId::ROOT, true)];
        while let Some((id, enter)) = stack.pop() {
            if !enter {
                hash.update(&[0xFF]);
                continue;
            }
            if !store.is_live(id) {
                continue;
            }
            let (tag, _) = self.renderer.registry().resolve_tag(store, id);
            hash.update(&id.raw().to_le_bytes());
            hash.update(tag.as_bytes());
            if let Some(t) = store.try_row::<TextContent>(id) {
                hash.update(t.text().as_bytes());
            }
            stack.push((id, false));
            for &cid in store.children(id).iter().rev() {
                stack.push((cid, true));
            }
        }
        hash.digest()
    }

    /// Emits a self-contained HTML fragment for `store` (used by tests, benchmarks, and fragments).
    pub fn render_html(&self, store: &NodeStore) -> String {
        self.render_ssr(store).html
    }

    /// Emits Semantic HTML5 + deduplicated CSS (`SsrOutput`) directly from `store` + registered
    /// `WebTableSystem` plugins.
    pub fn render_ssr(&self, store: &NodeStore) -> SsrOutput {
        let root_hash = self.compute_ssr_tree_hash(store);
        let mut sheet = StyleSheetCache::new();
        let mut html = String::with_capacity(1024);
        // Infallible: streaming-slot validation only runs with a `StreamShellState`.
        let _ = self.emit_ssr_node(store, NodeId::ROOT, root_hash, &mut sheet, &mut html, None);
        let (style_css, rule_hashes) = sheet.serialize_css();
        SsrOutput {
            base_css: NVE_BASE_RESET_CSS,
            style_css,
            rule_hashes,
            html,
            root_hash,
        }
    }

    /// Emits a complete standalone HTML5 document (`<!doctype html>`) for `store` with opt-in
    /// CSP `nonce="..."` on every `<style>` tag and `</style` breakout escaping.
    pub fn render_ssr_document(&self, store: &NodeStore, opts: &SsrDocumentOptions) -> String {
        let ssr = self.render_ssr(store);
        let lang = opts
            .lang
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("en");
        let nonce = opts.style_nonce.as_deref();
        let mut doc = String::with_capacity(ssr.html.len() + ssr.style_css.len() + 1024);
        doc.push_str("<!doctype html><html lang=\"");
        push_attr_escaped(lang, &mut doc);
        doc.push_str("\"><head><meta charset=\"utf-8\">");
        doc.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
        doc.push_str("<title>");
        push_text_escaped(&opts.title, &mut doc);
        doc.push_str("</title>");
        if let Some(ref desc) = opts.description {
            doc.push_str("<meta name=\"description\" content=\"");
            push_attr_escaped(desc, &mut doc);
            doc.push_str("\">");
        }
        if let Some(ref canon) = opts.canonical_url {
            doc.push_str("<link rel=\"canonical\" href=\"");
            push_attr_escaped(canon, &mut doc);
            doc.push_str("\">");
        }
        if let Some(ref og) = opts.og_image {
            doc.push_str("<meta property=\"og:image\" content=\"");
            push_attr_escaped(og, &mut doc);
            doc.push_str("\">");
        }
        doc.push_str("<style data-nve-base");
        write_style_nonce_attr(nonce, &mut doc);
        doc.push('>');
        push_style_css_escaped(ssr.base_css, &mut doc);
        // Always emitted (even empty) so every SSR document exposes one stable hydration anchor.
        doc.push_str("</style><style data-nve-ssr");
        write_style_nonce_attr(nonce, &mut doc);
        doc.push_str(" data-nve-hashes=\"");
        push_hashes_csv(&ssr.rule_hashes, &mut doc);
        doc.push_str("\">");
        push_style_css_escaped(&ssr.style_css, &mut doc);
        doc.push_str("</style>");
        if let Some(ref extra) = opts.head_extra {
            doc.push_str(extra);
        }
        doc.push_str("</head><body><div id=\"app\">");
        doc.push_str(&ssr.html);
        doc.push_str("</div>");
        if opts.enable_event_replay {
            doc.push_str(PRE_HYDRATION_REPLAY_SCRIPT_TAG);
        }
        if let Some(ref scripts) = opts.body_scripts {
            doc.push_str(scripts);
        }
        doc.push_str("</body></html>");
        doc
    }

    /// Emits `id`'s element subtree. With `stream`, every `data-nve-stream-slot` root is wrapped in
    /// `<slot name="nve-slot-{id}">` after Declarative Shadow DOM placement validation.
    pub(crate) fn emit_ssr_node<'s>(
        &self,
        store: &'s NodeStore,
        id: NodeId,
        root_hash: u64,
        sheet: &mut StyleSheetCache,
        out: &mut String,
        mut stream: Option<&mut StreamShellState<'s>>,
    ) -> Result<(), StreamingError> {
        if !store.is_live(id) {
            return Ok(());
        }
        let (tag, _ns) = self.renderer.registry().resolve_tag(store, id);
        let wrapped_in_slot = match stream.as_deref_mut() {
            Some(state) => state.open_slot(store, id, out)?,
            None => false,
        };
        let resolved = sheet.resolve_node(store, id);

        out.push('<');
        out.push_str(tag);
        let _ = write!(out, " data-nve-id=\"{}\"", id.raw());
        if id == NodeId::ROOT {
            let _ = write!(out, " data-nve-ssr-hash=\"{root_hash:016x}\"");
        }
        if !resolved.classes.is_empty() {
            write_attr("class", &resolved.classes, out);
        }

        // Inline style: compositor properties + fallback + plugin inline CSS + hot-patch CSS
        let mut inline = String::new();
        if !resolved.inline_fallback.is_empty() {
            inline.push_str(&resolved.inline_fallback);
        }
        write_compositor_css(store, id, &mut inline);
        for sys in self.renderer.registry().iter() {
            if store.has_slot(id, sys.slot()) {
                sys.write_inline_css(store, id, &mut inline);
            }
        }
        if let Some(patch) = self.patch_css.get(&id.raw()) {
            inline.push_str(patch);
        }
        if !inline.is_empty() {
            write_attr("style", &inline, out);
        }

        // Core table attributes: DebugId, StreamBoundary, Link, Image, TextInput, Accessibility, Interaction
        write_core_ssr_attrs(store, id, out);

        // Plugin attributes (`WebTableRegistry`)
        for sys in self.renderer.registry().iter() {
            if store.has_slot(id, sys.slot()) {
                sys.write_ssr_attrs(store, id, out);
            }
        }

        out.push('>');
        if !is_void_html_tag(tag) {
            if let Some(state) = stream.as_deref_mut() {
                state.ancestor_tags.push(tag);
            }
            // Plugin inner HTML (e.g. `<svg>` shapes) may override child traversal
            let mut suppressed_children = false;
            for sys in self.renderer.registry().iter() {
                if store.has_slot(id, sys.slot()) && sys.write_ssr_inner(store, id, out) {
                    suppressed_children = true;
                }
            }

            if !suppressed_children {
                if let Some(ti) = store.try_row::<TextInput>(id)
                    && ti.multiline()
                {
                    let v = ti.value();
                    if !v.is_empty() {
                        push_text_escaped(v, out);
                    }
                } else if store.children(id).is_empty()
                    && let Some(tc) = store.try_row::<TextContent>(id)
                {
                    let txt = tc.text();
                    if !txt.is_empty() {
                        push_text_escaped(txt, out);
                    }
                } else {
                    for &cid in store.children(id) {
                        self.emit_ssr_node(store, cid, root_hash, sheet, out, stream.as_deref_mut())?;
                    }
                }
            }
            close_html_element(tag, out);
            if let Some(state) = stream.as_deref_mut() {
                state.ancestor_tags.pop();
            }
        }
        if wrapped_in_slot && let Some(state) = stream {
            state.close_slot(out);
        }
        Ok(())
    }
}
