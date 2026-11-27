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
/// One-time session base allowance for slot-indexed arrays (`NodeStore::nodes`, static dense
/// columns, and `ResourceStore::entries`), reset only on `WireDecoder::reset()` or a keyframe
/// against a fresh store — never refilled on delta frames.
const SESSION_BASE_SLOT_BYTES: usize = 16 * 1024 * 1024;
/// Multiplier earning additional slot-memory budget strictly proportional to wire bytes received.
const WIRE_SLOT_MULTIPLIER: usize = 16;
/// Hard per-session ceiling on cumulative slot-array growth across all frames.
const SESSION_MAX_SLOT_BYTES: usize = 256 * 1024 * 1024;
/// Hard upper ceiling on wire `ResourceId` indices (`1 << 20` = 1,048,576 slots), matching the
/// bounded 24-bit `NodeId::MAX_INDEX` space.
const MAX_RESOURCE_SLOTS: u32 = 1 << 20;

/// Session-anchored slot-memory budget preventing both single-frame sparse-index OOM and
/// multi-frame stride amplification across nodes, static dense columns, and resources, while
/// allowing legitimate churned keyframes (`encode_full`) to round-trip cleanly.
#[derive(Debug, Clone, Default)]
struct SessionBudget {
    started: bool,
    limit_bytes: usize,
    used_bytes: usize,
}

impl SessionBudget {
    fn reset(&mut self) {
        self.started = false;
        self.limit_bytes = 0;
        self.used_bytes = 0;
    }

    fn begin_frame(&mut self, frame: u64, wire_bytes: usize, fresh_store: bool) {
        let earned = wire_bytes.saturating_mul(WIRE_SLOT_MULTIPLIER);
        if !self.started || (frame == 1 && fresh_store) {
            self.started = true;
            self.used_bytes = 0;
            self.limit_bytes = SESSION_BASE_SLOT_BYTES
                .saturating_add(earned)
                .min(SESSION_MAX_SLOT_BYTES);
        } else {
            self.limit_bytes = self
                .limit_bytes
                .saturating_add(earned)
                .min(SESSION_MAX_SLOT_BYTES);
        }
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.limit_bytes.saturating_sub(self.used_bytes)
    }

    #[inline]
    fn charge(&mut self, delta_bytes: usize) -> Result<(), WireError> {
        let next = self
            .used_bytes
            .checked_add(delta_bytes)
            .ok_or(WireError::Budget)?;
        if next > self.limit_bytes {
            return Err(WireError::Budget);
        }
        self.used_bytes = next;
        Ok(())
    }
}

/// Receiver-side session state: wire slot → local slot, session-scoped dynamic table descriptors
/// (never leaked into global `REGISTRY`), session slot-memory budget, and child scratch buffer.
#[derive(Debug, Clone)]
pub struct WireDecoder {
    map: [u8; MAX_TABLE_SLOTS],
    dynamic_descs: Vec<(TableSlot, Arc<OwnedTableDesc>)>,
    next_dynamic_slot: TableSlot,
    dynamic_exhausted: bool,
    kids_scratch: Vec<NodeId>,
    budget: SessionBudget,
}

impl Default for WireDecoder {
    fn default() -> Self {
        Self {
            map: [u8::MAX; MAX_TABLE_SLOTS],
            dynamic_descs: Vec::new(),
            next_dynamic_slot: TableSlot((MAX_TABLE_SLOTS - 1) as u8),
            dynamic_exhausted: false,
            kids_scratch: Vec::new(),
            budget: SessionBudget::default(),
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
        self.budget.reset();
    }

    /// Maximum `NodeStore` slot hint a Frame-0 pre-reservation may claim for `wire_bytes` without
    /// exceeding the session slot-memory ceiling before decoding starts.
    #[inline]
    pub fn max_frame0_node_hint(wire_bytes: usize) -> usize {
        let max_budget = SESSION_BASE_SLOT_BYTES
            .saturating_add(wire_bytes.saturating_mul(WIRE_SLOT_MULTIPLIER))
            .min(SESSION_MAX_SLOT_BYTES);
        (wire_bytes / 4)
            .min(max_budget / size_of::<crate::store::NodeRow>())
            .min(NodeId::MAX_INDEX as usize)
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
        let fresh_store = tx.store().frame() == 0
            && tx.store().capacity() <= 1
            && tx.store().resources.slots() <= 1
            && tx.store().table_slots().next().is_none();
        self.budget.begin_frame(frame, bytes.len(), fresh_store);
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
            self.op(tx, op, (head >> 8) as u8, count, &mut body_r)?;
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
                let node_row_bytes = size_of::<crate::store::NodeRow>();
                let existing_creatable = tx.store().capacity().saturating_sub(1);
                let min_new_slots = (count as usize).saturating_sub(existing_creatable);
                let min_new_bytes = min_new_slots
                    .checked_mul(node_row_bytes)
                    .ok_or(WireError::Budget)?;
                if min_new_bytes > self.budget.remaining() {
                    return Err(WireError::Budget);
                }
                let max_new_slots = self.budget.remaining() / node_row_bytes;
                tx.store_mut()
                    .reserve_for_frame0((count as usize).min(max_new_slots), r.remaining())?;
                for _ in 0..count {
                    let id = r.node()?;
                    let index = id.index();
                    if index > NodeId::MAX_INDEX as usize {
                        return Err(WireError::NodeExists(id.0));
                    }
                    let needed = index.saturating_add(1);
                    let cur_cap = tx.store().capacity();
                    let delta_bytes = if needed > cur_cap {
                        let bytes = (needed - cur_cap)
                            .checked_mul(node_row_bytes)
                            .ok_or(WireError::Budget)?;
                        if bytes > self.budget.remaining() {
                            return Err(WireError::Budget);
                        }
                        bytes
                    } else {
                        0
                    };
                    if !tx.store_mut().try_create_at(id)? {
                        return Err(WireError::NodeExists(id.0));
                    }
                    if delta_bytes > 0 {
                        self.budget.charge(delta_bytes)?;
                    }
                }
            }
            Op::DefineResources => {
                // Minimum entry size: id + kind_lo + kind_hi + ephemeral + len = 20 bytes.
                if (count as usize) > r.remaining() / 20 {
                    return Err(WireError::Truncated);
                }
                for _ in 0..count {
                    let id = r.u32()?;
                    if id == 0 {
                        return Err(WireError::Resource { id, kind: 0 });
                    }
                    if id >= MAX_RESOURCE_SLOTS {
                        return Err(WireError::Budget);
                    }
                    let kind = Key64::from_halves(r.u32()?, r.u32()?);
                    let ephemeral = r.u32()? != 0;
                    let len = r.u32()? as usize;
                    let blob = r.bytes(len)?;
                    let needed = (id as usize).saturating_add(1);
                    let cur_slots = tx.store().resources.slots();
                    let delta_bytes = if needed > cur_slots {
                        let bytes = (needed - cur_slots)
                            .checked_mul(crate::resource::ResourceStore::ENTRY_BYTES)
                            .ok_or(WireError::Budget)?;
                        if bytes > self.budget.remaining() {
                            return Err(WireError::Budget);
                        }
                        bytes
                    } else {
                        0
                    };
                    if !tx.store_mut().resources.try_define_at(
                        ResourceId(id),
                        kind,
                        ephemeral,
                        blob,
                    )? {
                        return Err(WireError::Resource { id, kind: kind.0 });
                    }
                    if delta_bytes > 0 {
                        self.budget.charge(delta_bytes)?;
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
                    &mut self.budget.used_bytes,
                    self.budget.limit_bytes,
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
                    &mut self.budget.used_bytes,
                    self.budget.limit_bytes,
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
                    tx.try_detach_slot(id, slot)?;
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
