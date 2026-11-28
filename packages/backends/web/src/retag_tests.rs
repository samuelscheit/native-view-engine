//! Generic plugin-lifecycle guards for imperatively owned attributes (`forget_attr`) across a
//! retag, and for cross-table watchers (`WebTableSystem::watches_slot`).

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

use nve_abi::hash::HashMap;
use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, TableSlot};

use crate::table_system::RetagElementSink;
use crate::{WebStoreRenderer, WebSystemCommitCtx, WebTableSystem};

#[derive(Default)]
struct MockEl {
    attrs: RefCell<Vec<(String, String)>>,
}
impl RetagElementSink for MockEl {
    fn set_attr(&self, name: &str, value: &str) {
        self.attrs
            .borrow_mut()
            .push((name.to_owned(), value.to_owned()));
    }
    fn set_inner_html(&self, _html: &str) {}
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "web.test.disclosure", storage = "sparse")]
struct Disclosure {
    #[effect(paint)]
    imperative: bool,
    #[effect(paint)]
    open: bool,
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "web.test.modality", storage = "sparse")]
struct Modality {
    #[effect(paint)]
    modal: bool,
}

/// `<details>` mirrors `open`; once retagged to `<dialog>` the plugin owns `open` imperatively.
struct DisclosureSys {
    watched_commits: &'static AtomicU32,
}
impl WebTableSystem for DisclosureSys {
    fn slot(&self) -> TableSlot {
        Disclosure::slot()
    }
    fn tag_name<'a>(&self, store: &'a NodeStore, node: NodeId) -> Option<&'a str> {
        let row = store.try_row::<Disclosure>(node)?;
        Some(if row.imperative() {
            "dialog"
        } else {
            "details"
        })
    }
    fn watches_slot(&self, slot: TableSlot) -> bool {
        slot == Modality::slot()
    }
    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        dirty: u32,
    ) {
        let Some(row) = store.try_row::<Disclosure>(node) else {
            return;
        };
        if dirty == 0 {
            self.watched_commits.fetch_add(1, Ordering::Relaxed);
        }
        if ctx.resolved_tag(store, node) == "dialog" {
            ctx.forget_attr(node, "open");
        } else if row.open() {
            ctx.set_attr(node, "open", "");
        } else {
            ctx.remove_attr(node, "open");
        }
    }
}

fn frame(store: &mut NodeStore, build: impl FnOnce(&mut nve_store::Transaction<'_>)) -> ChangeSet {
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    build(&mut tx);
    tx.commit_into(&mut changes);
    changes
}

#[test]
fn forgotten_attr_is_dropped_from_mirror_and_never_replayed_onto_a_retagged_element() {
    static WATCHED: AtomicU32 = AtomicU32::new(0);
    let mut store = NodeStore::new();
    let mut renderer = WebStoreRenderer::new().with_table_system(DisclosureSys {
        watched_commits: &WATCHED,
    });
    let mut n = NodeId::ROOT;
    let changes = frame(&mut store, |tx| {
        n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Disclosure>(n).open(true);
    });
    renderer.commit(&store, &changes).unwrap();
    assert_eq!(renderer.realized(n).unwrap().tag, "details");
    assert_eq!(
        renderer.plugin_attrs(n),
        &[("open".to_owned(), String::new())]
    );

    // `<details open>` -> `<dialog>`: the stale mirrored `open` must not reach the fresh element.
    let changes = frame(&mut store, |tx| {
        tx.edit::<Disclosure>(n).imperative(true);
    });
    let mut live: HashMap<NodeId, MockEl> = HashMap::default();
    live.insert(n, MockEl::default());
    renderer.commit_plugins_active_with(
        &store,
        &changes,
        #[cfg(target_arch = "wasm32")]
        None,
        Some(&live),
        &[n],
        |_, _, _| {},
    );
    let replayed = live[&n].attrs.borrow();
    assert!(
        !replayed.iter().any(|(k, _)| k == "open"),
        "stale open replayed: {replayed:?}"
    );
    assert!(renderer.plugin_attrs(n).iter().all(|(k, _)| k != "open"));
}

#[test]
fn watched_table_changes_recommit_watchers_only_on_nodes_carrying_their_table() {
    static WATCHED: AtomicU32 = AtomicU32::new(0);
    let mut store = NodeStore::new();
    let mut renderer = WebStoreRenderer::new().with_table_system(DisclosureSys {
        watched_commits: &WATCHED,
    });
    let (mut dialog, mut plain) = (NodeId::ROOT, NodeId::ROOT);
    let changes = frame(&mut store, |tx| {
        dialog = tx.create();
        plain = tx.create();
        tx.set_children(NodeId::ROOT, &[dialog, plain]);
        tx.edit::<Disclosure>(dialog).imperative(true).open(true);
        tx.edit::<Modality>(dialog).modal(true);
        tx.edit::<Modality>(plain).modal(true);
    });
    renderer.commit(&store, &changes).unwrap();
    // Frame 1 commits the watcher's own row (dirty) AND re-commits it for the `Modality` change.
    let base = WATCHED.load(Ordering::Relaxed);
    assert_eq!(
        base, 1,
        "one watched re-commit (dialog only; `plain` lacks the table)"
    );

    // A modal-only edit (watched table) re-commits the watcher with `dirty_words == 0`.
    let changes = frame(&mut store, |tx| {
        tx.edit::<Modality>(dialog).modal(false);
        tx.edit::<Modality>(plain).modal(false);
    });
    renderer.commit(&store, &changes).unwrap();
    assert_eq!(WATCHED.load(Ordering::Relaxed), base + 1);

    // Same-frame retag + watched change: the watcher re-commit counts as committed, so the
    // retag pass does not sync it a second time.
    let changes = frame(&mut store, |tx| {
        tx.edit::<Modality>(dialog).modal(true);
    });
    let mut live: HashMap<NodeId, MockEl> = HashMap::default();
    live.insert(dialog, MockEl::default());
    let mut synced: Vec<(TableSlot, NodeId)> = Vec::new();
    renderer.commit_plugins_active_with(
        &store,
        &changes,
        #[cfg(target_arch = "wasm32")]
        None,
        Some(&live),
        &[dialog],
        |sys, nid, _| synced.push((sys.slot(), nid)),
    );
    assert_eq!(WATCHED.load(Ordering::Relaxed), base + 2);
    assert!(
        synced.is_empty(),
        "watcher re-commit must suppress the retag sync: {synced:?}"
    );
}
