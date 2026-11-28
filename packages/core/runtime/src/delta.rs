//! `SceneDelta`: the per-frame node change sets published in `NveSceneView` — content-dirty
//! (`dirty_indices`), geometry-only (`moved_indices`), and `removed_ids` — plus the published rects.
//!
//! The two index sets are exact and disjoint: a node is in `dirty` when it was created or its row,
//! a table row, or its child list changed; otherwise it is in `moved` when its parent-relative box
//! (origin relative to its parent's rect, width, height) changed. Hosts position views relative to
//! their parent, so the descendants of a moved container — whose absolute rects change with it —
//! are not listed (`rects` still carries their new absolute boxes). Both sets are deduplicated
//! through one epoch-stamped slot array, so recording is O(changed + node capacity on relayout,
//! one pass) with zero heap allocations once capacities warm.
//!
//! Inherited styling is content too: a `Typography` row change / attach / detach, or a subtree
//! moving under another parent, dirties every text / text-input descendant inheriting it
//! (paint-only edits move no rect), so retained hosts restyle exactly `dirty` instead of walking
//! the whole tree.

use nve_abi::NveLayoutRect;
use nve_abi::table::effect;
use nve_tables::tables::{TextContent, TextInput, Typography};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};

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
    /// `epoch` once a slot is a root of, or was visited by, this frame's inherited-typography
    /// walk.
    inherit_stamp: Vec<u32>,
    /// Deduplicated roots of this frame's inherited-typography walk (reused).
    inherit_roots: Vec<NodeId>,
    /// Reusable DFS stack of the inherited-typography walk.
    inherit_stack: Vec<NodeId>,
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
            self.inherit_stamp.fill(0);
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
        self.mark_inherited_typography(store, changes);
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
            // Dead slots reset to the default box, so a reused slot compares against nothing.
            let local = match id {
                None => NveLayoutRect::default(),
                Some(id) => match store.parent(id).and_then(&layout_of) {
                    Some(p) => NveLayoutRect { x: r.x - p.x, y: r.y - p.y, ..r },
                    None => r,
                },
            };
            if !same_bits(&self.locals[idx], &local) {
                self.locals[idx] = local;
                if id.is_some() {
                    self.mark(idx, false);
                }
            }
        }
    }

    /// Roots are the nodes whose `Typography` row changed / attached / detached and the
    /// containers that moved under another parent (touched with `MEASURE`; leaves have nothing
    /// to walk). O(union of the roots' subtrees): roots are deduplicated and stamped first, and
    /// every visited node is stamped, so a walk never enters another root's subtree (that root
    /// walks it) and nested or repeated roots visit each node once.
    fn mark_inherited_typography(&mut self, store: &NodeStore, changes: &ChangeSet) {
        let cap = self.stamp.len();
        if self.inherit_stamp.len() < cap {
            self.inherit_stamp.resize(cap, 0);
        }
        let epoch = self.epoch;
        let typo = changes
            .table(Typography::slot())
            .map(|t| (t.nodes.as_slice(), t.detached.as_slice()))
            .unwrap_or_default();
        let moved = changes
            .touched
            .iter()
            .zip(&changes.effects)
            .filter(|&(&id, &fx)| fx & effect::MEASURE != 0 && !store.children(id).is_empty())
            .map(|(&id, _)| id);
        self.inherit_roots.clear();
        for id in typo.0.iter().chain(typo.1).copied().chain(moved) {
            let idx = id.index();
            if idx < cap && store.is_live(id) && self.inherit_stamp[idx] != epoch {
                self.inherit_stamp[idx] = epoch;
                self.inherit_roots.push(id);
            }
        }
        let (text, input) = (TextContent::slot(), TextInput::slot());
        let roots = std::mem::take(&mut self.inherit_roots);
        for &root in &roots {
            self.inherit_stack.clear();
            self.inherit_stack.extend_from_slice(store.children(root));
            while let Some(id) = self.inherit_stack.pop() {
                let idx = id.index();
                if idx >= cap || !store.is_live(id) || self.inherit_stamp[idx] == epoch {
                    continue;
                }
                self.inherit_stamp[idx] = epoch;
                if store.has_slot(id, text) || store.has_slot(id, input) {
                    self.mark(idx, true);
                }
                self.inherit_stack.extend_from_slice(store.children(id));
            }
        }
        self.inherit_roots = roots;
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

/// Bit equality: total and reflexive (a NaN box equals itself; `-0.0` differs from `0.0`), so a
/// non-finite rect can neither re-list a node every frame nor hide a real change.
#[inline]
fn same_bits(a: &NveLayoutRect, b: &NveLayoutRect) -> bool {
    [a.x, a.y, a.width, a.height].map(f32::to_bits) == [b.x, b.y, b.width, b.height].map(f32::to_bits)
}
