//! `NodeStore`: the single owner of every node, table row, child list, string, and resource.

use std::collections::VecDeque;
use std::sync::Arc;

use nve_abi::table::{TableDesc, Word, MAX_TABLE_SLOTS};

use crate::column::Column;
use crate::id::{IndexVec, NodeId, NodeIndex, ResourceId, TableSet};
use crate::resource::{Resource, ResourceStore};
use crate::row::{DirtyList, RowRef};
use crate::span_pool::{ChildSpan, SpanPool, StrArena, StrSpan};
use crate::table::{self, NveTable, OwnedTableDesc, TableMeta, TableSlot};

/// Node is allocated (exported as `nve_abi::NVE_NODE_ROW_FLAG_LIVE`).
pub const LIVE: u8 = 1 << 0;

const _: () = {
    assert!(LIVE == nve_abi::NVE_NODE_ROW_FLAG_LIVE);
};
/// Node was created during the current frame.
pub(crate) const CREATED: u8 = 1 << 1;
/// Node's child list changed during the current frame (already queued).
pub(crate) const CHILDREN_DIRTY: u8 = 1 << 2;
/// Node slot is queued in `pending_free` for end-of-frame recycling.
pub(crate) const PENDING_FREE: u8 = 1 << 3;
/// Node slot is currently queued in the FIFO `free` list.
pub(crate) const IN_FREE: u8 = 1 << 4;
/// Node slot reached generation `255` and is permanently retired (prevents ABA wraparound).
pub(crate) const RETIRED: u8 = 1 << 5;
/// Scratch flag used during `try_set_children` for O(K) zero-alloc duplicate detection.
pub(crate) const CHILD_SEEN: u8 = 1 << 6;
/// Scratch flag used during `try_set_children` for O(K + depth) zero-alloc cycle detection.
pub(crate) const ANCESTOR_MARK: u8 = 1 << 7;

/// Per-node hot record: exactly 32 bytes, 32-byte aligned (two per cache line, never straddling).
#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeRow {
    /// Bit `slot` set: table `slot` is attached.
    pub tables: TableSet,
    /// Parent `NodeId` (`NodeId::NULL` = detached / root).
    pub parent: NodeId,
    /// Ordered child list span in `NodeStore::children`.
    pub children: ChildSpan,
    pub generation: u8,
    /// Highest-priority realizing [`TableSlot`] (`TableSlot::NONE` = plain container), see [`RealizationOrder`].
    pub primary_slot: TableSlot,
    /// Union of `nve_abi::table::effect` bits changed this frame.
    pub effects: u8,
    pub(crate) flags: u8,
}

// Complete compile-time proof that `NodeRow` and `nve_abi::NveNodeRow` are bit-for-bit identical
// in size, alignment compatibility, little-endian `[u32; 4]` bitset encoding, and EVERY field
// offset (including `pub(crate)` `flags` and `ChildSpan::{off, len}`) for zero-copy C FFI export.
const _: () = {
    use core::mem::{align_of, offset_of, size_of};
    use nve_abi::NveNodeRow;
    assert!(cfg!(target_endian = "little"));
    assert!(size_of::<NodeRow>() == 32);
    assert!(align_of::<NodeRow>() == 32);
    assert!(size_of::<NodeRow>() == size_of::<NveNodeRow>());
    assert!(align_of::<NodeRow>() >= align_of::<NveNodeRow>());
    assert!(size_of::<TableSet>() == size_of::<[u32; 4]>());
    assert!(offset_of!(NodeRow, tables) == offset_of!(NveNodeRow, table_set));
    assert!(offset_of!(NodeRow, parent) == offset_of!(NveNodeRow, parent));
    assert!(
        offset_of!(NodeRow, children) + offset_of!(ChildSpan, off)
            == offset_of!(NveNodeRow, child_off)
    );
    assert!(
        offset_of!(NodeRow, children) + offset_of!(ChildSpan, len)
            == offset_of!(NveNodeRow, child_len)
    );
    assert!(offset_of!(NodeRow, generation) == offset_of!(NveNodeRow, generation));
    assert!(offset_of!(NodeRow, primary_slot) == offset_of!(NveNodeRow, primary_slot));
    assert!(offset_of!(NodeRow, effects) == offset_of!(NveNodeRow, effects));
    assert!(offset_of!(NodeRow, flags) == offset_of!(NveNodeRow, flags));
    assert!(LIVE == nve_abi::NVE_NODE_ROW_FLAG_LIVE);
};

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

/// Memory and live-count snapshot of a [`NodeStore`].
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

/// Zero-cost validated read handle to a live node (`Copy`, 2 registers: `&NodeStore` + `NodeIndex`).
///
/// Created via [`NodeStore::get`], which validates bounds, `LIVE`, and generation once; every
/// subsequent read on the handle skips repeated liveness/generation checks.
#[derive(Clone, Copy)]
pub struct NodeRef<'a> {
    store: &'a NodeStore,
    index: NodeIndex,
}

impl<'a> NodeRef<'a> {
    #[inline(always)]
    pub(crate) const fn new_unchecked(store: &'a NodeStore, index: NodeIndex) -> Self {
        Self { store, index }
    }

    #[inline(always)]
    pub fn raw_row(self) -> &'a NodeRow {
        &self.store.nodes[self.index]
    }

    #[inline(always)]
    pub fn index(self) -> NodeIndex {
        self.index
    }

    #[inline(always)]
    pub fn id(self) -> NodeId {
        NodeId::from_index(self.index, self.raw_row().generation)
    }

    #[inline(always)]
    pub fn tables(self) -> TableSet {
        self.raw_row().tables
    }

    #[inline(always)]
    pub fn primary_slot(self) -> TableSlot {
        self.raw_row().primary_slot
    }

    #[inline(always)]
    pub fn parent(self) -> Option<NodeId> {
        let p = self.raw_row().parent;
        (!p.is_null()).then_some(p)
    }

    #[inline(always)]
    pub fn children(self) -> &'a [NodeId] {
        self.store.children.get(self.raw_row().children)
    }

    #[inline(always)]
    pub fn has<T: NveTable>(self) -> bool {
        self.raw_row().tables.contains(T::slot())
    }

    #[inline(always)]
    pub fn has_slot(self, slot: TableSlot) -> bool {
        self.raw_row().tables.contains(slot)
    }

    #[inline(always)]
    pub fn words(self, slot: TableSlot) -> Option<&'a [Word]> {
        if !self.has_slot(slot) {
            return None;
        }
        let col = self.store.columns[slot.0 as usize].as_ref()?;
        Some(col.words(col.row_of(self.index)))
    }

    #[inline(always)]
    fn words_for<T: NveTable>(self) -> Option<&'a [Word]> {
        let slot = T::slot();
        let w = self.words(slot)?;
        debug_assert_eq!(
            self.store.columns[slot.0 as usize].as_ref().map(|c| c.desc.key()),
            Some(T::DESC.key),
            "table slot {} key mismatch for `{}`",
            slot.0,
            T::DESC.name
        );
        Some(w)
    }

    #[inline(always)]
    pub fn try_row<T: NveTable>(self) -> Option<T::Ref<'a>> {
        self.words_for::<T>()
            .map(|w| T::from_row_ref(RowRef::new(w, &self.store.text, T::DESC.str_words)))
    }

    #[inline(always)]
    pub fn row<T: NveTable>(self) -> T::Ref<'a> {
        let w = self.words_for::<T>().unwrap_or(T::DESC.defaults);
        T::from_row_ref(RowRef::new(w, &self.store.text, T::DESC.str_words))
    }

    /// Checked UTF-8 lookup for a detached [`StrSpan`] (`None` if stale or invalid UTF-8).
    #[inline]
    pub fn try_str(self, span: StrSpan) -> Option<&'a str> {
        self.store.text.try_str(span)
    }

    /// Checked UTF-8 lookup for a detached [`StrSpan`] (returns `""` if stale).
    #[inline]
    pub fn str(self, span: StrSpan) -> &'a str {
        self.store.text.str(span)
    }

    #[inline(always)]
    pub fn resource<R: Resource>(self, id: ResourceId) -> Option<&'a R> {
        self.store.resources.get(id)
    }
}

/// The single owner of every node, table row, child list, string, and resource.
pub struct NodeStore {
    pub(crate) nodes: IndexVec<NodeIndex, NodeRow>,
    pub(crate) columns: Vec<Option<Column>>,
    pub(crate) children: SpanPool<NodeId>,
    pub(crate) text: StrArena,
    pub(crate) resources: ResourceStore,
    pub(crate) free: VecDeque<NodeIndex>,
    pub(crate) pending_free: Vec<NodeIndex>,
    pub(crate) touched: DirtyList,
    pub(crate) created: Vec<NodeId>,
    pub(crate) removed: Vec<NodeId>,
    pub(crate) reparented: DirtyList,
    pub(crate) commands: Vec<crate::changes::Command>,
    pub(crate) command_bytes: Vec<u8>,
    pub(crate) scratch_stack: Vec<NodeId>,
    pub(crate) realization: RealizationOrder,
    pub(crate) realization_mask: TableSet,
    pub(crate) frame: u64,
}

impl Default for NodeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeStore {
    /// Creates a store holding only the root node ([`NodeId::ROOT`]).
    pub fn new() -> Self {
        let mut columns = Vec::with_capacity(MAX_TABLE_SLOTS);
        columns.resize_with(MAX_TABLE_SLOTS, || None);
        let root = NodeRow {
            generation: 0,
            primary_slot: TableSlot::NONE,
            flags: LIVE | CREATED,
            ..NodeRow::default()
        };
        Self {
            nodes: IndexVec::from_vec(vec![NodeRow::default(), root]),
            columns,
            children: SpanPool::default(),
            text: StrArena::default(),
            resources: ResourceStore::default(),
            free: VecDeque::new(),
            pending_free: Vec::new(),
            touched: DirtyList::default(),
            created: vec![NodeId::ROOT],
            removed: Vec::new(),
            reparented: DirtyList::default(),
            commands: Vec::new(),
            command_bytes: Vec::new(),
            scratch_stack: Vec::new(),
            realization: RealizationOrder::default(),
            realization_mask: TableSet::EMPTY,
            frame: 0,
        }
    }

    /// Creates an empty replica store (no root): every node, including the root, arrives through
    /// `wire::WireDecoder::apply`.
    pub fn replica() -> Self {
        let mut s = Self::new();
        s.nodes.truncate(1);
        s.created.clear();
        s
    }

    /// Installs the backend's ordered realization priority list used to derive
    /// `NodeRow::primary_slot` at commit.
    pub fn set_realization_order(&mut self, order: RealizationOrder) {
        self.realization_mask = TableSet(order.slots.iter().fold(0u128, |m, &s| m | s.bit()));
        self.realization = order;
        let n = self.nodes.len();
        for i in 1..n {
            let row = &mut self.nodes[NodeIndex(i as u32)];
            if row.flags & LIVE != 0 {
                row.primary_slot = self.realization.primary_slot(row.tables);
            }
        }
    }

    /// Frames committed so far.
    #[inline]
    pub fn frame(&self) -> u64 {
        self.frame
    }

    #[inline(always)]
    pub fn is_live(&self, id: NodeId) -> bool {
        self.nodes
            .get(id.node_index())
            .is_some_and(|r| r.flags & LIVE != 0 && r.generation == id.generation())
    }

    /// Validates `id` once (bounds + `LIVE` + generation) and returns a zero-cost [`NodeRef`].
    #[inline(always)]
    pub fn get(&self, id: NodeId) -> Option<NodeRef<'_>> {
        let idx = id.node_index();
        let r = self.nodes.get(idx)?;
        if r.flags & LIVE != 0 && r.generation == id.generation() {
            Some(NodeRef::new_unchecked(self, idx))
        } else {
            None
        }
    }

    /// Hot record of a live node.
    #[inline(always)]
    pub fn node(&self, id: NodeId) -> Option<&NodeRow> {
        self.nodes
            .get(id.node_index())
            .filter(|r| r.flags & LIVE != 0 && r.generation == id.generation())
    }

    /// Live `NodeId` currently occupying `index` (`None` if free).
    #[inline]
    pub fn id_at(&self, index: usize) -> Option<NodeId> {
        let idx = NodeIndex(index as u32);
        let r = self.nodes.get(idx)?;
        (r.flags & LIVE != 0).then(|| NodeId::from_index(idx, r.generation))
    }

    /// Number of node slots (including the null slot `0` and free slots).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.nodes.len()
    }

    /// Dense contiguous slice of all [`NodeRow`] slots (slot 0 is reserved null; live nodes live
    /// in `1..capacity()` with `flags & LIVE != 0`). Published zero-copy in `NveRuntimeSceneView`.
    #[inline]
    pub fn node_rows(&self) -> &[NodeRow] {
        self.nodes.as_slice()
    }

    /// Iterates every live node id.
    pub fn live_nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        (1..self.nodes.len()).filter_map(|i| self.id_at(i))
    }

    #[inline]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.get(id).and_then(NodeRef::parent)
    }

    #[inline]
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.get(id).map_or(&[], NodeRef::children)
    }

    #[inline]
    pub fn primary_slot(&self, id: NodeId) -> TableSlot {
        self.get(id)
            .map_or(TableSlot::NONE, NodeRef::primary_slot)
    }

    #[inline(always)]
    pub fn has<T: NveTable>(&self, id: NodeId) -> bool {
        self.get(id).is_some_and(NodeRef::has::<T>)
    }

    #[inline(always)]
    pub fn has_slot(&self, id: NodeId, slot: TableSlot) -> bool {
        self.get(id).is_some_and(|n| n.has_slot(slot))
    }

    /// Raw words of `slot`'s row on `id`, if attached.
    #[inline]
    pub fn words(&self, id: NodeId, slot: TableSlot) -> Option<&[Word]> {
        self.get(id)?.words(slot)
    }

    /// Typed row of `T` on `id`, if attached.
    #[inline]
    pub fn try_row<T: NveTable>(&self, id: NodeId) -> Option<T::Ref<'_>> {
        self.get(id)?.try_row::<T>()
    }

    /// Typed row of `T` on `id`, or the table defaults when not attached.
    #[inline]
    pub fn row<T: NveTable>(&self, id: NodeId) -> T::Ref<'_> {
        self.get(id).map_or_else(
            || T::from_row_ref(RowRef::new(T::DESC.defaults, &self.text, T::DESC.str_words)),
            NodeRef::row::<T>,
        )
    }

    /// Checked UTF-8 text lookup for a detached [`StrSpan`] (`None` if stale or invalid UTF-8).
    #[inline]
    pub fn try_text(&self, span: StrSpan) -> Option<&str> {
        self.text.try_str(span)
    }

    /// Checked UTF-8 text lookup for a detached [`StrSpan`] (prefer direct `row.<field>()` getters
    /// for O(1) unchecked `&str` reads tied to the store borrow).
    #[inline]
    pub fn text(&self, span: StrSpan) -> &str {
        self.text.str(span)
    }

    /// The contiguous UTF-8 text slab (published as `NveSceneView::text_slab`).
    #[inline]
    pub fn text_slab(&self) -> &[u8] {
        self.text.data()
    }

    /// The contiguous child id slab (published as `NveSceneView::children_slab`).
    #[inline]
    pub fn children_slab(&self) -> &[NodeId] {
        self.children.data()
    }

    #[inline]
    pub fn resources(&self) -> &ResourceStore {
        &self.resources
    }

    #[inline]
    pub fn resource<R: Resource>(&self, id: ResourceId) -> Option<&R> {
        self.resources.get(id)
    }

    /// Descriptor of a compiled/static table slot used in this store.
    #[inline]
    pub fn table_desc(&self, slot: TableSlot) -> Option<&'static TableDesc> {
        self.columns[slot.0 as usize]
            .as_ref()
            .and_then(|c| c.desc.as_static())
    }

    /// Internal metadata for either a static or wire-declared dynamic column in this store.
    #[inline]
    pub(crate) fn table_meta(&self, slot: TableSlot) -> Option<&TableMeta> {
        self.columns[slot.0 as usize].as_ref().map(|c| &c.desc)
    }

    /// Installs a session-scoped dynamic table column strictly in the dynamic slot partition
    /// (`MAX_STATIC_TABLE_SLOTS..MAX_TABLE_SLOTS`), returning `false` on slot-range violation or
    /// schema mismatch against an already-installed column at `slot`.
    pub(crate) fn install_dynamic_table(
        &mut self,
        slot: TableSlot,
        desc: Arc<OwnedTableDesc>,
    ) -> bool {
        let slot_idx = slot.0 as usize;
        if !(table::MAX_STATIC_TABLE_SLOTS..MAX_TABLE_SLOTS).contains(&slot_idx) {
            return false;
        }
        match &self.columns[slot_idx] {
            None => {
                self.columns[slot_idx] = Some(Column::new(TableMeta::Dynamic(desc)));
                true
            }
            Some(col) => matches!(&col.desc, TableMeta::Dynamic(existing) if existing.as_ref() == desc.as_ref()),
        }
    }

    /// Slots of every table that has a column in this store.
    pub fn table_slots(&self) -> impl Iterator<Item = TableSlot> + '_ {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_some())
            .map(|(i, _)| TableSlot(i as u8))
    }

    /// Dense rows (`row_count * words`) and sparse owners of a table, for scene publishing.
    pub fn table_storage(&self, slot: TableSlot) -> Option<(&[Word], &[u32], usize)> {
        let c = self.columns[slot.0 as usize].as_ref()?;
        Some((&c.rows, &c.owners, c.row_count()))
    }

    pub fn stats(&self) -> StoreStats {
        let live_nodes = (1..self.nodes.len())
            .filter(|&i| self.nodes[NodeIndex(i as u32)].flags & LIVE != 0)
            .count();
        StoreStats {
            live_nodes,
            node_capacity: self.nodes.len(),
            children_live: self.children.live(),
            children_capacity: self.children.data().len(),
            text_live: self.text.live(),
            text_capacity: self.text.data().len(),
            resources_live: self.resources.live(),
            table_count: self.columns.iter().filter(|c| c.is_some()).count(),
        }
    }

    pub(crate) fn column_mut(&mut self, slot: TableSlot) -> &mut Column {
        let slot_idx = slot.0 as usize;
        self.columns[slot_idx].get_or_insert_with(|| {
            assert!(
                slot_idx < table::MAX_STATIC_TABLE_SLOTS,
                "cannot lazily allocate static column in dynamic slot {}",
                slot.0
            );
            Column::new(TableMeta::Static(table::desc(slot)))
        })
    }

    #[inline]
    pub(crate) fn column_mut_typed<T: NveTable>(&mut self, slot: TableSlot) -> &mut Column {
        let col = self.column_mut(slot);
        debug_assert_eq!(
            col.desc.key(),
            T::DESC.key,
            "table slot {} key mismatch for `{}`",
            slot.0,
            T::DESC.name
        );
        col
    }
}
