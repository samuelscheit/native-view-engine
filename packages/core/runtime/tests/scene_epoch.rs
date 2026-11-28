//! `NveSceneView::scene_epoch`: hosts detect every scene reset (keyframe, detach) even when the new scene reuses the same node ids.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use nve_tables::tables::TextContent;
use nve_abi::{FrameId, ValidatedEvents};
use nve_runtime::{AppFrame, RuntimeApp, RuntimeBuilder, SurfaceRuntime};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, NodeId, NodeStore};

/// A fresh one-text keyframe (frame 1 of a new session: same node ids every time).
fn keyframe(text: &str) -> Vec<u8> {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let n = tx.create();
    tx.edit::<TextContent>(n).text(text);
    tx.set_children(NodeId::ROOT, &[n]);
    tx.commit_into(&mut changes);
    let mut out = Vec::new();
    WireEncoder::default().encode(&store, &changes, &mut out);
    out
}

/// Renders one keyframe, then stays clean.
struct OneFrameApp(Option<Vec<u8>>, Vec<u8>);

impl RuntimeApp for OneFrameApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        match self.0.take() {
            Some(frame) => {
                self.1 = frame;
                AppFrame::Wire(&self.1)
            }
            None => AppFrame::Clean,
        }
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        false
    }
}

fn epoch(rt: &SurfaceRuntime) -> (u32, u64) {
    let guard = rt.acquire_scene().expect("acquire");
    (guard.view().scene_epoch, guard.view().frame_id)
}

#[test]
fn every_scene_reset_bumps_the_epoch_and_plain_frames_do_not() {
    let mut rt = RuntimeBuilder::new().build();
    let (e0, _) = epoch(&rt);
    rt.submit_nve2(&keyframe("a")).expect("first keyframe");
    let (e1, f1) = epoch(&rt);
    assert_eq!(e1, e0, "the first session starts in the current epoch");

    rt.submit_nve2(&keyframe("b")).expect("second keyframe resets the replica");
    let (e2, f2) = epoch(&rt);
    assert_eq!(e2, e1.wrapping_add(1), "a new keyframe discards the old scene");
    assert!(f2 > f1, "frame ids stay monotonic across epochs");

    assert!(rt.submit_nve2(&[1, 2, 3, 4]).is_err(), "corrupt header");
    let (e3, _) = epoch(&rt);
    assert_eq!(e3, e2, "a frame rejected before applying keeps the scene and its epoch");

    rt.detach_app().expect("detach without an app");
    assert_eq!(epoch(&rt).0, e3, "detaching nothing keeps the scene");

    rt.attach_app(Box::new(OneFrameApp(Some(keyframe("app")), Vec::new()))).expect("attach");
    let _ = rt.tick(0.0);
    let (e4, _) = epoch(&rt);
    assert_eq!(e4, e3.wrapping_add(1), "attaching an app discards the submitted scene");
    let _ = rt.tick(0.0);
    assert_eq!(epoch(&rt).0, e4, "ticking an unchanged surface keeps the epoch");
    rt.detach_app().expect("detach");
    assert_eq!(epoch(&rt).0, e4.wrapping_add(1), "detaching the app discards its scene");
}

/// Valid `NVE2` header and first op, then an unknown op: the body fails mid-transaction.
fn mid_frame_corrupt(frame_num: u32) -> Vec<u8> {
    let words: [u32; 12] = [
        nve_store::wire::MAGIC,
        u32::from(nve_store::wire::VERSION),
        frame_num,
        0,
        2,
        12, // header: "NVE2", current version, frame, flags, 2 ops, 12 words
        3 | (1 << 16),
        1,
        99, // CreateNodes(count 1) -> node 99
        0xFE | (1 << 16),
        1,
        0xDEAD_BEEF, // unknown op
    ];
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn a_frame_failing_mid_body_resets_the_scene_into_a_new_epoch() {
    let mut rt = RuntimeBuilder::new().build();
    rt.submit_nve2(&keyframe("a")).expect("keyframe");
    let (e0, _) = epoch(&rt);
    assert!(rt.submit_nve2(&mid_frame_corrupt(2)).is_err(), "corrupt body");
    let (e1, _) = epoch(&rt);
    assert_eq!(e1, e0.wrapping_add(1), "partially applied ops are discarded with a new epoch");
    rt.submit_nve2(&keyframe("b")).expect("recovery keyframe");
    assert_eq!(epoch(&rt).0, e1, "the first keyframe after the reset stays in the reset epoch");
}

#[test]
fn swapping_one_app_for_another_starts_a_new_epoch() {
    let mut rt = RuntimeBuilder::new().build();
    rt.attach_app(Box::new(OneFrameApp(Some(keyframe("one")), Vec::new()))).expect("attach one");
    let _ = rt.tick(0.0);
    let (e1, _) = epoch(&rt);
    rt.attach_app(Box::new(OneFrameApp(Some(keyframe("two")), Vec::new()))).expect("attach two");
    let _ = rt.tick(0.0);
    let (e2, _) = epoch(&rt);
    assert_eq!(e2, e1.wrapping_add(1), "same node ids, different app: exactly one new epoch");
}

/// Direct-store app that switches to a different boxed `NodeStore` (a different resource
/// namespace) without any runtime reset. `command`: 1 re-renders, 2 switches stores (async work).
struct StoreSwitchingApp {
    stores: [Box<NodeStore>; 2],
    nodes: [NodeId; 2],
    active: usize,
    changes: ChangeSet,
    /// Creation changes of each store, rendered the first time it is active.
    created: [Option<ChangeSet>; 2],
    command: Arc<AtomicU8>,
    switch: bool,
}

impl StoreSwitchingApp {
    fn new(command: Arc<AtomicU8>) -> Self {
        let build = |label: &str| {
            let mut store = Box::new(NodeStore::new());
            let mut tx = store.tx();
            let n = tx.create();
            tx.edit::<TextContent>(n).text(label);
            tx.set_children(NodeId::ROOT, &[n]);
            let mut created = ChangeSet::default();
            tx.commit_into(&mut created);
            (store, n, created)
        };
        let (left, l, lc) = build("left");
        let (right, r, rc) = build("right");
        Self { stores: [left, right], nodes: [l, r], active: 0, changes: ChangeSet::default(), created: [Some(lc), Some(rc)], command, switch: false }
    }
}

impl RuntimeApp for StoreSwitchingApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        if std::mem::take(&mut self.switch) {
            self.active ^= 1;
        }
        if let Some(created) = self.created[self.active].take() {
            self.changes = created;
            return AppFrame::Direct { changes: &self.changes };
        }
        let (store, node) = (&mut self.stores[self.active], self.nodes[self.active]);
        let mut tx = store.tx();
        tx.edit::<TextContent>(node).text("rendered");
        tx.commit_into(&mut self.changes);
        AppFrame::Direct { changes: &self.changes }
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        false
    }

    fn poll_async_work(&mut self) -> bool {
        match self.command.swap(0, Ordering::AcqRel) {
            0 => false,
            c => {
                self.switch = c == 2;
                true
            }
        }
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.stores[self.active])
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.stores[self.active])
    }
}

#[test]
fn a_direct_app_switching_resource_stores_starts_a_new_epoch() {
    let command = Arc::new(AtomicU8::new(0));
    let mut rt = RuntimeBuilder::new().app(Box::new(StoreSwitchingApp::new(Arc::clone(&command)))).build();
    let _ = rt.tick(0.0).expect("mount");
    let (e0, f0) = epoch(&rt);
    command.store(1, Ordering::Release);
    assert!(rt.tick(0.0).expect("same store"));
    let (e1, f1) = epoch(&rt);
    assert!(f1 > f0 && e1 == e0, "re-rendering the same store keeps the epoch");
    command.store(2, Ordering::Release);
    assert!(rt.tick(0.0).expect("switched store"));
    assert_eq!(epoch(&rt).0, e0.wrapping_add(1), "another store's resource ids are another namespace");
    command.store(1, Ordering::Release);
    assert!(rt.tick(0.0).expect("stay"));
    assert_eq!(epoch(&rt).0, e0.wrapping_add(1), "staying on the new store keeps its epoch");
}

/// Direct-store app that replaces its store in place (`*store = NodeStore::new()`: the same heap
/// address, a new namespace) when `command` is 2; 1 re-renders.
struct InlineResetApp {
    store: Box<NodeStore>,
    node: NodeId,
    changes: ChangeSet,
    command: Arc<AtomicU8>,
    reset: bool,
}

/// Builds `labels` as root children of `store`, returning the first node and the changes.
fn populate(store: &mut NodeStore, labels: &[&str]) -> (NodeId, ChangeSet) {
    let mut tx = store.tx();
    let nodes: Vec<NodeId> = labels
        .iter()
        .map(|label| {
            let n = tx.create();
            tx.edit::<TextContent>(n).text(label);
            n
        })
        .collect();
    tx.set_children(NodeId::ROOT, &nodes);
    let mut changes = ChangeSet::default();
    tx.commit_into(&mut changes);
    (nodes[0], changes)
}

impl RuntimeApp for InlineResetApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        if std::mem::take(&mut self.reset) {
            *self.store = NodeStore::new();
            (self.node, self.changes) = populate(&mut self.store, &["a much longer replacement label"]);
            return AppFrame::Direct { changes: &self.changes };
        }
        let mut tx = self.store.tx();
        tx.edit::<TextContent>(self.node).text("rendered");
        tx.commit_into(&mut self.changes);
        AppFrame::Direct { changes: &self.changes }
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        false
    }

    fn poll_async_work(&mut self) -> bool {
        match self.command.swap(0, Ordering::AcqRel) {
            0 => false,
            c => {
                self.reset = c == 2;
                true
            }
        }
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

fn published_rect(rt: &SurfaceRuntime, id: NodeId) -> (f32, f32, f32, f32) {
    let guard = rt.acquire_scene().expect("acquire");
    let v = guard.view();
    assert!(id.index() < v.node_capacity as usize);
    // SAFETY: The guard keeps the published rects alive; index checked above.
    let r = unsafe { *v.rects.add(id.index()) };
    (r.x, r.y, r.width, r.height)
}

#[test]
fn a_direct_store_replaced_in_place_starts_a_new_epoch_with_fresh_layout() {
    let command = Arc::new(AtomicU8::new(0));
    let mut store = Box::new(NodeStore::new());
    let (node, changes) = populate(&mut store, &["one", "two", "three"]);
    let app = InlineResetApp { store, node, changes, command: Arc::clone(&command), reset: false };
    let mut rt = RuntimeBuilder::new().app(Box::new(app)).build();
    let _ = rt.tick(0.0).expect("mount");
    let (e0, _) = epoch(&rt);
    command.store(1, Ordering::Release);
    assert!(rt.tick(0.0).expect("re-render"));
    assert_eq!(epoch(&rt).0, e0, "the same store keeps its epoch");

    command.store(2, Ordering::Release);
    assert!(rt.tick(0.0).expect("in-place reset"));
    assert_eq!(epoch(&rt).0, e0.wrapping_add(1), "same address, new store: exactly one new epoch");

    // Layout and published rects equal a runtime that only ever saw the replacement store.
    let mut fresh_store = Box::new(NodeStore::new());
    let (fresh_node, fresh_changes) = populate(&mut fresh_store, &["a much longer replacement label"]);
    let fresh_app = InlineResetApp {
        store: fresh_store,
        node: fresh_node,
        changes: fresh_changes,
        command: Arc::new(AtomicU8::new(0)),
        reset: false,
    };
    let mut fresh = RuntimeBuilder::new().app(Box::new(fresh_app)).build();
    let _ = fresh.tick(0.0).expect("fresh mount");
    assert_eq!(published_rect(&rt, fresh_node), published_rect(&fresh, fresh_node));
    command.store(1, Ordering::Release);
    assert!(rt.tick(0.0).expect("stay"));
    assert_eq!(epoch(&rt).0, e0.wrapping_add(1), "the replacement keeps its epoch");
}
