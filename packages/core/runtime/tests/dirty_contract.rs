//! `NveRuntimeSceneView::dirty_indices` contract: every live node whose row, tables, children OR
//! layout rect changed appears exactly once; nodes untouched by both edits and relayout never do.

use nve_components::tables::{Layout, Visual};
use nve_engine::Viewport;
use nve_runtime::{AppFrame, FrameTickHook, RuntimeApp, RuntimeBuilder};
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

/// Alternates the height of node `target` (column child 1) on every tick after the first.
struct GrowFirst {
    target: usize,
    step: u32,
    paint_only: bool,
}

impl FrameTickHook for GrowFirst {
    fn tick(&mut self, tx: &mut Transaction<'_>, _dt_s: f32) -> bool {
        self.step += 1;
        if self.step == 1 {
            return false;
        }
        let Some(id) = tx.store().id_at(self.target) else { return false };
        if self.paint_only {
            tx.edit::<Visual>(id).background(Rgba::rgb(self.step as u8, 0, 0));
        } else {
            let h = if self.step.is_multiple_of(2) { 60.0 } else { 30.0 };
            tx.edit::<Layout>(id).height(Length::points(h));
        }
        true
    }
}

/// ROOT -> column[a (h=30), b (h=40), c (h=50)]; `c` sits after `b`, so growing `a` moves both.
fn column_store() -> (NodeStore, ChangeSet) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
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
        tx.commit_into(&mut changes);
    }
    (store, changes)
}

fn dirty_after_second_tick(paint_only: bool) -> (Vec<u32>, Vec<f32>) {
    let (store, changes) = column_store();
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport { width: 400.0, height: 300.0, scale_factor: 1.0 })
        .app(Box::new(StoreApp { store, changes, dirty: true }))
        .tick_hook(Box::new(GrowFirst { target: 3, step: 0, paint_only }))
        .build();
    assert!(rt.tick(0.016).expect("mount"));
    assert!(rt.tick(0.016).expect("edit"));
    let guard = rt.acquire_scene().expect("acquire");
    let v = guard.view();
    // SAFETY: `guard` holds the reader lock for `dirty_count` indices / `node_capacity` rects.
    let (dirty, rects) = unsafe {
        (
            std::slice::from_raw_parts(v.dirty_indices, v.dirty_count as usize),
            std::slice::from_raw_parts(v.rects, v.node_capacity as usize),
        )
    };
    let mut dirty = dirty.to_vec();
    dirty.sort_unstable();
    (dirty, rects.iter().map(|r| r.y).collect())
}

#[test]
fn relayout_moved_siblings_are_dirty_exactly_once() {
    // Slots: 1 ROOT, 2 column, 3 a, 4 b, 5 c. Growing `a` (30 -> 60) moves `b` and `c` and grows
    // the auto-height column; ROOT keeps its viewport rect.
    let (dirty, ys) = dirty_after_second_tick(false);
    assert_eq!(dirty, [2, 3, 4, 5]);
    assert_eq!((ys[4], ys[5]), (60.0, 100.0));
}

#[test]
fn paint_only_edit_marks_only_the_touched_node() {
    assert_eq!(dirty_after_second_tick(true).0, [3]);
}
