use std::fmt;
use std::io;

use crate::hash::Xxh64Hasher;

/// 8-byte file magic identifying an NVE instant-boot snapshot container.
pub const NVE_SNAP_MAGIC: [u8; 8] = *b"NVE_SNAP";

/// Current snapshot container format version.
pub const NVE_SNAP_FORMAT_VERSION: u32 = 1;

/// Fixed byte length of the aligned `NVE_SNAP` header (32-byte prefix + 48-byte section directory).
pub const SNAPSHOT_HEADER_SIZE: usize = 80;

/// Byte alignment enforced for every payload section offset.
pub const SNAPSHOT_SECTION_ALIGN: usize = 8;

/// Offset of the 8-byte `checksum_xxh64` field inside the header.
pub(super) const CHECKSUM_FIELD_OFFSET: usize = 24;
pub(super) const CHECKSUM_FIELD_END: usize = 32;

#[inline]
pub(super) fn align_up_8(n: usize) -> usize {
    (n + (SNAPSHOT_SECTION_ALIGN - 1)) & !(SNAPSHOT_SECTION_ALIGN - 1)
}

#[inline]
pub(super) fn pad_to_8(buf: &mut Vec<u8>) {
    let aligned = align_up_8(buf.len());
    if aligned > buf.len() {
        buf.resize(aligned, 0);
    }
}

/// Errors produced when writing, mapping, validating, or reading an NVE snapshot.
#[derive(Debug)]
pub enum SnapshotError {
    Io(io::Error),
    Truncated {
        expected: usize,
        actual: usize,
    },
    InvalidMagic([u8; 8]),
    UnsupportedFormatVersion(u32),
    InvalidReservedHeader(u32),
    SchemaVersionMismatch {
        expected: u32,
        actual: u32,
    },
    ChecksumMismatch {
        expected: u64,
        actual: u64,
    },
    UnalignedSection {
        section: &'static str,
        offset: u64,
    },
    SectionOutOfBounds {
        section: &'static str,
        offset: u64,
        len: u64,
        file_len: u64,
    },
    OverlappingSections {
        first: &'static str,
        second: &'static str,
    },
    InvalidRelocatableArena(&'static str),
}

impl From<io::Error> for SnapshotError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "snapshot I/O error: {e}"),
            Self::Truncated { expected, actual } => {
                write!(f, "truncated snapshot: expected at least {expected} bytes, got {actual}")
            }
            Self::InvalidMagic(magic) => write!(f, "invalid snapshot magic: {magic:?}"),
            Self::UnsupportedFormatVersion(v) => {
                write!(f, "unsupported snapshot format version: {v}")
            }
            Self::InvalidReservedHeader(v) => {
                write!(f, "non-zero reserved snapshot header field: {v:#x}")
            }
            Self::SchemaVersionMismatch { expected, actual } => {
                write!(f, "snapshot schema version mismatch: expected {expected}, got {actual}")
            }
            Self::ChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "snapshot XXH64 checksum mismatch: expected {expected:#018x}, computed {actual:#018x}"
                )
            }
            Self::UnalignedSection { section, offset } => {
                write!(
                    f,
                    "snapshot section '{section}' offset {offset} is not {SNAPSHOT_SECTION_ALIGN}-byte aligned"
                )
            }
            Self::SectionOutOfBounds {
                section,
                offset,
                len,
                file_len,
            } => {
                write!(
                    f,
                    "snapshot section '{section}' ({offset}..+{len}) out of bounds for {file_len}-byte snapshot"
                )
            }
            Self::OverlappingSections { first, second } => {
                write!(f, "snapshot sections '{first}' and '{second}' overlap")
            }
            Self::InvalidRelocatableArena(msg) => {
                write!(f, "invalid relocatable state arena: {msg}")
            }
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Decoded fixed header of an `NVE_SNAP` binary container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub magic: [u8; 8],
    pub format_version: u32,
    pub app_schema_version: u32,
    pub flags: u32,
    pub checksum_xxh64: u64,
    pub frame0_offset: u64,
    pub frame0_len: u64,
    pub state_offset: u64,
    pub state_len: u64,
    pub meta_offset: u64,
    pub meta_len: u64,
}

impl SnapshotHeader {
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        if bytes.len() < SNAPSHOT_HEADER_SIZE {
            return Err(SnapshotError::Truncated {
                expected: SNAPSHOT_HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        let magic: [u8; 8] = bytes[0..8].try_into().unwrap();
        if magic != NVE_SNAP_MAGIC {
            return Err(SnapshotError::InvalidMagic(magic));
        }
        let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if format_version != NVE_SNAP_FORMAT_VERSION {
            return Err(SnapshotError::UnsupportedFormatVersion(format_version));
        }
        let app_schema_version = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let flags = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let reserved = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        if reserved != 0 {
            return Err(SnapshotError::InvalidReservedHeader(reserved));
        }
        let checksum_xxh64 = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let frame0_offset = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        let frame0_len = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
        let state_offset = u64::from_le_bytes(bytes[48..56].try_into().unwrap());
        let state_len = u64::from_le_bytes(bytes[56..64].try_into().unwrap());
        let meta_offset = u64::from_le_bytes(bytes[64..72].try_into().unwrap());
        let meta_len = u64::from_le_bytes(bytes[72..80].try_into().unwrap());

        let header = Self {
            magic,
            format_version,
            app_schema_version,
            flags,
            checksum_xxh64,
            frame0_offset,
            frame0_len,
            state_offset,
            state_len,
            meta_offset,
            meta_len,
        };
        header.validate_sections(bytes.len() as u64)?;
        Ok(header)
    }

    fn validate_sections(&self, file_len: u64) -> Result<(), SnapshotError> {
        let sections = [
            ("frame0", self.frame0_offset, self.frame0_len),
            ("state", self.state_offset, self.state_len),
            ("meta", self.meta_offset, self.meta_len),
        ];

        let mut spans: [(&'static str, u64, u64); 3] = [("", 0, 0); 3];
        for (idx, &(name, offset, len)) in sections.iter().enumerate() {
            if !offset.is_multiple_of(SNAPSHOT_SECTION_ALIGN as u64) {
                return Err(SnapshotError::UnalignedSection {
                    section: name,
                    offset,
                });
            }
            if offset < SNAPSHOT_HEADER_SIZE as u64 {
                return Err(SnapshotError::SectionOutOfBounds {
                    section: name,
                    offset,
                    len,
                    file_len,
                });
            }
            let end = offset
                .checked_add(len)
                .ok_or(SnapshotError::SectionOutOfBounds {
                    section: name,
                    offset,
                    len,
                    file_len,
                })?;
            if end > file_len || usize::try_from(end).is_err() {
                return Err(SnapshotError::SectionOutOfBounds {
                    section: name,
                    offset,
                    len,
                    file_len,
                });
            }
            spans[idx] = (name, offset, end);
        }

        // Reject overlapping non-empty sections.
        for i in 0..spans.len() {
            let (name_a, start_a, end_a) = spans[i];
            if start_a == end_a {
                continue;
            }
            for &(name_b, start_b, end_b) in &spans[(i + 1)..] {
                if start_b == end_b {
                    continue;
                }
                if start_a < end_b && start_b < end_a {
                    return Err(SnapshotError::OverlappingSections {
                        first: name_a,
                        second: name_b,
                    });
                }
            }
        }

        Ok(())
    }
}

/// Computes the canonical container **XXH64** checksum over the header prefix (`0..24`),
/// header tail (`32..80`), and all payload + padding bytes (`80..`).
pub(super) fn compute_snapshot_checksum(bytes: &[u8]) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(&bytes[0..CHECKSUM_FIELD_OFFSET]);
    h.update(&bytes[CHECKSUM_FIELD_END..]);
    h.finish()
}
