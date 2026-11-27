//! Standalone embedded WebView capability (`packages/modules/webview`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_store::{NodeId, NveTable, Str, Transaction};

/// Embedded WebView table (`nve.web_view`).
#[derive(NveTable)]
#[nve_table(name = "nve.web_view", storage = "sparse", flags = "publish")]
pub struct WebView {
    #[effect(realize, paint)]
    pub url: Str,
    #[effect(semantics)]
    #[default(true)]
    pub allows_back_forward: bool,
}

/// Typed imperative command ops dispatched on [`WebView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebViewCommand {
    Navigate(String),
    Reload,
    GoBack,
    GoForward,
    EvaluateScript(String),
}

impl WebViewCommand {
    pub const OP_NAVIGATE: u32 = 1;
    pub const OP_RELOAD: u32 = 2;
    pub const OP_GO_BACK: u32 = 3;
    pub const OP_GO_FORWARD: u32 = 4;
    pub const OP_EVALUATE_SCRIPT: u32 = 5;

    pub fn dispatch(&self, tx: &mut Transaction<'_>, node: NodeId) {
        match self {
            Self::Navigate(url) => tx.command::<WebView>(node, Self::OP_NAVIGATE, url.as_bytes()),
            Self::Reload => tx.command::<WebView>(node, Self::OP_RELOAD, &[]),
            Self::GoBack => tx.command::<WebView>(node, Self::OP_GO_BACK, &[]),
            Self::GoForward => tx.command::<WebView>(node, Self::OP_GO_FORWARD, &[]),
            Self::EvaluateScript(js) => {
                tx.command::<WebView>(node, Self::OP_EVALUATE_SCRIPT, js.as_bytes())
            }
        }
    }

    pub fn decode(op: u32, payload: &[u8]) -> Option<Self> {
        match op {
            Self::OP_NAVIGATE => std::str::from_utf8(payload)
                .ok()
                .map(|s| Self::Navigate(s.to_owned())),
            Self::OP_RELOAD if payload.is_empty() => Some(Self::Reload),
            Self::OP_GO_BACK if payload.is_empty() => Some(Self::GoBack),
            Self::OP_GO_FORWARD if payload.is_empty() => Some(Self::GoForward),
            Self::OP_EVALUATE_SCRIPT => std::str::from_utf8(payload)
                .ok()
                .map(|s| Self::EvaluateScript(s.to_owned())),
            _ => None,
        }
    }
}

/// Typed `TableEvent` IDs emitted back from WebView hosts.
pub mod events {
    pub const LOAD_START: u32 = 1;
    pub const LOAD_END: u32 = 2;
    pub const ERROR: u32 = 3;
    pub const MESSAGE: u32 = 4;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, NodeStore};

    #[test]
    fn webview_table_and_commands_roundtrip() {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<WebView>(n).url("https://nve.dev");
        WebViewCommand::Navigate("https://nve.dev/docs".into()).dispatch(&mut tx, n);
        WebViewCommand::Reload.dispatch(&mut tx, n);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let row = store.try_row::<WebView>(n).unwrap();
        assert_eq!(row.url(), "https://nve.dev");
        assert!(row.allows_back_forward());
        assert_eq!(changes.commands.len(), 2);
        assert_eq!(
            WebViewCommand::decode(
                changes.commands[0].op,
                changes.command_payload(&changes.commands[0])
            ),
            Some(WebViewCommand::Navigate("https://nve.dev/docs".into()))
        );
        assert_eq!(
            WebViewCommand::decode(
                changes.commands[1].op,
                changes.command_payload(&changes.commands[1])
            ),
            Some(WebViewCommand::Reload)
        );
    }
}
