//! Zero-JSON binary Dioxus Signal Table Archiver & incremental WAL delta codec.
//!
//! Stores typed signal slots keyed by deterministic 64-bit FNV-1a hashes of
//! `(component_path, signal_name)` so `DioxusSurface` apps can snapshot state into
//! [`nve_engine::MmapSnapshot::state_bytes`], stream incremental dirty-slot deltas into
//! [`nve_engine::SnapshotWal`], compact deterministically, and hydrate safely across app
//! upgrades (tolerating added, removed, or re-typed signal slots without crashing).
//!
//! # Binary Layouts (little-endian, FNV-1a 64-bit checksummed)
//!
//! ## Full Snapshot (`NVE_SIG1`, 28-byte header)
//! - `0..8`:   `magic = *b"NVE_SIG1"`
//! - `8..12`:  `format_version: u32 = 1`
//! - `12..16`: `schema_version: u32`
//! - `16..20`: `slot_count: u32`
//! - `20..28`: `checksum_fnv64: u64` (over `0..20` + `28..len`)
//! - Followed by `slot_count` strictly sorted entries: `slot_id: u64`, `tag: u8`, value payload.
//!
//! ## Incremental Delta (`NVE_SGD1`, 28-byte header)
//! - `0..8`:   `magic = *b"NVE_SGD1"`
//! - `8..12`:  `format_version: u32 = 1`
//! - `12..16`: `schema_version: u32`
//! - `16..20`: `op_count: u32`
//! - `20..28`: `checksum_fnv64: u64` (over `0..20` + `28..len`)
//! - Followed by `op_count` strictly sorted ops: `op_kind: u8` (`1=Upsert`, `2=Remove`),
//!   `slot_id: u64`, and (for `Upsert`) `tag: u8` + value payload.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;

use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_abi::{encode_bin_value_to_vec, BinValueView, DecodeError, NveValueRef};
use nve_engine::snapshot::fnv1a_64_update;
use nve_engine::{MmapSnapshot, SnapshotHeader, SnapshotWal, WalError, WalRecordKind};

/// 8-byte magic identifying a full binary signal table archive.
pub const NVE_SIGNAL_ARCHIVE_MAGIC: [u8; 8] = *b"NVE_SIG1";

/// 8-byte magic identifying an incremental dirty-slot signal delta.
pub const NVE_SIGNAL_DELTA_MAGIC: [u8; 8] = *b"NVE_SGD1";

/// Format version for both `NVE_SIG1` and `NVE_SGD1` containers.
pub const NVE_SIGNAL_ARCHIVE_VERSION: u32 = 1;

const HEADER_SIZE: usize = 28;
const CHECKSUM_OFFSET: usize = 20;
const CHECKSUM_END: usize = 28;
const FNV1A_64_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

const DELTA_OP_UPSERT: u8 = 1;
const DELTA_OP_REMOVE: u8 = 2;

/// Computes the stable 64-bit FNV-1a slot identifier for `(component_path, signal_name)`.
///
/// Uses an explicit NUL separator between `component_path` and `signal_name` so
/// `("app/nav", "tab")` and `("app/navt", "ab")` never collide.
#[inline]
pub fn signal_slot_id(component_path: &str, signal_name: &str) -> u64 {
    let h = fnv1a_64_update(FNV1A_64_OFFSET_BASIS, component_path.as_bytes());
    let h = fnv1a_64_update(h, &[0u8]);
    fnv1a_64_update(h, signal_name.as_bytes())
}

/// Discriminant tag for archived signal values.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalValueTag {
    Null = 0,
    Bool = 1,
    I64 = 2,
    U64 = 3,
    F64 = 4,
    Str = 5,
    Bytes = 6,
    BinValue = 7,
}

impl SignalValueTag {
    #[inline]
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Null),
            1 => Some(Self::Bool),
            2 => Some(Self::I64),
            3 => Some(Self::U64),
            4 => Some(Self::F64),
            5 => Some(Self::Str),
            6 => Some(Self::Bytes),
            7 => Some(Self::BinValue),
            _ => None,
        }
    }
}

/// Typed value stored in one archived signal slot.
#[derive(Debug, Clone)]
pub enum SignalValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
    /// Compact binary `NveBinValue` payload validated by [`nve_abi::BinValueView::parse`].
    BinValue(Vec<u8>),
}

impl PartialEq for SignalValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::I64(a), Self::I64(b)) => a == b,
            (Self::U64(a), Self::U64(b)) => a == b,
            // Bitwise equality ensures NaN and -0.0/+0.0 diff deterministically.
            (Self::F64(a), Self::F64(b)) => a.to_bits() == b.to_bits(),
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::BinValue(a), Self::BinValue(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for SignalValue {}

impl SignalValue {
    #[inline]
    pub const fn tag(&self) -> SignalValueTag {
        match self {
            Self::Null => SignalValueTag::Null,
            Self::Bool(_) => SignalValueTag::Bool,
            Self::I64(_) => SignalValueTag::I64,
            Self::U64(_) => SignalValueTag::U64,
            Self::F64(_) => SignalValueTag::F64,
            Self::Str(_) => SignalValueTag::Str,
            Self::Bytes(_) => SignalValueTag::Bytes,
            Self::BinValue(_) => SignalValueTag::BinValue,
        }
    }

    /// Encodes a zero-copy `nve_abi::NveValueRef` into [`SignalValue::BinValue`].
    pub fn from_nve_value(value: NveValueRef<'_>) -> Result<Self, SignalArchiveError> {
        let encoded = encode_bin_value_to_vec(value)?;
        Ok(Self::BinValue(encoded))
    }

    /// Wraps raw binary `NveBinValue` bytes after validating structure via [`BinValueView::parse`].
    pub fn from_bin_bytes(bytes: Vec<u8>) -> Result<Self, SignalArchiveError> {
        let _ = BinValueView::parse(&bytes)?;
        Ok(Self::BinValue(bytes))
    }

    /// Returns a zero-copy [`BinValueView`] when this slot holds a [`SignalValue::BinValue`].
    pub fn as_bin_value(&self) -> Result<Option<BinValueView<'_>>, SignalArchiveError> {
        match self {
            Self::BinValue(bytes) => Ok(Some(BinValueView::parse(bytes)?)),
            _ => Ok(None),
        }
    }
}

/// Errors produced when encoding, decoding, validating, or replaying signal archives.
#[derive(Debug)]
pub enum SignalArchiveError {
    Truncated {
        expected: usize,
        actual: usize,
    },
    InvalidMagic([u8; 8]),
    UnsupportedFormatVersion(u32),
    ChecksumMismatch {
        expected: u64,
        actual: u64,
    },
    InvalidValueTag(u8),
    InvalidBool(u8),
    InvalidDeltaOp(u8),
    InvalidUtf8,
    InvalidBinValue(DecodeError),
    UnsortedOrDuplicateSlot(u64),
    TrailingBytes(usize),
    PayloadTooLarge(usize),
    Wal(WalError),
}

impl From<DecodeError> for SignalArchiveError {
    fn from(err: DecodeError) -> Self {
        Self::InvalidBinValue(err)
    }
}

impl From<WalError> for SignalArchiveError {
    fn from(err: WalError) -> Self {
        Self::Wal(err)
    }
}

impl fmt::Display for SignalArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { expected, actual } => {
                write!(f, "truncated signal archive: expected at least {expected} bytes, got {actual}")
            }
            Self::InvalidMagic(m) => write!(f, "invalid signal archive magic: {m:?}"),
            Self::UnsupportedFormatVersion(v) => {
                write!(f, "unsupported signal archive format version: {v}")
            }
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "signal archive checksum mismatch: expected {expected:#018x}, computed {actual:#018x}"
            ),
            Self::InvalidValueTag(t) => write!(f, "invalid signal value tag: {t}"),
            Self::InvalidBool(b) => write!(f, "invalid signal bool byte: {b}"),
            Self::InvalidDeltaOp(op) => write!(f, "invalid signal delta op: {op}"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 string in signal archive"),
            Self::InvalidBinValue(e) => write!(f, "invalid NveBinValue in signal archive: {e:?}"),
            Self::UnsortedOrDuplicateSlot(id) => {
                write!(f, "unsorted or duplicate signal slot_id {id:#018x}")
            }
            Self::TrailingBytes(n) => write!(f, "signal archive has {n} trailing bytes"),
            Self::PayloadTooLarge(n) => write!(f, "signal payload length {n} exceeds u32::MAX"),
            Self::Wal(e) => write!(f, "signal archive WAL error: {e}"),
        }
    }
}

impl std::error::Error for SignalArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Wal(e) => Some(e),
            _ => None,
        }
    }
}

#[inline]
fn compute_container_checksum(bytes: &[u8]) -> u64 {
    let h = fnv1a_64_update(FNV1A_64_OFFSET_BASIS, &bytes[0..CHECKSUM_OFFSET]);
    fnv1a_64_update(h, &bytes[CHECKSUM_END..])
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
fn read_slice<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], SignalArchiveError> {
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
            let len = u32::try_from(b.len()).map_err(|_| SignalArchiveError::PayloadTooLarge(b.len()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
        SignalValue::Bytes(b) => {
            let len = u32::try_from(b.len()).map_err(|_| SignalArchiveError::PayloadTooLarge(b.len()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
        SignalValue::BinValue(b) => {
            let _ = BinValueView::parse(b)?;
            let len = u32::try_from(b.len()).map_err(|_| SignalArchiveError::PayloadTooLarge(b.len()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
    }
    Ok(())
}

fn decode_value(bytes: &[u8], pos: &mut usize) -> Result<SignalValue, SignalArchiveError> {
    let raw_tag = read_u8(bytes, pos)?;
    let tag = SignalValueTag::from_u8(raw_tag).ok_or(SignalArchiveError::InvalidValueTag(raw_tag))?;
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
    let stored_checksum = u64::from_le_bytes(bytes[CHECKSUM_OFFSET..CHECKSUM_END].try_into().unwrap());
    let computed_checksum = compute_container_checksum(bytes);
    if computed_checksum != stored_checksum {
        return Err(SignalArchiveError::ChecksumMismatch {
            expected: stored_checksum,
            actual: computed_checksum,
        });
    }
    Ok((schema_version, count))
}

fn write_header(out: &mut [u8], magic: [u8; 8], schema_version: u32, count: u32) {
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
                return Err(SignalArchiveError::UnsortedOrDuplicateSlot(pair[0].slot_id()));
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
        write_header(&mut buf, NVE_SIGNAL_DELTA_MAGIC, self.schema_version, op_count);
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
            if let Some(prev) = prev_slot {
                if slot_id <= prev {
                    return Err(SignalArchiveError::UnsortedOrDuplicateSlot(slot_id));
                }
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
        write_header(&mut buf, NVE_SIGNAL_ARCHIVE_MAGIC, self.schema_version, slot_count);
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
            if let Some(prev) = prev_slot {
                if slot_id <= prev {
                    return Err(SignalArchiveError::UnsortedOrDuplicateSlot(slot_id));
                }
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

/// Typed conversion between Rust signal types and zero-JSON [`SignalValue`].
pub trait SignalCodec: Sized {
    fn to_signal_value(&self) -> SignalValue;
    fn from_signal_value(value: &SignalValue) -> Option<Self>;
}

/// Wrapper type for storing validated `NveBinValue` bytes inside a [`SignalRegistry`] slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinValueSlot(pub Vec<u8>);

impl SignalCodec for SignalValue {
    fn to_signal_value(&self) -> SignalValue {
        self.clone()
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        Some(value.clone())
    }
}

impl SignalCodec for bool {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::Bool(*self)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::Bool(v) => Some(*v),
            _ => None,
        }
    }
}

impl SignalCodec for i64 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(*self)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => Some(*v),
            _ => None,
        }
    }
}

impl SignalCodec for i32 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(i64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => i32::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for i16 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(i64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => i16::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for i8 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(i64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => i8::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for isize {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::I64(*self as i64)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::I64(v) => isize::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for u64 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(*self)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => Some(*v),
            _ => None,
        }
    }
}

impl SignalCodec for u32 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(u64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => u32::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for u16 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(u64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => u16::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for u8 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(u64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => u8::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for usize {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::U64(*self as u64)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::U64(v) => usize::try_from(*v).ok(),
            _ => None,
        }
    }
}

impl SignalCodec for f64 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::F64(*self)
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::F64(v) => Some(*v),
            _ => None,
        }
    }
}

impl SignalCodec for f32 {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::F64(f64::from(*self))
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::F64(v) => Some(*v as f32),
            _ => None,
        }
    }
}

impl SignalCodec for String {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::Str(self.clone())
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
}

impl SignalCodec for Vec<u8> {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::Bytes(self.clone())
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::Bytes(b) => Some(b.clone()),
            _ => None,
        }
    }
}

impl SignalCodec for BinValueSlot {
    fn to_signal_value(&self) -> SignalValue {
        SignalValue::BinValue(self.0.clone())
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::BinValue(b) => Some(Self(b.clone())),
            _ => None,
        }
    }
}

impl<T: SignalCodec> SignalCodec for Option<T> {
    fn to_signal_value(&self) -> SignalValue {
        match self {
            Some(v) => v.to_signal_value(),
            None => SignalValue::Null,
        }
    }

    fn from_signal_value(value: &SignalValue) -> Option<Self> {
        match value {
            SignalValue::Null => Some(None),
            other => T::from_signal_value(other).map(Some),
        }
    }
}

/// Summary returned by [`SignalRegistry::hydrate_from_table`] describing exact schema-evolution
/// behavior on cold start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HydrationReport {
    /// Registered slots restored successfully from the archive.
    pub hydrated_slots: usize,
    /// Archived slots absent from the current app version (safely ignored).
    pub skipped_unknown_slots: usize,
    /// Archived slots whose stored value type did not match the current slot's type (safely skipped).
    pub skipped_type_mismatches: usize,
    /// Newly added registered slots absent from the archive (retained their default values).
    pub untouched_new_slots: usize,
    /// Schema version recorded in the hydrated archive.
    pub source_schema_version: u32,
}

struct RegisteredSlot {
    slot_id: u64,
    component_path: String,
    signal_name: String,
    read: Box<dyn Fn() -> SignalValue>,
    write: Box<dyn Fn(&SignalValue) -> bool>,
}

/// App/surface-scoped signal table registry for one-time signal registration, deterministic
/// full & incremental snapshotting, WAL delta streaming, and schema-tolerant cold hydration.
pub struct SignalRegistry {
    schema_version: u32,
    slots: BTreeMap<u64, RegisteredSlot>,
    last_persisted: SignalTableSnapshot,
}

impl Default for SignalRegistry {
    fn default() -> Self {
        Self::new(1)
    }
}

impl SignalRegistry {
    pub fn new(schema_version: u32) -> Self {
        Self {
            schema_version,
            slots: BTreeMap::new(),
            last_persisted: SignalTableSnapshot::new(schema_version),
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

    /// Registers a typed signal backed by custom getter/setter closures.
    pub fn register_slot<T: SignalCodec + 'static>(
        &mut self,
        component_path: impl Into<String>,
        signal_name: impl Into<String>,
        read: impl Fn() -> T + 'static,
        write: impl Fn(T) + 'static,
    ) -> u64 {
        let component_path = component_path.into();
        let signal_name = signal_name.into();
        let slot_id = signal_slot_id(&component_path, &signal_name);
        self.slots.insert(
            slot_id,
            RegisteredSlot {
                slot_id,
                component_path,
                signal_name,
                read: Box::new(move || read().to_signal_value()),
                write: Box::new(move |val| {
                    if let Some(decoded) = T::from_signal_value(val) {
                        write(decoded);
                        true
                    } else {
                        false
                    }
                }),
            },
        );
        slot_id
    }

    /// Registers a live `dioxus_signals::Signal<T>` directly for snapshot/WAL persistence and
    /// schema-safe cold hydration.
    pub fn register_signal<T: SignalCodec + Clone + 'static>(
        &mut self,
        component_path: impl Into<String>,
        signal_name: impl Into<String>,
        signal: Signal<T>,
    ) -> u64 {
        self.register_slot(
            component_path,
            signal_name,
            move || signal.cloned(),
            move |val| {
                let mut sig = signal;
                sig.set(val);
            },
        )
    }

    /// Unregisters a slot by `(component_path, signal_name)`.
    pub fn unregister(&mut self, component_path: &str, signal_name: &str) -> bool {
        self.slots
            .remove(&signal_slot_id(component_path, signal_name))
            .is_some()
    }

    /// Returns `(slot_id, component_path, signal_name)` for all registered slots.
    pub fn registered_slots(&self) -> Vec<(u64, &str, &str)> {
        self.slots
            .values()
            .map(|s| (s.slot_id, s.component_path.as_str(), s.signal_name.as_str()))
            .collect()
    }

    /// Captures all currently registered signals into a deterministic [`SignalTableSnapshot`].
    pub fn capture(&self) -> SignalTableSnapshot {
        let mut snap = SignalTableSnapshot::new(self.schema_version);
        for (&slot_id, entry) in &self.slots {
            snap.set(slot_id, (entry.read)());
        }
        snap
    }

    /// Encodes a full `NVE_SIG1` snapshot of all registered signals for `MmapSnapshot.state_bytes()`.
    pub fn encode_full(&self) -> Result<Vec<u8>, SignalArchiveError> {
        self.capture().encode()
    }

    /// Captures current signal state and marks it as the clean baseline for future incremental diffs.
    pub fn mark_baseline(&mut self) -> SignalTableSnapshot {
        let current = self.capture();
        self.last_persisted = current.clone();
        current
    }

    /// Computes dirty slots since the last persisted baseline and returns `Some(delta_bytes)` if
    /// any registered slot changed, advancing the baseline.
    pub fn encode_delta_since_last(&mut self) -> Result<Option<Vec<u8>>, SignalArchiveError> {
        let current = self.capture();
        let delta = current.diff_since(&self.last_persisted);
        if delta.is_empty() {
            return Ok(None);
        }
        let encoded = delta.encode()?;
        self.last_persisted = current;
        Ok(Some(encoded))
    }

    /// Appends an incremental dirty-slot delta to `wal` when any signal changed since last persist.
    ///
    /// Returns `Ok(Some(seq))` if a delta record was appended, or `Ok(None)` if clean.
    pub fn persist_delta_to_wal(&mut self, wal: &mut SnapshotWal) -> Result<Option<u64>, SignalArchiveError> {
        let current = self.capture();
        let delta = current.diff_since(&self.last_persisted);
        if delta.is_empty() {
            return Ok(None);
        }
        let encoded = delta.encode()?;
        let seq = wal.append_delta(WalRecordKind::StateDelta, &encoded)?;
        self.last_persisted = current;
        Ok(Some(seq))
    }

    /// Captures the full signal table, atomically compacts Frame-0 + state into `snapshot_path`,
    /// rotates `wal_path`, and refreshes the incremental diff baseline.
    pub fn compact_to_snapshot(
        &mut self,
        snapshot_path: impl AsRef<Path>,
        wal_path: impl AsRef<Path>,
        frame0_bytes: &[u8],
        metadata_bytes: &[u8],
    ) -> Result<(SnapshotHeader, SnapshotWal), SignalArchiveError> {
        let current = self.capture();
        let state_bytes = current.encode()?;
        let res = SnapshotWal::compact_into_snapshot(
            snapshot_path,
            wal_path,
            self.schema_version,
            frame0_bytes,
            &state_bytes,
            metadata_bytes,
        )?;
        self.last_persisted = current;
        Ok(res)
    }

    /// Hydrates all matching registered signals from `table` with strict schema-evolution safety:
    /// unknown/removed slots and re-typed slots are safely skipped instead of failing boot.
    pub fn hydrate_from_table(&mut self, table: &SignalTableSnapshot) -> HydrationReport {
        let mut report = HydrationReport {
            source_schema_version: table.schema_version(),
            ..Default::default()
        };
        let mut seen_registered = HashSet::with_capacity(self.slots.len());

        for (slot_id, value) in table.iter() {
            if let Some(entry) = self.slots.get(&slot_id) {
                seen_registered.insert(slot_id);
                if (entry.write)(value) {
                    report.hydrated_slots += 1;
                } else {
                    report.skipped_type_mismatches += 1;
                }
            } else {
                report.skipped_unknown_slots += 1;
            }
        }

        report.untouched_new_slots = self.slots.len().saturating_sub(seen_registered.len());
        self.last_persisted = self.capture();
        report
    }

    /// Decodes a full `NVE_SIG1` byte slice and hydrates matching registered signals.
    pub fn hydrate_from_bytes(&mut self, bytes: &[u8]) -> Result<HydrationReport, SignalArchiveError> {
        let table = SignalTableSnapshot::decode(bytes)?;
        Ok(self.hydrate_from_table(&table))
    }

    /// Loads state from an [`MmapSnapshot`], folds any companion [`SnapshotWal`] state deltas on
    /// top, and hydrates all matching registered signals deterministically.
    pub fn hydrate_from_snapshot_and_wal(
        &mut self,
        snapshot: &MmapSnapshot,
        wal: Option<&SnapshotWal>,
    ) -> Result<HydrationReport, SignalArchiveError> {
        let table = SignalArchive::open_from_snapshot_and_wal(snapshot, wal)?;
        Ok(self.hydrate_from_table(&table))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{NveMapEntry, NveValue};
    use nve_engine::write_snapshot_atomic;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let n = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "nve_sig_archive_{label}_{}_{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn roundtrip_all_signal_value_types_and_bin_value() {
        let map_entries = [
            NveMapEntry::new("theme", NveValue::from_str("midnight")),
            NveMapEntry::new("zoom", NveValue::f64(1.25)),
            NveMapEntry::new("count", NveValue::i64(-42)),
        ];
        let nve_val = NveValue::map(&map_entries);
        let bin_val = SignalValue::from_nve_value(unsafe { nve_val.as_ref() }).unwrap();

        let mut table = SignalTableSnapshot::new(7);
        table.set_named("Root", "empty", SignalValue::Null);
        table.set_named("Root", "dark_mode", SignalValue::Bool(true));
        table.set_named("Counter", "delta", SignalValue::I64(-9_876_543_210));
        table.set_named("Counter", "generation", SignalValue::U64(u64::MAX - 7));
        table.set_named("Canvas", "scale", SignalValue::F64(3.141_592_653_589_793));
        table.set_named("Canvas", "nan_guard", SignalValue::F64(f64::NAN));
        table.set_named("Editor", "title", SignalValue::Str("Aether Studio 🚀".into()));
        table.set_named("Editor", "raw", SignalValue::Bytes(vec![0, 1, 2, 254, 255]));
        let bin_id = table.set_named("Settings", "prefs", bin_val);

        let encoded = SignalArchive::encode_snapshot(&table).unwrap();
        assert_eq!(&encoded[0..8], &NVE_SIGNAL_ARCHIVE_MAGIC);

        let decoded = SignalArchive::decode_snapshot(&encoded).unwrap();
        assert_eq!(decoded, table);
        assert_eq!(decoded.schema_version(), 7);

        let view = decoded.get(bin_id).unwrap().as_bin_value().unwrap().unwrap();
        assert_eq!(view.get("theme").and_then(|v| v.as_str()), Some("midnight"));
        assert_eq!(view.get("zoom").and_then(|v| v.as_f64()), Some(1.25));
        assert_eq!(view.get("count").and_then(|v| v.as_i64()), Some(-42));

        // Slot ID separator prevents prefix/suffix collisions.
        assert_ne!(
            signal_slot_id("app/nav", "tab"),
            signal_slot_id("app/navt", "ab")
        );
    }

    #[test]
    fn incremental_delta_folding_and_compaction_equivalence() {
        let dir = TempDir::new("compact_eq");
        let snap_path = dir.path().join("state.nvesnap");
        let wal_path = dir.path().join("state.nvewal");

        let mut base = SignalTableSnapshot::new(2);
        base.set_named("App", "count", SignalValue::I64(1));
        base.set_named("App", "tab", SignalValue::Str("board".into()));
        base.set_named("App", "ephemeral", SignalValue::Bool(true));

        let base_bytes = base.encode().unwrap();
        let snap_hdr = write_snapshot_atomic(&snap_path, 2, 0, b"FRAME0_BASE", &base_bytes, b"")
            .unwrap();
        let mut wal = SnapshotWal::open_or_create(&wal_path, 2, snap_hdr.checksum_fnv64).unwrap();

        // Delta 1: mutate count, add filter.
        let mut step1 = base.clone();
        step1.set_named("App", "count", SignalValue::I64(2));
        step1.set_named("App", "filter", SignalValue::Str("open".into()));
        let d1 = step1.diff_since(&base);
        assert_eq!(d1.len(), 2);
        wal.append_delta(WalRecordKind::StateDelta, &d1.encode().unwrap())
            .unwrap();

        // Delta 2: mutate count again, remove ephemeral, update tab.
        let mut step2 = step1.clone();
        step2.set_named("App", "count", SignalValue::I64(42));
        step2.set_named("App", "tab", SignalValue::Str("metrics".into()));
        step2.remove(signal_slot_id("App", "ephemeral"));
        let d2 = step2.diff_since(&step1);
        assert_eq!(d2.len(), 3);
        wal.append_delta(WalRecordKind::StateDelta, &d2.encode().unwrap())
            .unwrap();

        // Unchanged diff produces empty delta.
        assert!(step2.diff_since(&step2).is_empty());

        // Fold base + WAL and compare against direct full snapshot compaction.
        let base_mmap = MmapSnapshot::open_with_schema(&snap_path, 2).unwrap();
        let folded = SignalArchive::open_from_snapshot_and_wal(&base_mmap, Some(&wal)).unwrap();
        assert_eq!(folded, step2);

        let compacted_state = folded.encode().unwrap();
        let (_new_hdr, rotated_wal) = SnapshotWal::compact_into_snapshot(
            &snap_path,
            &wal_path,
            2,
            b"FRAME0_COMPACTED",
            &compacted_state,
            b"",
        )
        .unwrap();

        let compacted_mmap = MmapSnapshot::open_with_schema(&snap_path, 2).unwrap();
        let from_compacted =
            SignalArchive::open_from_snapshot_and_wal(&compacted_mmap, Some(&rotated_wal)).unwrap();
        assert_eq!(from_compacted, folded);
        assert_eq!(from_compacted, step2);
    }

    #[test]
    fn registry_handles_schema_evolution_added_removed_and_retyped_slots() {
        let dir = TempDir::new("schema_evo");
        let snap_path = dir.path().join("evo.nvesnap");
        let wal_path = dir.path().join("evo.nvewal");

        // App v1 state:
        let v1_count = Rc::new(RefCell::new(10i32));
        let v1_theme = Rc::new(RefCell::new("dark".to_string()));
        let v1_legacy_flag = Rc::new(RefCell::new(true));
        let v1_retyped_zoom = Rc::new(RefCell::new(2i64)); // i64 in v1, becomes f64 in v2!

        let mut reg_v1 = SignalRegistry::new(1);
        {
            let c = Rc::clone(&v1_count);
            let c2 = Rc::clone(&v1_count);
            reg_v1.register_slot("App", "count", move || *c.borrow(), move |v| *c2.borrow_mut() = v);
        }
        {
            let t = Rc::clone(&v1_theme);
            let t2 = Rc::clone(&v1_theme);
            reg_v1.register_slot(
                "App",
                "theme",
                move || t.borrow().clone(),
                move |v| *t2.borrow_mut() = v,
            );
        }
        {
            let f = Rc::clone(&v1_legacy_flag);
            let f2 = Rc::clone(&v1_legacy_flag);
            reg_v1.register_slot(
                "App",
                "legacy_flag",
                move || *f.borrow(),
                move |v| *f2.borrow_mut() = v,
            );
        }
        {
            let z = Rc::clone(&v1_retyped_zoom);
            let z2 = Rc::clone(&v1_retyped_zoom);
            reg_v1.register_slot(
                "Canvas",
                "zoom",
                move || *z.borrow(),
                move |v| *z2.borrow_mut() = v,
            );
        }

        let (_hdr, mut wal) = reg_v1
            .compact_to_snapshot(&snap_path, &wal_path, b"FRAME0_V1", b"")
            .unwrap();

        // Mutate count and stream delta into WAL.
        *v1_count.borrow_mut() = 77;
        assert_eq!(reg_v1.persist_delta_to_wal(&mut wal).unwrap(), Some(1));
        // Clean persist returns None.
        assert_eq!(reg_v1.persist_delta_to_wal(&mut wal).unwrap(), None);

        // App v2 upgrades schema:
        // - keeps `App/count` (i32) and `App/theme` (String)
        // - removes `App/legacy_flag`
        // - re-types `Canvas/zoom` from i64 -> f64 (default 1.5)
        // - adds new `App/sidebar_open` (bool, default true)
        let v2_count = Rc::new(RefCell::new(0i32));
        let v2_theme = Rc::new(RefCell::new("light".to_string()));
        let v2_zoom = Rc::new(RefCell::new(1.5f64));
        let v2_sidebar = Rc::new(RefCell::new(true));

        let mut reg_v2 = SignalRegistry::new(2);
        {
            let c = Rc::clone(&v2_count);
            let c2 = Rc::clone(&v2_count);
            reg_v2.register_slot("App", "count", move || *c.borrow(), move |v| *c2.borrow_mut() = v);
        }
        {
            let t = Rc::clone(&v2_theme);
            let t2 = Rc::clone(&v2_theme);
            reg_v2.register_slot(
                "App",
                "theme",
                move || t.borrow().clone(),
                move |v| *t2.borrow_mut() = v,
            );
        }
        {
            let z = Rc::clone(&v2_zoom);
            let z2 = Rc::clone(&v2_zoom);
            reg_v2.register_slot("Canvas", "zoom", move || *z.borrow(), move |v| *z2.borrow_mut() = v);
        }
        {
            let s = Rc::clone(&v2_sidebar);
            let s2 = Rc::clone(&v2_sidebar);
            reg_v2.register_slot(
                "App",
                "sidebar_open",
                move || *s.borrow(),
                move |v| *s2.borrow_mut() = v,
            );
        }

        let snap = MmapSnapshot::open(&snap_path).unwrap();
        let report = reg_v2
            .hydrate_from_snapshot_and_wal(&snap, Some(&wal))
            .unwrap();

        assert_eq!(report.source_schema_version, 1);
        assert_eq!(report.hydrated_slots, 2);
        assert_eq!(report.skipped_unknown_slots, 1); // legacy_flag
        assert_eq!(report.skipped_type_mismatches, 1); // zoom i64 -> f64
        assert_eq!(report.untouched_new_slots, 1); // sidebar_open

        // Matching slots restored (including WAL delta count=77); incompatible/new slots kept safe defaults.
        assert_eq!(*v2_count.borrow(), 77);
        assert_eq!(*v2_theme.borrow(), "dark");
        assert_eq!(*v2_zoom.borrow(), 1.5);
        assert!(*v2_sidebar.borrow());
    }

    #[test]
    fn corrupt_and_truncated_archives_are_rejected() {
        let mut table = SignalTableSnapshot::new(1);
        table.set_named("A", "x", SignalValue::I64(123));
        table.set_named("A", "s", SignalValue::Str("hello".into()));
        let valid = table.encode().unwrap();

        // Every strict prefix must fail decoding.
        for len in 0..valid.len() {
            assert!(
                SignalTableSnapshot::decode(&valid[..len]).is_err(),
                "prefix of len {len} unexpectedly decoded"
            );
        }

        // Trailing bytes rejected.
        let mut with_trailing = valid.clone();
        with_trailing.push(0);
        assert!(SignalTableSnapshot::decode(&with_trailing).is_err());

        // Single bit flip in payload triggers checksum mismatch.
        let mut flipped = valid.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0x01;
        assert!(matches!(
            SignalTableSnapshot::decode(&flipped),
            Err(SignalArchiveError::ChecksumMismatch { .. })
        ));

        // Bad magic rejected.
        let mut bad_magic = valid.clone();
        bad_magic[0..8].copy_from_slice(b"BADMAGIC");
        assert!(matches!(
            SignalTableSnapshot::decode(&bad_magic),
            Err(SignalArchiveError::InvalidMagic(_))
        ));

        // Corrupt BinValue inside archive rejected even if checksum is recomputed.
        let mut bad_bin = Vec::new();
        bad_bin.resize(HEADER_SIZE, 0);
        bad_bin.extend_from_slice(&42u64.to_le_bytes());
        bad_bin.push(SignalValueTag::BinValue as u8);
        bad_bin.extend_from_slice(&2u32.to_le_bytes());
        bad_bin.extend_from_slice(&[0xFF, 0x00]); // invalid NveValueTag
        write_header(&mut bad_bin, NVE_SIGNAL_ARCHIVE_MAGIC, 1, 1);
        assert!(matches!(
            SignalTableSnapshot::decode(&bad_bin),
            Err(SignalArchiveError::InvalidBinValue(_))
        ));

        // Truncated/corrupted delta rejected.
        let mut next = table.clone();
        next.set_named("A", "x", SignalValue::I64(999));
        let delta_bytes = next.diff_since(&table).encode().unwrap();
        for len in 0..delta_bytes.len() {
            assert!(SignalDelta::decode(&delta_bytes[..len]).is_err());
        }
        let mut bad_delta = delta_bytes.clone();
        let last_d = bad_delta.len() - 1;
        bad_delta[last_d] ^= 0x80;
        assert!(matches!(
            SignalDelta::decode(&bad_delta),
            Err(SignalArchiveError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn live_dioxus_signal_registers_snapshots_and_hydrates() {
        use dioxus_core::{Element, VirtualDom};
        use dioxus_core_macro::rsx;
        use std::sync::Mutex;

        static CAPTURED: Mutex<Option<Vec<u8>>> = Mutex::new(None);
        static HYDRATED_VAL: Mutex<Option<i32>> = Mutex::new(None);

        fn app() -> Element {
            let mut count = crate::use_signal(|| 5i32);
            let mut reg = SignalRegistry::new(1);
            reg.register_signal("CounterApp", "count", count);

            count.set(19);
            let snap = reg.encode_full().unwrap();
            *CAPTURED.lock().unwrap() = Some(snap.clone());

            count.set(0);
            let report = reg.hydrate_from_bytes(&snap).unwrap();
            assert_eq!(report.hydrated_slots, 1);
            *HYDRATED_VAL.lock().unwrap() = Some(count());

            rsx! {}
        }

        let mut vdom = VirtualDom::new(app);
        let mut writer = crate::NveMutationWriter::new(1);
        vdom.rebuild(&mut writer);

        assert!(CAPTURED.lock().unwrap().is_some());
        assert_eq!(*HYDRATED_VAL.lock().unwrap(), Some(19));
    }
}
