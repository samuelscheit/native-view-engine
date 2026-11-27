//! Semantic HTML5 + Option 5 Hybrid structural CSS Server-Side Rendering (`render_ssr` and
//! `render_ssr_document`).

use core::fmt::Write as _;
use std::collections::BTreeMap;

use nve_abi::hash::HashSet;
use nve_abi::{ComponentKind, NodeId, RawPropValue, ROOT_NODE_ID};
use nve_engine::Xxh64Hasher;

use crate::css::{resolve_semantic_tag, NVE_BASE_RESET_CSS};
use crate::dom::is_truthy_raw_prop;
use crate::event_replay::PRE_HYDRATION_REPLAY_SCRIPT_TAG;
use crate::html::{
    close_html_element, is_void_html_tag, push_attr_escaped, push_style_css_escaped,
    push_text_escaped, write_attr, write_style_nonce_attr,
};
use crate::media_css::CompiledMediaRule;
use crate::state_css::CompiledStateRule;
use crate::stylesheet::{resolve_node_style, MAX_DYNAMIC_RULES};
use crate::tree::Visit;
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

/// Reusable Option 5 Hybrid CSS collector shared by [`WebBackend::render_ssr`],
/// [`WebBackend::render_ssr_streaming_shell`], and [`WebBackend::render_ssr_subtree_delta`].
pub(crate) struct SsrStyleCollector<'a> {
    pub(crate) rules: BTreeMap<String, String>,
    pub(crate) media_rules: Vec<CompiledMediaRule>,
    pub(crate) media_seen: HashSet<String>,
    pub(crate) state_rules: Vec<CompiledStateRule>,
    pub(crate) state_seen: HashSet<String>,
    pub(crate) already_sent: Option<&'a HashSet<String>>,
}

impl<'a> SsrStyleCollector<'a> {
    pub(crate) fn new(already_sent: Option<&'a HashSet<String>>) -> Self {
        Self {
            rules: BTreeMap::new(),
            media_rules: Vec::new(),
            media_seen: HashSet::default(),
            state_rules: Vec::new(),
            state_seen: HashSet::default(),
            already_sent,
        }
    }

    #[inline]
    pub(crate) fn is_already_sent(&self, cls: &str) -> bool {
        self.already_sent.is_some_and(|s| s.contains(cls))
    }

    #[inline]
    pub(crate) fn can_insert_new(&self) -> bool {
        let sent_len = self.already_sent.map_or(0, HashSet::len);
        sent_len + self.rules.len() + self.media_rules.len() + self.state_rules.len()
            < MAX_DYNAMIC_RULES
    }

    #[inline]
    pub(crate) fn contains_rule(&self, cls: &str) -> bool {
        self.is_already_sent(cls) || self.rules.contains_key(cls)
    }

    /// Finalizes collected rules in deterministic cascade order:
    /// 1. Base structural rules (`.s...`, `.nve-c-...`, `.nve-t-...`),
    /// 2. `@media` responsive rules,
    /// 3. Interactive pseudo-state rules (`Hover < Focus < FocusVisible < Active < Disabled`).
    pub(crate) fn finish(mut self) -> (String, Vec<String>) {
        self.state_rules.sort_by_key(|r| r.state.cascade_order());

        let total_rules = self.rules.len() + self.media_rules.len() + self.state_rules.len();
        let mut style_css = String::with_capacity(total_rules.saturating_mul(64));
        let mut rule_hashes = Vec::with_capacity(total_rules);
        for (cls, decls) in self.rules {
            let _ = writeln!(&mut style_css, ".{cls}{{{decls}}}");
            rule_hashes.push(cls);
        }
        for rule in self.media_rules {
            rule.write_css_rule_into(&mut style_css);
            style_css.push('\n');
            rule_hashes.push(rule.class_name);
        }
        for rule in self.state_rules {
            rule.write_css_rule_into(&mut style_css);
            style_css.push('\n');
            rule_hashes.push(rule.class_name);
        }
        (style_css, rule_hashes)
    }
}

impl WebBackend {
    /// Computes a deterministic `xxh64` structural hash of the current tree for `data-nve-ssr-hash`.
    pub(crate) fn compute_ssr_tree_hash(&self) -> u64 {
        let mut hasher = Xxh64Hasher::new(0);
        self.walk(ROOT_NODE_ID, |id, _, step| {
            if step == Visit::Exit {
                hasher.update(&[0xFF]);
                return false;
            }
            let Some(el) = self.nodes.get(&id) else {
                return false;
            };
            let sem = resolve_semantic_tag(el.kind, &el.raw_props);
            let tag = if el.kind == ComponentKind::Custom {
                self.effective_html_tag(id, el)
            } else {
                sem.tag
            };
            hasher.update(&id.to_le_bytes());
            hasher.update(tag.as_bytes());
            hasher.update(&[0x00, u8::from(sem.wrap_code)]);
            if let Some(t) = &el.text {
                hasher.update(t.as_bytes());
            }
            hasher.update(&[0x01]);
            true
        });
        hasher.finish()
    }

    /// Writes the start tag, attributes, an open `<code>` wrapper and the text of a childless
    /// node; returns `Some((tag, wrap_code))` when the element has content to close.
    pub(crate) fn open_ssr_element(
        &self,
        id: NodeId,
        parent: Option<NodeId>,
        root_hash: Option<u64>,
        styles: &mut SsrStyleCollector<'_>,
        out: &mut String,
    ) -> Option<(&str, bool)> {
        let el = self.nodes.get(&id)?;
        let parent = parent.and_then(|pid| self.nodes.get(&pid));
        let sem = resolve_semantic_tag(el.kind, &el.raw_props);
        let tag = if el.kind == ComponentKind::Custom {
            self.effective_html_tag(id, el)
        } else {
            sem.tag
        };
        let allow_new = styles.can_insert_new();
        #[allow(unused_mut)]
        let mut resolved = resolve_node_style(el, parent, allow_new, |cls| styles.contains_rule(cls));
        if let Some((cls, decls)) = resolved.structural_rule
            && !styles.is_already_sent(&cls)
        {
            styles.rules.entry(cls).or_insert(decls);
        }
        if let Some((cls, decls)) = resolved.cursor_rule
            && !styles.is_already_sent(cls)
        {
            styles.rules.entry(cls.to_owned()).or_insert_with(|| decls.to_owned());
        }
        if let Some((cls, decls)) = resolved.transition_rule
            && !styles.is_already_sent(&cls)
        {
            styles.rules.entry(cls).or_insert(decls);
        }
        for media_rule in resolved.media_rules {
            if !styles.is_already_sent(&media_rule.class_name)
                && styles.media_seen.insert(media_rule.class_name.clone())
            {
                styles.media_rules.push(media_rule);
            }
        }
        for state_rule in resolved.state_rules {
            if !styles.is_already_sent(&state_rule.class_name)
                && styles.state_seen.insert(state_rule.class_name.clone())
            {
                styles.state_rules.push(state_rule);
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

        if id == ROOT_NODE_ID
            && let Some(h) = root_hash
        {
            let _ = write!(out, " data-nve-ssr-hash=\"{h:016x}\"");
        }

        write_attr("class", &resolved.class_name, out);

        if !resolved.inline_style.is_empty() {
            write_attr("style", &resolved.inline_style, out);
        }
        if let Some(dir) = crate::layout_ext_css::node_dir_attr(el.layout_ext.as_deref()) {
            write_attr("dir", dir, out);
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
            crate::input_attrs::write_input_hint_attrs_html(&el.raw_props, out);
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

        crate::semantic_attrs::write_semantic_html_attrs(el.kind, tag, &el.raw_props, out);
        crate::paint_svg::write_svg_open_attrs_html(el, out);

        out.push('>');
        if is_void_html_tag(tag) {
            return None;
        }

        if sem.wrap_code {
            out.push_str("<code>");
        }
        if el.svg.is_some() && el.children.is_empty() {
            crate::paint_svg::write_svg_inner_html(el, out);
        } else if let Some(t) = el.text.as_deref().filter(|_| el.children.is_empty()) {
            push_text_escaped(t, out);
        }
        Some((tag, sem.wrap_code))
    }

    /// Emits Semantic HTML5 + deterministically ordered Option 5 Hybrid structural CSS rules,
    /// followed by `@media` rules, followed by interactive pseudo-state rules (`Hover < Focus <
    /// FocusVisible < Active < Disabled`) so overrides always win in the cascade.
    pub fn render_ssr(&self) -> SsrOutput {
        let root_hash = self.compute_ssr_tree_hash();
        let mut styles = SsrStyleCollector::new(None);
        let mut html = String::with_capacity(self.nodes.len().saturating_mul(96).max(64));
        let mut open: Vec<(&str, bool)> = Vec::new();
        self.walk(ROOT_NODE_ID, |id, parent, step| match step {
            Visit::Enter => self
                .open_ssr_element(id, parent, Some(root_hash), &mut styles, &mut html)
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

        let (style_css, rule_hashes) = styles.finish();
        SsrOutput {
            base_css: NVE_BASE_RESET_CSS,
            style_css,
            rule_hashes,
            html,
            root_hash,
        }
    }

    /// Emits a complete standalone HTML5 document with `<style data-nve-base>`,
    /// `<style data-nve-ssr data-nve-hashes="...">`, opt-in CSP `nonce="..."`, OpenGraph/SEO
    /// metadata, and SSR markup.
    pub fn render_ssr_document(&self, opts: &SsrDocumentOptions) -> String {
        let ssr = self.render_ssr();
        let lang = opts
            .lang
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("en");
        let nonce = opts.style_nonce.as_deref();

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

        out.push_str("<style data-nve-base");
        write_style_nonce_attr(nonce, &mut out);
        out.push('>');
        push_style_css_escaped(ssr.base_css, &mut out);
        out.push_str("</style>\n<style data-nve-ssr");
        write_style_nonce_attr(nonce, &mut out);
        out.push_str(" data-nve-hashes=\"");
        for (idx, h) in ssr.rule_hashes.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            out.push_str(h);
        }
        out.push_str("\">");
        push_style_css_escaped(&ssr.style_css, &mut out);
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
