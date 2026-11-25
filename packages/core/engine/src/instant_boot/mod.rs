//! Reusable Native Instant-Boot Store (`NativeStateStore` + `NativeInstantBootStore`).
//!
//! Combines the 2 canonical persistence formats into one generic, app-agnostic pipeline:
//! 1. **Pre-baked default `frame0.nvereloc`** (`NVERELOC` `SceneRelocView`) for 0-latency cold
//!    first launch before `VirtualDom` boots.
//! 2. **Per-user cached `user_frame0.nvereloc`** (atomically updated on save/shutdown) so
//!    returning users see *their own* exact last frame instantaneously on restart.
//! 3. **Per-user crash-atomic `state.nvemmap`** (`MmapOffsetArena`, double-buffered Slots A/B
//!    with XXH64 header & payload checksums) restoring full interactive application state with
//!    schema-version invalidation, automatic compaction/growth, torn-header recovery, and reset.

mod boot_store;
mod state_store;

use std::fmt;
use std::io;
use std::mem::{align_of, offset_of, size_of};

use crate::mmap_arena::{ArenaError, ArenaPod, OffsetSlice};
use crate::reloc::RelocError;

pub use boot_store::{InstantFrame0, NativeInstantBootStore};
pub use state_store::NativeStateStore;

/// Default file name for per-user crash-atomic `MmapOffsetArena` state storage.
pub const DEFAULT_STATE_ARENA_FILE: &str = "state.nvemmap";

/// Default file name for per-user cached Frame-0 `NVERELOC` scene snapshot.
pub const DEFAULT_USER_FRAME0_FILE: &str = "user_frame0.nvereloc";

/// Default initial arena capacity (256 KiB; grows automatically on demand via atomic compaction).
pub const DEFAULT_STATE_ARENA_CAPACITY: usize = 256 * 1024;

/// Default headroom added after compaction (64 KiB).
pub const DEFAULT_STATE_COMPACT_HEADROOM: usize = 64 * 1024;

/// 24-byte `#[repr(C)]` root record stored inside `MmapOffsetArena` for [`NativeStateStore`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateBlobRoot {
    pub schema_version: u32,
    pub flags: u32,
    pub payload: OffsetSlice<u8>,
    pub payload_checksum: u64,
}

// SAFETY: `StateBlobRoot` is `#[repr(C)]`, 24 bytes, 8-byte aligned, zero implicit padding.
unsafe impl ArenaPod for StateBlobRoot {}

const _: () = {
    assert!(size_of::<StateBlobRoot>() == 24);
    assert!(align_of::<StateBlobRoot>() == 8);
    assert!(offset_of!(StateBlobRoot, schema_version) == 0);
    assert!(offset_of!(StateBlobRoot, flags) == 4);
    assert!(offset_of!(StateBlobRoot, payload) == 8);
    assert!(offset_of!(StateBlobRoot, payload_checksum) == 16);
};

/// Configuration for [`NativeStateStore`] durability, initial capacity, and compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateStoreConfig {
    pub initial_capacity: usize,
    pub compact_headroom: usize,
    pub sync_on_save: bool,
    pub auto_compact_every: u64,
}

impl Default for StateStoreConfig {
    fn default() -> Self {
        Self {
            initial_capacity: DEFAULT_STATE_ARENA_CAPACITY,
            compact_headroom: DEFAULT_STATE_COMPACT_HEADROOM,
            sync_on_save: false,
            auto_compact_every: 32,
        }
    }
}

/// Recovery diagnostics collected when opening [`NativeStateStore`] or [`NativeInstantBootStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateRecoveryStats {
    pub loaded_from_disk: bool,
    pub schema_mismatch_reset: bool,
    pub recovered_from_corrupt_arena: bool,
    pub active_slot: usize,
    pub generation: u64,
}

/// Summary returned by [`NativeStateStore::save`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateSaveOutcome {
    pub generation: u64,
    pub active_slot: usize,
    pub compacted: bool,
    pub used_bytes: usize,
    pub capacity_bytes: usize,
    pub payload_checksum: u64,
}

/// Errors surfaced by [`NativeStateStore`] and [`NativeInstantBootStore`].
#[derive(Debug)]
pub enum InstantBootError {
    Io(io::Error),
    Arena(ArenaError),
    Reloc(RelocError),
}

impl fmt::Display for InstantBootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "instant-boot I/O error: {e}"),
            Self::Arena(e) => write!(f, "instant-boot arena error: {e}"),
            Self::Reloc(e) => write!(f, "instant-boot reloc error: {e}"),
        }
    }
}

impl std::error::Error for InstantBootError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Arena(e) => Some(e),
            Self::Reloc(e) => Some(e),
        }
    }
}

impl From<io::Error> for InstantBootError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ArenaError> for InstantBootError {
    fn from(value: ArenaError) -> Self {
        Self::Arena(value)
    }
}

impl From<RelocError> for InstantBootError {
    fn from(value: RelocError) -> Self {
        Self::Reloc(value)
    }
}
