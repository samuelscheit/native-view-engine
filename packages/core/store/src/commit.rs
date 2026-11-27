//! Frame close: drains per-frame tracking into a reusable [`ChangeSet`], derives host classes,
//! and recycles removed node / resource slots at the frame boundary.

use nve_abi::table::effect;

use crate::changes::ChangeSet;
use crate::id::NodeId;
use crate::store::{CHILDREN_DIRTY, CREATED, IN_FREE, LIVE, PENDING_FREE, RETIRED};
use crate::table::TableSlot;
use crate::tx::Transaction;

impl Transaction<'_> {
    /// Closes the frame: refills `changes` in place (zero steady-state allocations) and clears all
    /// per-frame tracking. Removed node indices and released resources are recycled afterwards, so
    /// ids seen in this frame never alias within it.
    pub fn commit_into(self, changes: &mut ChangeSet) {
        let s = self.store;
        changes.clear();
        s.frame += 1;
        changes.frame = s.frame;

        for &id in &s.created {
            let row = &mut s.nodes[id.node_index()];
            if row.flags & LIVE != 0 && row.generation == id.generation() {
                row.flags &= !CREATED;
                changes.created.push(id);
            }
        }
        s.created.clear();
        changes.removed.append(&mut s.removed);

        for &index in s.reparented.as_slice() {
            let row = &mut s.nodes[index];
            if row.flags & LIVE != 0 {
                row.flags &= !CHILDREN_DIRTY;
                changes
                    .children
                    .push(NodeId::from_index(index, row.generation));
            }
        }
        s.reparented.clear();

        for &index in s.touched.as_slice() {
            let row = &mut s.nodes[index];
            if row.flags & LIVE != 0 && row.effects != 0 {
                if row.effects & effect::REALIZE != 0 {
                    row.primary_slot = s.realization.primary_slot(row.tables);
                }
                changes
                    .touched
                    .push(NodeId::from_index(index, row.generation));
                changes.effects.push(row.effects);
            }
            row.effects = 0;
        }
        s.touched.clear();

        for (slot, col) in s.columns.iter_mut().enumerate() {
            let Some(col) = col else { continue };
            let slot = TableSlot(slot as u8);
            let mut advance_epoch = false;
            if !col.relocated_rows.is_empty() {
                advance_epoch = true;
                if col.desc.flags() & nve_abi::table::table_flags::PUBLISH_TO_SCENE != 0 {
                    let row_count = col.row_count() as u32;
                    for r in col.relocated_rows.drain(..) {
                        // Rows with non-zero mask are already emitted via `t.nodes` below, so
                        // checking `masks[r] == 0` guarantees disjoint, O(1)-deduped dirty rows.
                        if r < row_count && col.masks[r as usize] == 0 {
                            changes.relocated_rows.push((slot, r));
                        }
                    }
                } else {
                    col.relocated_rows.clear();
                }
            }
            if col.dirty.as_slice().is_empty() && col.detached.is_empty() {
                if advance_epoch {
                    col.next_frame_epoch();
                }
                continue;
            }
            let t = changes.next_table(slot);
            for &index in col.dirty.as_slice() {
                let node = &s.nodes[index];
                let attached = node.flags & LIVE != 0 && node.tables.contains(slot);
                let Some(row) = col.find_row(index) else { continue };
                let m = col.masks[row];
                col.masks[row] = 0;
                if attached && m != 0 {
                    t.nodes.push(NodeId::from_index(index, node.generation));
                    t.masks.push(m);
                }
            }
            col.dirty.clear();
            for id in col.detached.drain(..) {
                let node = &s.nodes[id.node_index()];
                if node.flags & LIVE != 0
                    && node.generation == id.generation()
                    && !node.tables.contains(slot)
                {
                    t.detached.push(id);
                }
            }
            col.next_frame_epoch();
            changes.pop_empty_table();
        }

        let res = &mut s.resources;
        changes
            .resources_defined
            .extend(res.defined.iter().copied().filter(|&id| res.is_live(id)));
        changes
            .resources_released
            .extend(res.released.iter().copied().filter(|&id| !res.is_live(id)));
        res.end_frame();

        // Filter out commands whose target node was removed before commit so NVE2 replicas never
        // reject a valid frame with `DeadNode`.
        let nodes = &s.nodes;
        s.commands.retain(|c| {
            nodes
                .get(c.node.node_index())
                .is_some_and(|r| r.flags & LIVE != 0 && r.generation == c.node.generation())
        });
        core::mem::swap(&mut changes.commands, &mut s.commands);
        core::mem::swap(&mut changes.command_bytes, &mut s.command_bytes);
        s.commands.clear();
        s.command_bytes.clear();

        for index in s.pending_free.drain(..) {
            let row = &mut s.nodes[index];
            if row.flags & LIVE == 0 && row.flags & PENDING_FREE != 0 {
                if row.generation == u8::MAX {
                    // Permanently retire slot at generation 255 to prevent 8-bit ABA wraparound.
                    row.flags = RETIRED;
                } else {
                    row.generation += 1;
                    row.flags = IN_FREE;
                    s.free.push_back(index);
                }
            }
        }
    }
}
