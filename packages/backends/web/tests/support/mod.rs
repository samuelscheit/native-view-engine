//! Shared `NodeStore` frame builders and a 3P semantic-tag `WebTableSystem` for the streaming
//! SSR + hydration integration suites.

#![allow(dead_code)]

use nve_backend_web::{WebBackend, WebTableSystem};
use nve_components::tables::{
    Accessibility, AriaRole, StreamBoundary, TextContent, Typography, Visual,
};
use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, Rgba, Str, TableSlot, Transaction};

/// 3P semantic tag override (supplies `<table>`/`<tr>`/`<select>`/... without module deps).
#[derive(NveTable)]
#[nve_table(name = "test.tag_override", storage = "sparse", flags = "publish")]
pub struct TagOverride {
    #[effect(realize)]
    tag: Str,
}

pub struct TagOverrideSystem;

impl WebTableSystem for TagOverrideSystem {
    fn slot(&self) -> TableSlot {
        TagOverride::slot()
    }

    fn tag_name<'a>(&self, store: &'a NodeStore, node: NodeId) -> Option<&'a str> {
        store.try_row::<TagOverride>(node).map(|r| r.tag())
    }
}

pub fn web() -> WebBackend {
    let mut web = WebBackend::new();
    web.register_table_system(TagOverrideSystem);
    web
}

/// Builds one frame into `store` and commits it into `web`.
pub fn commit(
    web: &mut WebBackend,
    store: &mut NodeStore,
    build: impl FnOnce(&mut Transaction<'_>),
) {
    let mut tx = store.tx();
    build(&mut tx);
    let mut changes = ChangeSet::default();
    tx.commit_into(&mut changes);
    web.commit(store, &changes).expect("commit frame");
}

pub fn node(tx: &mut Transaction<'_>, id: u32) -> NodeId {
    let n = NodeId(id);
    assert!(tx.create_at(n), "node {id} already live");
    n
}

pub fn text(tx: &mut Transaction<'_>, id: u32, s: &str) -> NodeId {
    let n = node(tx, id);
    tx.edit::<TextContent>(n).text(s);
    n
}

pub fn tagged(tx: &mut Transaction<'_>, id: u32, tag: &str) -> NodeId {
    let n = node(tx, id);
    tx.edit::<TagOverride>(n).tag(tag);
    n
}

pub fn suspense(tx: &mut Transaction<'_>, id: u32, slot: &str) -> NodeId {
    let n = node(tx, id);
    tx.edit::<StreamBoundary>(n).suspense(slot);
    n
}

pub fn stream_slot(tx: &mut Transaction<'_>, id: u32, slot: &str) -> NodeId {
    let n = node(tx, id);
    tx.edit::<StreamBoundary>(n).stream_slot(slot);
    n
}

pub fn shared_style(tx: &mut Transaction<'_>, n: NodeId) {
    tx.edit::<Visual>(n).background(Rgba::rgb(15, 23, 42));
    tx.edit::<Typography>(n).color(Rgba::WHITE);
}

pub fn delta_style(tx: &mut Transaction<'_>, n: NodeId) {
    tx.edit::<Visual>(n)
        .background(Rgba::rgb(22, 163, 74))
        .border_radius(10.0);
}

pub fn heading(tx: &mut Transaction<'_>, id: u32) -> NodeId {
    let n = text(tx, id, "Dashboard");
    tx.edit::<Accessibility>(n).role(AriaRole::Heading).level(1);
    shared_style(tx, n);
    n
}

/// `[heading, suspense(slot1) -> ..., suspense(slot2) -> ...]` dashboard with ids `base + {2..}`.
pub fn pending_slot(
    tx: &mut Transaction<'_>,
    sus: u32,
    slot_node: u32,
    fb: u32,
    slot: &str,
) -> NodeId {
    let s = suspense(tx, sus, slot);
    let p = stream_slot(tx, slot_node, slot);
    let f = text(tx, fb, &format!("Loading {slot}..."));
    tx.set_children(p, &[f]);
    tx.set_children(s, &[p]);
    s
}

pub fn no_new_css_rules(web: &WebBackend) -> bool {
    web.stylesheet().serialize_css().1.is_empty()
}
