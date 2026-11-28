//! Generic plugin-lifecycle guards for imperatively owned attributes (`forget_attr`) across a
//! retag, and for cross-table watchers (`WebTableSystem::watches_slot`).

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

use nve_abi::hash::{HashMap, HashSet};
use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, TableSlot};

use crate::table_system::{needs_connected_sync_on_place, RetagElementSink};
use crate::{WebStoreRenderer, WebSystemCommitCtx, WebTableSystem};

#[derive(Default)]
struct MockEl {
    attrs: RefCell<Vec<(String, String)>>,
    inner_html: RefCell<Vec<String>>,
}
impl RetagElementSink for MockEl {
    fn set_attr(&self, name: &str, value: &str) {
        self.attrs
            .borrow_mut()
            .push((name.to_owned(), value.to_owned()));
    }
    fn set_inner_html(&self, html: &str) {
        self.inner_html.borrow_mut().push(html.to_owned());
    }
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

/// Detaching a watched table (e.g. `Accessibility` from an open modal `<dialog>`) re-commits the
/// watcher on live nodes still carrying its table, so modality-derived state is re-applied;
/// nodes without the watcher's table and removed nodes are skipped.
#[test]
fn detaching_a_watched_table_recommits_watchers_on_live_nodes_only() {
    static WATCHED: AtomicU32 = AtomicU32::new(0);
    let mut store = NodeStore::new();
    let mut renderer = WebStoreRenderer::new().with_table_system(DisclosureSys {
        watched_commits: &WATCHED,
    });
    let (mut dialog, mut plain, mut doomed) = (NodeId::ROOT, NodeId::ROOT, NodeId::ROOT);
    let changes = frame(&mut store, |tx| {
        dialog = tx.create();
        plain = tx.create();
        doomed = tx.create();
        tx.set_children(NodeId::ROOT, &[dialog, plain, doomed]);
        for n in [dialog, doomed] {
            tx.edit::<Disclosure>(n).imperative(true).open(true);
        }
        for n in [dialog, plain, doomed] {
            tx.edit::<Modality>(n).modal(true);
        }
    });
    renderer.commit(&store, &changes).unwrap();
    let base = WATCHED.load(Ordering::Relaxed);
    assert_eq!(base, 2, "dialog + doomed carry the watcher's table");

    let changes = frame(&mut store, |tx| {
        tx.detach::<Modality>(dialog);
        tx.detach::<Modality>(plain);
    });
    renderer.commit(&store, &changes).unwrap();
    assert_eq!(
        WATCHED.load(Ordering::Relaxed),
        base + 1,
        "only `dialog` re-commits on detach"
    );

    let changes = frame(&mut store, |tx| {
        tx.detach::<Modality>(doomed);
        tx.remove(doomed);
    });
    renderer.commit(&store, &changes).unwrap();
    assert_eq!(
        WATCHED.load(Ordering::Relaxed),
        base + 1,
        "a removed node is never re-committed"
    );
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "web.test.open_hint", storage = "sparse")]
struct OpenHint {
    #[effect(paint)]
    value: u32,
}

/// A second plugin that mirrors `open="hint"`, plus optional `inline_css` / `inner_html` when
/// `value >= 10` (so tests can cover all three mirror channels on retag).
struct OpenHintSys;
impl WebTableSystem for OpenHintSys {
    fn slot(&self) -> TableSlot {
        OpenHint::slot()
    }
    fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, store: &NodeStore, node: NodeId, _: u32) {
        ctx.set_attr(node, "open", "hint");
        if let Some(row) = store.try_row::<OpenHint>(node)
            && row.value() >= 10
        {
            ctx.set_inline_css(node, format!("--v:{};", row.value()));
            ctx.set_inner_html(node, format!("<span>{}</span>", row.value()));
        }
    }
}

/// Multi-slot ownership: forgetting the NEWEST write falls the mirror back to the older slot's
/// value instead of leaving the forgotten value for retag replay.
#[test]
fn forget_attr_falls_back_to_an_older_slot_write_in_the_mirror() {
    static WATCHED: AtomicU32 = AtomicU32::new(0);
    let mut store = NodeStore::new();
    let mut renderer = WebStoreRenderer::new()
        .with_table_system(DisclosureSys {
            watched_commits: &WATCHED,
        })
        .with_table_system(OpenHintSys);
    let mut n = NodeId::ROOT;
    let changes = frame(&mut store, |tx| {
        n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<OpenHint>(n).value(1);
    });
    renderer.commit(&store, &changes).unwrap();
    assert_eq!(
        renderer.plugin_attrs(n),
        &[("open".to_owned(), "hint".to_owned())]
    );

    // Newer `<details open>` write wins the mirror.
    let changes = frame(&mut store, |tx| {
        tx.edit::<Disclosure>(n).open(true);
    });
    renderer.commit(&store, &changes).unwrap();
    assert_eq!(
        renderer.plugin_attrs(n),
        &[("open".to_owned(), String::new())]
    );

    // Imperative: the disclosure plugin forgets `open`; the older hint resurfaces.
    let changes = frame(&mut store, |tx| {
        tx.edit::<Disclosure>(n).imperative(true);
    });
    renderer.commit(&store, &changes).unwrap();
    assert_eq!(
        renderer.plugin_attrs(n),
        &[("open".to_owned(), "hint".to_owned())]
    );
}

/// Retag replay writes only effective values NOT already written by this commit's Steps 1-2
/// (those landed on the fresh live element directly); older mirrored values are replayed.
#[test]
fn retag_replay_skips_values_written_by_this_commit() {
    static WATCHED: AtomicU32 = AtomicU32::new(0);
    let mut store = NodeStore::new();
    let mut renderer = WebStoreRenderer::new()
        .with_table_system(DisclosureSys {
            watched_commits: &WATCHED,
        })
        .with_table_system(OpenHintSys);
    let (mut a, mut b) = (NodeId::ROOT, NodeId::ROOT);
    let changes = frame(&mut store, |tx| {
        a = tx.create();
        b = tx.create();
        tx.set_children(NodeId::ROOT, &[a, b]);
        for n in [a, b] {
            tx.edit::<OpenHint>(n).value(1);
            tx.edit::<Disclosure>(n).open(false);
        }
    });
    renderer.commit(&store, &changes).unwrap();

    // Both retag to `<dialog>`; only `b`'s hint is re-committed (re-written) in this commit.
    let changes = frame(&mut store, |tx| {
        tx.edit::<Disclosure>(a).imperative(true);
        tx.edit::<Disclosure>(b).imperative(true);
        tx.edit::<OpenHint>(b).value(2);
    });
    let mut live: HashMap<NodeId, MockEl> = HashMap::default();
    live.insert(a, MockEl::default());
    live.insert(b, MockEl::default());
    renderer.commit_plugins_active_with(
        &store,
        &changes,
        #[cfg(target_arch = "wasm32")]
        None,
        Some(&live),
        &[a, b],
        |_, _, _| {},
    );
    assert_eq!(
        *live[&a].attrs.borrow(),
        [("open".to_owned(), "hint".to_owned())],
        "an older mirrored value is replayed onto the fresh element"
    );
    assert!(
        live[&b].attrs.borrow().is_empty(),
        "a value written by this commit is not replayed twice"
    );
}

/// Retag replay gates `inline_css` and `inner_html` by `write_seq` alongside `attrs`: older
/// mirrors replay once onto the fresh element; writes made during the same commit are skipped.
#[test]
fn retag_replay_gates_inline_css_and_inner_html_by_write_seq() {
    static WATCHED: AtomicU32 = AtomicU32::new(0);
    let mut store = NodeStore::new();
    let mut renderer = WebStoreRenderer::new()
        .with_table_system(DisclosureSys {
            watched_commits: &WATCHED,
        })
        .with_table_system(OpenHintSys);
    let (mut a, mut b) = (NodeId::ROOT, NodeId::ROOT);
    let changes = frame(&mut store, |tx| {
        a = tx.create();
        b = tx.create();
        tx.set_children(NodeId::ROOT, &[a, b]);
        for n in [a, b] {
            tx.edit::<OpenHint>(n).value(10);
            tx.edit::<Disclosure>(n).open(false);
        }
    });
    renderer.commit(&store, &changes).unwrap();

    // Retag both; only `b` re-writes `OpenHint` (`attr` + `inline_css` + `inner_html`) in this commit.
    let changes = frame(&mut store, |tx| {
        tx.edit::<Disclosure>(a).imperative(true);
        tx.edit::<Disclosure>(b).imperative(true);
        tx.edit::<OpenHint>(b).value(20);
    });
    let mut live: HashMap<NodeId, MockEl> = HashMap::default();
    live.insert(a, MockEl::default());
    live.insert(b, MockEl::default());
    renderer.commit_plugins_active_with(
        &store,
        &changes,
        #[cfg(target_arch = "wasm32")]
        None,
        Some(&live),
        &[a, b],
        |_, _, _| {},
    );
    assert_eq!(
        *live[&a].attrs.borrow(),
        [
            ("open".to_owned(), "hint".to_owned()),
            ("data-nve-mod-style".to_owned(), "--v:10;".to_owned()),
        ],
        "older attr and inline_css replay once on retag"
    );
    assert_eq!(
        *live[&a].inner_html.borrow(),
        ["<span>10</span>".to_owned()],
        "older inner_html replays once on retag"
    );
    assert!(
        live[&b].attrs.borrow().is_empty(),
        "same-commit attr and inline_css must not replay twice"
    );
    assert!(
        live[&b].inner_html.borrow().is_empty(),
        "same-commit inner_html must not replay twice: {:?}",
        live[&b].inner_html.borrow()
    );
}

/// Top-down `NVE2` mounts call `queue_connected_sync_subtree` at every parent->child level along a
/// deep chain: the per-commit `connected_visited` set must cap total visits at $O(N)$ (each node
/// visited once, never $O(N^2)$), childless newly created leaves are skipped at placement, and
/// previously detached subtrees (and subsequent commits after drain) still sync every plugin node.
#[test]
fn connected_sync_subtree_top_down_chain_is_linear_and_resyncs_detached_attach() {
    static WATCHED: AtomicU32 = AtomicU32::new(0);
    const DEPTH: usize = 200;
    let mut store = NodeStore::new();
    let mut renderer = WebStoreRenderer::new().with_table_system(DisclosureSys {
        watched_commits: &WATCHED,
    });
    let mut chain = Vec::with_capacity(DEPTH);
    let changes = frame(&mut store, |tx| {
        let mut parent = NodeId::ROOT;
        for i in 0..DEPTH {
            let n = tx.create();
            tx.set_children(parent, &[n]);
            if i == 0 || i == DEPTH / 2 || i == DEPTH - 1 {
                tx.edit::<Disclosure>(n).imperative(true).open(true);
            }
            chain.push(n);
            parent = n;
        }
    });
    renderer.commit(&store, &changes).unwrap();

    let created: HashSet<NodeId> = changes.created.iter().copied().collect();
    let mut live: HashMap<NodeId, MockEl> = HashMap::default();
    for &n in &chain {
        live.insert(n, MockEl::default());
    }
    // Newly created childless leaf is skipped at placement (Step 4 already committed it), while
    // newly created internal nodes are still walked in case they adopt pre-existing descendants.
    assert!(!needs_connected_sync_on_place(&created, &store, chain[DEPTH - 1]));
    assert!(needs_connected_sync_on_place(&created, &store, chain[0]));

    // Simulate top-down `changes.children` order: ROOT -> chain[0] -> chain[1] -> ... -> chain[N-1].
    let mut total_visits = 0usize;
    for &cid in &chain {
        if needs_connected_sync_on_place(&created, &store, cid) {
            total_visits += renderer.queue_connected_sync_subtree(&store, cid);
        }
    }
    assert_eq!(
        total_visits, DEPTH,
        "top-down chain of {DEPTH} nodes must visit each node at most once (not O(N^2))"
    );

    let mut synced = Vec::new();
    renderer.drain_connected_sync(&store, &live, |sys, nid, _| synced.push((sys.slot(), nid)));
    assert_eq!(
        synced,
        vec![
            (Disclosure::slot(), chain[0]),
            (Disclosure::slot(), chain[DEPTH / 2]),
            (Disclosure::slot(), chain[DEPTH - 1]),
        ],
        "every plugin node along the chain syncs once in top-down order"
    );

    // Commit A: build an unparented wrapper subtree AND a standalone detached leaf.
    let (mut wrapper, mut detached_dlg, mut solo_dlg) = (NodeId::ROOT, NodeId::ROOT, NodeId::ROOT);
    let changes = frame(&mut store, |tx| {
        wrapper = tx.create();
        detached_dlg = tx.create();
        solo_dlg = tx.create();
        tx.edit::<Disclosure>(detached_dlg).imperative(true).open(true);
        tx.edit::<Disclosure>(solo_dlg).imperative(true).open(true);
        tx.set_children(wrapper, &[detached_dlg]);
    });
    renderer.commit(&store, &changes).unwrap();
    for n in [wrapper, detached_dlg, solo_dlg] {
        live.insert(n, MockEl::default());
    }
    renderer.drain_connected_sync(&store, &live, |_, _, _| {});

    // Commit B: attach both previously detached subtrees with ZERO edits to them (`created` empty).
    let changes = frame(&mut store, |tx| {
        tx.set_children(NodeId::ROOT, &[chain[0], wrapper, solo_dlg]);
    });
    renderer.commit(&store, &changes).unwrap();
    let empty_created: HashSet<NodeId> = HashSet::default();
    assert!(needs_connected_sync_on_place(&empty_created, &store, wrapper));
    assert!(needs_connected_sync_on_place(&empty_created, &store, solo_dlg));
    assert_eq!(renderer.queue_connected_sync_subtree(&store, wrapper), 2);
    assert_eq!(renderer.queue_connected_sync_subtree(&store, solo_dlg), 1);
    synced.clear();
    renderer.drain_connected_sync(&store, &live, |sys, nid, _| synced.push((sys.slot(), nid)));
    assert_eq!(
        synced,
        vec![
            (Disclosure::slot(), detached_dlg),
            (Disclosure::slot(), solo_dlg),
        ],
        "attaching previously detached subtrees (and leaves) syncs every plugin node"
    );

    // Commit C: `connected_visited` was cleared by `drain_connected_sync`, so a subsequent fallback
    // move of `wrapper` re-walks and re-syncs `detached_dlg` again.
    assert_eq!(renderer.queue_connected_sync_subtree(&store, wrapper), 2);
    synced.clear();
    renderer.drain_connected_sync(&store, &live, |sys, nid, _| synced.push((sys.slot(), nid)));
    assert_eq!(synced, vec![(Disclosure::slot(), detached_dlg)]);
}

struct DataAttrSys {
    slot: TableSlot,
    write_open: bool,
}
impl WebTableSystem for DataAttrSys {
    fn slot(&self) -> TableSlot {
        self.slot
    }
    fn commit_node(&self, ctx: &mut WebSystemCommitCtx<'_>, _s: &NodeStore, node: NodeId, _: u32) {
        ctx.set_attr(node, "data-shared", "same");
        if self.write_open {
            ctx.set_attr(node, "open", "");
        }
    }
}

/// `set_attr` short-circuits unchanged `data-*` on non-retagged elements when this slot owns the
/// newest `seq`, writes on retag, NEVER skips `open`, and preserves multi-slot `seq` ordering.
#[test]
fn set_attr_short_circuits_only_newest_data_attrs_and_never_skips_open_or_retag() {
    let mut store = NodeStore::new();
    let mut renderer = WebStoreRenderer::new()
        .with_table_system(DataAttrSys {
            slot: Disclosure::slot(),
            write_open: true,
        })
        .with_table_system(DataAttrSys {
            slot: OpenHint::slot(),
            write_open: false,
        });

    let mut n = NodeId::ROOT;
    let c1 = frame(&mut store, |tx| {
        n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Disclosure>(n).open(true);
    });
    renderer.commit(&store, &c1).expect("commit 1");
    let seq_data_1 = renderer.latest_attr_seq_for_test(n, "data-shared");
    let seq_open_1 = renderer.latest_attr_seq_for_test(n, "open");
    assert!(seq_data_1 > 0 && seq_open_1 > 0);

    // 1. Steady-state unchanged: `data-shared` short-circuits, `open` MUST NOT short-circuit.
    let c2 = frame(&mut store, |tx| {
        tx.edit::<Disclosure>(n).imperative(true);
    });
    renderer.commit(&store, &c2).expect("commit 2");
    assert_eq!(renderer.latest_attr_seq_for_test(n, "data-shared"), seq_data_1);
    let seq_open_2 = renderer.latest_attr_seq_for_test(n, "open");
    assert!(seq_open_2 > seq_open_1, "`open` must never short-circuit");

    // 2. Retag with unchanged `data-shared`: MUST write directly and not double-replay.
    let c3 = frame(&mut store, |tx| {
        tx.edit::<Disclosure>(n).imperative(false);
    });
    let mut live: HashMap<NodeId, MockEl> = HashMap::default();
    live.insert(n, MockEl::default());
    renderer.commit_plugins_active_with(
        &store,
        &c3,
        #[cfg(target_arch = "wasm32")]
        None,
        Some(&live),
        &[n],
        |_, _, _| {},
    );
    let seq_retag = renderer.latest_attr_seq_for_test(n, "data-shared");
    assert!(seq_retag > seq_data_1, "retag must write unchanged data-*");
    assert!(live[&n].attrs.borrow().is_empty(), "no duplicate Step-2a replay");

    // 3. Multi-slot `seq`: Slot B writes newer `data-shared`, then Slot A rewrites `"same"` ->
    //    Slot A MUST advance `seq` above Slot B!
    let c4 = frame(&mut store, |tx| {
        tx.edit::<OpenHint>(n).value(1);
    });
    renderer.commit(&store, &c4).expect("slot B commit");
    let seq_b = renderer.latest_attr_seq_for_test(n, "data-shared");
    assert!(seq_b > seq_retag);
    let c5 = frame(&mut store, |tx| {
        tx.edit::<Disclosure>(n).open(false);
    });
    renderer.commit(&store, &c5).expect("slot A over slot B");
    assert!(
        renderer.latest_attr_seq_for_test(n, "data-shared") > seq_b,
        "slot A must advance OwnedAttr::seq above slot B's newer write"
    );
}
