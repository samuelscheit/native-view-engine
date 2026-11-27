//! `NveRuntimeSceneView` change-set contract:
//!
//! - `dirty_indices`: every live node created this frame or whose row, table rows, or child list
//!   changed — exactly once;
//! - `moved_indices`: every other live node whose absolute rect or origin relative to its parent's
//!   rect changed — exactly once;
//! - the two are disjoint, hold only live slots, and a node untouched by edits and relayout is in
//!   neither.
//!
//! Every frame is checked against an independent oracle (the previous frame's rects and parents);
//! the scenarios pin the exact sets.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use nve_abi::{NveLayoutRect, NveNodeRow, NVE_NODE_ROW_FLAG_LIVE};
use nve_components::tables::{Layout, Visual};
use nve_engine::Viewport;
use nve_runtime::{AppFrame, FrameTickHook, RuntimeApp, RuntimeBuilder, SurfaceRuntime};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, Transaction};

struct StoreApp {
    store: NodeStore,
    changes: ChangeSet,
    dirty: bool,
}

impl RuntimeApp for StoreApp {
    fn render_frame(&mut self, _frame_id: u64) -> AppFrame<'_> {
        if !std::mem::take(&mut self.dirty) {
            return AppFrame::Clean;
        }
        AppFrame::Direct { changes: &self.changes }
    }

    fn dispatch_events(&mut self, _events: nve_abi::NveEventBatchRef<'_>) -> bool {
        false
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(&self.store)
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(&mut self.store)
    }
}

type Step = Box<dyn FnMut(&mut Transaction<'_>) + Send>;

/// Applies one queued edit per tick (none on the mount tick).
struct Script(Arc<Mutex<VecDeque<Step>>>);

impl FrameTickHook for Script {
    fn tick(&mut self, tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        match self.0.lock().expect("hook steps poisoned").pop_front() {
            Some(mut step) => {
                step(tx);
                true
            }
            None => false,
        }
    }
}

/// One published frame, copied out of the scene view.
#[derive(Clone, Default)]
struct Frame {
    dirty: Vec<u32>,
    moved: Vec<u32>,
    removed: Vec<u32>,
    rects: Vec<NveLayoutRect>,
    rows: Vec<NveNodeRow>,
}

impl Frame {
    fn live(&self, i: usize) -> bool {
        self.rows.get(i).is_some_and(|r| r.flags & NVE_NODE_ROW_FLAG_LIVE != 0)
    }

    /// Origin relative to the parent's rect (own origin for parentless nodes).
    /// Box of slot `i` relative to its parent's rect (absolute for roots).
    fn local(&self, i: usize) -> NveLayoutRect {
        let r = self.rects[i];
        match NodeId::from_raw(self.rows[i].parent).index() {
            0 => r,
            p => NveLayoutRect { x: r.x - self.rects[p].x, y: r.y - self.rects[p].y, ..r },
        }
    }

    fn sorted(v: &[u32]) -> Vec<u32> {
        let mut v = v.to_vec();
        v.sort_unstable();
        v
    }
}

struct Harness {
    rt: SurfaceRuntime,
    steps: Arc<Mutex<VecDeque<Step>>>,
    last: Frame,
}

impl Harness {
    /// Mounts `build`'s tree (its returned ids are handed back) and checks the first frame.
    fn mount(build: impl FnOnce(&mut Transaction<'_>) -> Vec<NodeId>) -> (Self, Vec<NodeId>) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let ids = {
            let mut tx = store.tx();
            let ids = build(&mut tx);
            tx.commit_into(&mut changes);
            ids
        };
        let steps = Arc::new(Mutex::new(VecDeque::new()));
        let mut rt = RuntimeBuilder::new()
            .viewport(Viewport { width: 400.0, height: 300.0, scale_factor: 1.0 })
            .app(Box::new(StoreApp { store, changes, dirty: true }))
            .tick_hook(Box::new(Script(steps.clone())))
            .build();
        assert!(rt.tick(0.016).expect("mount"));
        let mut h = Self { rt, steps, last: Frame::default() };
        let first = h.read();
        assert!(first.moved.is_empty(), "a first frame has no previous geometry to move from");
        h.last = first;
        (h, ids)
    }

    /// Applies `step` in one tick and returns the checked frame.
    fn step(&mut self, step: impl FnMut(&mut Transaction<'_>) + Send + 'static) -> Frame {
        self.steps.lock().expect("hook steps poisoned").push_back(Box::new(step));
        assert!(self.rt.tick(0.016).expect("tick"));
        self.next_frame()
    }

    fn next_frame(&mut self) -> Frame {
        let f = self.read();
        self.check(&f);
        self.last = f.clone();
        f
    }

    fn read(&self) -> Frame {
        let guard = self.rt.acquire_scene().expect("acquire");
        let v = guard.view();
        let slice = |p: *const u32, n: u32| {
            if n == 0 {
                Vec::new()
            } else {
                // SAFETY: `guard` holds the reader lock for every published slab.
                unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec()
            }
        };
        let cap = v.node_capacity as usize;
        Frame {
            dirty: slice(v.dirty_indices, v.dirty_count),
            moved: slice(v.moved_indices, v.moved_count),
            removed: slice(v.removed_ids, v.removed_count),
            // SAFETY: `rows` and `rects` span `node_capacity` entries under the reader lock.
            rects: unsafe { std::slice::from_raw_parts(v.rects, cap) }.to_vec(),
            // SAFETY: as above.
            rows: unsafe { std::slice::from_raw_parts(v.rows, cap) }.to_vec(),
        }
    }

    /// Oracle: `moved` is exactly the live, non-dirty slots whose parent-relative box differs from
    /// the previous frame; every slot that (re)appeared is dirty; no duplicates, no overlap, no
    /// dead slots.
    fn check(&self, f: &Frame) {
        let (old, mut seen) = (&self.last, vec![0u8; f.rows.len()]);
        for &i in &f.dirty {
            assert!(f.live(i as usize), "dead slot {i} in dirty_indices");
            seen[i as usize] += 1;
        }
        for &i in &f.moved {
            assert!(f.live(i as usize), "dead slot {i} in moved_indices");
            seen[i as usize] += 1;
        }
        assert!(seen.iter().all(|&n| n <= 1), "duplicate or overlapping index: {:?} / {:?}", f.dirty, f.moved);
        let mut expected_moved = Vec::new();
        for i in 1..f.rows.len() {
            if !f.live(i) || f.dirty.contains(&(i as u32)) {
                continue;
            }
            let reappeared = !old.live(i) || old.rows[i].generation != f.rows[i].generation;
            assert!(!reappeared, "slot {i} became live without being dirty");
            if old.local(i) != f.local(i) {
                expected_moved.push(i as u32);
            }
        }
        assert_eq!(Frame::sorted(&f.moved), expected_moved, "moved_indices are not exact");
    }
}

/// ROOT -> column[a (h=30), b (h=40), c (h=50)] (slots 1, 2, 3, 4, 5).
fn column(tx: &mut Transaction<'_>) -> Vec<NodeId> {
    let column = tx.create();
    tx.edit::<Layout>(column).width(Length::points(100.0));
    let kids: Vec<NodeId> = [30.0, 40.0, 50.0]
        .into_iter()
        .map(|h| {
            let n = tx.create();
            tx.edit::<Layout>(n).height(Length::points(h));
            n
        })
        .collect();
    tx.set_children(column, &kids);
    tx.set_children(NodeId::ROOT, &[column]);
    let mut ids = vec![column];
    ids.extend(kids);
    ids
}

#[test]
fn relayout_splits_content_changes_from_moves() {
    let (mut h, ids) = Harness::mount(column);
    let a = ids[1];
    // Growing `a` (30 -> 60): `a` is dirty; `b` and `c` shift and the auto-height column grows,
    // so those three only moved. ROOT keeps its viewport rect.
    let f = h.step(move |tx| {
        tx.edit::<Layout>(a).height(Length::points(60.0));
    });
    assert_eq!(Frame::sorted(&f.dirty), [3]);
    assert_eq!(Frame::sorted(&f.moved), [2, 4, 5]);
    assert_eq!((f.rects[4].y, f.rects[5].y), (60.0, 100.0));
}

#[test]
fn paint_only_edit_marks_only_the_touched_node() {
    let (mut h, ids) = Harness::mount(column);
    let a = ids[1];
    let f = h.step(move |tx| {
        tx.edit::<Visual>(a).background(Rgba::rgb(9, 0, 0));
    });
    assert_eq!(f.dirty, [3]);
    assert!(f.moved.is_empty());
}

#[test]
fn inserting_at_the_top_of_a_long_list_moves_without_dirtying() {
    const N: usize = 1_000;
    let (mut h, ids) = Harness::mount(|tx| {
        let list = tx.create();
        tx.edit::<Layout>(list).width(Length::points(100.0));
        let kids: Vec<NodeId> = (0..N)
            .map(|_| {
                let n = tx.create();
                tx.edit::<Layout>(n).height(Length::points(10.0));
                n
            })
            .collect();
        tx.set_children(list, &kids);
        tx.set_children(NodeId::ROOT, &[list]);
        vec![list]
    });
    let list = ids[0];
    let f = h.step(move |tx| {
        let n = tx.create();
        tx.edit::<Layout>(n).height(Length::points(10.0));
        tx.insert_child(list, 0, n);
    });
    // The new row and the list (child list changed) are dirty; every old row and nothing else moved.
    assert_eq!(f.dirty.len(), 2);
    assert!(f.dirty.contains(&(list.index() as u32)));
    assert_eq!(f.moved.len(), N);
}

#[test]
fn remove_create_and_slot_reuse_in_one_relayout_frame() {
    let (mut h, ids) = Harness::mount(column);
    let (col, b) = (ids[0], ids[2]);
    let f = h.step(move |tx| {
        tx.remove(b);
        let d = tx.create();
        tx.edit::<Layout>(d).height(Length::points(5.0));
        tx.insert_child(col, 0, d);
    });
    assert_eq!(f.removed, [b.raw()]);
    let d = f.dirty.iter().copied().find(|&i| i as usize != col.index()).expect("created node dirty");
    assert_eq!(Frame::sorted(&f.dirty), Frame::sorted(&[col.index() as u32, d]));
    // `a` shifts down by 5 and `c` up by 40 - 5: both only moved (the column is dirty: children).
    assert_eq!(Frame::sorted(&f.moved), [3, 5]);
    // Reusing a just-freed slot in the next frame is a creation, never a move.
    let f = h.step(move |tx| {
        let e = tx.create();
        tx.edit::<Layout>(e).height(Length::points(7.0));
        tx.append_child(col, e);
    });
    assert!(f.moved.iter().all(|&i| f.dirty.binary_search(&i).is_err()));
}

#[test]
fn parent_origin_move_moves_a_child_whose_absolute_rect_is_unchanged() {
    // ROOT -> column[spacer (h=20), p[x (h=10)]]. Shrinking the spacer by 10 while padding `p` by
    // 10 keeps `x` at y=20 but moves its origin inside `p` from 0 to 10.
    let (mut h, ids) = Harness::mount(|tx| {
        let column = tx.create();
        tx.edit::<Layout>(column).width(Length::points(100.0));
        let spacer = tx.create();
        tx.edit::<Layout>(spacer).height(Length::points(20.0));
        let p = tx.create();
        let x = tx.create();
        tx.edit::<Layout>(x).height(Length::points(10.0));
        tx.set_children(p, &[x]);
        tx.set_children(column, &[spacer, p]);
        tx.set_children(NodeId::ROOT, &[column]);
        vec![spacer, p, x]
    });
    let (spacer, p, x) = (ids[0], ids[1], ids[2]);
    let y_before = h.last.rects[x.index()].y;
    let f = h.step(move |tx| {
        tx.edit::<Layout>(spacer).height(Length::points(10.0));
        tx.edit::<Layout>(p).padding_top(Length::points(10.0));
    });
    assert_eq!(f.rects[x.index()].y, y_before, "x keeps its absolute position");
    assert_eq!(Frame::sorted(&f.dirty), Frame::sorted(&[spacer.index() as u32, p.index() as u32]));
    assert_eq!(f.moved, [x.index() as u32]);
}

#[test]
fn reparenting_with_an_unchanged_absolute_rect_moves_by_its_new_parent_origin() {
    // ROOT -> column[p1 (h=20, padding-top 20)[x (h=10)], p2 (h=20)]: x sits at y=20, 20 below
    // p1's origin; p2 starts at y=20, so x reparented into p2 keeps y=20 at origin 0 there.
    let (mut h, ids) = Harness::mount(|tx| {
        let column = tx.create();
        tx.edit::<Layout>(column).width(Length::points(100.0));
        let p1 = tx.create();
        tx.edit::<Layout>(p1).height(Length::points(20.0)).padding_top(Length::points(20.0));
        let p2 = tx.create();
        tx.edit::<Layout>(p2).height(Length::points(20.0));
        let x = tx.create();
        tx.edit::<Layout>(x).height(Length::points(10.0));
        tx.set_children(p1, &[x]);
        tx.set_children(column, &[p1, p2]);
        tx.set_children(NodeId::ROOT, &[column]);
        vec![p1, p2, x]
    });
    let (p1, p2, x) = (ids[0], ids[1], ids[2]);
    assert_eq!((h.last.rects[x.index()].y, h.last.rects[p2.index()].y), (20.0, 20.0));
    let f = h.step(move |tx| tx.set_children(p2, &[x]));
    assert_eq!(f.rects[x.index()].y, 20.0, "x keeps its absolute rect across the reparent");
    assert_eq!(Frame::sorted(&f.dirty), Frame::sorted(&[p1.index() as u32, p2.index() as u32]));
    assert_eq!(f.moved, [x.index() as u32], "its origin inside the new parent changed (20 -> 0)");
    // And back: same absolute rect again, origin 0 -> 20.
    let f = h.step(move |tx| tx.set_children(p1, &[x]));
    assert_eq!(f.rects[x.index()].y, 20.0);
    assert_eq!(f.moved, [x.index() as u32]);
}

#[test]
fn moving_a_container_moves_only_the_container_not_its_subtree() {
    // ROOT -> column[spacer (h=10), box[row[leaf, leaf], row[leaf, leaf]]]: growing the spacer
    // shifts `box` and its whole nested subtree in absolute terms, but only `box` moves relative to
    // its parent; every descendant keeps its parent-relative box.
    let (mut h, ids) = Harness::mount(|tx| {
        let column = tx.create();
        tx.edit::<Layout>(column).width(Length::points(100.0));
        let spacer = tx.create();
        tx.edit::<Layout>(spacer).height(Length::points(10.0));
        let bx = tx.create();
        let rows: Vec<NodeId> = (0..2)
            .map(|_| {
                let row = tx.create();
                let leaves: Vec<NodeId> = (0..2)
                    .map(|_| {
                        let leaf = tx.create();
                        tx.edit::<Layout>(leaf).height(Length::points(5.0));
                        leaf
                    })
                    .collect();
                tx.set_children(row, &leaves);
                row
            })
            .collect();
        tx.set_children(bx, &rows);
        tx.set_children(column, &[spacer, bx]);
        tx.set_children(NodeId::ROOT, &[column]);
        vec![spacer, bx]
    });
    let (spacer, bx) = (ids[0], ids[1]);
    let before = h.last.rects.clone();
    let f = h.step(move |tx| {
        tx.edit::<Layout>(spacer).height(Length::points(30.0));
    });
    assert_eq!(f.dirty, [spacer.index() as u32]);
    // The column grew (size), `box` shifted by 20: nothing below `box` is listed.
    assert_eq!(Frame::sorted(&f.moved), Frame::sorted(&[2, bx.index() as u32]));
    let shifted = (1..f.rects.len()).filter(|&i| f.live(i) && f.rects[i].y == before[i].y + 20.0).count();
    assert_eq!(shifted, 7, "box, 2 rows and 4 leaves moved in absolute terms (rects republished)");
}

#[test]
fn a_node_both_edited_and_moved_is_dirty_exactly_once() {
    let (mut h, ids) = Harness::mount(column);
    let (col, a, b) = (ids[0], ids[1], ids[2]);
    // `b` is touched (paint), its parent's child list changes, and `a` grows so `b` also moves.
    let f = h.step(move |tx| {
        tx.edit::<Layout>(a).height(Length::points(45.0));
        tx.edit::<Visual>(b).background(Rgba::rgb(1, 2, 3));
        let c = tx.store().children(col)[2];
        tx.set_children(col, &[a, b, c]);
    });
    assert_eq!(f.dirty.iter().filter(|&&i| i as usize == b.index()).count(), 1);
    assert!(!f.moved.contains(&(b.index() as u32)));
}

#[test]
fn viewport_resize_moves_without_dirtying() {
    let (mut h, _) = Harness::mount(|tx| {
        let fill = tx.create();
        tx.edit::<Layout>(fill).width(Length::percent(100.0)).height(Length::points(10.0));
        tx.set_children(NodeId::ROOT, &[fill]);
        vec![fill]
    });
    assert!(h.rt.set_viewport(Viewport { width: 200.0, height: 300.0, scale_factor: 1.0 }).expect("viewport"));
    assert!(h.rt.tick(0.016).expect("tick"));
    let f = h.next_frame();
    assert!(f.dirty.is_empty());
    assert_eq!(Frame::sorted(&f.moved), [1, 2]);
}
