//! `ChangeSet`: everything one committed frame changed, read by `Backend::commit`.
//!
//! Buffers are reused across frames (`Transaction::commit_into` refills in place).

use nve_abi::table::effect;

use crate::id::{NodeId, ResourceId};
use crate::table::TableSlot;

/// An imperative command addressed to a node's table (e.g. `webview.reload`, `video.play`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub node: NodeId,
    pub table: TableSlot,
    pub op: u32,
    pub(crate) off: u32,
    pub(crate) len: u32,
}

/// Row changes of one table in one frame.
#[derive(Debug, Clone, Default)]
pub struct TableChanges {
    pub slot: TableSlot,
    /// Nodes whose row changed (newly attached rows report every word).
    pub nodes: Vec<NodeId>,
    /// Changed-word mask per entry of `nodes`.
    pub masks: Vec<u32>,
    /// Live nodes that detached this table.
    pub detached: Vec<NodeId>,
}

impl TableChanges {
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, u32)> + '_ {
        self.nodes.iter().copied().zip(self.masks.iter().copied())
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.detached.is_empty()
    }
}

impl Default for TableSlot {
    fn default() -> Self {
        TableSlot(u8::MAX)
    }
}

/// Number of active tables (`0..=MAX_TABLE_SLOTS`) recorded in a [`ChangeSet`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TableCount(pub u8);

impl TableCount {
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// One frame of changes.
#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
    pub frame: u64,
    /// Nodes created this frame (and still live), in creation order.
    pub created: Vec<NodeId>,
    /// Nodes removed this frame that existed before it (descendants before ancestors).
    pub removed: Vec<NodeId>,
    /// Parents whose child list changed.
    pub children: Vec<NodeId>,
    /// Nodes with any effect this frame, with their effect bits.
    pub touched: Vec<NodeId>,
    pub effects: Vec<u8>,
    /// Per-table row changes (only tables that changed).
    pub tables: Vec<TableChanges>,
    pub(crate) table_len: TableCount,
    /// Sparse rows relocated via swap-remove this frame (`(slot, row_idx)`), merged into
    /// `NveTableView::dirty_rows` without causing `NVE2` to re-transmit unmodified rows.
    pub(crate) relocated_rows: Vec<(TableSlot, u32)>,
    pub resources_defined: Vec<ResourceId>,
    pub resources_released: Vec<ResourceId>,
    pub commands: Vec<Command>,
    pub command_bytes: Vec<u8>,
}

impl ChangeSet {
    /// Changes of `slot`, if it changed this frame.
    #[inline]
    pub fn table(&self, slot: TableSlot) -> Option<&TableChanges> {
        self.tables().iter().find(|t| t.slot == slot)
    }

    /// Changed tables.
    #[inline]
    pub fn tables(&self) -> &[TableChanges] {
        &self.tables[..self.table_len.as_usize()]
    }

    /// Union of all effects this frame.
    pub fn effects_union(&self) -> u8 {
        self.effects.iter().fold(0, |a, &e| a | e)
    }

    /// Whether any node needs relayout.
    pub fn needs_layout(&self) -> bool {
        !self.created.is_empty()
            || !self.removed.is_empty()
            || !self.children.is_empty()
            || self.effects_union() & (effect::LAYOUT | effect::MEASURE) != 0
    }

    pub fn command_payload(&self, c: &Command) -> &[u8] {
        &self.command_bytes[c.off as usize..(c.off + c.len) as usize]
    }

    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.removed.is_empty()
            && self.children.is_empty()
            && self.touched.is_empty()
            && self.table_len.0 == 0
            && self.commands.is_empty()
            && self.resources_defined.is_empty()
            && self.resources_released.is_empty()
    }

    /// Fallibly pre-sizes top-level vectors ahead of a bulk `NVE2` keyframe commit (`Frame0File`
    /// boot), bounded by `payload_bytes / 4` and [`NodeId::MAX_INDEX`].
    pub fn reserve_for_frame0(
        &mut self,
        node_count: usize,
        payload_bytes: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        let capped = node_count
            .min(payload_bytes / 4)
            .min(NodeId::MAX_INDEX as usize);
        if capped == 0 {
            return Ok(());
        }
        if self.created.capacity() < capped {
            self.created
                .try_reserve(capped.saturating_sub(self.created.len()))?;
        }
        if self.children.capacity() < capped {
            self.children
                .try_reserve(capped.saturating_sub(self.children.len()))?;
        }
        if self.touched.capacity() < capped {
            self.touched
                .try_reserve(capped.saturating_sub(self.touched.len()))?;
        }
        if self.effects.capacity() < capped {
            self.effects
                .try_reserve(capped.saturating_sub(self.effects.len()))?;
        }
        Ok(())
    }

    /// Overwrites `self` with a copy of `other` while reusing existing `Vec` capacities.
    pub fn copy_from(&mut self, other: &ChangeSet) {
        self.clear();
        self.frame = other.frame;
        self.merge_from(other);
    }

    /// Merges `other` into `self` while preserving `Vec` capacities, reconciling nodes created in
    /// `self` and removed in `other` (preventing create-then-remove resurrection across multi-pump
    /// ticks), and keeping `relocated_rows` ordered by `TableSlot` for linear `publish_table_views`.
    pub fn merge_from(&mut self, other: &ChangeSet) {
        if other.frame > self.frame {
            self.frame = other.frame;
        }
        if !other.removed.is_empty() {
            // Strip nodes removed in `other` from earlier `self` lists. Nodes born in `self.created`
            // and killed in `other.removed` never existed before this merged frame and cancel out.
            for &rid in &other.removed {
                if let Some(pos) = self.created.iter().position(|&c| c == rid) {
                    self.created.remove(pos);
                } else {
                    self.removed.push(rid);
                }
            }
            self.children.retain(|id| !other.removed.contains(id));
            let mut write = 0usize;
            for read in 0..self.touched.len() {
                if !other.removed.contains(&self.touched[read]) {
                    self.touched[write] = self.touched[read];
                    self.effects[write] = self.effects[read];
                    write += 1;
                }
            }
            self.touched.truncate(write);
            self.effects.truncate(write);
            for tbl in &mut self.tables[..self.table_len.as_usize()] {
                let mut tw = 0usize;
                for tr in 0..tbl.nodes.len() {
                    if !other.removed.contains(&tbl.nodes[tr]) {
                        tbl.nodes[tw] = tbl.nodes[tr];
                        tbl.masks[tw] = tbl.masks[tr];
                        tw += 1;
                    }
                }
                tbl.nodes.truncate(tw);
                tbl.masks.truncate(tw);
                tbl.detached.retain(|id| !other.removed.contains(id));
            }
        }
        self.created.extend_from_slice(&other.created);
        self.children.extend_from_slice(&other.children);
        self.touched.extend_from_slice(&other.touched);
        self.effects.extend_from_slice(&other.effects);
        for src in other.tables() {
            if let Some(dst) = self.tables[..self.table_len.as_usize()]
                .iter_mut()
                .find(|t| t.slot == src.slot)
            {
                dst.nodes.extend_from_slice(&src.nodes);
                dst.masks.extend_from_slice(&src.masks);
                dst.detached.extend_from_slice(&src.detached);
            } else {
                let dst = self.next_table(src.slot);
                dst.nodes.extend_from_slice(&src.nodes);
                dst.masks.extend_from_slice(&src.masks);
                dst.detached.extend_from_slice(&src.detached);
            }
        }
        if !other.relocated_rows.is_empty() {
            let had = !self.relocated_rows.is_empty();
            self.relocated_rows.extend_from_slice(&other.relocated_rows);
            if had {
                self.relocated_rows.sort_unstable();
            }
        }
        self.resources_defined
            .extend_from_slice(&other.resources_defined);
        self.resources_released
            .extend_from_slice(&other.resources_released);
        if !other.commands.is_empty() {
            let base = self.command_bytes.len() as u32;
            self.command_bytes.extend_from_slice(&other.command_bytes);
            for cmd in &other.commands {
                self.commands.push(Command {
                    node: cmd.node,
                    table: cmd.table,
                    op: cmd.op,
                    off: base.saturating_add(cmd.off),
                    len: cmd.len,
                });
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.created.clear();
        self.removed.clear();
        self.children.clear();
        self.touched.clear();
        self.effects.clear();
        self.table_len = TableCount(0);
        self.relocated_rows.clear();
        self.resources_defined.clear();
        self.resources_released.clear();
        self.commands.clear();
        self.command_bytes.clear();
    }

    pub(crate) fn next_table(&mut self, slot: TableSlot) -> &mut TableChanges {
        let len = self.table_len.as_usize();
        if len == self.tables.len() {
            self.tables.push(TableChanges::default());
        }
        let t = &mut self.tables[len];
        self.table_len = TableCount((len + 1) as u8);
        t.slot = slot;
        t.nodes.clear();
        t.masks.clear();
        t.detached.clear();
        t
    }

    pub(crate) fn pop_empty_table(&mut self) {
        let len = self.table_len.as_usize();
        if len > 0 && self.tables[len - 1].is_empty() {
            self.table_len = TableCount((len - 1) as u8);
        }
    }
}
