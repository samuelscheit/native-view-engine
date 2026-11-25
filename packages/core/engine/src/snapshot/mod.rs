//! Instant-Boot Frame-0 `CommandBuffer` freeze-dry + zero-copy `mmap` state snapshot.
//!
//! # Binary Layout (`NVE_SNAP`, little-endian, 8-byte aligned sections)
//!
//! Fixed 80-byte header (`SNAPSHOT_HEADER_SIZE`):
//! - `0..8`:   `magic: [u8; 8]` = `*b"NVE_SNAP"`
//! - `8..12`:  `format_version: u32` = `1`
//! - `12..16`: `app_schema_version: u32` (caller-supplied schema version)
//! - `16..20`: `flags: u32`
//! - `20..24`: `reserved: u32` (must be `0`, preserves 8-byte alignment)
//! - `24..32`: `checksum_xxh64: u64` (XXH64 over `0..24` + `32..file_len`)
//! - `32..40`: `frame0_offset: u64` (8-byte aligned offset of the Frame-0 `CommandBuffer`)
//! - `40..48`: `frame0_len: u64`
//! - `48..56`: `state_offset: u64` (8-byte aligned offset of relocatable app state)
//! - `56..64`: `state_len: u64`
//! - `64..72`: `meta_offset: u64` (8-byte aligned offset of optional metadata/viewport bytes)
//! - `72..80`: `meta_len: u64`
//!
//! Followed by zero-padded, 8-byte aligned payload sections (`frame0`, `state`, `meta`).

mod arena;
mod format;
mod mmap;
mod writer;

pub use arena::{
    RelocatableStateArena, RelocatableTableView, RelocatableValue, RELOCATABLE_ARENA_MAGIC,
    RELOCATABLE_ARENA_VERSION,
};
pub use format::{
    SnapshotError, SnapshotHeader, NVE_SNAP_FORMAT_VERSION, NVE_SNAP_MAGIC, SNAPSHOT_HEADER_SIZE,
    SNAPSHOT_SECTION_ALIGN,
};
pub use mmap::MmapSnapshot;
pub use writer::{encode_snapshot, write_snapshot_atomic};
