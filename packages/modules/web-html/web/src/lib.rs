//! Web DOM & SSR adapters for `nve-web-html` (`WebTableSystem`).
//!
//! 100% uniform plugins implementing BOTH live DOM (`commit_node`) and SSR (`write_ssr_*`)
//! for all 6 HTML5 tables (`WebHtml`, `HtmlSemantics`, `HtmlInvoker`, `HtmlForm`,
//! `HtmlFormControl`, `HtmlDataAttrs`) with zero special-casing inside `nve-backend-web`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod overlay;
mod semantics;

pub use overlay::{should_close_dialog_on_backdrop_click, DialogRect};
use nve_backend_web::html::write_attr as push_attr;
use nve_backend_web::{
    is_safe_custom_tag, is_safe_web_attr, is_safe_web_url, WebSystemCommitCtx, WebTableRegistry,
    WebTableSystem,
};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};
use nve_web_html::{
    DataAttrList, HtmlDataAttrs, HtmlForm, HtmlFormControl, HtmlInvoker, WebHtml,
};
pub use semantics::HtmlSemanticsWebSystem;

/// Registers every `nve-web-html` live + SSR plugin onto `registry`.
pub fn register_web_html_systems(registry: &mut WebTableRegistry) {
    registry
        .register(WebHtmlWebSystem)
        .register(HtmlSemanticsWebSystem)
        .register(HtmlInvokerWebSystem)
        .register(HtmlFormWebSystem)
        .register(HtmlFormControlWebSystem)
        .register(HtmlDataAttrsWebSystem);
}

/// `WebTableSystem` plugin for [`WebHtml`] (`nve.web_html`).
///
/// # Security Contract
///
/// [`WebHtml::html`] is an explicit trusted-HTML escape hatch (analogous to `dangerouslySetInnerHTML`).
/// Its bytes are emitted verbatim into SSR output and `Element::set_inner_html`. Callers MUST only
/// pass static/author-controlled markup or HTML sanitized by an allow-list sanitizer before writing
/// into `WebHtml::html`.
#[derive(Debug, Default, Clone, Copy)]
pub struct WebHtmlWebSystem;

impl WebTableSystem for WebHtmlWebSystem {
    fn slot(&self) -> TableSlot {
        WebHtml::slot()
    }

    fn priority(&self) -> u8 {
        105
    }

    fn tag_name<'a>(&self, store: &'a NodeStore, node: NodeId) -> Option<&'a str> {
        let row = store.try_row::<WebHtml>(node)?;
        let tag = row.tag();
        if !tag.is_empty() && is_safe_custom_tag(tag) {
            Some(tag)
        } else {
            Some("div")
        }
    }

    fn write_ssr_inner(&self, store: &NodeStore, node: NodeId, out: &mut String) -> bool {
        let Some(row) = store.try_row::<WebHtml>(node) else {
            return false;
        };
        out.push_str(row.html());
        true
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        if let Some(row) = store.try_row::<WebHtml>(node) {
            ctx.set_inner_html(node, row.html().to_owned());
        }
    }
}

/// `WebTableSystem` plugin for [`HtmlInvoker`] (`nve.html.invoker`).
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlInvokerWebSystem;

impl WebTableSystem for HtmlInvokerWebSystem {
    fn slot(&self) -> TableSlot {
        HtmlInvoker::slot()
    }

    fn priority(&self) -> u8 {
        40
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<HtmlInvoker>(node) else {
            return;
        };
        let pt = row.popover_target();
        if !pt.is_empty() {
            push_attr("popovertarget", pt, out);
        }
        let cf = row.command_for();
        if !cf.is_empty() {
            push_attr("commandfor", cf, out);
        }
        let cmd = row.command();
        if !cmd.is_empty() {
            push_attr("command", cmd, out);
        }
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        let Some(row) = store.try_row::<HtmlInvoker>(node) else {
            return;
        };
        for (name, val) in [
            ("popovertarget", row.popover_target()),
            ("commandfor", row.command_for()),
            ("command", row.command()),
        ] {
            if !val.is_empty() {
                ctx.set_attr(node, name, val);
            } else {
                ctx.remove_attr(node, name);
            }
        }
    }
}

/// `WebTableSystem` plugin for [`HtmlForm`] (`nve.html.form`).
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlFormWebSystem;

impl WebTableSystem for HtmlFormWebSystem {
    fn slot(&self) -> TableSlot {
        HtmlForm::slot()
    }

    fn priority(&self) -> u8 {
        50
    }

    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        Some("form")
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<HtmlForm>(node) else {
            return;
        };
        let action = row.action();
        if !action.is_empty() && is_safe_web_url(action) {
            push_attr("action", action, out);
        }
        let target = row.target();
        if !target.is_empty() {
            push_attr("target", target, out);
        }
        let name = row.name();
        if !name.is_empty() {
            push_attr("name", name, out);
        }
        if let Some(m) = row.method() {
            push_attr("method", m.as_html_attr(), out);
        }
        if let Some(e) = row.enctype() {
            push_attr("enctype", e.as_html_attr(), out);
        }
        if row.novalidate() {
            out.push_str(" novalidate");
        }
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        let Some(row) = store.try_row::<HtmlForm>(node) else {
            return;
        };
        let action = row.action();
        if !action.is_empty() && is_safe_web_url(action) {
            ctx.set_attr(node, "action", action);
        } else {
            ctx.remove_attr(node, "action");
        }
        for (attr, val) in [("target", row.target()), ("name", row.name())] {
            if !val.is_empty() {
                ctx.set_attr(node, attr, val);
            } else {
                ctx.remove_attr(node, attr);
            }
        }
        if let Some(m) = row.method() {
            ctx.set_attr(node, "method", m.as_html_attr());
        } else {
            ctx.remove_attr(node, "method");
        }
        if let Some(e) = row.enctype() {
            ctx.set_attr(node, "enctype", e.as_html_attr());
        } else {
            ctx.remove_attr(node, "enctype");
        }
        if row.novalidate() {
            ctx.set_attr(node, "novalidate", "");
        } else {
            ctx.remove_attr(node, "novalidate");
        }
    }
}

/// `WebTableSystem` plugin for [`HtmlFormControl`] (`nve.html.form_control`).
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlFormControlWebSystem;

impl WebTableSystem for HtmlFormControlWebSystem {
    fn slot(&self) -> TableSlot {
        HtmlFormControl::slot()
    }

    fn priority(&self) -> u8 {
        40
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<HtmlFormControl>(node) else {
            return;
        };
        let name = row.name();
        if !name.is_empty() {
            push_attr("name", name, out);
        }
        let form = row.form();
        if !form.is_empty() {
            push_attr("form", form, out);
        }
        let val = row.value();
        if !val.is_empty() {
            push_attr("value", val, out);
        }
        let fa = row.form_action();
        if !fa.is_empty() && is_safe_web_url(fa) {
            push_attr("formaction", fa, out);
        }
        if let Some(bt) = row.button_type() {
            push_attr("type", bt.as_html_attr(), out);
        }
        if let Some(fm) = row.form_method() {
            push_attr("formmethod", fm.as_html_attr(), out);
        }
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        let Some(row) = store.try_row::<HtmlFormControl>(node) else {
            return;
        };
        for (attr, val) in [
            ("name", row.name()),
            ("form", row.form()),
            ("value", row.value()),
        ] {
            if !val.is_empty() {
                ctx.set_attr(node, attr, val);
            } else {
                ctx.remove_attr(node, attr);
            }
        }
        let fa = row.form_action();
        if !fa.is_empty() && is_safe_web_url(fa) {
            ctx.set_attr(node, "formaction", fa);
        } else {
            ctx.remove_attr(node, "formaction");
        }
        if let Some(bt) = row.button_type() {
            ctx.set_attr(node, "type", bt.as_html_attr());
        } else {
            ctx.remove_attr(node, "type");
        }
        if let Some(fm) = row.form_method() {
            ctx.set_attr(node, "formmethod", fm.as_html_attr());
        } else {
            ctx.remove_attr(node, "formmethod");
        }
    }
}

/// `WebTableSystem` plugin for [`HtmlDataAttrs`] (`nve.html.data_attrs`).
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlDataAttrsWebSystem;

impl WebTableSystem for HtmlDataAttrsWebSystem {
    fn slot(&self) -> TableSlot {
        HtmlDataAttrs::slot()
    }

    fn priority(&self) -> u8 {
        30
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<HtmlDataAttrs>(node) else {
            return;
        };
        let Some(list) = store.resource::<DataAttrList>(row.entries()) else {
            return;
        };
        for (k, v) in list.iter() {
            let full: std::borrow::Cow<'_, str> = if k.starts_with("data-") {
                std::borrow::Cow::Borrowed(k)
            } else {
                std::borrow::Cow::Owned(format!("data-{k}"))
            };
            if is_safe_web_attr(&full, v) {
                push_attr(&full, v, out);
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
        let Some(row) = store.try_row::<HtmlDataAttrs>(node) else {
            return;
        };
        let list = store.resource::<DataAttrList>(row.entries());
        if let Some(list) = list {
            for (k, v) in list.iter() {
                let full: std::borrow::Cow<'_, str> = if k.starts_with("data-") {
                    std::borrow::Cow::Borrowed(k)
                } else {
                    std::borrow::Cow::Owned(format!("data-{k}"))
                };
                if is_safe_web_attr(&full, v) {
                    ctx.set_attr(node, &full, v);
                }
            }
        }
        ctx.retain_slot_attrs(node, |existing| {
            list.is_some_and(|l| {
                l.iter().any(|(k, v)| {
                    let matches_name = if k.starts_with("data-") {
                        k == existing
                    } else {
                        existing
                            .strip_prefix("data-")
                            .is_some_and(|rest| rest == k)
                    };
                    matches_name && is_safe_web_attr(existing, v)
                })
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_backend_web::WebStoreRenderer;
    use nve_engine::StoreBackend;
    use nve_store::ChangeSet;
    use nve_backend_web::html::FOSTER_PARENT_TRAP_TAGS;
    use nve_backend_web::{StreamingError, WebBackend};
    use nve_tables::tables::StreamBoundary;
    use nve_web_html::{FormMethod, HtmlSemantics, SemanticElement};

    /// `<select>` without `<option>` children is unusable, so `SemanticElement` deliberately does
    /// not offer it; 3P plugins produce it (see [`SelectTagSystem`]).
    const PLUGIN_ONLY_TRAP_TAGS: &[&str] = &["select"];

    /// Test-only plugin rendering any node carrying `HtmlDataAttrs` as `<select>`.
    struct SelectTagSystem;

    impl WebTableSystem for SelectTagSystem {
        fn slot(&self) -> TableSlot {
            HtmlDataAttrs::slot()
        }

        fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
            Some("select")
        }
    }

    /// Every foster-parenting trap tag `SemanticElement` offers round-trips through `parse` /
    /// `as_html_tag`; the rest are exactly the documented plugin-only tags.
    #[test]
    fn every_foster_parent_trap_tag_round_trips_through_semantic_element() {
        for &tag in FOSTER_PARENT_TRAP_TAGS {
            let el = SemanticElement::parse(tag);
            if PLUGIN_ONLY_TRAP_TAGS.contains(&tag) {
                assert_eq!(el, None, "{tag} must stay plugin-only");
                continue;
            }
            let el = el.unwrap_or_else(|| panic!("{tag} has no SemanticElement"));
            assert_eq!(el.as_html_tag(), tag);
            assert_eq!(SemanticElement::parse(&format!(" {} ", tag.to_uppercase())), Some(el));
        }
        for &el in SemanticElement::ALL {
            let parsed = SemanticElement::parse(el.as_html_tag());
            assert_eq!(parsed, Some(el), "{el:?} must round-trip");
        }
        assert_eq!(SemanticElement::parse("marquee"), None);
    }

    #[test]
    fn stream_slot_directly_inside_semantic_tfoot_or_plugin_select_fails_placement_hygiene() {
        for tag in ["tfoot", "select"] {
            let mut store = NodeStore::new();
            let mut changes = ChangeSet::default();
            {
                let mut tx = store.tx();
                let parent = tx.create();
                let slot = tx.create();
                if tag == "select" {
                    tx.edit::<HtmlDataAttrs>(parent).entries(None);
                } else {
                    tx.edit::<HtmlSemantics>(parent).element(Some(SemanticElement::Tfoot));
                }
                tx.edit::<StreamBoundary>(slot).stream_slot("rows");
                tx.set_children(parent, &[slot]);
                tx.set_children(NodeId::ROOT, &[parent]);
                tx.commit_into(&mut changes);
            }
            let mut web = WebBackend::new();
            register_web_html_systems(web.table_registry_mut());
            web.table_registry_mut().register(SelectTagSystem);
            web.commit(&store, &changes).unwrap();
            match web.render_ssr_streaming_shell(&store) {
                Err(StreamingError::InvalidSlotPlacement(msg)) => {
                    assert!(msg.contains(&format!("<{tag}>")), "{msg}");
                }
                other => panic!("<{tag}> parent must be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn web_html_and_html5_plugins_work_uniformly_on_live_and_ssr() {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let html_n;
        let dialog_n;
        let form_n;
        {
            let mut tx = store.tx();
            html_n = tx.create();
            dialog_n = tx.create();
            form_n = tx.create();
            tx.set_children(NodeId::ROOT, &[html_n, dialog_n, form_n]);
            tx.edit::<WebHtml>(html_n)
                .tag("article")
                .html("<h1>Hello</h1><p>World</p>");
            tx.edit::<HtmlSemantics>(dialog_n)
                .element(Some(SemanticElement::Dialog))
                .open(true);
            tx.edit::<HtmlDataAttrs>(dialog_n)
                .entries(Some(DataAttrList::from_pairs(vec![(
                    "data-state".into(),
                    "open".into(),
                )])));
            tx.edit::<HtmlForm>(form_n)
                .action("/api/save")
                .method(Some(FormMethod::Post))
                .novalidate(true);
            tx.commit_into(&mut changes);
        }

        // Without plugins registered, WebStoreRenderer has ZERO built-in knowledge of HTML5 tables.
        let mut bare = WebStoreRenderer::new();
        bare.commit(&store, &changes).unwrap();
        assert_eq!(bare.realized(html_n).unwrap().tag, "div");
        assert_eq!(bare.realized(dialog_n).unwrap().tag, "div");
        assert_eq!(bare.realized(form_n).unwrap().tag, "div");
        assert!(bare.plugin_inner_html(html_n).is_none());
        assert!(!bare.render_ssr(&store, NodeId::ROOT).contains("<h1>Hello</h1>"));

        // With uniform plugins registered, BOTH live DOM and SSR emit full HTML5 semantics.
        let mut with_plugins = WebStoreRenderer::new();
        register_web_html_systems(with_plugins.registry_mut());
        with_plugins.commit(&store, &changes).unwrap();
        assert_eq!(with_plugins.realized(html_n).unwrap().tag, "article");
        assert_eq!(with_plugins.realized(dialog_n).unwrap().tag, "dialog");
        assert_eq!(with_plugins.realized(form_n).unwrap().tag, "form");
        assert_eq!(
            with_plugins.plugin_inner_html(html_n),
            Some("<h1>Hello</h1><p>World</p>")
        );
        let ssr = with_plugins.render_ssr(&store, NodeId::ROOT);
        assert!(ssr.contains("<article "));
        assert!(ssr.contains("<h1>Hello</h1><p>World</p></article>"));
        assert!(ssr.contains("<dialog "));
        assert!(ssr.contains(" open"));
        assert!(ssr.contains("data-state=\"open\""));
        assert!(ssr.contains("<form "));
        assert!(ssr.contains("action=\"/api/save\""));
        assert!(ssr.contains("method=\"post\""));
        assert!(ssr.contains(" novalidate"));
    }
}
