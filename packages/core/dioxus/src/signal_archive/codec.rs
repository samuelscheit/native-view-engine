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
        CodecError::BadEnum { raw, .. } => SignalArchiveError::InvalidValueTag(raw as u8),
        CodecError::BadVersion(v) => SignalArchiveError::UnsupportedFormatVersion(v),
        other @ (CodecError::BadMagic
        | CodecError::NonZeroReserved(_)
        | CodecError::DepthExceeded { .. }) => SignalArchiveError::Codec(other),
    }
}

struct ArchiveReader<'a> {
    inner: ByteReader<'a>,
    total_len: usize,
}

impl<'a> ArchiveReader<'a> {
    #[inline]
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            inner: ByteReader::new(bytes),
            total_len: bytes.len(),
        }
    }

    #[inline]
    fn map<T>(&self, res: Result<T, CodecError>) -> Result<T, SignalArchiveError> {
        res.map_err(|e| map_codec_err(e, self.total_len, self.inner.pos()))
    }

    #[inline]
    const fn remaining(&self) -> usize {
        self.inner.remaining()
    }

    #[inline]
    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], SignalArchiveError> {
        let res = self.inner.read_array::<N>();
        self.map(res)
    }

    #[inline]
    fn read_u8(&mut self) -> Result<u8, SignalArchiveError> {
        let res = self.inner.read_u8();
        self.map(res)
    }

    #[inline]
    fn read_bool(&mut self) -> Result<bool, SignalArchiveError> {
        let res = self.inner.read_bool();
        self.map(res)
    }

    #[inline]
    fn read_u32(&mut self) -> Result<u32, SignalArchiveError> {
        let res = self.inner.read_u32();
        self.map(res)
    }

    #[inline]
    fn read_i64(&mut self) -> Result<i64, SignalArchiveError> {
        let res = self.inner.read_i64();
        self.map(res)
    }

    #[inline]
    fn read_u64(&mut self) -> Result<u64, SignalArchiveError> {
        let res = self.inner.read_u64();
        self.map(res)
    }

    #[inline]
    fn read_f64_raw(&mut self) -> Result<f64, SignalArchiveError> {
        let res = self.inner.read_f64_raw();
        self.map(res)
    }

    #[inline]
    fn read_str(&mut self, max: usize) -> Result<&'a str, SignalArchiveError> {
        let res = self.inner.read_str(max);
        self.map(res)
    }

    #[inline]
    fn read_bytes(&mut self, max: usize) -> Result<&'a [u8], SignalArchiveError> {
        let res = self.inner.read_bytes(max);
        self.map(res)
    }

    #[inline]
    fn finish(self) -> Result<(), SignalArchiveError> {
        let pos = self.inner.pos();
        let total_len = self.total_len;
        self.inner
            .finish()
            .map_err(|e| map_codec_err(e, total_len, pos))
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

fn decode_value(r: &mut ArchiveReader<'_>) -> Result<SignalValue, SignalArchiveError> {
    let raw_tag = r.read_u8()?;
    let tag =
        SignalValueTag::from_repr(raw_tag).ok_or(SignalArchiveError::InvalidValueTag(raw_tag))?;
    match tag {
        SignalValueTag::Null => Ok(SignalValue::Null),
        SignalValueTag::Bool => Ok(SignalValue::Bool(r.read_bool()?)),
        SignalValueTag::I64 => Ok(SignalValue::I64(r.read_i64()?)),
        SignalValueTag::U64 => Ok(SignalValue::U64(r.read_u64()?)),
        SignalValueTag::F64 => Ok(SignalValue::F64(r.read_f64_raw()?)),
        SignalValueTag::Str => Ok(SignalValue::Str(r.read_str(u32::MAX as usize)?.to_owned())),
        SignalValueTag::Bytes => Ok(SignalValue::Bytes(r.read_bytes(u32::MAX as usize)?.to_vec())),
        SignalValueTag::BinValue => {
            let raw = r.read_bytes(u32::MAX as usize)?;
            let _ = BinValueView::parse(raw)?;
            Ok(SignalValue::BinValue(raw.to_vec()))
        }
    }
}

fn decode_header<'a>(
    bytes: &'a [u8],
    expected_magic: [u8; 8],
) -> Result<(u32, usize, ArchiveReader<'a>), SignalArchiveError> {
    if bytes.len() < HEADER_SIZE {
        return Err(SignalArchiveError::Truncated {
            expected: HEADER_SIZE,
            actual: bytes.len(),
        });
    }
    let mut r = ArchiveReader::new(bytes);
    let magic = r.read_array::<8>()?;
    if magic != expected_magic {
        return Err(SignalArchiveError::InvalidMagic(magic));
    }
    let format_version = r.read_u32()?;
    if format_version != NVE_SIGNAL_ARCHIVE_VERSION {
        return Err(SignalArchiveError::UnsupportedFormatVersion(format_version));
    }
    let schema_version = r.read_u32()?;
    let count = r.read_u32()? as usize;
    let stored_checksum = r.read_u64()?;
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
        w.write_zeroes(HEADER_SIZE)
            .map_err(|_| SignalArchiveError::PayloadTooLarge(HEADER_SIZE))?;
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
            let op_kind = r.read_u8()?;
            let slot_id = r.read_u64()?;
            if let Some(prev) = prev_slot
                && slot_id <= prev
            {
                return Err(SignalArchiveError::UnsortedOrDuplicateSlot(slot_id));
            }
            prev_slot = Some(slot_id);
            match op_kind {
                DELTA_OP_UPSERT => {
                    let value = decode_value(&mut r)?;
                    ops.push(SignalDeltaOp::Upsert { slot_id, value });
                }
                DELTA_OP_REMOVE => {
                    ops.push(SignalDeltaOp::Remove { slot_id });
                }
                other => return Err(SignalArchiveError::InvalidDeltaOp(other)),
            }
        }

        r.finish()?;

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
        w.write_zeroes(HEADER_SIZE)
            .map_err(|_| SignalArchiveError::PayloadTooLarge(HEADER_SIZE))?;
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
            let slot_id = r.read_u64()?;
            if let Some(prev) = prev_slot
                && slot_id <= prev
            {
                return Err(SignalArchiveError::UnsortedOrDuplicateSlot(slot_id));
            }
            prev_slot = Some(slot_id);
            let value = decode_value(&mut r)?;
            slots.insert(slot_id, value);
        }

        r.finish()?;

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
