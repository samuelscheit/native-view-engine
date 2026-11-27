//! `NVE2`: closed, 4-byte-aligned, table-major wire for one committed frame.
//!
//! Used only where a frame crosses a process / `dlopen` / network / replay boundary; in-process
//! backends read `&NodeStore` + `&ChangeSet` directly.
//!
//! ```text
//! header  : magic "NVE2" u32 | version u16 | flags u16 | frame u64 | op_count u32 | words u32
//! op      : head u32 (opcode u8 | table u8 | count_u16; 0xFFFF => u32 count word follows) | payload_words u32 | payload...
//! ```
//! Ops are emitted (and must be applied) in this order: `DeclareTables`, `CreateNodes`,
//! `DefineResources`, per table `Patch` / `SetStrings` / `Detach`, `SetChildren`, `RemoveNodes`,
//! `ReleaseResources`, `Commands`. The receiving store is a replica: node and resource ids are
//! mirrored exactly. Table slots are session-scoped: a sender declares each table key once per
//! [`WireEncoder`] session and refers to it by its wire slot afterwards.

pub(crate) mod budget;
mod decode;
mod encode;

pub use decode::WireDecoder;
pub use encode::WireEncoder;

/// `"NVE2"` little-endian.
pub const MAGIC: u32 = u32::from_le_bytes(*b"NVE2");
pub const VERSION: u16 = 1;
/// Header size in words.
pub const HEADER_WORDS: usize = 6;

/// The closed opcode set.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    DeclareTables = 1,
    RemoveNodes = 2,
    CreateNodes = 3,
    DefineResources = 4,
    ReleaseResources = 5,
    Patch = 6,
    Detach = 7,
    SetStrings = 8,
    SetChildren = 9,
    Commands = 10,
}

impl Op {
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::DeclareTables,
            2 => Self::RemoveNodes,
            3 => Self::CreateNodes,
            4 => Self::DefineResources,
            5 => Self::ReleaseResources,
            6 => Self::Patch,
            7 => Self::Detach,
            8 => Self::SetStrings,
            9 => Self::SetChildren,
            10 => Self::Commands,
            _ => return None,
        })
    }
}

/// Decoding / apply failure. The replica must be discarded (or resynced) after an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    BadMagic(u32),
    UnsupportedVersion(u16),
    UnsupportedFlags(u16),
    Truncated,
    TrailingBytes,
    Misaligned,
    UnknownOp(u8),
    /// Wire slot used before being declared.
    UndeclaredTable(u8),
    /// A declared key exists locally with another schema / words, or violates table invariants.
    SchemaMismatch {
        key: u64,
    },
    TableRegistry,
    NodeExists(u32),
    DeadNode(u32),
    InvalidChildren,
    BadMask {
        mask: u32,
        words: u32,
    },
    InvalidUtf8,
    /// Resource kind has no registered codec, or its bytes failed to decode.
    Resource {
        id: u32,
        kind: u64,
    },
    /// Session or frame slot-memory budget exceeded while decoding a wire frame.
    Budget,
    /// Transient OS memory reservation failed while decoding or pre-sizing a frame.
    Alloc,
}

impl core::fmt::Display for WireError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "NVE2: {self:?}")
    }
}

impl std::error::Error for WireError {}

#[inline(always)]
pub(crate) const fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

#[inline(always)]
pub(crate) const fn pad4_checked(n: usize) -> Option<usize> {
    match n.checked_add(3) {
        Some(v) => Some(v & !3),
        None => None,
    }
}

#[cfg(test)]
mod tests;
