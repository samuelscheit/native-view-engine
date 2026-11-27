//! `NVE2` bytes → replica `NodeStore` mutations (inside a [`Transaction`]).

use std::sync::Arc;

use nve_abi::Key64;
use nve_abi::table::{MAX_TABLE_SLOTS, Storage, table_flags};

use crate::id::{NodeId, ResourceId};
use crate::store::NodeStore;
use crate::table::{self, OwnedTableDesc, TableSlot};
use crate::tx::Transaction;

use super::{HEADER_WORDS, MAGIC, Op, VERSION, WireError, pad4_checked};

/// Maximum length of a runtime-declared table name in `Op::DeclareTables`.
const MAX_TABLE_NAME_BYTES: usize = 128;
/// Base per-frame dense-column allocation allowance so small delta frames can attach a new static
/// dense table to an existing store without failing.
const DENSE_ALLOC_BASE_BYTES: usize = 16 * 1024 * 1024;
/// Multiplier scaling the per-frame dense-column budget with wire frame length.
const DENSE_ALLOC_WIRE_MULTIPLIER: usize = 16;
/// Hard per-frame ceiling on newly allocated dense-column bytes across all tables so a hostile
/// 64 MiB frame cannot amplify `CreateNodes` into multi-GiB dense allocations.
const MAX_FRAME_DENSE_ALLOC_BYTES: usize = 256 * 1024 * 1024;

struct FrameBudget {
    initial_node_cap: usize,
    cumulative_created_nodes: usize,
    initial_res_slots: usize,
    cumulative_defined_resources: usize,
    dense_bytes_used: usize,
    dense_byte_budget: usize,
}

/// Receiver-side session state: wire slot → local slot, session-scoped dynamic table descriptors
/// (never leaked into global `REGISTRY`), and reusable child scratch buffer.
#[derive(Debug, Clone)]
pub struct WireDecoder {
    map: [u8; MAX_TABLE_SLOTS],
    dynamic_descs: Vec<(TableSlot, Arc<OwnedTableDesc>)>,
    next_dynamic_slot: TableSlot,
    dynamic_exhausted: bool,
    kids_scratch: Vec<NodeId>,
}

impl Default for WireDecoder {
    fn default() -> Self {
        Self {
            map: [u8::MAX; MAX_TABLE_SLOTS],
            dynamic_descs: Vec::new(),
            next_dynamic_slot: TableSlot((MAX_TABLE_SLOTS - 1) as u8),
            dynamic_exhausted: false,
            kids_scratch: Vec::new(),
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> R<'a> {
    #[inline(always)]
    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.at)
    }

    #[inline]
    fn u32(&mut self) -> Result<u32, WireError> {
        let end = self.at.checked_add(4).ok_or(WireError::Truncated)?;
        let s = self.b.get(self.at..end).ok_or(WireError::Truncated)?;
        self.at = end;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let end = self.at.checked_add(len).ok_or(WireError::Truncated)?;
        let next = pad4_checked(end).ok_or(WireError::Truncated)?;
        if next > self.b.len() {
            return Err(WireError::Truncated);
        }
        let s = &self.b[self.at..end];
        self.at = next;
        Ok(s)
    }

    fn node(&mut self) -> Result<NodeId, WireError> {
        self.u32().map(NodeId)
    }
}

impl WireDecoder {
    pub fn reset(&mut self) {
        self.map = [u8::MAX; MAX_TABLE_SLOTS];
        self.dynamic_descs.clear();
        self.next_dynamic_slot = TableSlot((MAX_TABLE_SLOTS - 1) as u8);
        self.dynamic_exhausted = false;
        self.kids_scratch.clear();
    }

    fn slot_layout(store: &NodeStore, slot: TableSlot) -> Option<(usize, u32, u32)> {
        if let Some(m) = store.table_meta(slot) {
            return Some((m.words(), m.str_words(), m.resource_words()));
        }
        table::try_desc(slot).map(|d| (d.words(), d.str_words, d.resource_words))
    }

    fn local(&self, wire: u8) -> Result<TableSlot, WireError> {
        match self.map.get(wire as usize) {
            Some(&s) if s != u8::MAX => Ok(TableSlot(s)),
            _ => Err(WireError::UndeclaredTable(wire)),
        }
    }

    /// Validates the `NVE2` 24-byte header (alignment, magic, version, flags == 0, exact word
    /// length) and returns `(frame, op_count)` without mutating any state.
    pub fn validate_header(bytes: &[u8]) -> Result<(u64, u32), WireError> {
        if !bytes.len().is_multiple_of(4) {
            return Err(WireError::Misaligned);
        }
        let mut r = R { b: bytes, at: 0 };
        let magic = r.u32()?;
        if magic != MAGIC {
            return Err(WireError::BadMagic(magic));
        }
        let ver_flags = r.u32()?;
        let version = ver_flags as u16;
        if version != VERSION {
            return Err(WireError::UnsupportedVersion(version));
        }
        let flags = (ver_flags >> 16) as u16;
        if flags != 0 {
            return Err(WireError::UnsupportedFlags(flags));
        }
        let frame = r.u32()? as u64 | (r.u32()? as u64) << 32;
        let ops = r.u32()?;
        let words = r.u32()? as usize;
        if words.checked_mul(4) != Some(bytes.len()) || r.at != HEADER_WORDS * 4 {
            return Err(WireError::Truncated);
        }
        Ok((frame, ops))
    }

    /// Applies one `NVE2` frame to the replica behind `tx`; returns the frame number.
    pub fn apply(&mut self, tx: &mut Transaction<'_>, bytes: &[u8]) -> Result<u64, WireError> {
        let (frame, ops) = Self::validate_header(bytes)?;
        // Cap created node and resource indices strictly against `initial + 1024 + cumulative`
        // (incremented only by `Op::CreateNodes` / `Op::DefineResources`, NEVER derived from total
        // `bytes.len()`) so padding ops like `Op::ReleaseResources` cannot amplify allocation bounds.
        let mut budget = FrameBudget {
            initial_node_cap: tx.store().capacity(),
            cumulative_created_nodes: 0,
            initial_res_slots: tx.store().resources.slots(),
            cumulative_defined_resources: 0,
            dense_bytes_used: 0,
            dense_byte_budget: bytes
                .len()
                .saturating_mul(DENSE_ALLOC_WIRE_MULTIPLIER)
                .saturating_add(DENSE_ALLOC_BASE_BYTES)
                .min(MAX_FRAME_DENSE_ALLOC_BYTES),
        };
        let mut r = R {
            b: bytes,
            at: HEADER_WORDS * 4,
        };
        for _ in 0..ops {
            let head = r.u32()?;
            let len = (r.u32()? as usize)
                .checked_mul(4)
                .ok_or(WireError::Truncated)?;
            let end = r.at.checked_add(len).ok_or(WireError::Truncated)?;
            let body = bytes.get(r.at..end).ok_or(WireError::Truncated)?;
            r.at = end;
            let op = Op::from_u8(head as u8).ok_or(WireError::UnknownOp(head as u8))?;
            let inline_count = head >> 16;
            let mut body_r = R { b: body, at: 0 };
            let count = if inline_count != 0xFFFF {
                inline_count
            } else {
                body_r.u32()?
            };
            self.op(tx, op, (head >> 8) as u8, count, &mut body_r, &mut budget)?;
            if body_r.at != body.len() {
                return Err(WireError::TrailingBytes);
            }
        }
        if r.at != bytes.len() {
            return Err(WireError::TrailingBytes);
        }
        Ok(frame)
    }

    fn op(
        &mut self,
        tx: &mut Transaction<'_>,
        op: Op,
        table: u8,
        count: u32,
        r: &mut R<'_>,
        budget: &mut FrameBudget,
    ) -> Result<(), WireError> {
        match op {
            Op::DeclareTables => {
                // Minimum entry size: 7 header words + >=1 default + >=1 effect word + name_len = 40 bytes.
                if (count as usize) > r.remaining() / 40 {
                    return Err(WireError::Truncated);
                }
                for _ in 0..count {
                    self.declare(tx, r)?;
                }
            }
            Op::CreateNodes => {
                if (count as usize) > r.remaining() / 4 {
                    return Err(WireError::Truncated);
                }
                budget.cumulative_created_nodes = budget
                    .cumulative_created_nodes
                    .saturating_add(count as usize);
                let max_node_index = budget
                    .initial_node_cap
                    .saturating_add(1024)
                    .saturating_add(budget.cumulative_created_nodes)
                    .min(NodeId::MAX_INDEX as usize + 1);
                tx.store_mut()
                    .reserve_for_frame0(count as usize, r.remaining())
                    .map_err(|_| WireError::Alloc)?;
                for _ in 0..count {
                    let id = r.node()?;
                    if id.index() >= max_node_index {
                        return Err(WireError::NodeExists(id.0));
                    }
                    let created = tx
                        .store_mut()
                        .try_create_at(id)
                        .map_err(|_| WireError::Alloc)?;
                    if !created {
                        return Err(WireError::NodeExists(id.0));
                    }
                }
            }
            Op::DefineResources => {
                // Minimum entry size: id + kind_lo + kind_hi + ephemeral + len = 20 bytes.
                if (count as usize) > r.remaining() / 20 {
                    return Err(WireError::Truncated);
                }
                budget.cumulative_defined_resources = budget
                    .cumulative_defined_resources
                    .saturating_add(count as usize);
                let max_resource_id = budget
                    .initial_res_slots
                    .saturating_add(1024)
                    .saturating_add(budget.cumulative_defined_resources);
                for _ in 0..count {
                    let id = r.u32()?;
                    if id as usize >= max_resource_id {
                        return Err(WireError::Resource { id, kind: 0 });
                    }
                    let kind = Key64::from_halves(r.u32()?, r.u32()?);
                    let ephemeral = r.u32()? != 0;
                    let len = r.u32()? as usize;
                    let blob = r.bytes(len)?;
                    let ok = tx
                        .store_mut()
                        .resources
                        .try_define_at(ResourceId(id), kind, ephemeral, blob)
                        .map_err(|_| WireError::Alloc)?;
                    if !ok {
                        return Err(WireError::Resource { id, kind: kind.0 });
                    }
                }
            }
            Op::Patch => {
                if (count as usize) > r.remaining() / 8 {
                    return Err(WireError::Truncated);
                }
                let slot = self.local(table)?;
                if count == 0 {
                    return Ok(());
                }
                let (words_usize, str_words, resource_words) =
                    Self::slot_layout(tx.store(), slot).ok_or(WireError::UndeclaredTable(table))?;
                let store_cap = tx.store().capacity();
                tx.store_mut().reserve_column_bulk(
                    slot,
                    count as usize,
                    store_cap,
                    &mut budget.dense_bytes_used,
                    budget.dense_byte_budget,
                )?;
                let words = words_usize as u32;
                for _ in 0..count {
                    let id = self.live(tx, r.node()?)?;
                    let mask = r.u32()?;
                    if words < 32 && mask >> words != 0 {
                        return Err(WireError::BadMask { mask, words });
                    }
                    if str_words & mask != 0 || (str_words << 1) & mask != 0 {
                        return Err(WireError::BadMask { mask, words });
                    }
                    let mut vals = [0u32; 32];
                    let mut bits = mask;
                    while bits != 0 {
                        let i = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        vals[i] = r.u32()?;
                        let res = ResourceId(vals[i]);
                        if resource_words & (1 << i) != 0
                            && !res.is_none()
                            && !tx.store().resources.is_defined(res)
                        {
                            return Err(WireError::Resource { id: res.0, kind: 0 });
                        }
                    }
                    let mut w = tx.try_writer(id, slot)?;
                    let mut bits = mask;
                    while bits != 0 {
                        let i = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        if resource_words & (1 << i) != 0 {
                            w.put_resource_id(i, ResourceId(vals[i]));
                        } else {
                            w.put(i, 0, u32::MAX, vals[i]);
                        }
                    }
                }
            }
            Op::SetStrings => {
                if (count as usize) > r.remaining() / 8 {
                    return Err(WireError::Truncated);
                }
                let slot = self.local(table)?;
                if count == 0 {
                    return Ok(());
                }
                let (_, str_words, _) =
                    Self::slot_layout(tx.store(), slot).ok_or(WireError::UndeclaredTable(table))?;
                let store_cap = tx.store().capacity();
                tx.store_mut().reserve_column_bulk(
                    slot,
                    count as usize,
                    store_cap,
                    &mut budget.dense_bytes_used,
                    budget.dense_byte_budget,
                )?;
                for _ in 0..count {
                    let id = self.live(tx, r.node()?)?;
                    let word_len = r.u32()?;
                    let word = word_len & 0xFF;
                    let len = (word_len >> 8) as usize;
                    let text =
                        core::str::from_utf8(r.bytes(len)?).map_err(|_| WireError::InvalidUtf8)?;
                    if word >= 32 || str_words & (1 << word) == 0 {
                        return Err(WireError::BadMask {
                            mask: 1 << (word & 31),
                            words: word,
                        });
                    }
                    tx.try_writer(id, slot)?.put_str(word as usize, text);
                }
            }
            Op::Detach => {
                if (count as usize) > r.remaining() / 4 {
                    return Err(WireError::Truncated);
                }
                let slot = self.local(table)?;
                for _ in 0..count {
                    let id = self.live(tx, r.node()?)?;
                    tx.detach_slot(id, slot);
                }
            }
            Op::SetChildren => {
                if (count as usize) > r.remaining() / 8 {
                    return Err(WireError::Truncated);
                }
                for _ in 0..count {
                    let parent = self.live(tx, r.node()?)?;
                    let n = r.u32()? as usize;
                    if n > r.remaining() / 4 {
                        return Err(WireError::Truncated);
                    }
                    self.kids_scratch.clear();
                    if self.kids_scratch.capacity() < n {
                        self.kids_scratch
                            .try_reserve(n)
                            .map_err(|_| WireError::Alloc)?;
                    }
                    for _ in 0..n {
                        self.kids_scratch.push(self.live(tx, r.node()?)?);
                    }
                    if !tx.try_set_children(parent, &self.kids_scratch) {
                        return Err(WireError::InvalidChildren);
                    }
                }
            }
            Op::RemoveNodes => {
                if (count as usize) > r.remaining() / 4 {
                    return Err(WireError::Truncated);
                }
                for _ in 0..count {
                    // Descendants may already be gone through their removed ancestor.
                    let id = r.node()?;
                    tx.remove(id);
                }
            }
            Op::ReleaseResources => {
                if (count as usize) > r.remaining() / 4 {
                    return Err(WireError::Truncated);
                }
                // Refcounts mirror row references, so releases already happened through the row
                // writes / removals above; the list is consumed for framing only.
                for _ in 0..count {
                    r.u32()?;
                }
            }
            Op::Commands => {
                if (count as usize) > r.remaining() / 16 {
                    return Err(WireError::Truncated);
                }
                for _ in 0..count {
                    let id = self.live(tx, r.node()?)?;
                    let raw_slot = r.u32()?;
                    if raw_slot >= MAX_TABLE_SLOTS as u32 {
                        return Err(WireError::UndeclaredTable(u8::MAX));
                    }
                    let slot = self.local(raw_slot as u8)?;
                    let opcode = r.u32()?;
                    let len = r.u32()? as usize;
                    let payload = r.bytes(len)?;
                    tx.command_slot(id, slot, opcode, payload);
                }
            }
        }
        Ok(())
    }

    fn live(&self, tx: &Transaction<'_>, id: NodeId) -> Result<NodeId, WireError> {
        if tx.store().is_live(id) {
            Ok(id)
        } else {
            Err(WireError::DeadNode(id.0))
        }
    }

    fn declare(&mut self, tx: &mut Transaction<'_>, r: &mut R<'_>) -> Result<(), WireError> {
        let key = Key64::from_halves(r.u32()?, r.u32()?);
        let schema_hash = r.u32()? as u64 | (r.u32()? as u64) << 32;
        let packed = r.u32()?;
        let (wire_slot, storage_raw, flags, words) = (
            packed as u8,
            (packed >> 8) as u8,
            (packed >> 16) as u8,
            (packed >> 24) as usize,
        );
        let str_words = r.u32()?;
        let resource_words = r.u32()?;
        if words == 0 || words > 32 || wire_slot as usize >= MAX_TABLE_SLOTS {
            return Err(WireError::BadMask {
                mask: 0,
                words: words as u32,
            });
        }
        let storage = match storage_raw {
            0 => Storage::Dense,
            1 => Storage::Sparse,
            _ => return Err(WireError::SchemaMismatch { key: key.0 }),
        };
        let mut defaults = Vec::with_capacity(words);
        for _ in 0..words {
            defaults.push(r.u32()?);
        }
        let effects = r.bytes(words)?.to_vec();
        let name_len = r.u32()? as usize;
        if name_len > MAX_TABLE_NAME_BYTES {
            return Err(WireError::SchemaMismatch { key: key.0 });
        }
        let name = core::str::from_utf8(r.bytes(name_len)?).map_err(|_| WireError::InvalidUtf8)?;
        if !table::validate_desc_invariants(words, str_words, resource_words, &defaults, &effects) {
            return Err(WireError::SchemaMismatch { key: key.0 });
        }

        let spec = table::TableDescSpec {
            schema_hash,
            storage,
            flags,
            defaults: &defaults,
            word_effects: &effects,
            str_words,
            resource_words,
        };

        // 1. Check session-scoped dynamic descriptors FIRST so wire-slot mappings stay stable
        //    within an active decoder session. Typed Rust module tables must be registered before
        //    wire decoding begins so they resolve directly through step 2 below.
        if let Some((dyn_slot, existing)) = self
            .dynamic_descs
            .iter()
            .find(|(_, d)| d.key == key)
            .cloned()
        {
            if !existing.matches_spec(&spec) {
                return Err(WireError::SchemaMismatch { key: key.0 });
            }
            if !tx.store_mut().install_dynamic_table(dyn_slot, existing) {
                return Err(WireError::SchemaMismatch { key: key.0 });
            }
            self.map[wire_slot as usize] = dyn_slot.0;
            return Ok(());
        }

        // 2. Compiled static descriptors in `0..MAX_STATIC_TABLE_SLOTS` require full field equality.
        if let Some((static_slot, existing)) = table::lookup_static(key) {
            if !table::static_desc_matches(existing, &spec) {
                return Err(WireError::SchemaMismatch { key: key.0 });
            }
            self.map[wire_slot as usize] = static_slot.0;
            return Ok(());
        }

        // 3. Allocate a fresh per-session dynamic slot strictly inside
        //    `MAX_STATIC_TABLE_SLOTS..MAX_TABLE_SLOTS` (disjoint from static slots, zero `Box::leak`).
        let min_slot = table::MAX_STATIC_TABLE_SLOTS;
        let mut chosen = None;
        while !self.dynamic_exhausted {
            let cand = self.next_dynamic_slot;
            if (cand.0 as usize) < min_slot {
                self.dynamic_exhausted = true;
                break;
            }
            if cand.0 as usize == min_slot {
                self.dynamic_exhausted = true;
            } else {
                self.next_dynamic_slot = TableSlot(cand.0 - 1);
            }
            if tx.store().table_meta(cand).is_none() {
                chosen = Some(cand);
                break;
            }
        }
        let dyn_slot = chosen.ok_or(WireError::TableRegistry)?;
        let desc = Arc::new(OwnedTableDesc {
            key,
            name: name.into(),
            schema_hash,
            storage,
            flags: flags | table_flags::HOST_SLOT,
            defaults: defaults.into_boxed_slice(),
            word_effects: effects.into_boxed_slice(),
            str_words,
            resource_words,
        });
        if !tx
            .store_mut()
            .install_dynamic_table(dyn_slot, Arc::clone(&desc))
        {
            return Err(WireError::SchemaMismatch { key: key.0 });
        }
        self.dynamic_descs.push((dyn_slot, desc));
        self.map[wire_slot as usize] = dyn_slot.0;
        Ok(())
    }
}
