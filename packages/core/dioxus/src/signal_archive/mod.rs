//! Binary Dioxus Signal Table Archiver & incremental delta codec.
//!
//! Stores typed signal slots keyed by deterministic 64-bit XXH64 hashes of
//! `(component_path, signal_name)` so `DioxusSurface` apps can snapshot state into
//! [`nve_engine::NativeStateStore`] (`MmapOffsetArena`), stream incremental dirty-slot deltas,
//! compact deterministically, and hydrate safely across app upgrades (tolerating added, removed,
//! or re-typed signal slots without crashing).
//!
//! # Binary Layouts (little-endian, XXH64 checksummed)
//!
//! ## Full Snapshot (`NVE_SIG1`, 28-byte header)
//! - `0..8`:   `magic = *b"NVE_SIG1"`
//! - `8..12`:  `format_version: u32 = 1`
//! - `12..16`: `schema_version: u32`
//! - `16..20`: `slot_count: u32`
//! - `20..28`: `checksum_xxh64: u64` (over `0..20` + `28..len`)
//! - Followed by `slot_count` strictly sorted entries: `slot_id: u64`, `tag: u8`, value payload.
//!
//! ## Incremental Delta (`NVE_SGD1`, 28-byte header)
//! - `0..8`:   `magic = *b"NVE_SGD1"`
//! - `8..12`:  `format_version: u32 = 1`
//! - `12..16`: `schema_version: u32`
//! - `16..20`: `op_count: u32`
//! - `20..28`: `checksum_xxh64: u64` (over `0..20` + `28..len`)
//! - Followed by `op_count` strictly sorted ops: `op_kind: u8` (`1=Upsert`, `2=Remove`),
//!   `slot_id: u64`, and (for `Upsert`) `tag: u8` + value payload.

mod codec;
mod registry;
mod signal_codec;
#[cfg(test)]
mod tests;

pub use codec::{SignalArchive, SignalDelta, SignalDeltaOp, SignalTableSnapshot};
pub use registry::{HydrationReport, SignalRegistry};
pub use signal_codec::{BinValueSlot, SignalCodec};

use std::fmt;

use nve_abi::{encode_bin_value_to_vec, BinValueView, DecodeError, NveValueRef};
use nve_components::reconcile::{
    reconcile_three_way, ConflictPolicy, ReconcileDecision, ThreeWayEntry,
};
use nve_engine::hash::Xxh64Hasher;
use nve_engine::{ArenaError, InstantBootError};

/// Reconciles one signal slot during hot-reload using 3-way merge (`ConflictPolicy::PreferLocal`).
///
/// - When the user never modified the signal (`archived_local == base_default`), adopts
///   `incoming_default` (`DivergenceKind::IncomingOnly`, `apply_local = true`).
/// - When the user mutated the signal (`archived_local != base_default`), preserves
///   `archived_local` even when the code default changed (`LocalOnlyDirty` / `Conflict`).
/// - When both changed identically, classifies as `DivergenceKind::Converged`.
#[inline]
pub fn reconcile_signal_entry<V: PartialEq + Clone>(
    base_default: V,
    archived_local: V,
    incoming_default: V,
) -> ReconcileDecision<(), V> {
    reconcile_three_way(
        ThreeWayEntry {
            key: (),
            base: base_default,
            local: archived_local,
            incoming: incoming_default,
            local_clock: None,
            incoming_clock: None,
        },
        ConflictPolicy::PreferLocal,
    )
}

/// 8-byte magic identifying a full binary signal table archive.
pub const NVE_SIGNAL_ARCHIVE_MAGIC: [u8; 8] = *b"NVE_SIG1";

/// 8-byte magic identifying an incremental dirty-slot signal delta.
pub const NVE_SIGNAL_DELTA_MAGIC: [u8; 8] = *b"NVE_SGD1";

/// Format version for both `NVE_SIG1` and `NVE_SGD1` containers.
pub const NVE_SIGNAL_ARCHIVE_VERSION: u32 = 1;

/// Computes the stable 64-bit XXH64 slot identifier for `(component_path, signal_name)`.
///
/// Uses an explicit NUL separator between `component_path` and `signal_name` so
/// `("app/nav", "tab")` and `("app/navt", "ab")` never collide.
#[inline]
pub fn signal_slot_id(component_path: &str, signal_name: &str) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(component_path.as_bytes());
    h.update(&[0u8]);
    h.update(signal_name.as_bytes());
    h.finish()
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
    Arena(ArenaError),
    InstantBoot(InstantBootError),
}

impl From<DecodeError> for SignalArchiveError {
    fn from(err: DecodeError) -> Self {
        Self::InvalidBinValue(err)
    }
}

impl From<ArenaError> for SignalArchiveError {
    fn from(err: ArenaError) -> Self {
        Self::Arena(err)
    }
}

impl From<InstantBootError> for SignalArchiveError {
    fn from(err: InstantBootError) -> Self {
        Self::InstantBoot(err)
    }
}

impl fmt::Display for SignalArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { expected, actual } => {
                write!(
                    f,
                    "truncated signal archive: expected at least {expected} bytes, got {actual}"
                )
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
            Self::Arena(e) => write!(f, "signal archive arena error: {e}"),
            Self::InstantBoot(e) => write!(f, "signal archive instant-boot error: {e}"),
        }
    }
}

impl std::error::Error for SignalArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Arena(e) => Some(e),
            Self::InstantBoot(e) => Some(e),
            _ => None,
        }
    }
}
