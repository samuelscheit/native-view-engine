//! Binary container codec (`NVE_SIG1` / `NVE_SGD1`), snapshot/delta types, and typed `SignalCodec`.

use std::collections::BTreeMap;

use nve_abi::{BinValueView, Xxh64Hasher};
use nve_codec::{ByteReader, ByteWriter, CodecError};
use nve_engine::{NativeStateStore, StateSaveOutcome};

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
    h.digest()
}

#[inline]
fn map_codec_err(err: CodecError, total_len: usize, pos: usize) -> SignalArchiveError {
    match err {
        CodecError::Truncated { needed, .. } => SignalArchiveError::Truncated {
            expected: pos.saturating_add(needed),
            actual: total_len,
        },
        CodecError::BadBool(b) => SignalArchiveError::InvalidBool(b),
        CodecError::InvalidUtf8 => SignalArchiveError::InvalidUtf8,
        CodecError::TrailingBytes(n) => SignalArchiveError::TrailingBytes(n),
        CodecError::LenOverflow { len, .. } => SignalArchiveError::PayloadTooLarge(len),
        _ => SignalArchiveError::Truncated {
            expected: pos,
            actual: total_len,
        },
    }
}

fn encode_value(value: &SignalValue, w: &mut ByteWriter) -> Result<(), SignalArchiveError> {
    w.write_u8(value.tag() as u8);
    match value {
        SignalValue::Null => {}
        SignalValue::Bool(b) => w.write_bool(*b),
        SignalValue::I64(v) => w.write_i64(*v),
        SignalValue::U64(v) => w.write_u64(*v),
        SignalValue::F64(v) => w.write_f64_raw(*v),
        SignalValue::Str(s) => w
            .write_str(s)
            .map_err(|_| SignalArchiveError::PayloadTooLarge(s.len()))?,
        SignalValue::Bytes(b) => w
            .write_bytes(b)
            .map_err(|_| SignalArchiveError::PayloadTooLarge(b.len()))?,
        SignalValue::BinValue(b) => {
            let _ = BinValueView::parse(b)?;
            w.write_bytes(b)
                .map_err(|_| SignalArchiveError::PayloadTooLarge(b.len()))?;
        }
    }
    Ok(())
}

fn decode_value(r: &mut ByteReader<'_>, total_len: usize) -> Result<SignalValue, SignalArchiveError> {
    let raw_tag = r
        .read_u8()
        .map_err(|e| map_codec_err(e, total_len, r.pos()))?;
    let tag =
        SignalValueTag::from_u8(raw_tag).ok_or(SignalArchiveError::InvalidValueTag(raw_tag))?;
    match tag {
        SignalValueTag::Null => Ok(SignalValue::Null),
        SignalValueTag::Bool => Ok(SignalValue::Bool(
            r.read_bool()
                .map_err(|e| map_codec_err(e, total_len, r.pos()))?,
        )),
        SignalValueTag::I64 => Ok(SignalValue::I64(
            r.read_i64()
                .map_err(|e| map_codec_err(e, total_len, r.pos()))?,
        )),
        SignalValueTag::U64 => Ok(SignalValue::U64(
            r.read_u64()
                .map_err(|e| map_codec_err(e, total_len, r.pos()))?,
        )),
        SignalValueTag::F64 => Ok(SignalValue::F64(
            r.read_f64_raw()
                .map_err(|e| map_codec_err(e, total_len, r.pos()))?,
        )),
        SignalValueTag::Str => {
            let s = r
                .read_str(u32::MAX as usize)
                .map_err(|e| map_codec_err(e, total_len, r.pos()))?;
            Ok(SignalValue::Str(s.to_owned()))
        }
        SignalValueTag::Bytes => {
            let raw = r
                .read_bytes(u32::MAX as usize)
                .map_err(|e| map_codec_err(e, total_len, r.pos()))?;
            Ok(SignalValue::Bytes(raw.to_vec()))
        }
        SignalValueTag::BinValue => {
            let raw = r
                .read_bytes(u32::MAX as usize)
                .map_err(|e| map_codec_err(e, total_len, r.pos()))?;
            let _ = BinValueView::parse(raw)?;
            Ok(SignalValue::BinValue(raw.to_vec()))
        }
    }
}

fn decode_header<'a>(
    bytes: &'a [u8],
    expected_magic: [u8; 8],
) -> Result<(u32, usize, ByteReader<'a>), SignalArchiveError> {
    if bytes.len() < HEADER_SIZE {
        return Err(SignalArchiveError::Truncated {
            expected: HEADER_SIZE,
            actual: bytes.len(),
        });
    }
    let mut r = ByteReader::new(bytes);
    let magic = r
        .read_array::<8>()
        .map_err(|e| map_codec_err(e, bytes.len(), 0))?;
    if magic != expected_magic {
        return Err(SignalArchiveError::InvalidMagic(magic));
    }
    let format_version = r
        .read_u32()
        .map_err(|e| map_codec_err(e, bytes.len(), r.pos()))?;
    if format_version != NVE_SIGNAL_ARCHIVE_VERSION {
        return Err(SignalArchiveError::UnsupportedFormatVersion(format_version));
    }
    let schema_version = r
        .read_u32()
        .map_err(|e| map_codec_err(e, bytes.len(), r.pos()))?;
    let count = r
        .read_u32()
        .map_err(|e| map_codec_err(e, bytes.len(), r.pos()))? as usize;
    let stored_checksum = r
        .read_u64()
        .map_err(|e| map_codec_err(e, bytes.len(), r.pos()))?;
    let computed_checksum = compute_container_checksum(bytes);
    if computed_checksum != stored_checksum {
        return Err(SignalArchiveError::ChecksumMismatch {
            expected: stored_checksum,
            actual: computed_checksum,
        });
    }
    Ok((schema_version, count, r))
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

/// Incremental dirty-slot delta for lightweight differential state updates.
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

    /// Encodes this delta into a checksummed, binary FFI `NVE_SGD1` binary payload.
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
        let mut w = ByteWriter::with_capacity(HEADER_SIZE + sorted_ops.len() * 16);
        w.write_zeroes(HEADER_SIZE);
        for op in &sorted_ops {
            match op {
                SignalDeltaOp::Upsert { slot_id, value } => {
                    w.write_u8(DELTA_OP_UPSERT);
                    w.write_u64(*slot_id);
                    encode_value(value, &mut w)?;
                }
                SignalDeltaOp::Remove { slot_id } => {
                    w.write_u8(DELTA_OP_REMOVE);
                    w.write_u64(*slot_id);
                }
            }
        }
        write_header(
            w.as_mut_slice(),
            NVE_SIGNAL_DELTA_MAGIC,
            self.schema_version,
            op_count,
        );
        Ok(w.into_bytes())
    }

    /// Decodes and validates an `NVE_SGD1` binary delta payload.
    pub fn decode(bytes: &[u8]) -> Result<Self, SignalArchiveError> {
        let (schema_version, op_count, mut r) = decode_header(bytes, NVE_SIGNAL_DELTA_MAGIC)?;
        let mut ops = Vec::with_capacity(op_count.min(r.remaining() / 9));
        let mut prev_slot: Option<u64> = None;

        for _ in 0..op_count {
            let op_kind = r
                .read_u8()
                .map_err(|e| map_codec_err(e, bytes.len(), r.pos()))?;
            let slot_id = r
                .read_u64()
                .map_err(|e| map_codec_err(e, bytes.len(), r.pos()))?;
            if let Some(prev) = prev_slot
                && slot_id <= prev
            {
                return Err(SignalArchiveError::UnsortedOrDuplicateSlot(slot_id));
            }
            prev_slot = Some(slot_id);
            match op_kind {
                DELTA_OP_UPSERT => {
                    let value = decode_value(&mut r, bytes.len())?;
                    ops.push(SignalDeltaOp::Upsert { slot_id, value });
                }
                DELTA_OP_REMOVE => {
                    ops.push(SignalDeltaOp::Remove { slot_id });
                }
                other => return Err(SignalArchiveError::InvalidDeltaOp(other)),
            }
        }

        let end_pos = r.pos();
        r.finish()
            .map_err(|e| map_codec_err(e, bytes.len(), end_pos))?;

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

    /// Encodes the full signal table into a checksummed, binary FFI `NVE_SIG1` binary buffer
    /// suitable for [`nve_engine::NativeStateStore`] (`MmapOffsetArena`) persistence.
    pub fn encode(&self) -> Result<Vec<u8>, SignalArchiveError> {
        let slot_count = u32::try_from(self.slots.len())
            .map_err(|_| SignalArchiveError::PayloadTooLarge(self.slots.len()))?;
        let mut w = ByteWriter::with_capacity(HEADER_SIZE + self.slots.len() * 18);
        w.write_zeroes(HEADER_SIZE);
        for (&slot_id, value) in &self.slots {
            w.write_u64(slot_id);
            encode_value(value, &mut w)?;
        }
        write_header(
            w.as_mut_slice(),
            NVE_SIGNAL_ARCHIVE_MAGIC,
            self.schema_version,
            slot_count,
        );
        Ok(w.into_bytes())
    }

    /// Decodes and validates a full `NVE_SIG1` binary archive.
    pub fn decode(bytes: &[u8]) -> Result<Self, SignalArchiveError> {
        let (schema_version, slot_count, mut r) = decode_header(bytes, NVE_SIGNAL_ARCHIVE_MAGIC)?;
        let mut slots = BTreeMap::new();
        let mut prev_slot: Option<u64> = None;

        for _ in 0..slot_count {
            let slot_id = r
                .read_u64()
                .map_err(|e| map_codec_err(e, bytes.len(), r.pos()))?;
            if let Some(prev) = prev_slot
                && slot_id <= prev
            {
                return Err(SignalArchiveError::UnsortedOrDuplicateSlot(slot_id));
            }
            prev_slot = Some(slot_id);
            let value = decode_value(&mut r, bytes.len())?;
            slots.insert(slot_id, value);
        }

        let end_pos = r.pos();
        r.finish()
            .map_err(|e| map_codec_err(e, bytes.len(), end_pos))?;

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
}

/// High-level stateless entry points for encoding, decoding, and persisting signal tables into
/// [`NativeStateStore`] (`MmapOffsetArena`).
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

    /// Crash-atomically persists `snapshot` into a double-buffered [`NativeStateStore`].
    pub fn save_to_store(
        store: &mut NativeStateStore,
        snapshot: &SignalTableSnapshot,
    ) -> Result<StateSaveOutcome, SignalArchiveError> {
        let bytes = snapshot.encode()?;
        Ok(store.save(&bytes)?)
    }

    /// Loads the latest committed signal table directly from a [`NativeStateStore`].
    pub fn load_from_store(
        store: &NativeStateStore,
    ) -> Result<SignalTableSnapshot, SignalArchiveError> {
        match store.load_bytes()? {
            Some(bytes) if !bytes.is_empty() => SignalTableSnapshot::decode(bytes),
            _ => Ok(SignalTableSnapshot::new(store.schema_version())),
        }
    }
}
