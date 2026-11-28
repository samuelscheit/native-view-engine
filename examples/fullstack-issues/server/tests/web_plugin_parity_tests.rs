//! The issues SSR server and the browser shell's issues session register ONE shared plugin set
//! (`nve_example_issues_client::register_web_systems`), so Frame-0 tags never diverge.

use nve_abi::{NodeId, SurfaceId};
use nve_backend_web::{WebBackend, WebTableRegistry};
use nve_canvas::Canvas;
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_engine::Viewport;
use nve_example_issues_server::ssr::issues_web_backend;
use nve_store::{ChangeSet, NodeStore, NveTable, TableSlot};
use nve_web_html::{
    DataAttrList, HtmlDataAttrs, HtmlForm, HtmlSemantics, SemanticElement, WebHtml,
};
use nve_web_shell::WebAppSession;

fn empty() -> Element {
    rsx! {}
}

fn slots(registry: &WebTableRegistry) -> Vec<TableSlot> {
    let mut slots: Vec<TableSlot> = registry.iter().iter().map(|sys| sys.slot()).collect();
    slots.sort_unstable();
    slots
}

/// One node per web table kind the issues tree can carry, plus a canvas node.
fn probe_store() -> (NodeStore, Vec<NodeId>) {
    nve_tables::tables::register_core_tables();
    nve_web_html::register_web_html_tables();
    let mut store = NodeStore::new();
    let mut nodes = Vec::new();
    {
        let mut tx = store.tx();
        let _ = tx.create_at(NodeId::ROOT);
        for kind in 0..6 {
            let n = tx.create();
            match kind {
                0 => {
                    tx.edit::<HtmlForm>(n).action("/notes/save");
                }
                1 => {
                    tx.edit::<HtmlSemantics>(n)
                        .element(Some(SemanticElement::Dialog));
                }
                2 => {
                    tx.edit::<HtmlSemantics>(n)
                        .element(Some(SemanticElement::Tfoot));
                }
                3 => {
                    tx.edit::<WebHtml>(n).tag("article").html("<p>x</p>");
                }
                4 => {
                    tx.edit::<HtmlDataAttrs>(n)
                        .entries(Some(DataAttrList::from_pairs(vec![(
                            "k".into(),
                            "v".into(),
                        )])));
                }
                _ => {
                    tx.edit::<Canvas>(n);
                }
            }
            nodes.push(n);
        }
        tx.set_children(NodeId::ROOT, &nodes);
        tx.commit_into(&mut ChangeSet::default());
    }
    (store, nodes)
}

#[test]
fn issues_server_and_browser_session_register_identical_plugins_and_resolve_identical_tags() {
    let server = issues_web_backend();
    let session = WebAppSession::new(
        SurfaceId::ROOT,
        DioxusSurface::new(SurfaceId::ROOT, empty),
        nve_web_shell::register_issues_web_systems,
        WebBackend::new(),
        Viewport::default(),
    );
    let client = session.backend();
    assert_eq!(
        slots(server.table_registry()),
        slots(client.table_registry())
    );
    assert!(
        !slots(server.table_registry()).contains(&Canvas::slot()),
        "the issues family draws no canvas; registering it only bloats the bundle"
    );

    let (store, nodes) = probe_store();
    let tags: Vec<(&str, &str)> = nodes
        .iter()
        .map(|&n| {
            (
                server.table_registry().resolve_tag(&store, n).0,
                client.table_registry().resolve_tag(&store, n).0,
            )
        })
        .collect();
    for (server_tag, client_tag) in &tags {
        assert_eq!(server_tag, client_tag);
    }
    let server_tags: Vec<&str> = tags.iter().map(|(s, _)| *s).collect();
    assert_eq!(
        server_tags,
        ["form", "dialog", "tfoot", "article", "div", "div"]
    );
}
