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

    /// Pre-sizes top-level vectors ahead of a bulk `NVE2` keyframe commit (`Frame0File` boot).
    pub fn reserve_for_frame0(&mut self, node_count: usize) {
        if self.created.capacity() < node_count {
            self.created
                .reserve(node_count.saturating_sub(self.created.len()));
        }
        if self.children.capacity() < node_count {
            self.children
                .reserve(node_count.saturating_sub(self.children.len()));
        }
        if self.touched.capacity() < node_count {
            self.touched
                .reserve(node_count.saturating_sub(self.touched.len()));
        }
        if self.effects.capacity() < node_count {
            self.effects
                .reserve(node_count.saturating_sub(self.effects.len()));
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
