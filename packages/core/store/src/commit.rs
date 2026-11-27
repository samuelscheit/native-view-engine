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
            let dirty_len = col.dirty.as_slice().len();
            if t.nodes.capacity() < dirty_len {
                t.nodes.reserve(dirty_len);
            }
            if t.masks.capacity() < dirty_len {
                t.masks.reserve(dirty_len);
            }
            for &index in col.dirty.as_slice() {
                let node = &s.nodes[index];
                let attached = node.flags & LIVE != 0 && node.tables.contains(slot);
                let Some(row) = col.find_row(index) else {
                    continue;
                };
                let m = col.masks[row];
                col.masks[row] = 0;
                if attached && m != 0 {
                    t.nodes.push(NodeId::from_index(index, node.generation));
                    t.masks.push(m);
                }
            }
            col.dirty.clear();
            if col.detached.len() > 1 {
                col.detached.sort_unstable_by_key(|id| id.0);
                col.detached.dedup();
            }
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

/// Deduplicates a sorted `u32` slice in-place with zero heap allocations; returns the unique length.
#[inline]
fn dedup_sorted_u32(slice: &mut [u32]) -> usize {
    if slice.len() <= 1 {
        return slice.len();
    }
    let mut write = 1usize;
    for read in 1..slice.len() {
        if slice[read] != slice[write - 1] {
            slice[write] = slice[read];
            write += 1;
        }
    }
    write
}

impl crate::store::NodeStore {
    /// Populates `out` with zero-copy [`nve_abi::table::NveTableView`] entries for every
    /// `PUBLISH_TO_SCENE` table currently allocated in this store — zero heap allocations when
    /// `out` and `dirty_buf` have sufficient capacity.
    ///
    /// Dirty row indices are sorted and deduplicated per table inside `dirty_buf` (which must live
    /// as long as `out` is read).
    pub fn publish_table_views(
        &self,
        changes: &ChangeSet,
        out: &mut Vec<nve_abi::table::NveTableView>,
        dirty_buf: &mut Vec<u32>,
    ) {
        out.clear();
        dirty_buf.clear();
        let mut ranges = [(TableSlot(0), 0usize, 0usize); nve_abi::table::MAX_TABLE_SLOTS];
        let mut range_len = 0usize;
        let mut reloc_pos = 0usize;
        for slot in self.table_slots() {
            let Some(col) = self.columns[slot.0 as usize].as_ref() else {
                continue;
            };
            if col.desc.flags() & nve_abi::table::table_flags::PUBLISH_TO_SCENE == 0 {
                continue;
            }
            let start = dirty_buf.len();
            if let Some(tc) = changes.table(slot) {
                for &nid in &tc.nodes {
                    if self.is_live(nid)
                        && let Some(row_idx) = col.find_row(nid.node_index())
                    {
                        dirty_buf.push(row_idx as u32);
                    }
                }
            }
            while reloc_pos < changes.relocated_rows.len()
                && changes.relocated_rows[reloc_pos].0 < slot
            {
                reloc_pos += 1;
            }
            let row_count_u32 = col.row_count() as u32;
            while reloc_pos < changes.relocated_rows.len()
                && changes.relocated_rows[reloc_pos].0 == slot
            {
                let row_idx = changes.relocated_rows[reloc_pos].1;
                if row_idx < row_count_u32 {
                    dirty_buf.push(row_idx);
                }
                reloc_pos += 1;
            }
            if dirty_buf.len() - start > 1 {
                let slice = &mut dirty_buf[start..];
                slice.sort_unstable();
                let unique = dedup_sorted_u32(slice);
                dirty_buf.truncate(start + unique);
            }
            ranges[range_len] = (slot, start, dirty_buf.len() - start);
            range_len += 1;
        }

        for &(slot, start, dirty_count) in &ranges[..range_len] {
            let Some(col) = self.columns[slot.0 as usize].as_ref() else {
                continue;
            };
            let dense = col.is_dense();
            let row_count = col.published_row_count(self.capacity());
            let rows_ptr = if col.rows.is_empty() {
                core::ptr::null()
            } else {
                col.rows.as_ptr()
            };
            let owners_ptr = if dense || col.owners.is_empty() {
                core::ptr::null()
            } else {
                col.owners.as_ptr()
            };
            let dirty_ptr = if dirty_count == 0 {
                core::ptr::null()
            } else {
                // SAFETY: `start` is within `dirty_buf` (captured before pushing `dirty_count > 0` items).
                unsafe { dirty_buf.as_ptr().add(start) }
            };
            out.push(nve_abi::table::NveTableView {
                key_lo: col.desc.key().lo(),
                key_hi: col.desc.key().hi(),
                slot: slot.0 as u32,
                words: col.w as u32,
                row_count: row_count as u32,
                dirty_count: dirty_count as u32,
                rows: rows_ptr,
                owners: owners_ptr,
                dirty_rows: dirty_ptr,
            });
        }
    }
}
