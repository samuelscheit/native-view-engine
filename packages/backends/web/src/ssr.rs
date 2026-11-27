//! Semantic HTML5 + deduplicated XXH64 structural/`@media`/state CSS Server-Side Rendering
//! (`render_ssr`, `render_ssr_document`, `render_html`) directly over `NodeStore` + `WebTableRegistry`.

use core::fmt::Write as _;

use nve_abi::Xxh64Hasher;
use nve_components::tables::{TextContent, TextInput};
use nve_store::{NodeId, NodeStore};

use crate::css::{write_compositor_css, NVE_BASE_RESET_CSS};
use crate::event_replay::PRE_HYDRATION_REPLAY_SCRIPT_TAG;
use crate::html::{
    close_html_element, is_void_html_tag, push_attr_escaped, push_text_escaped, write_attr,
};
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
        self.emit_ssr_node(store, NodeId::ROOT, root_hash, &mut sheet, &mut html);
        let (style_css, rule_hashes) = sheet.serialize_css();
        SsrOutput {
            base_css: NVE_BASE_RESET_CSS,
            style_css,
            rule_hashes,
            html,
            root_hash,
        }
    }

    /// Emits a complete standalone HTML5 document (`<!doctype html>`) for `store`.
    pub fn render_ssr_document(&self, store: &NodeStore, opts: &SsrDocumentOptions) -> String {
        let ssr = self.render_ssr(store);
        let lang = opts
            .lang
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("en");
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
        doc.push_str("<style data-nve-base>");
        doc.push_str(ssr.base_css);
        doc.push_str("</style>");
        if !ssr.style_css.is_empty() {
            doc.push_str("<style data-nve-ssr data-nve-hashes=\"");
            for (i, h) in ssr.rule_hashes.iter().enumerate() {
                if i > 0 {
                    doc.push(',');
                }
                doc.push_str(h);
            }
            doc.push_str("\">");
            push_style_content_escaped(&ssr.style_css, &mut doc);
            doc.push_str("</style>");
        }
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

    fn emit_ssr_node(
        &self,
        store: &NodeStore,
        id: NodeId,
        root_hash: u64,
        sheet: &mut StyleSheetCache,
        out: &mut String,
    ) {
        if !store.is_live(id) {
            return;
        }
        let (tag, _ns) = self.renderer.registry().resolve_tag(store, id);
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
        if let Some(patch) = self.patch_css.get(&id) {
            inline.push_str(patch);
        }
        if !inline.is_empty() {
            write_attr("style", &inline, out);
        }

        // Core table attributes: DebugId, Link, Image, TextInput, Accessibility, Interaction
        write_core_ssr_attrs(store, id, out);

        // Plugin attributes (`WebTableRegistry`)
        for sys in self.renderer.registry().iter() {
            if store.has_slot(id, sys.slot()) {
                sys.write_ssr_attrs(store, id, out);
            }
        }

        out.push('>');
        if is_void_html_tag(tag) {
            return;
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
                    self.emit_ssr_node(store, cid, root_hash, sheet, out);
                }
            }
        }

        close_html_element(tag, out);
    }
}

/// Escapes any case-insensitive `</style` sequence inside `<style>...</style>` so CSS content can
/// never break out of the raw-text `<style>` element.
pub(crate) fn push_style_content_escaped(css: &str, out: &mut String) {
    let bytes = css.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + 7 <= bytes.len() {
        if bytes[i] == b'<'
            && bytes[i + 1] == b'/'
            && bytes[i + 2..i + 7].eq_ignore_ascii_case(b"style")
        {
            out.push_str(&css[start..i]);
            out.push_str("<\\/");
            start = i + 2;
            i += 7;
        } else {
            i += 1;
        }
    }
    out.push_str(&css[start..]);
}
