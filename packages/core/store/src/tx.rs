//! [`TxCore`] (every mutation), [`Transaction`] (adds `commit_into`, which closes the frame) and
//! validated [`NodeMut`]: the only way to mutate a [`NodeStore`].

use std::ops::{Deref, DerefMut};

mod children;

use nve_abi::table::effect;

use crate::changes::Command;
use crate::id::{NodeId, NodeIndex, ResourceId};
use crate::node_mut::{NodeMut, TempResources};
use crate::resource::FlatResource;
use crate::row::{DirtyList, RowWriter};
use crate::row_patch::RowPatchView;
use crate::span_pool::StrSpan;
use crate::store::{NodeRow, NodeStore, CREATED, IN_FREE, LIVE, PENDING_FREE};
use crate::table::{NveTable, Prop, ResProp, ScalarField, StrProp, TableSlot};

/// Non-committing mutation handle: every node, table, and resource edit of a frame. Reached through
/// a [`Transaction`] (via `Deref`) or [`TempResources::tx`], which must not close the frame while
/// temporary resource handles are still retained.
pub struct TxCore<'s> {
    pub(crate) store: &'s mut NodeStore,
}

/// Mutable frame scope over a [`NodeStore`]: [`TxCore`] plus `commit_into`.
pub struct Transaction<'s> {
    pub(crate) core: TxCore<'s>,
}

impl<'s> Deref for Transaction<'s> {
    type Target = TxCore<'s>;

    #[inline(always)]
    fn deref(&self) -> &TxCore<'s> {
        &self.core
    }
}

impl DerefMut for Transaction<'_> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl NodeStore {
    /// Opens a mutation scope. Changes accumulate across transactions until `commit_into`.
    #[inline]
    pub fn tx(&mut self) -> Transaction<'_> {
        Transaction {
            core: TxCore { store: self },
        }
    }
}

#[inline(always)]
pub(crate) fn touch(node: &mut NodeRow, touched: &mut DirtyList, index: NodeIndex, fx: u8) {
    let had = node.effects;
    node.effects = had | fx;
    touched.push_if(index, (had == 0) & (fx != 0));
}

/// Typed writer resolving `T::slot()` and ensuring its column once before mutating words.
#[inline]
pub(crate) fn writer_at_typed<T: NveTable>(
    store: &mut NodeStore,
    index: NodeIndex,
) -> RowWriter<'_> {
    let slot = T::slot();
    store.column_mut_typed::<T>(slot);
    writer_at_existing(store, index, slot)
}

/// Internal writer taking an already-validated `NodeIndex` (zero redundant liveness checks).
#[inline]
fn writer_at(store: &mut NodeStore, index: NodeIndex, slot: TableSlot) -> RowWriter<'_> {
    store.column_mut(slot);
    writer_at_existing(store, index, slot)
}

#[inline]
fn writer_at_existing(store: &mut NodeStore, index: NodeIndex, slot: TableSlot) -> RowWriter<'_> {
    let NodeStore {
        nodes,
        columns,
        text,
        resources,
        touched,
        realization_mask,
        ..
    } = store;
    let node = &mut nodes[index];
    let col = columns[slot.0 as usize]
        .as_mut()
        .expect("column created above");
    if !node.tables.contains(slot) {
        let id = NodeId::from_index(index, node.generation);
        col.attach(id, index);
        node.tables.insert(slot);
        let realize = if realization_mask.contains(slot) {
            effect::REALIZE
        } else {
            0
        };
        touch(node, touched, index, realize | col.desc.effects());
    }
    let row = col.row_of(index);
    let w = col.w;
    RowWriter {
        row: &mut col.rows[row * w..(row + 1) * w],
        mask: &mut col.masks[row],
        dirty: &mut col.dirty,
        node_index: index,
        node,
        touched,
        desc: &col.desc,
        text,
        resources,
    }
}

/// Fallible internal row writer for wire decoding: uses `try_reserve` + `Column::try_attach`
/// against `SessionBudget`, returning `WireError::Budget` on slot-budget overrun or
/// `WireError::Alloc` on transient OOM instead of aborting.
#[inline]
fn try_writer_at_existing<'a>(
    store: &'a mut NodeStore,
    index: NodeIndex,
    slot: TableSlot,
    budget: &mut crate::wire::budget::SessionBudget,
) -> Result<RowWriter<'a>, crate::wire::WireError> {
    let NodeStore {
        nodes,
        columns,
        text,
        resources,
        touched,
        realization_mask,
        ..
    } = store;
    let node = &mut nodes[index];
    let col = columns[slot.0 as usize]
        .as_mut()
        .ok_or(crate::wire::WireError::UndeclaredTable(slot.0))?;
    touched.try_reserve(usize::from(node.effects == 0))?;
    let row = if !node.tables.contains(slot) {
        let id = NodeId::from_index(index, node.generation);
        let r = col.try_attach(id, index, budget)?;
        node.tables.insert(slot);
        let realize = if realization_mask.contains(slot) {
            effect::REALIZE
        } else {
            0
        };
        touch(node, touched, index, realize | col.desc.effects());
        r
    } else {
        let r = col.row_of(index);
        col.dirty.try_reserve(usize::from(col.masks[r] == 0))?;
        r
    };
    let w = col.w;
    Ok(RowWriter {
        row: &mut col.rows[row * w..(row + 1) * w],
        mask: &mut col.masks[row],
        dirty: &mut col.dirty,
        node_index: index,
        node,
        touched,
        desc: &col.desc,
        text,
        resources,
    })
}

impl TxCore<'_> {
    /// Read access to the store mid-transaction.
    #[inline]
    pub fn store(&self) -> &NodeStore {
        self.store
    }

    /// Allocates a node (detached, no tables).
    pub fn create(&mut self) -> NodeId {
        let index = self.alloc_slot();
        NodeId::from_index(index, self.store.nodes[index].generation)
    }

    /// Allocates a node and returns a validated [`NodeMut`] handle for chained zero-check edits.
    #[inline]
    pub fn spawn(&mut self) -> NodeMut<'_> {
        let index = self.alloc_slot();
        NodeMut::new(self.store, index)
    }

    /// Validates `id` once and returns a [`NodeMut`] handle (`None` if dead/stale).
    #[inline(always)]
    pub fn node_mut(&mut self, id: NodeId) -> Option<NodeMut<'_>> {
        let index = id.node_index();
        let r = self.store.nodes.get(index)?;
        if r.flags & LIVE != 0 && r.generation == id.generation() {
            Some(NodeMut::new(self.store, index))
        } else {
            None
        }
    }

    fn alloc_slot(&mut self) -> NodeIndex {
        let s = &mut *self.store;
        let mut reused = None;
        while let Some(i) = s.free.pop_front() {
            if s.nodes[i].flags & IN_FREE != 0 {
                reused = Some(i);
                break;
            }
        }
        let index = match reused {
            Some(i) => i,
            None => {
                let raw = s.nodes.len() as u32;
                assert!(
                    raw <= NodeId::MAX_INDEX,
                    "NodeStore: node index space exhausted"
                );
                s.nodes.push(NodeRow::default())
            }
        };
        let row = &mut s.nodes[index];
        row.flags = LIVE | CREATED;
        row.primary_slot = TableSlot::NONE;
        let id = NodeId::from_index(index, row.generation);
        s.created.push(id);
        index
    }

    /// Creates `id` exactly (replica stores applying a wire frame). Returns `false` if the slot
    /// is occupied by a live node. O(1) — clears `IN_FREE`/`PENDING_FREE` flags without scanning.
    pub fn create_at(&mut self, id: NodeId) -> bool {
        self.store.try_create_at(id, None).unwrap_or(false)
    }

    /// Removes `id` and its whole subtree, detaching it from its parent (zero heap allocations
    /// once `scratch_stack` is warm). Records `removed` in postorder (descendants before ancestors).
    pub fn remove(&mut self, id: NodeId) {
        if !self.store.is_live(id) {
            return;
        }
        self.detach_child(id);
        let removed_start = self.store.removed.len();
        self.store.scratch_stack.clear();
        self.store.scratch_stack.push(id);
        while let Some(n) = self.store.scratch_stack.pop() {
            let row = self.store.nodes[n.node_index()];
            if row.flags & LIVE == 0 || row.generation != n.generation() {
                continue;
            }
            let children_span = row.children;
            let NodeStore {
                children,
                scratch_stack,
                ..
            } = &mut *self.store;
            scratch_stack.extend_from_slice(children.get(children_span));
            self.free_node(n);
        }
        // Reverse this subtree's recorded removals so descendants precede ancestors.
        self.store.removed[removed_start..].reverse();
    }

    fn free_node(&mut self, id: NodeId) {
        let index = id.node_index();
        let row = self.store.nodes[index];
        debug_assert!(
            row.flags & LIVE != 0 && row.generation == id.generation(),
            "free_node called on non-live or stale node {id:?}"
        );
        if row.flags & LIVE == 0 || row.generation != id.generation() {
            return;
        }
        let tables = row.tables;
        for slot in tables.iter_slots() {
            self.release_row(index, slot);
        }
        let s = &mut *self.store;
        let row = s.nodes[index];
        s.children.free(row.children);
        if row.flags & CREATED == 0 {
            s.removed.push(id);
        }
        s.nodes[index] = NodeRow {
            generation: row.generation,
            flags: PENDING_FREE,
            ..NodeRow::default()
        };
        s.pending_free.push(index);
    }

    /// Releases strings/resources referenced by `slot`'s row and drops the row.
    fn release_row(&mut self, index: NodeIndex, slot: TableSlot) {
        let s = &mut *self.store;
        let Some(col) = s.columns[slot.0 as usize].as_mut() else {
            return;
        };
        let Some(row) = col.find_row(index) else {
            return;
        };
        let (str_words, res_words) = (col.desc.str_words(), col.desc.resource_words());
        let words = &mut col.rows[row * col.w..(row + 1) * col.w];
        let mut m = str_words;
        while m != 0 {
            let w = m.trailing_zeros() as usize;
            m &= m - 1;
            s.text.free(StrSpan::new(words[w], words[w + 1]));
            words[w] = 0;
            words[w + 1] = 0;
        }
        let mut m = res_words;
        while m != 0 {
            let w = m.trailing_zeros() as usize;
            m &= m - 1;
            s.resources.release(ResourceId(words[w]));
            words[w] = 0;
        }
        if col.is_dense() {
            col.masks[row] = 0;
        }
        col.remove_row(index);
    }

    // -----------------------------------------------------------------------------------------
    // Tables
    // -----------------------------------------------------------------------------------------

    /// Fallible crate-private row writer for `slot` on `id`, attaching the table (defaults) when
    /// absent via `try_attach` charged to `budget`. Sealed against raw scalar overwrites.
    pub(crate) fn try_writer<'a>(
        &'a mut self,
        id: NodeId,
        slot: TableSlot,
        budget: &mut crate::wire::budget::SessionBudget,
    ) -> Result<RowWriter<'a>, crate::wire::WireError> {
        if !self.store.is_live(id) {
            return Err(crate::wire::WireError::DeadNode(id.raw()));
        }
        self.store.column_mut(slot);
        try_writer_at_existing(self.store, id.node_index(), slot, budget)
    }

    /// Fluent typed writer for `T` on `id` (attaches `T` with defaults when absent).
    #[inline]
    pub fn edit<T: NveTable>(&mut self, id: NodeId) -> T::Mut<'_> {
        assert!(
            self.store.is_live(id),
            "NodeStore: write to dead node {id:?}"
        );
        T::from_row_writer(writer_at_typed::<T>(self.store, id.node_index()))
    }

    /// Attaches `T` with its defaults (no-op if attached).
    #[inline]
    pub fn attach<T: NveTable>(&mut self, id: NodeId) {
        assert!(
            self.store.is_live(id),
            "NodeStore: write to dead node {id:?}"
        );
        writer_at_typed::<T>(self.store, id.node_index());
    }

    /// Sets one scalar or wide-optional property.
    #[inline(always)]
    pub fn set<T: NveTable, V: ScalarField>(&mut self, id: NodeId, prop: Prop<T, V>, value: V) {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        let mut w = writer_at_typed::<T>(self.store, id.node_index());
        value.write_row(
            &mut w,
            prop.word as usize,
            prop.shift as u32,
            prop.mask,
            prop.val_word as usize,
        );
        if prop.explicit_bit != u8::MAX {
            w.set_explicit_bit(prop.explicit_bit as u32, prop.field_effects);
        }
    }

    /// Sets one string property.
    #[inline]
    pub fn set_str<T: NveTable>(&mut self, id: NodeId, prop: StrProp<T>, text: &str) {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        let mut w = writer_at_typed::<T>(self.store, id.node_index());
        w.put_str(prop.word as usize, text);
        if prop.explicit_bit != u8::MAX {
            w.set_explicit_bit(prop.explicit_bit as u32, prop.field_effects);
        }
    }

    /// Opens a leak-free temporary resource scope for building nested resources (e.g.
    /// `ResponsiveRules` / `StateRules` embedding child `BackgroundFillRes` / `GridTracks`).
    #[inline]
    pub fn with_temp_resources<R>(&mut self, f: impl FnOnce(&mut TempResources<'_>) -> R) -> R {
        let mut scope = TempResources::new(self.store);
        f(&mut scope)
    }

    /// Sets (interned) or clears one resource property.
    #[inline]
    pub fn set_resource<T: NveTable, R: FlatResource>(
        &mut self,
        id: NodeId,
        prop: ResProp<T, R>,
        value: Option<R>,
    ) {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        let mut w = writer_at_typed::<T>(self.store, id.node_index());
        w.put_resource(prop.word as usize, value.as_ref());
        if prop.explicit_bit != u8::MAX {
            w.set_explicit_bit(prop.explicit_bit as u32, prop.field_effects);
        }
    }

    /// Points one resource property at a live [`ResourceId`] of type `R` (or `ResourceId::NONE`).
    /// Returns `false` without mutating if `rid` is stale, dead, or of a mismatched kind/type.
    #[inline]
    pub fn set_resource_id<T: NveTable, R: FlatResource>(
        &mut self,
        id: NodeId,
        prop: ResProp<T, R>,
        rid: ResourceId,
    ) -> bool {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        // Reject before attaching `T` so an invalid id never realizes the row.
        if !rid.is_none() && !self.store.resources.is_live_of::<R>(rid) {
            return false;
        }
        let mut w = writer_at_typed::<T>(self.store, id.node_index());
        if !w.put_resource_id::<R>(prop.word as usize, rid) {
            return false;
        }
        if prop.explicit_bit != u8::MAX {
            w.set_explicit_bit(prop.explicit_bit as u32, prop.field_effects);
        }
        true
    }

    /// Applies every valid table operation in `patch` onto live node `id`, attaching a touched
    /// table only when at least one op passes word, liveness, and resource-kind checks.
    pub fn apply_row_patch(&mut self, id: NodeId, patch: RowPatchView<'_>) {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        for table in patch.tables() {
            let Some(slot) = crate::table::slot_of(table.key()) else {
                continue;
            };
            let desc = crate::table::desc(slot);
            let words_len = desc.words();
            let str_mask = desc.str_words | (desc.str_words << 1);
            let res_mask = desc.resource_words;
            let has_valid = table.ops().any(|op| {
                let wi = op.word() as usize;
                let bit = 1u32 << (wi & 31);
                if wi >= words_len || (str_mask & bit) != 0 || op.is_resource() != ((res_mask & bit) != 0) {
                    return false;
                }
                if !op.is_resource() {
                    return true;
                }
                let rid = ResourceId(op.set_bits());
                rid.is_none()
                    || desc
                        .resource_kind(wi)
                        .is_some_and(|k| self.store.resources.is_live_with_kind(rid, k))
            });
            if !has_valid {
                continue;
            }
            let mut w = writer_at(self.store, id.node_index(), slot);
            let inherited = desc.inherited;
            for op in table.ops() {
                let wi = op.word() as usize;
                let bit = 1u32 << (wi & 31);
                if wi >= words_len || (str_mask & bit) != 0 || op.is_resource() != ((res_mask & bit) != 0) {
                    continue;
                }
                if op.is_resource() {
                    let rid = ResourceId(op.set_bits());
                    if rid.is_none()
                        || desc
                            .resource_kind(wi)
                            .is_some_and(|k| w.resources.is_live_with_kind(rid, k))
                    {
                        w.put_resource_id_wire(wi, rid);
                    }
                } else if wi == 0 && inherited {
                    let merged = op.clear_mask().apply(w.words()[0], op.set_bits());
                    w.put_explicit_word(merged);
                } else {
                    w.put(wi, 0, op.clear_mask().raw(), op.set_bits());
                }
            }
        }
    }

    /// Detaches table `T` from `id`, releasing its strings and resources.
    #[inline]
    pub fn detach<T: NveTable>(&mut self, id: NodeId) {
        self.detach_slot(id, T::slot());
    }

    pub fn detach_slot(&mut self, id: NodeId, slot: TableSlot) {
        if !self.store.has_slot(id, slot) {
            return;
        }
        self.detach_live_slot(id, slot);
    }

    /// Fallible crate-private detach for [`crate::wire::WireDecoder`], reserving `touched` and
    /// `col.detached` before mutating state so hostile `Op::Detach` frames never OOM-abort.
    pub(crate) fn try_detach_slot(
        &mut self,
        id: NodeId,
        slot: TableSlot,
    ) -> Result<(), crate::wire::WireError> {
        if !self.store.has_slot(id, slot) {
            return Ok(());
        }
        let index = id.node_index();
        let node = &self.store.nodes[index];
        let created = node.flags & CREATED != 0;
        let need_touch = usize::from(node.effects == 0);
        self.store.touched.try_reserve(need_touch)?;
        if !created && let Some(col) = self.store.columns[slot.0 as usize].as_mut() {
            col.detached
                .try_reserve(1)
                .map_err(|_| crate::wire::WireError::Alloc)?;
        }
        self.detach_live_slot(id, slot);
        Ok(())
    }

    fn detach_live_slot(&mut self, id: NodeId, slot: TableSlot) {
        let index = id.node_index();
        self.release_row(index, slot);
        let s = &mut *self.store;
        let fx = s.columns[slot.0 as usize]
            .as_ref()
            .map_or(0, |c| c.desc.effects());
        let node = &mut s.nodes[index];
        node.tables.remove(slot);
        let created = node.flags & CREATED != 0;
        let realize = if s.realization_mask.contains(slot) {
            effect::REALIZE
        } else {
            0
        };
        touch(node, &mut s.touched, index, realize | fx);
        if !created && let Some(col) = s.columns[slot.0 as usize].as_mut() {
            col.record_detached(id);
        }
    }

    /// Enqueues an imperative command targeting `T` on `id`.
    #[inline]
    pub fn command<T: NveTable>(&mut self, id: NodeId, op: u32, payload: &[u8]) {
        self.command_slot(id, T::slot(), op, payload);
    }

    pub fn command_slot(&mut self, id: NodeId, table: TableSlot, op: u32, payload: &[u8]) {
        let s = &mut *self.store;
        let off = s.command_bytes.len() as u32;
        s.command_bytes.extend_from_slice(payload);
        s.commands.push(Command {
            node: id,
            table,
            op,
            off,
            len: payload.len() as u32,
        });
    }

    pub(crate) fn store_mut(&mut self) -> &mut NodeStore {
        self.store
    }
}
