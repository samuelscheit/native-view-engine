//! `ChangeSet` + `NodeStore` → `NVE2` bytes.

use nve_abi::table::{table_flags, MAX_TABLE_SLOTS};

use crate::changes::ChangeSet;
use crate::id::ResourceId;
use crate::span_pool::StrSpan;
use crate::store::NodeStore;
use crate::table::TableSlot;

use super::{pad4, Op, HEADER_WORDS, MAGIC, VERSION};

/// Sender-side session state: which tables the receiver already knows.
#[derive(Debug, Default, Clone)]
pub struct WireEncoder {
    declared: u128,
}

struct W<'a> {
    out: &'a mut Vec<u8>,
    ops: u32,
    op_start: usize,
}

impl W<'_> {
    #[inline(always)]
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
        self.out.resize(pad4(self.out.len()), 0);
    }

    fn begin(&mut self, op: Op, table: u8, count: u32) {
        self.op_start = self.out.len();
        let inline = if count < 0xFFFF { count } else { 0xFFFF };
        self.u32(op as u32 | ((table as u32) << 8) | (inline << 16));
        self.u32(0);
        if count >= 0xFFFF {
            self.u32(count);
        }
    }

    fn end(&mut self) {
        let words = ((self.out.len() - self.op_start) / 4 - 2) as u32;
        self.out[self.op_start + 4..self.op_start + 8].copy_from_slice(&words.to_le_bytes());
        self.ops += 1;
    }

    /// Emits `op` with a count-prefixed list of raw words, skipping empty lists.
    fn list(&mut self, op: Op, table: u8, items: impl ExactSizeIterator<Item = u32>) {
        if items.len() == 0 {
            return;
        }
        self.begin(op, table, items.len() as u32);
        for v in items {
            self.u32(v);
        }
        self.end();
    }
}

impl WireEncoder {
    /// Forgets declared tables (e.g. after the receiver restarted).
    pub fn reset(&mut self) {
        self.declared = 0;
    }

    /// Appends a self-contained `NVE2` keyframe (`frame = 1`, `DeclareTables` + all live nodes,
    /// resources, non-default words, strings, and children) for `store` to `out`.
    pub fn encode_full(&mut self, store: &NodeStore, out: &mut Vec<u8>) {
        self.encode_full_with_labels(store, core::iter::empty(), out);
    }

    /// Appends a self-contained `NVE2` keyframe (`frame = 1`) while preserving an upstream
    /// producer's `(wire_slot, local_slot)` table labels (e.g. from
    /// [`super::WireDecoder::declared_wire_tables`]).
    ///
    /// Every `(wire_slot, local_slot)` in `wire_tables` is declared in `Op::DeclareTables`
    /// (including sparse tables with zero currently-attached rows) and all `Patch` / `SetStrings`
    /// ops use the matching upstream `wire_slot`, so subsequent incremental deltas from the
    /// original producer apply without `WireError::UndeclaredTable`.
    pub fn encode_full_with_labels(
        &mut self,
        store: &NodeStore,
        wire_tables: impl IntoIterator<Item = (u8, TableSlot)>,
        out: &mut Vec<u8>,
    ) {
        self.reset();
        let mut forced = Vec::new();
        let mut local_to_wire = [u8::MAX; MAX_TABLE_SLOTS];
        for (wire_slot, local_slot) in wire_tables {
            if (wire_slot as usize) < MAX_TABLE_SLOTS && (local_slot.0 as usize) < MAX_TABLE_SLOTS {
                local_to_wire[local_slot.0 as usize] = wire_slot;
                forced.push((wire_slot, local_slot));
            }
        }
        let has_labels = !forced.is_empty();

        let mut changes = ChangeSet {
            frame: 1,
            ..Default::default()
        };

        let mut live: Vec<crate::id::NodeId> = store.live_nodes().collect();
        live.sort_unstable_by_key(|id| id.raw());
        for &id in &live {
            changes.created.push(id);
            if !store.children(id).is_empty() {
                changes.children.push(id);
            }
        }

        let mut seen_res = std::collections::HashSet::new();
        for slot in store.table_slots() {
            let Some(desc) = store.table_meta(slot) else {
                continue;
            };
            let mut nodes = Vec::new();
            let mut masks = Vec::new();
            for &id in &live {
                let Some(words) = store.words(id, slot) else {
                    continue;
                };
                let mut mask = 0u32;
                for (i, (&w, &def)) in words.iter().zip(desc.defaults().iter()).enumerate() {
                    if w != def {
                        mask |= 1u32 << i;
                    }
                }
                // Include non-empty strings even if off == 0.
                let mut str_bits = desc.str_words();
                while str_bits != 0 {
                    let i = str_bits.trailing_zeros() as usize;
                    str_bits &= str_bits - 1;
                    if words.get(i + 1).copied().unwrap_or(0) != 0 {
                        mask |= 1u32 << i;
                    }
                }
                // Collect referenced live resources.
                let mut res_bits = desc.resource_words();
                while res_bits != 0 {
                    let i = res_bits.trailing_zeros() as usize;
                    res_bits &= res_bits - 1;
                    let rid = crate::id::ResourceId(words.get(i).copied().unwrap_or(0));
                    if !rid.is_none() && seen_res.insert(rid) {
                        changes.resources_defined.push(rid);
                    }
                }
                nodes.push(id);
                masks.push(mask);
            }
            if !nodes.is_empty() {
                let t = changes.next_table(slot);
                t.nodes = nodes;
                t.masks = masks;
            }
        }

        let label_map = has_labels.then_some(&local_to_wire);
        self.encode_inner(store, &changes, &forced, label_map, out);
    }

    /// Appends one `NVE2` frame for `changes` (read against the post-commit `store`) to `out`.
    pub fn encode(&mut self, store: &NodeStore, changes: &ChangeSet, out: &mut Vec<u8>) {
        self.encode_inner(store, changes, &[], None, out);
    }

    fn encode_inner(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        force_declare: &[(u8, TableSlot)],
        local_to_wire: Option<&[u8; MAX_TABLE_SLOTS]>,
        out: &mut Vec<u8>,
    ) {
        let wire_of = |local: TableSlot| -> u8 {
            local_to_wire
                .and_then(|map| map.get(local.0 as usize).copied())
                .filter(|&w| w != u8::MAX)
                .unwrap_or(local.0)
        };
        let base = out.len();
        let mut w = W { out, ops: 0, op_start: 0 };
        for _ in 0..HEADER_WORDS {
            w.u32(0);
        }

        // DeclareTables: every forced or changed/commanded table not yet declared in this session.
        let mut to_declare: Vec<(u8, TableSlot)> = Vec::new();
        let mut declared_now = 0u128;
        for &(wire_slot, local_slot) in force_declare {
            let bit = 1u128 << wire_slot;
            if (self.declared | declared_now) & bit == 0
                && (store.table_meta(local_slot).is_some()
                    || crate::table::try_desc(local_slot).is_some())
            {
                to_declare.push((wire_slot, local_slot));
                declared_now |= bit;
            }
        }
        let mut need = 0u128;
        for t in changes.tables() {
            need |= t.slot.bit();
        }
        for c in &changes.commands {
            need |= c.table.bit();
        }
        let mut need_bits = need;
        while need_bits != 0 {
            let local_slot = TableSlot(need_bits.trailing_zeros() as u8);
            need_bits &= need_bits - 1;
            let wire_slot = wire_of(local_slot);
            let bit = 1u128 << wire_slot;
            if (self.declared | declared_now) & bit == 0
                && (store.table_meta(local_slot).is_some()
                    || crate::table::try_desc(local_slot).is_some())
            {
                to_declare.push((wire_slot, local_slot));
                declared_now |= bit;
            }
        }
        if !to_declare.is_empty() {
            w.begin(Op::DeclareTables, 0, to_declare.len() as u32);
            for (wire_slot, local_slot) in to_declare {
                let (key, schema_hash, storage, flags, str_w, res_w, defaults, word_effects, name) =
                    if let Some(m) = store.table_meta(local_slot) {
                        (
                            m.key(),
                            m.schema_hash(),
                            m.storage(),
                            m.flags() & !table_flags::HOST_SLOT,
                            m.str_words(),
                            m.resource_words(),
                            m.defaults(),
                            m.word_effects(),
                            m.name(),
                        )
                    } else {
                        let d = crate::table::desc(local_slot);
                        (
                            d.key,
                            d.schema_hash,
                            d.storage,
                            d.flags & !table_flags::HOST_SLOT,
                            d.str_words,
                            d.resource_words,
                            d.defaults,
                            d.word_effects,
                            d.name,
                        )
                    };
                w.u32(key.lo());
                w.u32(key.hi());
                w.u32(schema_hash as u32);
                w.u32((schema_hash >> 32) as u32);
                w.u32(
                    wire_slot as u32
                        | (storage as u32) << 8
                        | (flags as u32) << 16
                        | (defaults.len() as u32) << 24,
                );
                w.u32(str_w);
                w.u32(res_w);
                for &v in defaults {
                    w.u32(v);
                }
                for &e in word_effects {
                    w.out.push(e);
                }
                w.out.resize(pad4(w.out.len()), 0);
                w.u32(name.len() as u32);
                w.bytes(name.as_bytes());
            }
            w.end();
            self.declared |= declared_now;
        }

        w.list(Op::CreateNodes, 0, changes.created.iter().map(|n| n.0));

        if !changes.resources_defined.is_empty() {
            w.begin(Op::DefineResources, 0, changes.resources_defined.len() as u32);
            let res = store.resources();
            for &id in &changes.resources_defined {
                let kind = res.kind(id).unwrap_or_default();
                w.u32(id.0);
                w.u32(kind.lo());
                w.u32(kind.hi());
                w.u32(res.is_ephemeral(id) as u32);
                let len_at = w.out.len();
                w.u32(0);
                let start = w.out.len();
                assert!(
                    res.encode(id, w.out),
                    "NVE2 encode: missing registered codec for resource {id:?} (kind {kind:?})"
                );
                let len = u32::try_from(w.out.len() - start)
                    .expect("NVE2 resource payload exceeds u32::MAX");
                w.out[len_at..len_at + 4].copy_from_slice(&len.to_le_bytes());
                w.out.resize(pad4(w.out.len()), 0);
            }
            w.end();
        }

        for t in changes.tables() {
            let slot = wire_of(t.slot);
            let Some(desc) = store.table_meta(t.slot) else { continue };
            let str_words = desc.str_words();
            let str_pairs = str_words | str_words << 1;
            if !t.nodes.is_empty() {
                let needs_patch = |mask: u32| -> bool {
                    let m = mask & !str_pairs;
                    m != 0 || (mask & str_words) == 0
                };
                let patch_count = t.iter().filter(|(_, mask)| needs_patch(*mask)).count() as u32;
                if patch_count > 0 {
                    w.begin(Op::Patch, slot, patch_count);
                    for (id, mask) in t.iter() {
                        if !needs_patch(mask) {
                            continue;
                        }
                        let words = store.words(id, t.slot).unwrap_or(&[]);
                        let m = mask & !str_pairs;
                        w.u32(id.0);
                        w.u32(m);
                        let mut bits = m;
                        while bits != 0 {
                            let i = bits.trailing_zeros() as usize;
                            bits &= bits - 1;
                            w.u32(words.get(i).copied().unwrap_or(0));
                        }
                    }
                    w.end();
                }
                if str_words != 0 {
                    let str_count: u32 = t
                        .iter()
                        .map(|(_, m)| (m & str_words).count_ones())
                        .sum();
                    if str_count > 0 {
                        w.begin(Op::SetStrings, slot, str_count);
                        for (id, mask) in t.iter() {
                            let mut bits = mask & str_words;
                            if bits == 0 {
                                continue;
                            }
                            let words = store.words(id, t.slot).unwrap_or(&[]);
                            while bits != 0 {
                                let i = bits.trailing_zeros() as usize;
                                bits &= bits - 1;
                                let text = store.text.str_unchecked(StrSpan::new(words[i], words[i + 1]));
                                let len = text.len();
                                assert!(
                                    len <= 0x00FF_FFFF,
                                    "NVE2 string length {len} exceeds 16 MiB wire limit"
                                );
                                w.u32(id.0);
                                w.u32((i as u32) | ((len as u32) << 8));
                                w.bytes(text.as_bytes());
                            }
                        }
                        w.end();
                    }
                }
            }
            w.list(Op::Detach, slot, t.detached.iter().map(|n| n.0));
        }

        if !changes.children.is_empty() {
            w.begin(Op::SetChildren, 0, changes.children.len() as u32);
            for &p in &changes.children {
                let kids = store.children(p);
                w.u32(p.0);
                w.u32(kids.len() as u32);
                for k in kids {
                    w.u32(k.0);
                }
            }
            w.end();
        }

        w.list(Op::RemoveNodes, 0, changes.removed.iter().map(|n| n.0));
        w.list(
            Op::ReleaseResources,
            0,
            changes.resources_released.iter().map(|r: &ResourceId| r.0),
        );

        if !changes.commands.is_empty() {
            w.begin(Op::Commands, 0, changes.commands.len() as u32);
            for c in &changes.commands {
                let payload = changes.command_payload(c);
                w.u32(c.node.0);
                w.u32(wire_of(c.table) as u32);
                w.u32(c.op);
                w.u32(payload.len() as u32);
                w.bytes(payload);
            }
            w.end();
        }

        let ops = w.ops;
        let words = ((out.len() - base) / 4) as u32;
        let header = [
            MAGIC,
            VERSION as u32,
            changes.frame as u32,
            (changes.frame >> 32) as u32,
            ops,
            words,
        ];
        for (i, v) in header.iter().enumerate() {
            out[base + i * 4..base + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}
