//! Crash-atomic double-buffered root header slots (Slots A & B) for [`super::MmapOffsetArena`].
//!
//! Layout at the start of every `NVEARENA` file (`ARENA_HEADER_REGION_SIZE = 128` bytes):
//! - Bytes `0..64`:   Slot A (`ArenaHeaderSlot`, 64 bytes / 1 cache line)
//! - Bytes `64..128`: Slot B (`ArenaHeaderSlot`, 64 bytes / 1 cache line)
//! - Bytes `128..`:   Arena payload region (`ARENA_DATA_START = 128`, 64-byte aligned)
//!
//! Each `commit_root(...)` writes into the *inactive* slot with `generation + 1` and a fresh
//! **XXH64** checksum, then `msync`s. On `open(...)`, both slots are validated and the valid
//! slot with the strictly higher `generation` is selected — recovering transparently if a crash
//! tore either Slot A or Slot B mid-write.

use std::mem::{align_of, offset_of, size_of};

use crate::hash::Xxh64Hasher;

use super::ArenaError;

/// 8-byte file magic identifying an `MmapOffsetArena` file.
pub const ARENA_MAGIC: [u8; 8] = *b"NVEARENA";

/// Current `MmapOffsetArena` on-disk header format version (double-buffered XXH64 slots).
pub const ARENA_VERSION: u32 = 2;

/// Fixed size of each double-buffered header slot (1 cache line = 64 bytes).
pub const ARENA_HEADER_SLOT_SIZE: usize = 64;

/// Total size of the two-slot header region (`Slot A + Slot B = 128` bytes).
pub const ARENA_HEADER_REGION_SIZE: usize = ARENA_HEADER_SLOT_SIZE * 2;

/// First byte offset available for payload allocations (`128`, 64-byte aligned).
pub const ARENA_DATA_START: usize = ARENA_HEADER_REGION_SIZE;

/// Flag indicating all [`super::TrustedUtf8Str`] allocations written to the arena were
/// statically validated as UTF-8 on write.
pub const ARENA_FLAG_UTF8_TRUSTED: u32 = 1 << 0;

const CHECKSUM_OFFSET: usize = 40;
const CHECKSUM_END: usize = 48;

/// 64-byte, 8-byte-aligned double-buffered header slot stored at offsets `0` (Slot A) and
/// `64` (Slot B) of every [`super::MmapOffsetArena`] file.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaHeaderSlot {
    pub magic: [u8; 8],
    pub version: u32,
    pub flags: u32,
    pub capacity: u32,
    pub used_bytes: u32,
    pub root_offset: u32,
    pub string_count: u32,
    pub generation: u64,
    pub header_checksum: u64,
    pub reserved: [u64; 2],
}

const _: () = {
    assert!(size_of::<ArenaHeaderSlot>() == ARENA_HEADER_SLOT_SIZE);
    assert!(align_of::<ArenaHeaderSlot>() == 8);
    assert!(offset_of!(ArenaHeaderSlot, magic) == 0);
    assert!(offset_of!(ArenaHeaderSlot, version) == 8);
    assert!(offset_of!(ArenaHeaderSlot, flags) == 12);
    assert!(offset_of!(ArenaHeaderSlot, capacity) == 16);
    assert!(offset_of!(ArenaHeaderSlot, used_bytes) == 20);
    assert!(offset_of!(ArenaHeaderSlot, root_offset) == 24);
    assert!(offset_of!(ArenaHeaderSlot, string_count) == 28);
    assert!(offset_of!(ArenaHeaderSlot, generation) == 32);
    assert!(offset_of!(ArenaHeaderSlot, header_checksum) == CHECKSUM_OFFSET);
    assert!(offset_of!(ArenaHeaderSlot, reserved) == CHECKSUM_END);
};

/// Computes the canonical **XXH64** checksum over a 64-byte header slot (skipping bytes `40..48`).
#[inline]
pub fn compute_slot_checksum(slot_bytes: &[u8; ARENA_HEADER_SLOT_SIZE]) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(&slot_bytes[0..CHECKSUM_OFFSET]);
    h.update(&slot_bytes[CHECKSUM_END..ARENA_HEADER_SLOT_SIZE]);
    h.finish()
}

/// Serializes an [`ArenaHeaderSlot`] in little-endian wire order and populates `header_checksum`.
#[inline]
pub fn encode_slot_with_checksum(slot: &mut ArenaHeaderSlot) -> [u8; ARENA_HEADER_SLOT_SIZE] {
    let mut out = [0u8; ARENA_HEADER_SLOT_SIZE];
    out[0..8].copy_from_slice(&slot.magic);
    out[8..12].copy_from_slice(&slot.version.to_le_bytes());
    out[12..16].copy_from_slice(&slot.flags.to_le_bytes());
    out[16..20].copy_from_slice(&slot.capacity.to_le_bytes());
    out[20..24].copy_from_slice(&slot.used_bytes.to_le_bytes());
    out[24..28].copy_from_slice(&slot.root_offset.to_le_bytes());
    out[28..32].copy_from_slice(&slot.string_count.to_le_bytes());
    out[32..40].copy_from_slice(&slot.generation.to_le_bytes());
    out[48..56].copy_from_slice(&slot.reserved[0].to_le_bytes());
    out[56..64].copy_from_slice(&slot.reserved[1].to_le_bytes());

    let ck = compute_slot_checksum(&out);
    slot.header_checksum = ck;
    out[CHECKSUM_OFFSET..CHECKSUM_END].copy_from_slice(&ck.to_le_bytes());
    out
}

/// Decodes and validates a single 64-byte header slot against `file_len`.
pub fn decode_and_validate_slot(
    raw: &[u8; ARENA_HEADER_SLOT_SIZE],
    file_len: usize,
) -> Result<ArenaHeaderSlot, ArenaError> {
    let magic: [u8; 8] = raw[0..8].try_into().unwrap();
    if magic != ARENA_MAGIC {
        return Err(ArenaError::InvalidMagic);
    }
    let version = u32::from_le_bytes(raw[8..12].try_into().unwrap());
    if version != ARENA_VERSION {
        return Err(ArenaError::UnsupportedVersion(version));
    }
    let flags = u32::from_le_bytes(raw[12..16].try_into().unwrap());
    let capacity = u32::from_le_bytes(raw[16..20].try_into().unwrap());
    let used_bytes = u32::from_le_bytes(raw[20..24].try_into().unwrap());
    let root_offset = u32::from_le_bytes(raw[24..28].try_into().unwrap());
    let string_count = u32::from_le_bytes(raw[28..32].try_into().unwrap());
    let generation = u64::from_le_bytes(raw[32..40].try_into().unwrap());
    let header_checksum = u64::from_le_bytes(raw[40..48].try_into().unwrap());
    let reserved0 = u64::from_le_bytes(raw[48..56].try_into().unwrap());
    let reserved1 = u64::from_le_bytes(raw[56..64].try_into().unwrap());

    let expected_ck = compute_slot_checksum(raw);
    if header_checksum != expected_ck {
        return Err(ArenaError::HeaderChecksumMismatch {
            expected: expected_ck,
            actual: header_checksum,
        });
    }
    if reserved0 != 0 || reserved1 != 0 {
        return Err(ArenaError::CorruptHeader("non-zero reserved header bytes"));
    }
    if generation == 0 {
        return Err(ArenaError::CorruptHeader("generation must be >= 1"));
    }
    let cap = capacity as usize;
    let used = used_bytes as usize;
    if cap != file_len {
        return Err(ArenaError::CorruptHeader("capacity != file size"));
    }
    if used < ARENA_DATA_START || used > cap {
        return Err(ArenaError::CorruptHeader("used_bytes out of range"));
    }
    if root_offset != 0 {
        let root = root_offset as usize;
        if root < ARENA_DATA_START || root >= used {
            return Err(ArenaError::CorruptHeader("root_offset out of range"));
        }
    }

    Ok(ArenaHeaderSlot {
        magic,
        version,
        flags,
        capacity,
        used_bytes,
        root_offset,
        string_count,
        generation,
        header_checksum,
        reserved: [reserved0, reserved1],
    })
}

/// Byte offset of header slot `slot_idx` (`0` = Slot A at 0, `1` = Slot B at 64).
#[inline]
pub const fn slot_byte_offset(slot_idx: usize) -> usize {
    slot_idx * ARENA_HEADER_SLOT_SIZE
}

/// Validates both Slot A (`0..64`) and Slot B (`64..128`) and returns the valid slot with the
/// strictly higher `generation`. If one slot was torn mid-write, recovers from the other slot.
pub fn select_active_slot(bytes: &[u8]) -> Result<(usize, ArenaHeaderSlot), ArenaError> {
    if bytes.len() < ARENA_HEADER_REGION_SIZE {
        return Err(ArenaError::Truncated);
    }
    let raw_a: &[u8; ARENA_HEADER_SLOT_SIZE] =
        bytes[0..ARENA_HEADER_SLOT_SIZE].try_into().unwrap();
    let raw_b: &[u8; ARENA_HEADER_SLOT_SIZE] = bytes
        [ARENA_HEADER_SLOT_SIZE..ARENA_HEADER_REGION_SIZE]
        .try_into()
        .unwrap();

    let res_a = decode_and_validate_slot(raw_a, bytes.len());
    let res_b = decode_and_validate_slot(raw_b, bytes.len());

    match (res_a, res_b) {
        (Ok(a), Ok(b)) => {
            if b.generation > a.generation {
                Ok((1, b))
            } else {
                Ok((0, a))
            }
        }
        (Ok(a), Err(_)) => Ok((0, a)),
        (Err(_), Ok(b)) => Ok((1, b)),
        (Err(err_a), Err(err_b)) => Err(pick_more_specific_error(err_a, err_b)),
    }
}

fn error_specificity(err: &ArenaError) -> u8 {
    match err {
        ArenaError::CorruptHeader(_) => 4,
        ArenaError::HeaderChecksumMismatch { .. } => 3,
        ArenaError::UnsupportedVersion(_) => 2,
        ArenaError::InvalidMagic => 1,
        _ => 0,
    }
}

fn pick_more_specific_error(a: ArenaError, b: ArenaError) -> ArenaError {
    if error_specificity(&b) > error_specificity(&a) {
        b
    } else {
        a
    }
}
