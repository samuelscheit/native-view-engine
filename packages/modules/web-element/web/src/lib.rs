//! Web DOM & SSR adapter for `nve-web-element` (`WebTableSystem`).

use nve_backend_web::html::write_attr;
use nve_backend_web::{is_safe_custom_tag, is_safe_web_attr, WebSystemCommitCtx, WebTableSystem};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};
use nve_web_element::{WebElement, WebElementAttrs};

/// `WebTableSystem` plugin for [`WebElement`] (`nve.web_element`).
#[derive(Debug, Default, Clone, Copy)]
pub struct WebElementWebSystem;

impl WebTableSystem for WebElementWebSystem {
    fn slot(&self) -> TableSlot {
        WebElement::slot()
    }

    fn priority(&self) -> u8 {
        105
    }

    fn tag_name<'a>(&self, store: &'a NodeStore, node: NodeId) -> Option<&'a str> {
        let row = store.try_row::<WebElement>(node)?;
        let tag = row.tag();
        is_safe_custom_tag(tag).then_some(tag)
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<WebElement>(node) else {
            return;
        };
        let Some(attrs) = store.resource::<WebElementAttrs>(row.attrs()) else {
            return;
        };
        for (k, v) in attrs.0.iter() {
            if is_safe_web_attr(k, v) {
                write_attr(k.trim(), v, out);
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
        let Some(row) = store.try_row::<WebElement>(node) else {
            return;
        };
        let attrs = store.resource::<WebElementAttrs>(row.attrs());
        if let Some(attrs) = attrs {
            for (k, v) in attrs.0.iter() {
                if is_safe_web_attr(k, v) {
                    ctx.set_attr(node, k, v);
                }
            }
        }
        ctx.retain_slot_attrs(node, |existing| {
            attrs.is_some_and(|a| {
                a.0.iter()
                    .any(|(k, v)| k == existing && is_safe_web_attr(k, v))
            })
        });
    }
}
