//! HTML serialization primitives shared by `render_html`, SSR and the native-view adapters.

use core::fmt::Write as _;

use nve_abi::{ComponentKind, NodeId, RawPropValue, ROOT_NODE_ID};

use crate::dom::is_truthy_raw_prop;
use crate::tree::Visit;
use crate::WebBackend;

/// Appends `val` escaped for a double-quoted attribute value.
pub(crate) fn push_attr_escaped(val: &str, out: &mut String) {
    for ch in val.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// Appends `val` escaped for element text content.
pub(crate) fn push_text_escaped(val: &str, out: &mut String) {
    for ch in val.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// Appends ` name="escaped value"`.
pub(crate) fn write_attr(name: &str, val: &str, out: &mut String) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_attr_escaped(val, out);
    out.push('"');
}

/// HTML void elements NVE can emit (no content, no end tag).
#[inline]
pub(crate) fn is_void_html_tag(tag: &str) -> bool {
    matches!(tag, "img" | "input" | "br" | "hr" | "meta" | "link")
}

/// Appends the end tag `</tag>`.
pub(crate) fn close_html_element(tag: &str, out: &mut String) {
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

impl WebBackend {
    /// Serializes the DOM mirror subtree rooted at `id` directly into `out` (inline-style
    /// diagnostic dump used by mirror tests and benchmarks; use [`Self::render_ssr`] for
    /// production Semantic HTML5 + Option 5 Hybrid CSS SSR).
    pub fn fmt_html_into(&self, id: NodeId, out: &mut String) {
        let mut open: Vec<&str> = Vec::new();
        self.walk(id, |nid, _, step| match step {
            Visit::Enter => self.open_html_element(nid, out).map(|tag| open.push(tag)).is_some(),
            Visit::Exit => {
                close_html_element(open.pop().unwrap_or("div"), out);
                false
            }
        });
    }

    /// Writes the start tag (and the text of a childless node); `Some(tag)` when the element
    /// has content to close.
    fn open_html_element(&self, id: NodeId, out: &mut String) -> Option<&str> {
        let el = self.nodes.get(&id)?;
        let tag = self.effective_html_tag(id, el);
        out.push('<');
        out.push_str(tag);
        out.push_str(" data-nve-id=\"");
        let _ = write!(out, "{id}");
        out.push('"');

        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        if el.kind == ComponentKind::Custom
            && let Some(nv) = self.native_views.get(&id)
        {
            crate::native_view::write_native_view_html_attrs(nv, out);
        }

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
        if el.kind == ComponentKind::TextInput {
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
        if el
            .raw_props
            .get("readOnly")
            .or_else(|| el.raw_props.get("read_only"))
            .or_else(|| el.raw_props.get("readonly"))
            .is_some_and(is_truthy_raw_prop)
        {
            out.push_str(" readonly");
        }

        let mut css = String::with_capacity(384);
        el.combined_css_into(&mut css);
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        if let Some(nv) = self.native_views.get(&id) {
            css.push_str(&nv.extra_css);
        }
        write_attr("style", &css, out);
        if let Some(dir) = crate::layout_ext_css::node_dir_attr(el.layout_ext.as_deref()) {
            write_attr("dir", dir, out);
        }
        out.push('>');
        if is_void_html_tag(tag) {
            return None;
        }
        // Text shows only while the node has no element children, as in the live DOM.
        if let Some(t) = el.text.as_deref().filter(|_| el.children.is_empty()) {
            push_text_escaped(t, out);
        }
        Some(tag)
    }

    pub fn render_html(&self) -> String {
        let mut out = String::with_capacity(self.nodes.len().saturating_mul(384).max(64));
        self.fmt_html_into(ROOT_NODE_ID, &mut out);
        out
    }
}
