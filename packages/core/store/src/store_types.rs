//! Public helper types paired with [`crate::NodeStore`]: [`RealizationOrder`], [`StoreStats`],
//! and [`TableStorage`].

use nve_abi::table::Word;

use crate::id::{NodeId, TableSet};
use crate::table::{NveTable, TableSlot};

/// Ordered realization priority list: the first attached [`TableSlot`] in `slots` becomes the
/// node's `primary_slot`; nodes with none of them get [`TableSlot::NONE`] (plain container).
#[derive(Debug, Clone, Default)]
pub struct RealizationOrder {
    pub slots: Vec<TableSlot>,
}

impl RealizationOrder {
    pub const fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Appends table `T` at the next (lower) realization priority.
    pub fn with<T: NveTable>(mut self) -> Self {
        self.slots.push(T::slot());
        self
    }

    /// Appends an arbitrary [`TableSlot`] at the next (lower) realization priority.
    pub fn with_slot(mut self, slot: TableSlot) -> Self {
        self.slots.push(slot);
        self
    }

    #[inline]
    pub fn primary_slot(&self, tables: TableSet) -> TableSlot {
        self.slots
            .iter()
            .copied()
            .find(|&slot| tables.contains(slot))
            .unwrap_or(TableSlot::NONE)
    }
}

/// Memory and live-count snapshot of a [`crate::NodeStore`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoreStats {
    pub live_nodes: usize,
    pub node_capacity: usize,
    pub children_live: usize,
    pub children_capacity: usize,
    pub text_live: usize,
    pub text_capacity: usize,
    pub resources_live: usize,
    pub table_count: usize,
}

/// One table's storage exactly as published in `NveTableView`.
#[derive(Debug, Clone, Copy)]
pub struct TableStorage<'a> {
    /// `row_count * words` row words (dense: indexed by node index).
    pub rows: &'a [Word],
    /// Sparse: raw owner `NodeId` per row. Dense: empty (row = node index, at most `capacity()`).
    pub owners: &'a [NodeId],
    pub row_count: usize,
    /// Sparse: page per `NVE_ROW_INDEX_PAGE_LEN` node indices (`NVE_ROW_INDEX_NO_PAGE` = none).
    pub row_index_dir: &'a [u32],
    /// Sparse: flattened pages of `row + 1` per node index (`0` = no row).
    pub row_index_pages: &'a [u32],
}
