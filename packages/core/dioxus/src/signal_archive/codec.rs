//! Binary container codec (`NVE_SIG1` / `NVE_SGD1`), snapshot/delta types, and typed `SignalCodec`.

use std::collections::BTreeMap;

use nve_abi::BinValueView;
use nve_engine::hash::Xxh64Hasher;
use nve_engine::{MmapSnapshot, SnapshotWal, WalRecordKind};

use super::{
    signal_slot_id, SignalArchiveError, SignalValue, SignalValueTag, NVE_SIGNAL_ARCHIVE_MAGIC,
    NVE_SIGNAL_ARCHIVE_VERSION, NVE_SIGNAL_DELTA_MAGIC,
};

pub(super) const HEADER_SIZE: usize = 28;
const CHECKSUM_OFFSET: usize = 20;
const CHECKSUM_END: usize = 28;

const DELTA_OP_UPSERT: u8 = 1;
const DELTA_OP_REMOVE: u8 = 2;

#[inline]
fn compute_container_checksum(bytes: &[u8]) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(&bytes[0..CHECKSUM_OFFSET]);
    h.update(&bytes[CHECKSUM_END..]);
    h.finish()
}

#[inline]
fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, SignalArchiveError> {
    let b = *bytes.get(*pos).ok_or(SignalArchiveError::Truncated {
        expected: *pos + 1,
        actual: bytes.len(),
    })?;
    *pos += 1;
    Ok(b)
}

#[inline]
fn read_u32_le(bytes: &[u8], pos: &mut usize) -> Result<u32, SignalArchiveError> {
    let end = pos.checked_add(4).ok_or(SignalArchiveError::Truncated {
        expected: usize::MAX,
        actual: bytes.len(),
    })?;
    let slice = bytes
        .get(*pos..end)
        .ok_or(SignalArchiveError::Truncated {
            expected: end,
            actual: bytes.len(),
        })?;
    *pos = end;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

#[inline]
fn read_u64_le(bytes: &[u8], pos: &mut usize) -> Result<u64, SignalArchiveError> {
    let end = pos.checked_add(8).ok_or(SignalArchiveError::Truncated {
        expected: usize::MAX,
        actual: bytes.len(),
    })?;
    let slice = bytes
        .get(*pos..end)
        .ok_or(SignalArchiveError::Truncated {
            expected: end,
            actual: bytes.len(),
        })?;
    *pos = end;
    Ok(u64::from_le_bytes(slice.try_into().unwrap()))
}

#[inline]
fn read_slice<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
    len: usize,
) -> Result<&'a [u8], SignalArchiveError> {
    let end = pos.checked_add(len).ok_or(SignalArchiveError::Truncated {
        expected: usize::MAX,
        actual: bytes.len(),
    })?;
    let slice = bytes
        .get(*pos..end)
        .ok_or(SignalArchiveError::Truncated {
            expected: end,
            actual: bytes.len(),
        })?;
    *pos = end;
    Ok(slice)
}

fn encode_value(value: &SignalValue, out: &mut Vec<u8>) -> Result<(), SignalArchiveError> {
    out.push(value.tag() as u8);
    match value {
        SignalValue::Null => {}
        SignalValue::Bool(b) => out.push(u8::from(*b)),
        SignalValue::I64(v) => out.extend_from_slice(&v.to_le_bytes()),
        SignalValue::U64(v) => out.extend_from_slice(&v.to_le_bytes()),
        SignalValue::F64(v) => out.extend_from_slice(&v.to_bits().to_le_bytes()),
        SignalValue::Str(s) => {
            let b = s.as_bytes();
            let len =
                u32::try_from(b.len()).map_err(|_| SignalArchiveError::PayloadTooLarge(b.len()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
        SignalValue::Bytes(b) => {
            let len =
                u32::try_from(b.len()).map_err(|_| SignalArchiveError::PayloadTooLarge(b.len()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
        SignalValue::BinValue(b) => {
            let _ = BinValueView::parse(b)?;
            let len =
                u32::try_from(b.len()).map_err(|_| SignalArchiveError::PayloadTooLarge(b.len()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
    }
    Ok(())
}

fn decode_value(bytes: &[u8], pos: &mut usize) -> Result<SignalValue, SignalArchiveError> {
    let raw_tag = read_u8(bytes, pos)?;
    let tag =
        SignalValueTag::from_u8(raw_tag).ok_or(SignalArchiveError::InvalidValueTag(raw_tag))?;
    match tag {
        SignalValueTag::Null => Ok(SignalValue::Null),
        SignalValueTag::Bool => {
            let b = read_u8(bytes, pos)?;
            match b {
                0 => Ok(SignalValue::Bool(false)),
                1 => Ok(SignalValue::Bool(true)),
                other => Err(SignalArchiveError::InvalidBool(other)),
            }
        }
        SignalValueTag::I64 => Ok(SignalValue::I64(read_u64_le(bytes, pos)? as i64)),
        SignalValueTag::U64 => Ok(SignalValue::U64(read_u64_le(bytes, pos)?)),
        SignalValueTag::F64 => Ok(SignalValue::F64(f64::from_bits(read_u64_le(bytes, pos)?))),
        SignalValueTag::Str => {
            let len = read_u32_le(bytes, pos)? as usize;
            let raw = read_slice(bytes, pos, len)?;
            let s = std::str::from_utf8(raw).map_err(|_| SignalArchiveError::InvalidUtf8)?;
            Ok(SignalValue::Str(s.to_owned()))
        }
        SignalValueTag::Bytes => {
            let len = read_u32_le(bytes, pos)? as usize;
            let raw = read_slice(bytes, pos, len)?;
            Ok(SignalValue::Bytes(raw.to_vec()))
        }
        SignalValueTag::BinValue => {
            let len = read_u32_le(bytes, pos)? as usize;
            let raw = read_slice(bytes, pos, len)?;
            let _ = BinValueView::parse(raw)?;
            Ok(SignalValue::BinValue(raw.to_vec()))
        }
    }
}

fn decode_header(
    bytes: &[u8],
    expected_magic: [u8; 8],
) -> Result<(u32, usize), SignalArchiveError> {
    if bytes.len() < HEADER_SIZE {
        return Err(SignalArchiveError::Truncated {
            expected: HEADER_SIZE,
            actual: bytes.len(),
        });
    }
    let magic: [u8; 8] = bytes[0..8].try_into().unwrap();
    if magic != expected_magic {
        return Err(SignalArchiveError::InvalidMagic(magic));
    }
    let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if format_version != NVE_SIGNAL_ARCHIVE_VERSION {
        return Err(SignalArchiveError::UnsupportedFormatVersion(format_version));
    }
    let schema_version = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let count = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let stored_checksum =
        u64::from_le_bytes(bytes[CHECKSUM_OFFSET..CHECKSUM_END].try_into().unwrap());
    let computed_checksum = compute_container_checksum(bytes);
    if computed_checksum != stored_checksum {
        return Err(SignalArchiveError::ChecksumMismatch {
            expected: stored_checksum,
            actual: computed_checksum,
        });
    }
    Ok((schema_version, count))
}

pub(super) fn write_header(out: &mut [u8], magic: [u8; 8], schema_version: u32, count: u32) {
    out[0..8].copy_from_slice(&magic);
    out[8..12].copy_from_slice(&NVE_SIGNAL_ARCHIVE_VERSION.to_le_bytes());
    out[12..16].copy_from_slice(&schema_version.to_le_bytes());
    out[16..20].copy_from_slice(&count.to_le_bytes());
    let checksum = compute_container_checksum(out);
    out[CHECKSUM_OFFSET..CHECKSUM_END].copy_from_slice(&checksum.to_le_bytes());
}

/// One dirty-slot operation inside a [`SignalDelta`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalDeltaOp {
    Upsert { slot_id: u64, value: SignalValue },
    Remove { slot_id: u64 },
}

impl SignalDeltaOp {
    #[inline]
    pub const fn slot_id(&self) -> u64 {
        match self {
            Self::Upsert { slot_id, .. } | Self::Remove { slot_id } => *slot_id,
        }
    }
}

/// Incremental dirty-slot delta suitable for direct [`SnapshotWal::append_delta`] persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalDelta {
    pub schema_version: u32,
    pub ops: Vec<SignalDeltaOp>,
}

impl SignalDelta {
    pub fn new(schema_version: u32) -> Self {
        Self {
            schema_version,
            ops: Vec::new(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Encodes this delta into a checksummed, zero-JSON `NVE_SGD1` binary payload.
    pub fn encode(&self) -> Result<Vec<u8>, SignalArchiveError> {
        let mut sorted_ops = self.ops.clone();
        sorted_ops.sort_unstable_by_key(SignalDeltaOp::slot_id);
        for pair in sorted_ops.windows(2) {
            if pair[0].slot_id() == pair[1].slot_id() {
                return Err(SignalArchiveError::UnsortedOrDuplicateSlot(
                    pair[0].slot_id(),
                ));
            }
        }
        let op_count = u32::try_from(sorted_ops.len())
            .map_err(|_| SignalArchiveError::PayloadTooLarge(sorted_ops.len()))?;
        let mut buf = Vec::with_capacity(HEADER_SIZE + sorted_ops.len() * 16);
        buf.resize(HEADER_SIZE, 0);
        for op in &sorted_ops {
            match op {
                SignalDeltaOp::Upsert { slot_id, value } => {
                    buf.push(DELTA_OP_UPSERT);
                    buf.extend_from_slice(&slot_id.to_le_bytes());
                    encode_value(value, &mut buf)?;
                }
                SignalDeltaOp::Remove { slot_id } => {
                    buf.push(DELTA_OP_REMOVE);
                    buf.extend_from_slice(&slot_id.to_le_bytes());
                }
            }
        }
        write_header(
            &mut buf,
            NVE_SIGNAL_DELTA_MAGIC,
            self.schema_version,
            op_count,
        );
        Ok(buf)
    }

    /// Decodes and validates an `NVE_SGD1` binary delta payload.
    pub fn decode(bytes: &[u8]) -> Result<Self, SignalArchiveError> {
        let (schema_version, op_count) = decode_header(bytes, NVE_SIGNAL_DELTA_MAGIC)?;
        let mut pos = HEADER_SIZE;
        let mut ops = Vec::with_capacity(op_count.min(bytes.len().saturating_sub(HEADER_SIZE) / 9));
        let mut prev_slot: Option<u64> = None;

        for _ in 0..op_count {
            let op_kind = read_u8(bytes, &mut pos)?;
            let slot_id = read_u64_le(bytes, &mut pos)?;
            if let Some(prev) = prev_slot
                && slot_id <= prev
            {
                return Err(SignalArchiveError::UnsortedOrDuplicateSlot(slot_id));
            }
            prev_slot = Some(slot_id);
            match op_kind {
                DELTA_OP_UPSERT => {
                    let value = decode_value(bytes, &mut pos)?;
                    ops.push(SignalDeltaOp::Upsert { slot_id, value });
                }
                DELTA_OP_REMOVE => {
                    ops.push(SignalDeltaOp::Remove { slot_id });
                }
                other => return Err(SignalArchiveError::InvalidDeltaOp(other)),
            }
        }

        if pos != bytes.len() {
            return Err(SignalArchiveError::TrailingBytes(bytes.len() - pos));
        }

        Ok(Self {
            schema_version,
            ops,
        })
    }
}

/// Complete deterministic snapshot of typed Dioxus signal slots keyed by stable `slot_id: u64`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalTableSnapshot {
    schema_version: u32,
    slots: BTreeMap<u64, SignalValue>,
}

impl Default for SignalTableSnapshot {
    fn default() -> Self {
        Self::new(1)
    }
}

impl SignalTableSnapshot {
    pub fn new(schema_version: u32) -> Self {
        Self {
            schema_version,
            slots: BTreeMap::new(),
        }
    }

    #[inline]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[inline]
    pub fn set_schema_version(&mut self, schema_version: u32) {
        self.schema_version = schema_version;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn set(&mut self, slot_id: u64, value: SignalValue) -> Option<SignalValue> {
        self.slots.insert(slot_id, value)
    }

    pub fn set_named(
        &mut self,
        component_path: &str,
        signal_name: &str,
        value: SignalValue,
    ) -> u64 {
        let id = signal_slot_id(component_path, signal_name);
        self.slots.insert(id, value);
        id
    }

    pub fn remove(&mut self, slot_id: u64) -> Option<SignalValue> {
        self.slots.remove(&slot_id)
    }

    #[inline]
    pub fn get(&self, slot_id: u64) -> Option<&SignalValue> {
        self.slots.get(&slot_id)
    }

    #[inline]
    pub fn get_named(&self, component_path: &str, signal_name: &str) -> Option<&SignalValue> {
        self.slots.get(&signal_slot_id(component_path, signal_name))
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, &SignalValue)> {
        self.slots.iter().map(|(&id, v)| (id, v))
    }

    /// Encodes the full signal table into a checksummed, zero-JSON `NVE_SIG1` binary buffer
    /// suitable for [`nve_engine::MmapSnapshot::state_bytes`].
    pub fn encode(&self) -> Result<Vec<u8>, SignalArchiveError> {
        let slot_count = u32::try_from(self.slots.len())
            .map_err(|_| SignalArchiveError::PayloadTooLarge(self.slots.len()))?;
        let mut buf = Vec::with_capacity(HEADER_SIZE + self.slots.len() * 18);
        buf.resize(HEADER_SIZE, 0);
        for (&slot_id, value) in &self.slots {
            buf.extend_from_slice(&slot_id.to_le_bytes());
            encode_value(value, &mut buf)?;
        }
        write_header(
            &mut buf,
            NVE_SIGNAL_ARCHIVE_MAGIC,
            self.schema_version,
            slot_count,
        );
        Ok(buf)
    }

    /// Decodes and validates a full `NVE_SIG1` binary archive.
    pub fn decode(bytes: &[u8]) -> Result<Self, SignalArchiveError> {
        let (schema_version, slot_count) = decode_header(bytes, NVE_SIGNAL_ARCHIVE_MAGIC)?;
        let mut pos = HEADER_SIZE;
        let mut slots = BTreeMap::new();
        let mut prev_slot: Option<u64> = None;

        for _ in 0..slot_count {
            let slot_id = read_u64_le(bytes, &mut pos)?;
            if let Some(prev) = prev_slot
                && slot_id <= prev
            {
                return Err(SignalArchiveError::UnsortedOrDuplicateSlot(slot_id));
            }
            prev_slot = Some(slot_id);
            let value = decode_value(bytes, &mut pos)?;
            slots.insert(slot_id, value);
        }

        if pos != bytes.len() {
            return Err(SignalArchiveError::TrailingBytes(bytes.len() - pos));
        }

        Ok(Self {
            schema_version,
            slots,
        })
    }

    /// Computes the minimal incremental [`SignalDelta`] transforming `baseline` into `self`.
    pub fn diff_since(&self, baseline: &SignalTableSnapshot) -> SignalDelta {
        let mut ops = Vec::new();
        for (&slot_id, cur_val) in &self.slots {
            if baseline.slots.get(&slot_id) != Some(cur_val) {
                ops.push(SignalDeltaOp::Upsert {
                    slot_id,
                    value: cur_val.clone(),
                });
            }
        }
        for &old_id in baseline.slots.keys() {
            if !self.slots.contains_key(&old_id) {
                ops.push(SignalDeltaOp::Remove { slot_id: old_id });
            }
        }
        ops.sort_unstable_by_key(SignalDeltaOp::slot_id);
        SignalDelta {
            schema_version: self.schema_version,
            ops,
        }
    }

    /// Applies one decoded [`SignalDelta`] in place, returning the number of operations applied.
    pub fn apply_delta(&mut self, delta: &SignalDelta) -> usize {
        self.schema_version = delta.schema_version;
        for op in &delta.ops {
            match op {
                SignalDeltaOp::Upsert { slot_id, value } => {
                    self.slots.insert(*slot_id, value.clone());
                }
                SignalDeltaOp::Remove { slot_id } => {
                    self.slots.remove(slot_id);
                }
            }
        }
        delta.ops.len()
    }

    /// Decodes an `NVE_SGD1` delta byte slice and applies it to `self`.
    pub fn apply_delta_bytes(&mut self, bytes: &[u8]) -> Result<usize, SignalArchiveError> {
        let delta = SignalDelta::decode(bytes)?;
        Ok(self.apply_delta(&delta))
    }

    /// Replays all [`WalRecordKind::StateDelta`] records from `wal` in sequence order into `self`.
    ///
    /// Returns the number of WAL state-delta records applied.
    pub fn replay_wal(&mut self, wal: &SnapshotWal) -> Result<usize, SignalArchiveError> {
        let mut applied_records = 0usize;
        wal.replay_into(|rec| -> Result<(), SignalArchiveError> {
            if rec.kind == WalRecordKind::StateDelta {
                self.apply_delta_bytes(rec.payload)?;
                applied_records += 1;
            }
            Ok(())
        })?;
        Ok(applied_records)
    }
}

/// High-level stateless entry points for encoding, decoding, and folding `MmapSnapshot` + `SnapshotWal`
/// signal tables.
pub struct SignalArchive;

impl SignalArchive {
    #[inline]
    pub fn slot_id(component_path: &str, signal_name: &str) -> u64 {
        signal_slot_id(component_path, signal_name)
    }

    pub fn encode_snapshot(snapshot: &SignalTableSnapshot) -> Result<Vec<u8>, SignalArchiveError> {
        snapshot.encode()
    }

    pub fn decode_snapshot(bytes: &[u8]) -> Result<SignalTableSnapshot, SignalArchiveError> {
        SignalTableSnapshot::decode(bytes)
    }

    pub fn encode_delta(delta: &SignalDelta) -> Result<Vec<u8>, SignalArchiveError> {
        delta.encode()
    }

    pub fn decode_delta(bytes: &[u8]) -> Result<SignalDelta, SignalArchiveError> {
        SignalDelta::decode(bytes)
    }

    /// Decodes a base full signal table (or starts empty when `base_state_bytes` is empty)
    /// and folds every [`WalRecordKind::StateDelta`] record from `wal` on top.
    pub fn fold_with_wal(
        base_state_bytes: &[u8],
        wal: &SnapshotWal,
    ) -> Result<SignalTableSnapshot, SignalArchiveError> {
        let mut table = if base_state_bytes.is_empty() {
            SignalTableSnapshot::new(wal.header().base_schema_version)
        } else {
            SignalTableSnapshot::decode(base_state_bytes)?
        };
        table.replay_wal(wal)?;
        Ok(table)
    }

    /// Loads the signal table from an [`MmapSnapshot`] and optionally folds a companion [`SnapshotWal`].
    pub fn open_from_snapshot_and_wal(
        snapshot: &MmapSnapshot,
        wal: Option<&SnapshotWal>,
    ) -> Result<SignalTableSnapshot, SignalArchiveError> {
        let state = snapshot.state_bytes();
        let mut table = if state.is_empty() {
            SignalTableSnapshot::new(snapshot.app_schema_version())
        } else {
            SignalTableSnapshot::decode(state)?
        };
        if let Some(wal) = wal {
            table.replay_wal(wal)?;
        }
        Ok(table)
    }
}
