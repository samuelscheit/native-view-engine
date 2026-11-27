//! `SceneDelta`: the per-frame node change sets published in `NveRuntimeSceneView` — content-dirty
//! (`dirty_indices`), geometry-only (`moved_indices`), and `removed_ids` — plus the published rects.
//!
//! The two index sets are exact and disjoint: a node is in `dirty` when it was created or its row,
//! a table row, or its child list changed; otherwise it is in `moved` when its absolute rect or its
//! origin relative to its parent's rect changed. Both are deduplicated through one epoch-stamped
//! slot array, so recording is O(changed + node capacity on relayout) with zero heap allocations
//! once capacities warm.
//!
//! Inherited styling is content too: a `Typography` row change / attach / detach dirties every
//! text / text-input descendant inheriting it (paint-only edits move no rect), so retained hosts
//! restyle exactly `dirty` instead of walking the whole tree.

use nve_abi::NveLayoutRect;
use nve_components::tables::{TextContent, TextInput, Typography};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable};

#[derive(Default)]
pub(crate) struct SceneDelta {
    /// Published absolute layout rect per node slot.
    pub(crate) rects: Vec<NveLayoutRect>,
    /// Published origin of each node slot relative to its parent's rect (its own origin for roots).
    local_origins: Vec<[f32; 2]>,
    pub(crate) dirty: Vec<u32>,
    pub(crate) moved: Vec<u32>,
    pub(crate) removed: Vec<u32>,
    /// `epoch` once a slot joined `dirty` or `moved` this frame.
    stamp: Vec<u32>,
    epoch: u32,
    /// `epoch`-stamped roots of this frame's inherited-typography walk.
    inherit_stamp: Vec<u32>,
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
        self.local_origins.clear();
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
        self.local_origins.resize(cap, [0.0; 2]);
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
        // Absolute rects first: a parent's new rect is needed for its children's origins below,
        // and a parent may sit at a higher slot than its child.
        for idx in 1..cap {
            let id = store.id_at(idx);
            let r = id.and_then(&layout_of).unwrap_or_default();
            if self.rects[idx] != r {
                self.rects[idx] = r;
                if id.is_some() {
                    self.mark(idx, false);
                }
            }
        }
        for idx in 1..cap {
            let Some(id) = store.id_at(idx) else {
                continue;
            };
            let r = self.rects[idx];
            let origin = match store.parent(id) {
                Some(p) => {
                    let pr = self.rects[p.index()];
                    [r.x - pr.x, r.y - pr.y]
                }
                None => [r.x, r.y],
            };
            if self.local_origins[idx] != origin {
                self.local_origins[idx] = origin;
                self.mark(idx, false);
            }
        }
    }

    /// O(union of the changed nodes' subtrees): changed roots are stamped first and a walk never
    /// enters another root's subtree (that root walks it), so nested changes visit a node once.
    fn mark_inherited_typography(&mut self, store: &NodeStore, changes: &ChangeSet) {
        let Some(typo) = changes.table(Typography::slot()) else {
            return;
        };
        let cap = self.stamp.len();
        if self.inherit_stamp.len() < cap {
            self.inherit_stamp.resize(cap, 0);
        }
        let epoch = self.epoch;
        let roots = || typo.nodes.iter().chain(&typo.detached).copied();
        for id in roots() {
            if id.index() < cap && store.is_live(id) {
                self.inherit_stamp[id.index()] = epoch;
            }
        }
        let (text, input) = (TextContent::slot(), TextInput::slot());
        for root in roots() {
            if !store.is_live(root) {
                continue;
            }
            self.inherit_stack.clear();
            self.inherit_stack.extend_from_slice(store.children(root));
            while let Some(id) = self.inherit_stack.pop() {
                let idx = id.index();
                if idx >= cap || !store.is_live(id) || self.inherit_stamp[idx] == epoch {
                    continue;
                }
                if store.has_slot(id, text) || store.has_slot(id, input) {
                    self.mark(idx, true);
                }
                self.inherit_stack.extend_from_slice(store.children(id));
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
