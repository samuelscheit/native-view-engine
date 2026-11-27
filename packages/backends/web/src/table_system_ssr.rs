//! Deterministic SSR rendering for [`WebStoreRenderer`] and shared core SSR attribute emission.

use core::fmt::Write as _;

use nve_components::tables::{
    Accessibility, DebugId, Image, ImageSrc, Interaction, Layout, Link, LinkTarget, Scroll,
    TextContent, TextInput, Transform, Visibility, Visual,
};
use nve_store::{NodeId, NodeStore};

use super::{is_safe_web_url, write_transform_css, WebStoreRenderer};
use crate::html::{
    close_html_element, is_void_html_tag, push_text_escaped, write_attr,
};

/// Emits shared core HTML attributes (`DebugId`, `Link`, `Image`, `TextInput`, `Interaction`,
/// `Accessibility`) for a single `NodeStore` node, used by both [`crate::WebBackend::render_ssr`]
/// and [`WebStoreRenderer::render_ssr`].
pub(crate) fn write_core_ssr_attrs(store: &NodeStore, id: NodeId, out: &mut String) {
    if let Some(dbg) = store.try_row::<DebugId>(id) {
        let id_s = dbg.id();
        if !id_s.is_empty() {
            write_attr("id", id_s, out);
        }
        let tid = dbg.test_id();
        if !tid.is_empty() {
            write_attr("data-testid", tid, out);
        }
        let slot_s = dbg.slot();
        if !slot_s.is_empty() {
            write_attr("slot", slot_s, out);
        }
    }
    if let Some(link) = store.try_row::<Link>(id) {
        let href = link.href();
        if is_safe_web_url(href) {
            write_attr("href", href, out);
        }
        let is_blank = link.target() == LinkTarget::Blank;
        match link.target() {
            LinkTarget::Default | LinkTarget::Self_ => {}
            LinkTarget::Blank => write_attr("target", "_blank", out),
            LinkTarget::Parent => write_attr("target", "_parent", out),
        }
        let rel_bits = if is_blank {
            link.rel() | 1 | 2
        } else {
            link.rel()
        };
        if rel_bits != 0 {
            let mut rel = String::new();
            for (bit, kw) in [(1u8, "noopener"), (2u8, "noreferrer"), (4u8, "nofollow")] {
                if rel_bits & bit != 0 {
                    if !rel.is_empty() {
                        rel.push(' ');
                    }
                    rel.push_str(kw);
                }
            }
            if !rel.is_empty() {
                write_attr("rel", &rel, out);
            }
        }
    }
    if let Some(img) = store.try_row::<Image>(id) {
        if !img.src().is_none()
            && let Some(ImageSrc::Url(url)) = store.resource::<ImageSrc>(img.src())
            && is_safe_web_url(url)
        {
            write_attr("src", url, out);
        }
        let alt = img.alt();
        if !alt.is_empty() {
            write_attr("alt", alt, out);
        }
    }
    if let Some(ti) = store.try_row::<TextInput>(id) {
        if !ti.multiline() {
            if ti.secure() {
                write_attr("type", "password", out);
            } else {
                let html_type = ti.input_kind().as_html_type();
                if html_type != "text" {
                    write_attr("type", html_type, out);
                }
            }
            let v = ti.value();
            if !v.is_empty() {
                write_attr("value", v, out);
            }
        }
        let ph = ti.placeholder();
        if !ph.is_empty() {
            write_attr("placeholder", ph, out);
        }
        if let Some(ac) = ti.autofill().get() {
            write_attr("autocomplete", ac.as_html_autocomplete(), out);
        }
        if ti.input_mode() != nve_components::InputMode::Text {
            write_attr("inputmode", ti.input_mode().as_html_inputmode(), out);
        }
        if ti.required() {
            out.push_str(" required");
        }
        if ti.read_only() {
            out.push_str(" readonly");
        }
        if ti.max_length() > 0 && ti.max_length() < u16::MAX {
            let _ = write!(out, " maxlength=\"{}\"", ti.max_length());
        }
    }
    if let Some(inter) = store.try_row::<Interaction>(id) {
        if inter.disabled() {
            out.push_str(" disabled aria-disabled=\"true\"");
        }
        if inter.tab_index_set() {
            let _ = write!(out, " tabindex=\"{}\"", inter.tab_index());
        }
    }
    if let Some(a11y) = store.try_row::<Accessibility>(id) {
        let role = a11y.role().as_aria_str();
        if !role.is_empty() {
            write_attr("role", role, out);
        }
        let label = a11y.label();
        if !label.is_empty() {
            write_attr("aria-label", label, out);
        }
        if let Some(live) = a11y.live().as_aria_str() {
            write_attr("aria-live", live, out);
        }
        if a11y.hidden() {
            write_attr("aria-hidden", "true", out);
        }
    }
}

impl WebStoreRenderer {
    /// Renders deterministic SSR HTML directly from `store` + installed [`WebTableSystem`] plugins.
    pub fn render_ssr(&self, store: &NodeStore, root: NodeId) -> String {
        let mut out = String::with_capacity(1024);
        self.write_ssr_node(store, root, &mut out);
        out
    }

    fn write_ssr_node(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        if !store.is_live(node) {
            return;
        }
        let (tag, _ns) = self.registry.resolve_tag(store, node);
        out.push('<');
        out.push_str(tag);
        let _ = write!(out, " data-nve-id=\"{}\"", node.0);

        write_core_ssr_attrs(store, node, out);

        // Inline style (core Visual/Transform + plugin inline CSS)
        let mut style = String::new();
        if let Some(vis) = store.try_row::<Visual>(node) {
            if vis.opacity() != 1.0 {
                let _ = write!(&mut style, "opacity:{};", vis.opacity());
            }
            match vis.visibility() {
                Visibility::Inherit => {}
                Visibility::Visible => style.push_str("visibility:visible;"),
                Visibility::Hidden => style.push_str("visibility:hidden;"),
            }
        }
        if let Some(tf) = store.try_row::<Transform>(node) {
            write_transform_css(tf, &mut style);
        }
        if store.has::<Scroll>(node) {
            style.push_str("overflow:auto;");
        }
        if let Some(l) = store.try_row::<Layout>(node)
            && l.display() == nve_components::tables::Display::None
        {
            style.push_str("display:none;");
        }
        for sys in self.registry.iter() {
            if store.has_slot(node, sys.slot()) {
                sys.write_inline_css(store, node, &mut style);
            }
        }
        if !style.is_empty() {
            write_attr("style", &style, out);
        }

        // Uniform plugin SSR attributes
        for sys in self.registry.iter() {
            if store.has_slot(node, sys.slot()) {
                sys.write_ssr_attrs(store, node, out);
            }
        }

        out.push('>');
        if is_void_html_tag(tag) {
            return;
        }

        // Uniform plugin SSR inner HTML (e.g. Svg, WebHtml)
        let mut owned_inner = false;
        for sys in self.registry.iter() {
            if store.has_slot(node, sys.slot()) && sys.write_ssr_inner(store, node, out) {
                owned_inner = true;
                break;
            }
        }

        if !owned_inner {
            if let Some(ti) = store.try_row::<TextInput>(node) {
                if ti.multiline() {
                    push_text_escaped(ti.value(), out);
                }
            } else if let Some(tc) = store.try_row::<TextContent>(node) {
                push_text_escaped(tc.text(), out);
            }
            for &child in store.children(node) {
                self.write_ssr_node(store, child, out);
            }
        }

        close_html_element(tag, out);
    }
}
