//! `NVE2` bytes → replica `NodeStore` mutations (inside a [`Transaction`]).

use std::sync::Arc;

use nve_abi::table::{table_flags, Storage, MAX_TABLE_SLOTS};
use nve_abi::Key64;

use crate::id::{NodeId, ResourceId};
use crate::store::NodeStore;
use crate::table::{self, OwnedTableDesc, TableSlot};
use crate::tx::Transaction;

use super::{pad4_checked, Op, WireError, HEADER_WORDS, MAGIC, VERSION};

/// Maximum length of a runtime-declared table name in `Op::DeclareTables`.
const MAX_TABLE_NAME_BYTES: usize = 128;

/// Receiver-side session state: wire slot → local slot, session-scoped dynamic table descriptors
/// (never leaked into global `REGISTRY`), and reusable child scratch buffer.
#[derive(Debug, Clone)]
pub struct WireDecoder {
    map: [u8; MAX_TABLE_SLOTS],
    dynamic_descs: Vec<(TableSlot, Arc<OwnedTableDesc>)>,
    next_dynamic_slot: TableSlot,
    dynamic_exhausted: bool,
    kids_scratch: Vec<NodeId>,
    res_children_scratch: Vec<ResourceId>,
    res_base_slots: Option<usize>,
    res_defined_total: usize,
}

impl Default for WireDecoder {
    fn default() -> Self {
        Self {
            map: [u8::MAX; MAX_TABLE_SLOTS],
            dynamic_descs: Vec::new(),
            next_dynamic_slot: TableSlot((MAX_TABLE_SLOTS - 1) as u8),
            dynamic_exhausted: false,
            kids_scratch: Vec::new(),
            res_children_scratch: Vec::new(),
            res_base_slots: None,
            res_defined_total: 0,
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
        self.u32().map(NodeId::from_raw_or_null)
    }
}

impl WireDecoder {
    pub fn reset(&mut self) {
        self.map = [u8::MAX; MAX_TABLE_SLOTS];
        self.dynamic_descs.clear();
        self.next_dynamic_slot = TableSlot((MAX_TABLE_SLOTS - 1) as u8);
        self.dynamic_exhausted = false;
        self.kids_scratch.clear();
        self.res_children_scratch.clear();
        self.res_base_slots = None;
        self.res_defined_total = 0;
    }

    fn slot_layout(store: &NodeStore, slot: TableSlot) -> Option<(usize, u32, u32)> {
        if let Some(m) = store.table_meta(slot) {
            return Some((m.words(), m.str_words(), m.resource_words()));
        }
        table::try_desc(slot).map(|d| (d.words(), d.str_words, d.resource_words))
    }

    /// Declared resource kind of `word` in `slot`, falling back to the static registry before the
    /// replica lazily allocates the column on its first `Op::Patch`.
    fn slot_resource_kind(store: &NodeStore, slot: TableSlot, word: usize) -> Option<Key64> {
        match store.table_meta(slot) {
            Some(m) => m.resource_kind(word),
            None => table::try_desc(slot).and_then(|d| d.resource_kind(word)),
        }
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
        // Compute capacity bounds ONCE per frame against `bytes.len()` so repeated ops in a single
        // frame cannot amplify allocation bounds by `+1024` per op.
        let max_node_index = tx
            .store()
            .capacity()
            .saturating_add(bytes.len() / 4)
            .saturating_add(1024)
            .min(NodeId::MAX_INDEX as usize);
        let base_res_slots = *self
            .res_base_slots
            .get_or_insert_with(|| tx.store().resources.slots());
        let max_resource_slot = base_res_slots
            .saturating_add(1024)
            .saturating_add(self.res_defined_total)
            .saturating_add(bytes.len() / 24)
            .min(ResourceId::MAX_INDEX as usize);
        let mut cycle_budget = (bytes.len() / 4).max(1024).saturating_mul(8);
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
            self.op(
                tx,
                op,
                (head >> 8) as u8,
                count,
                &mut body_r,
                max_node_index,
                max_resource_slot,
                &mut cycle_budget,
            )?;
            if body_r.at != body.len() {
                return Err(WireError::TrailingBytes);
            }
        }
        if r.at != bytes.len() {
            return Err(WireError::TrailingBytes);
        }
        Ok(frame)
    }

    #[allow(clippy::too_many_arguments)]
    fn op(
        &mut self,
        tx: &mut Transaction<'_>,
        op: Op,
        table: u8,
        count: u32,
        r: &mut R<'_>,
        max_node_index: usize,
        max_resource_slot: usize,
        cycle_budget: &mut usize,
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
                for _ in 0..count {
                    let id = r.node()?;
                    if id.index() >= max_node_index || !tx.create_at(id) {
                        return Err(WireError::NodeExists(id.raw()));
                    }
                }
            }
            Op::DefineResources => {
                // Minimum entry size: id + kind_lo + kind_hi + flags + child_count + byte_len = 24 bytes.
                if (count as usize) > r.remaining() / 24 {
                    return Err(WireError::Truncated);
                }
                for _ in 0..count {
                    let id = r.u32()?;
                    let rid = ResourceId(id);
                    if rid.index() == 0 || rid.index() >= max_resource_slot {
                        return Err(WireError::Resource { id, kind: 0 });
                    }
                    let kind = Key64::from_halves(r.u32()?, r.u32()?);
                    let flags = r.u32()?;
                    if (flags & !1u32) != 0 {
                        return Err(WireError::Resource { id, kind: kind.0 });
                    }
                    let ephemeral = (flags & 1) != 0;
                    let child_count = r.u32()? as usize;
                    if child_count > r.remaining() / 4 {
                        return Err(WireError::Truncated);
                    }
                    self.res_children_scratch.clear();
                    for _ in 0..child_count {
                        self.res_children_scratch.push(ResourceId(r.u32()?));
                    }
                    let len = r.u32()? as usize;
                    if !len.is_multiple_of(4) {
                        return Err(WireError::Resource { id, kind: kind.0 });
                    }
                    let blob = r.bytes(len)?;
                    if !tx.store_mut().resources.define_at_with_budget(
                        rid,
                        kind,
                        ephemeral,
                        &self.res_children_scratch,
                        blob,
                        cycle_budget,
                    ) {
                        return Err(WireError::Resource { id, kind: kind.0 });
                    }
                    self.res_defined_total = self.res_defined_total.saturating_add(1);
                }
            }
            Op::Patch => {
                if (count as usize) > r.remaining() / 8 {
                    return Err(WireError::Truncated);
                }
                let slot = self.local(table)?;
                let (words_usize, str_words, resource_words) =
                    Self::slot_layout(tx.store(), slot).ok_or(WireError::UndeclaredTable(table))?;
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
                        if resource_words & (1 << i) != 0 && !res.is_none() {
                            if let Some(expected) = Self::slot_resource_kind(tx.store(), slot, i) {
                                if tx.store().resources.kind(res) != Some(expected) {
                                    return Err(WireError::Resource { id: res.0, kind: expected.0 });
                                }
                            } else if !tx.store().resources.is_defined(res) {
                                return Err(WireError::Resource { id: res.0, kind: 0 });
                            }
                        }
                    }
                    let mut w = tx.writer(id, slot);
                    let mut bits = mask;
                    while bits != 0 {
                        let i = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        if resource_words & (1 << i) != 0 {
                            w.put_resource_id_wire(i, ResourceId(vals[i]));
                        } else if i == 0 && w.desc.inherited() {
                            w.put_explicit_word(vals[0]);
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
                let (_, str_words, _) =
                    Self::slot_layout(tx.store(), slot).ok_or(WireError::UndeclaredTable(table))?;
                for _ in 0..count {
                    let id = self.live(tx, r.node()?)?;
                    let word_len = r.u32()?;
                    let word = word_len & 0xFF;
                    let len = (word_len >> 8) as usize;
                    let text = core::str::from_utf8(r.bytes(len)?)
                        .map_err(|_| WireError::InvalidUtf8)?;
                    if word >= 32 || str_words & (1 << word) == 0 {
                        return Err(WireError::BadMask { mask: 1 << (word & 31), words: word });
                    }
                    tx.writer(id, slot).put_str(word as usize, text);
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
        if tx.store().is_live(id) { Ok(id) } else { Err(WireError::DeadNode(id.raw())) }
    }

    fn declare(&mut self, tx: &mut Transaction<'_>, r: &mut R<'_>) -> Result<(), WireError> {
        let key = Key64::from_halves(r.u32()?, r.u32()?);
        let schema_hash = r.u32()? as u64 | (r.u32()? as u64) << 32;
        let packed = r.u32()?;
        let (wire_slot, storage_raw, flags, words) =
            (packed as u8, (packed >> 8) as u8, (packed >> 16) as u8, (packed >> 24) as usize);
        let str_words = r.u32()?;
        let resource_words = r.u32()?;
        if words == 0 || words > 32 || wire_slot as usize >= MAX_TABLE_SLOTS {
            return Err(WireError::BadMask { mask: 0, words: words as u32 });
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
        if !table::validate_desc_invariants(
            words,
            flags,
            str_words,
            resource_words,
            &defaults,
            &effects,
        ) {
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
        if let Some((dyn_slot, existing)) =
            self.dynamic_descs.iter().find(|(_, d)| d.key == key).cloned()
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
