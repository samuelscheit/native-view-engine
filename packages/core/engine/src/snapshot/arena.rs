// Pointer-Free Relocatable Offset-Arena (`RelocatableStateArena` / `RelocatableTableView`)

use std::collections::BTreeMap;

use super::format::{fnv1a_64, SnapshotError};

/// Magic identifying a relocatable offset-indexed state table (`NVERELOC`).
pub const RELOCATABLE_ARENA_MAGIC: [u8; 8] = *b"NVERELOC";

/// Format version for `RelocatableStateArena` / `RelocatableTableView`.
pub const RELOCATABLE_ARENA_VERSION: u32 = 1;

const ARENA_HEADER_SIZE: usize = 16;
const ARENA_SLOT_SIZE: usize = 32;

const VALUE_KIND_I64: u32 = 1;
const VALUE_KIND_F64: u32 = 2;
const VALUE_KIND_STR: u32 = 3;
const VALUE_KIND_BYTES: u32 = 4;
const VALUE_KIND_BOOL: u32 = 5;

#[derive(Debug, Clone)]
enum OwnedValue {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
}

/// Borrowed zero-copy value read directly out of a [`RelocatableTableView`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RelocatableValue<'a> {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(&'a str),
    Bytes(&'a [u8]),
}

/// Builder for a pointer-free, offset-indexed key/value state blob suitable for embedding
/// inside an `NVE_SNAP` state section and reading directly from `mmap` with zero allocations.
#[derive(Debug, Default, Clone)]
pub struct RelocatableStateArena {
    entries: BTreeMap<String, OwnedValue>,
}

impl RelocatableStateArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_i64(&mut self, key: impl Into<String>, value: i64) -> &mut Self {
        self.entries.insert(key.into(), OwnedValue::I64(value));
        self
    }

    pub fn insert_f64(&mut self, key: impl Into<String>, value: f64) -> &mut Self {
        self.entries.insert(key.into(), OwnedValue::F64(value));
        self
    }

    pub fn insert_bool(&mut self, key: impl Into<String>, value: bool) -> &mut Self {
        self.entries.insert(key.into(), OwnedValue::Bool(value));
        self
    }

    pub fn insert_str(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.entries
            .insert(key.into(), OwnedValue::Str(value.into()));
        self
    }

    pub fn insert_bytes(&mut self, key: impl Into<String>, value: impl Into<Vec<u8>>) -> &mut Self {
        self.entries
            .insert(key.into(), OwnedValue::Bytes(value.into()));
        self
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serializes all entries into a compact, deterministic, pointer-free binary buffer sorted by
    /// `(fnv1a_64(key), key)` for `O(log N)` zero-allocation binary-search lookup.
    pub fn finish(&self) -> Vec<u8> {
        let mut sorted: Vec<(u64, &str, &OwnedValue)> = self
            .entries
            .iter()
            .map(|(k, v)| (fnv1a_64(k.as_bytes()), k.as_str(), v))
            .collect();
        sorted.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_bytes().cmp(b.1.as_bytes())));

        let entry_count = u32::try_from(sorted.len()).expect("too many entries in state arena");
        let table_size = sorted.len() * ARENA_SLOT_SIZE;
        let data_start = ARENA_HEADER_SIZE + table_size;

        let payload_bytes_estimate: usize = sorted
            .iter()
            .map(|(_, k, v)| {
                k.len()
                    + match v {
                        OwnedValue::Str(s) => s.len(),
                        OwnedValue::Bytes(b) => b.len(),
                        OwnedValue::I64(_) | OwnedValue::F64(_) | OwnedValue::Bool(_) => 0,
                    }
            })
            .sum();

        let mut buf = Vec::with_capacity(data_start + payload_bytes_estimate);
        buf.extend_from_slice(&RELOCATABLE_ARENA_MAGIC);
        buf.extend_from_slice(&RELOCATABLE_ARENA_VERSION.to_le_bytes());
        buf.extend_from_slice(&entry_count.to_le_bytes());
        buf.resize(data_start, 0);

        for (idx, (key_hash, key, val)) in sorted.into_iter().enumerate() {
            let key_offset = u32::try_from(buf.len()).expect("relocatable arena exceeds 4 GiB");
            let key_bytes = key.as_bytes();
            let key_len = u32::try_from(key_bytes.len()).expect("key exceeds 4 GiB");
            buf.extend_from_slice(key_bytes);

            let (value_kind, value_len, payload_or_offset): (u32, u32, u64) = match val {
                OwnedValue::I64(v) => (VALUE_KIND_I64, 8, *v as u64),
                OwnedValue::F64(v) => (VALUE_KIND_F64, 8, v.to_bits()),
                OwnedValue::Bool(v) => (VALUE_KIND_BOOL, 1, u64::from(*v)),
                OwnedValue::Str(s) => {
                    let offset = buf.len() as u64;
                    let b = s.as_bytes();
                    let len = u32::try_from(b.len()).expect("string value exceeds 4 GiB");
                    buf.extend_from_slice(b);
                    (VALUE_KIND_STR, len, offset)
                }
                OwnedValue::Bytes(b) => {
                    let offset = buf.len() as u64;
                    let len = u32::try_from(b.len()).expect("byte value exceeds 4 GiB");
                    buf.extend_from_slice(b);
                    (VALUE_KIND_BYTES, len, offset)
                }
            };

            let slot_pos = ARENA_HEADER_SIZE + idx * ARENA_SLOT_SIZE;
            buf[slot_pos..slot_pos + 8].copy_from_slice(&key_hash.to_le_bytes());
            buf[slot_pos + 8..slot_pos + 12].copy_from_slice(&key_offset.to_le_bytes());
            buf[slot_pos + 12..slot_pos + 16].copy_from_slice(&key_len.to_le_bytes());
            buf[slot_pos + 16..slot_pos + 20].copy_from_slice(&value_kind.to_le_bytes());
            buf[slot_pos + 20..slot_pos + 24].copy_from_slice(&value_len.to_le_bytes());
            buf[slot_pos + 24..slot_pos + 32].copy_from_slice(&payload_or_offset.to_le_bytes());
        }

        buf
    }
}

#[derive(Debug, Clone, Copy)]
struct ArenaSlot {
    key_hash: u64,
    key_offset: u32,
    key_len: u32,
    value_kind: u32,
    value_len: u32,
    payload_or_offset: u64,
}

/// Zero-copy, zero-allocation borrowed view into a [`RelocatableStateArena`] byte slice
/// (typically straight from [`super::MmapSnapshot::state_bytes`]).
#[derive(Debug, Clone, Copy)]
pub struct RelocatableTableView<'a> {
    bytes: &'a [u8],
    entry_count: usize,
    data_start: usize,
}

impl<'a> RelocatableTableView<'a> {
    /// Opens a relocatable table view in `O(1)` time without touching payload pages.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, SnapshotError> {
        if bytes.len() < ARENA_HEADER_SIZE {
            return Err(SnapshotError::InvalidRelocatableArena(
                "truncated relocatable arena header",
            ));
        }
        if bytes[0..8] != RELOCATABLE_ARENA_MAGIC {
            return Err(SnapshotError::InvalidRelocatableArena(
                "invalid relocatable arena magic",
            ));
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if version != RELOCATABLE_ARENA_VERSION {
            return Err(SnapshotError::InvalidRelocatableArena(
                "unsupported relocatable arena version",
            ));
        }
        let entry_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let table_bytes = entry_count
            .checked_mul(ARENA_SLOT_SIZE)
            .ok_or(SnapshotError::InvalidRelocatableArena(
                "entry table size overflow",
            ))?;
        let data_start = ARENA_HEADER_SIZE
            .checked_add(table_bytes)
            .ok_or(SnapshotError::InvalidRelocatableArena(
                "data_start overflow",
            ))?;
        if data_start > bytes.len() {
            return Err(SnapshotError::InvalidRelocatableArena(
                "entry table extends past end of buffer",
            ));
        }
        Ok(Self {
            bytes,
            entry_count,
            data_start,
        })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entry_count
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entry_count == 0
    }

    /// Eagerly validates every slot's bounds, key sort order, UTF-8 validity, and value kind.
    pub fn validate_all(&self) -> Result<(), SnapshotError> {
        let mut prev: Option<(u64, &'a [u8])> = None;
        for idx in 0..self.entry_count {
            let slot = self.read_slot(idx);
            let key_bytes = self
                .slot_key_bytes(slot)
                .ok_or(SnapshotError::InvalidRelocatableArena(
                    "slot key range out of bounds",
                ))?;
            let _ = std::str::from_utf8(key_bytes).map_err(|_| {
                SnapshotError::InvalidRelocatableArena("slot key is not valid UTF-8")
            })?;
            if fnv1a_64(key_bytes) != slot.key_hash {
                return Err(SnapshotError::InvalidRelocatableArena(
                    "slot key hash mismatch",
                ));
            }
            if let Some((prev_hash, prev_key)) = prev {
                let ord = prev_hash
                    .cmp(&slot.key_hash)
                    .then_with(|| prev_key.cmp(key_bytes));
                if !ord.is_lt() {
                    return Err(SnapshotError::InvalidRelocatableArena(
                        "entry table is not strictly sorted",
                    ));
                }
            }
            prev = Some((slot.key_hash, key_bytes));
            let _ = self
                .decode_slot_value(slot)
                .ok_or(SnapshotError::InvalidRelocatableArena(
                    "slot value is malformed or out of bounds",
                ))?;
        }
        Ok(())
    }

    #[inline]
    fn read_slot(&self, index: usize) -> ArenaSlot {
        let base = ARENA_HEADER_SIZE + index * ARENA_SLOT_SIZE;
        let s = &self.bytes[base..base + ARENA_SLOT_SIZE];
        ArenaSlot {
            key_hash: u64::from_le_bytes(s[0..8].try_into().unwrap()),
            key_offset: u32::from_le_bytes(s[8..12].try_into().unwrap()),
            key_len: u32::from_le_bytes(s[12..16].try_into().unwrap()),
            value_kind: u32::from_le_bytes(s[16..20].try_into().unwrap()),
            value_len: u32::from_le_bytes(s[20..24].try_into().unwrap()),
            payload_or_offset: u64::from_le_bytes(s[24..32].try_into().unwrap()),
        }
    }

    #[inline]
    fn slot_key_bytes(&self, slot: ArenaSlot) -> Option<&'a [u8]> {
        let start = slot.key_offset as usize;
        if start < self.data_start {
            return None;
        }
        let end = start.checked_add(slot.key_len as usize)?;
        self.bytes.get(start..end)
    }

    #[inline]
    fn slot_payload_bytes(&self, slot: ArenaSlot) -> Option<&'a [u8]> {
        let start = usize::try_from(slot.payload_or_offset).ok()?;
        if start < self.data_start {
            return None;
        }
        let end = start.checked_add(slot.value_len as usize)?;
        self.bytes.get(start..end)
    }

    fn decode_slot_value(&self, slot: ArenaSlot) -> Option<RelocatableValue<'a>> {
        match slot.value_kind {
            VALUE_KIND_I64 if slot.value_len == 8 => {
                Some(RelocatableValue::I64(slot.payload_or_offset as i64))
            }
            VALUE_KIND_F64 if slot.value_len == 8 => {
                Some(RelocatableValue::F64(f64::from_bits(slot.payload_or_offset)))
            }
            VALUE_KIND_BOOL if slot.value_len == 1 && slot.payload_or_offset <= 1 => {
                Some(RelocatableValue::Bool(slot.payload_or_offset != 0))
            }
            VALUE_KIND_STR => {
                let raw = self.slot_payload_bytes(slot)?;
                let s = std::str::from_utf8(raw).ok()?;
                Some(RelocatableValue::Str(s))
            }
            VALUE_KIND_BYTES => {
                let raw = self.slot_payload_bytes(slot)?;
                Some(RelocatableValue::Bytes(raw))
            }
            _ => None,
        }
    }

    fn find_slot(&self, key: &str) -> Option<ArenaSlot> {
        let key_bytes = key.as_bytes();
        let target_hash = fnv1a_64(key_bytes);
        let mut lo = 0usize;
        let mut hi = self.entry_count;

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let slot = self.read_slot(mid);
            let ord = match slot.key_hash.cmp(&target_hash) {
                std::cmp::Ordering::Equal => {
                    let mid_key = self.slot_key_bytes(slot)?;
                    mid_key.cmp(key_bytes)
                }
                other => other,
            };
            match ord {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(slot),
            }
        }
        None
    }

    /// Returns true if an entry exists for `key`.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.find_slot(key).is_some()
    }

    /// Looks up any typed value by `key` with zero allocations.
    #[inline]
    pub fn get(&self, key: &str) -> Option<RelocatableValue<'a>> {
        let slot = self.find_slot(key)?;
        self.decode_slot_value(slot)
    }

    /// Looks up an `i64` entry by `key` with zero allocations.
    #[inline]
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            RelocatableValue::I64(v) => Some(v),
            _ => None,
        }
    }

    /// Looks up an `f64` entry by `key` with zero allocations.
    #[inline]
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            RelocatableValue::F64(v) => Some(v),
            _ => None,
        }
    }

    /// Looks up a `bool` entry by `key` with zero allocations.
    #[inline]
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            RelocatableValue::Bool(v) => Some(v),
            _ => None,
        }
    }

    /// Looks up a borrowed `&'a str` entry directly inside the mapped buffer with zero allocations.
    #[inline]
    pub fn get_str(&self, key: &str) -> Option<&'a str> {
        match self.get(key)? {
            RelocatableValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Looks up a borrowed `&'a [u8]` entry directly inside the mapped buffer with zero allocations.
    #[inline]
    pub fn get_bytes(&self, key: &str) -> Option<&'a [u8]> {
        match self.get(key)? {
            RelocatableValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Iterates all valid `(key, value)` pairs in sorted hash order without allocating.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, RelocatableValue<'a>)> + '_ {
        (0..self.entry_count).filter_map(|idx| {
            let slot = self.read_slot(idx);
            let key = std::str::from_utf8(self.slot_key_bytes(slot)?).ok()?;
            let val = self.decode_slot_value(slot)?;
            Some((key, val))
        })
    }
}
