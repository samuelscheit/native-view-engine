//! Web DOM & SSR adapters for `nve-web-html` (`WebTableSystem`).
//!
//! 100% uniform plugins implementing BOTH live DOM (`commit_node`) and SSR (`write_ssr_*`)
//! for all 6 HTML5 tables (`WebHtml`, `HtmlSemantics`, `HtmlInvoker`, `HtmlForm`,
//! `HtmlFormControl`, `HtmlDataAttrs`) with zero special-casing inside `nve-backend-web`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_backend_web::html::write_attr as push_attr;
use nve_backend_web::{
    is_safe_custom_tag, is_safe_web_attr, is_safe_web_url, WebSystemCommitCtx, WebTableRegistry,
    WebTableSystem,
};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};
use nve_web_html::{
    DataAttrList, HtmlDataAttrs, HtmlForm, HtmlFormControl, HtmlInvoker, HtmlSemantics, WebHtml,
};

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
        if row.open() {
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
        if row.data_nve_open() {
            push_attr("data-nve-open", "true", out);
        }
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
        if row.open() {
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
        if row.data_nve_open() {
            ctx.set_attr(node, "data-nve-open", "true");
        } else {
            ctx.remove_attr(node, "data-nve-open");
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
    use nve_web_html::{FormMethod, SemanticElement};

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
