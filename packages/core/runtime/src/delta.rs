//! `SceneDelta`: the per-frame node change sets published in `NveRuntimeSceneView` — content-dirty
//! (`dirty_indices`), geometry-only (`moved_indices`), and `removed_ids` — plus the published rects.
//!
//! The two index sets are exact and disjoint: a node is in `dirty` when it was created or its row,
//! a table row, or its child list changed; otherwise it is in `moved` when its parent-relative box
//! (origin relative to its parent's rect, width, height) changed. Hosts position views relative to
//! their parent, so the descendants of a moved container — whose absolute rects change with it —
//! are not listed (`rects` still carries their new absolute boxes). Both sets are deduplicated
//! through one epoch-stamped slot array, so recording is O(changed + node capacity on relayout,
//! one pass) with zero heap allocations once capacities warm.

use nve_abi::NveLayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore};

#[derive(Default)]
pub(crate) struct SceneDelta {
    /// Published absolute layout rect per node slot.
    pub(crate) rects: Vec<NveLayoutRect>,
    /// Published box of each node slot relative to its parent's rect (absolute for roots).
    locals: Vec<NveLayoutRect>,
    pub(crate) dirty: Vec<u32>,
    pub(crate) moved: Vec<u32>,
    pub(crate) removed: Vec<u32>,
    /// `epoch` once a slot joined `dirty` or `moved` this frame.
    stamp: Vec<u32>,
    epoch: u32,
}

impl SceneDelta {
    /// Drops this frame's sets (the published view is invalidated).
    pub(crate) fn clear_sets(&mut self) {
        self.dirty.clear();
        self.moved.clear();
        self.removed.clear();
    }

    /// Forgets every published rect and origin (the store is replaced).
    pub(crate) fn reset(&mut self) {
        self.clear_sets();
        self.rects.clear();
        self.locals.clear();
    }

    /// Records one committed frame. `layout_of` reads the new layout; it is consulted only when
    /// `laid_out` (otherwise every rect is unchanged).
    pub(crate) fn record(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        laid_out: bool,
        layout_of: impl Fn(NodeId) -> Option<NveLayoutRect>,
    ) {
        let cap = store.capacity();
        self.rects.resize(cap, NveLayoutRect::default());
        self.locals.resize(cap, NveLayoutRect::default());
        if self.stamp.len() < cap {
            self.stamp.resize(cap, 0);
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.stamp.fill(0);
            self.epoch = 1;
        }
        self.clear_sets();

        for &id in changes
            .created
            .iter()
            .chain(&changes.touched)
            .chain(&changes.children)
        {
            if id.index() < cap && store.is_live(id) {
                self.mark(id.index(), true);
            }
        }
        self.removed.extend(changes.removed.iter().map(|id| id.raw()));

        if !laid_out {
            return;
        }
        // One pass: the parent's new rect comes from `layout_of` (a parent may sit at a higher
        // slot than its child, so `rects` may not hold it yet).
        for idx in 1..cap {
            let id = store.id_at(idx);
            let r = id.and_then(&layout_of).unwrap_or_default();
            self.rects[idx] = r;
            let local = match id {
                None => NveLayoutRect::default(),
                Some(id) => match store.parent(id).and_then(&layout_of) {
                    Some(p) => NveLayoutRect { x: r.x - p.x, y: r.y - p.y, ..r },
                    None => r,
                },
            };
            if self.locals[idx] != local {
                self.locals[idx] = local;
                if id.is_some() {
                    self.mark(idx, false);
                }
            }
        }
    }

    #[inline]
    fn mark(&mut self, idx: usize, content: bool) {
        if self.stamp[idx] != self.epoch {
            self.stamp[idx] = self.epoch;
            if content {
                self.dirty.push(idx as u32);
            } else {
                self.moved.push(idx as u32);
            }
        }
    }
}
