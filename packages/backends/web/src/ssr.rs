//! Semantic HTML5 + Option 5 Hybrid structural CSS Server-Side Rendering (`render_ssr` and
//! `render_ssr_document`).

use core::fmt::Write as _;
use std::collections::BTreeMap;

use nve_abi::hash::HashSet;
use nve_abi::{ComponentKind, NodeId, RawPropValue, ROOT_NODE_ID};

use crate::css::{resolve_semantic_tag, NVE_BASE_RESET_CSS};
use crate::dom::is_truthy_raw_prop;
use crate::event_replay::PRE_HYDRATION_REPLAY_SCRIPT_TAG;
use crate::html::{close_html_element, is_void_html_tag, push_attr_escaped, push_text_escaped, write_attr};
use crate::media_css::CompiledMediaRule;
use crate::stylesheet::{resolve_node_style, MAX_DYNAMIC_RULES};
use crate::tree::Visit;
use crate::WebBackend;

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

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


#[inline]
fn fnv1a_step(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

impl WebBackend {
    /// Computes a deterministic structural hash of the current tree for `data-nve-ssr-hash`.
    fn compute_ssr_tree_hash(&self) -> u64 {
        let mut hash = FNV1A_OFFSET_BASIS;
        self.walk(ROOT_NODE_ID, |id, _, step| {
            if step == Visit::Exit {
                hash = fnv1a_step(hash, &[0xFF]);
                return false;
            }
            let Some(el) = self.nodes.get(&id) else { return false };
            let sem = resolve_semantic_tag(el.kind, &el.raw_props);
            let tag = if el.kind == ComponentKind::Custom {
                self.effective_html_tag(id, el)
            } else {
                sem.tag
            };
            hash = fnv1a_step(hash, &id.to_le_bytes());
            hash = fnv1a_step(hash, tag.as_bytes());
            hash = fnv1a_step(hash, &[u8::from(sem.wrap_code)]);
            if let Some(t) = &el.text {
                hash = fnv1a_step(hash, t.as_bytes());
            }
            true
        });
        hash
    }

    /// Writes the start tag, attributes, an open `<code>` wrapper and the text of a childless
    /// node; `Some((tag, wrap_code))` when the element has content to close.
    #[allow(clippy::too_many_arguments)]
    fn open_ssr_element(
        &self,
        id: NodeId,
        parent: Option<NodeId>,
        root_hash: u64,
        rules: &mut BTreeMap<String, String>,
        media_rules: &mut Vec<CompiledMediaRule>,
        media_seen: &mut HashSet<String>,
        out: &mut String,
    ) -> Option<(&str, bool)> {
        let el = self.nodes.get(&id)?;
        let parent = parent.and_then(|pid| self.nodes.get(&pid));
        let sem = resolve_semantic_tag(el.kind, &el.raw_props);
        let tag = if el.kind == ComponentKind::Custom { self.effective_html_tag(id, el) } else { sem.tag };
        let allow_new = rules.len() + media_rules.len() < MAX_DYNAMIC_RULES;
        let mut resolved = resolve_node_style(el, parent, allow_new, |cls| rules.contains_key(cls));
        if let Some((cls, decls)) = resolved.structural_rule {
            rules.entry(cls).or_insert(decls);
        }
        for media_rule in resolved.media_rules {
            if media_seen.insert(media_rule.class_name.clone()) {
                media_rules.push(media_rule);
            }
        }
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        let native_view = (el.kind == ComponentKind::Custom)
            .then(|| self.native_views.get(&id))
            .flatten();
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        if let Some(nv) = native_view
            && !nv.extra_css.is_empty()
        {
            resolved.inline_style.push_str(&nv.extra_css);
        }

        out.push('<');
        out.push_str(tag);
        out.push_str(" data-nve-id=\"");
        let _ = write!(out, "{id}");
        out.push('"');

        if id == ROOT_NODE_ID {
            let _ = write!(out, " data-nve-ssr-hash=\"{root_hash:016x}\"");
        }

        write_attr("class", &resolved.class_name, out);

        if !resolved.inline_style.is_empty() {
            write_attr("style", &resolved.inline_style, out);
        }
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        if let Some(nv) = native_view {
            crate::native_view::write_native_view_html_attrs(nv, out);
        }

        // Semantic link `href`.
        if let Some(RawPropValue::String(href)) = el.raw_props.get("href")
            && !href.trim().is_empty()
        {
            write_attr("href", href.trim(), out);
        }

        // Progressive island metadata.
        if let Some(RawPropValue::String(island)) = el.raw_props.get("data-nve-island")
            && !island.is_empty()
        {
            write_attr("data-nve-island", island, out);
        }
        if let Some(RawPropValue::String(hydrate)) = el.raw_props.get("data-nve-hydrate")
            && !hydrate.is_empty()
        {
            write_attr("data-nve-hydrate", hydrate, out);
        }

        // Test IDs (matching lowercase HTML DOM attribute semantics).
        if let Some(RawPropValue::String(tid)) = el.raw_props.get("testID")
            && !tid.is_empty()
        {
            write_attr("testid", tid, out);
        }
        if let Some(RawPropValue::String(tid)) = el.raw_props.get("data-testid")
            && !tid.is_empty()
        {
            write_attr("data-testid", tid, out);
        }

        // Media & canvas attributes.
        if el.kind == ComponentKind::Image {
            if let Some(RawPropValue::String(src)) = el
                .raw_props
                .get("src")
                .or_else(|| el.raw_props.get("uri"))
            {
                write_attr("src", src, out);
            }
            if let Some(RawPropValue::String(alt)) = el.raw_props.get("alt") {
                write_attr("alt", alt, out);
            }
        } else if let Some(ops) = el.draw_ops_attr() {
            let _ = write!(out, " data-nve-draw-ops=\"{ops}\"");
        }

        // Input attributes.
        if el.kind == ComponentKind::TextInput {
            if let Some(RawPropValue::String(ph)) = el.raw_props.get("placeholder") {
                write_attr("placeholder", ph, out);
            }
            if let Some(RawPropValue::String(val)) = el.raw_props.get("value") {
                write_attr("value", val, out);
            } else if let Some(val) = &el.text {
                write_attr("value", val, out);
            }
            if el
                .raw_props
                .get("secureTextEntry")
                .or_else(|| el.raw_props.get("secure_text_entry"))
                .is_some_and(is_truthy_raw_prop)
            {
                out.push_str(" type=\"password\"");
            }
        }

        // Canonical ARIA & boolean accessibility attributes.
        if let Some(RawPropValue::String(label)) = el
            .raw_props
            .get("accessibilityLabel")
            .or_else(|| el.raw_props.get("accessibility_label"))
            && !label.is_empty()
        {
            write_attr("aria-label", label, out);
        }
        if let Some(RawPropValue::String(role)) = el
            .raw_props
            .get("accessibilityRole")
            .or_else(|| el.raw_props.get("accessibility_role"))
            && !role.is_empty()
        {
            write_attr("role", role, out);
        }
        if let Some(RawPropValue::String(hint)) = el
            .raw_props
            .get("accessibilityHint")
            .or_else(|| el.raw_props.get("accessibility_hint"))
            && !hint.is_empty()
        {
            write_attr("aria-description", hint, out);
        }
        if el.raw_props.get("disabled").is_some_and(is_truthy_raw_prop) {
            out.push_str(" disabled aria-disabled=\"true\"");
        }
        if el
            .raw_props
            .get("readOnly")
            .or_else(|| el.raw_props.get("read_only"))
            .or_else(|| el.raw_props.get("readonly"))
            .is_some_and(is_truthy_raw_prop)
        {
            out.push_str(" readonly");
        }

        out.push('>');
        if is_void_html_tag(tag) { return None; }

        if sem.wrap_code { out.push_str("<code>"); }
        // Text shows only while the node has no element children, as in the live DOM.
        if let Some(t) = el.text.as_deref().filter(|_| el.children.is_empty()) { push_text_escaped(t, out); }
        Some((tag, sem.wrap_code))
    }

    /// Emits Semantic HTML5 + deterministically ordered Option 5 Hybrid structural CSS rules
    /// followed by all responsive `@media` rules so overrides always win in the cascade.
    pub fn render_ssr(&self) -> SsrOutput {
        let root_hash = self.compute_ssr_tree_hash();
        let mut rules: BTreeMap<String, String> = BTreeMap::new();
        let mut media_rules: Vec<CompiledMediaRule> = Vec::new();
        let mut media_seen: HashSet<String> = HashSet::default();
        let mut html = String::with_capacity(self.nodes.len().saturating_mul(96).max(64));
        let mut open: Vec<(&str, bool)> = Vec::new();
        self.walk(ROOT_NODE_ID, |id, parent, step| match step {
            Visit::Enter => self
                .open_ssr_element(
                    id,
                    parent,
                    root_hash,
                    &mut rules,
                    &mut media_rules,
                    &mut media_seen,
                    &mut html,
                )
                .map(|closing| open.push(closing))
                .is_some(),
            Visit::Exit => {
                let (tag, wrap_code) = open.pop().unwrap_or(("div", false));
                if wrap_code {
                    html.push_str("</code>");
                }
                close_html_element(tag, &mut html);
                false
            }
        });

        let mut style_css =
            String::with_capacity((rules.len() + media_rules.len()).saturating_mul(64));
        let mut rule_hashes = Vec::with_capacity(rules.len() + media_rules.len());
        // 1. Emit ALL base structural rules FIRST.
        for (cls, decls) in rules {
            let _ = writeln!(&mut style_css, ".{cls}{{{decls}}}");
            rule_hashes.push(cls);
        }
        // 2. Emit ALL `@media` rules AFTER base structural rules so responsive overrides win.
        for rule in media_rules {
            rule.write_css_rule_into(&mut style_css);
            style_css.push('\n');
            rule_hashes.push(rule.class_name);
        }

        SsrOutput {
            base_css: NVE_BASE_RESET_CSS,
            style_css,
            rule_hashes,
            html,
            root_hash,
        }
    }

    /// Emits a complete standalone HTML5 document with `<style data-nve-base>`,
    /// `<style data-nve-ssr data-nve-hashes="...">`, OpenGraph/SEO metadata, and SSR markup.
    pub fn render_ssr_document(&self, opts: &SsrDocumentOptions) -> String {
        let ssr = self.render_ssr();
        let lang = opts
            .lang
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("en");

        let mut out = String::with_capacity(
            ssr.base_css.len() + ssr.style_css.len() + ssr.html.len() + 512,
        );
        out.push_str("<!doctype html>\n<html lang=\"");
        push_attr_escaped(lang, &mut out);
        out.push_str("\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>");
        push_text_escaped(&opts.title, &mut out);
        out.push_str("</title>\n");

        if let Some(desc) = opts.description.as_deref().filter(|s| !s.is_empty()) {
            out.push_str("<meta name=\"description\" content=\"");
            push_attr_escaped(desc, &mut out);
            out.push_str("\">\n<meta property=\"og:description\" content=\"");
            push_attr_escaped(desc, &mut out);
            out.push_str("\">\n");
        }
        if !opts.title.is_empty() {
            out.push_str("<meta property=\"og:title\" content=\"");
            push_attr_escaped(&opts.title, &mut out);
            out.push_str("\">\n");
        }
        if let Some(canonical) = opts.canonical_url.as_deref().filter(|s| !s.is_empty()) {
            out.push_str("<link rel=\"canonical\" href=\"");
            push_attr_escaped(canonical, &mut out);
            out.push_str("\">\n<meta property=\"og:url\" content=\"");
            push_attr_escaped(canonical, &mut out);
            out.push_str("\">\n");
        }
        if let Some(og_img) = opts.og_image.as_deref().filter(|s| !s.is_empty()) {
            out.push_str("<meta property=\"og:image\" content=\"");
            push_attr_escaped(og_img, &mut out);
            out.push_str("\">\n");
        }

        out.push_str("<style data-nve-base>");
        out.push_str(ssr.base_css);
        out.push_str("</style>\n<style data-nve-ssr data-nve-hashes=\"");
        for (idx, h) in ssr.rule_hashes.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            out.push_str(h);
        }
        out.push_str("\">");
        out.push_str(&ssr.style_css);
        out.push_str("</style>\n");

        if let Some(extra) = opts.head_extra.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(extra);
            if !extra.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("</head>\n<body>\n<div id=\"app\">");
        out.push_str(&ssr.html);
        out.push_str("</div>\n");
        if opts.enable_event_replay {
            out.push_str(PRE_HYDRATION_REPLAY_SCRIPT_TAG);
            out.push('\n');
        }
        if let Some(scripts) = opts.body_scripts.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(scripts);
            if !scripts.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("</body>\n</html>\n");
        out
    }
}
