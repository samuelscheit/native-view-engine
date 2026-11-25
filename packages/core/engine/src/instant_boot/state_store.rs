//! Crash-atomic per-user binary state store (`NativeStateStore`) over [`MmapOffsetArena`].

use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use crate::hash::xxh64;
use crate::mmap_arena::{ArenaError, MmapOffsetArena, OffsetPtr, ARENA_DATA_START};

use super::{
    InstantBootError, StateBlobRoot, StateRecoveryStats, StateSaveOutcome, StateStoreConfig,
};

/// Crash-atomic per-user binary state store backed directly by double-buffered [`MmapOffsetArena`].
pub struct NativeStateStore {
    arena: MmapOffsetArena,
    path: Option<PathBuf>,
    schema_version: u32,
    config: StateStoreConfig,
    recovery: StateRecoveryStats,
    saves_since_compact: u64,
    compaction_count: u64,
}

impl NativeStateStore {
    /// Opens or initializes a file-backed `MmapOffsetArena` state store at `path`.
    pub fn open(
        path: impl AsRef<Path>,
        schema_version: u32,
        config: StateStoreConfig,
    ) -> Result<Self, InstantBootError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }

        let mut recovery = StateRecoveryStats::default();
        if path.exists() {
            match MmapOffsetArena::open(&path) {
                Ok(arena) => match validate_state_root(&arena, schema_version) {
                    Ok(has_state) => {
                        recovery.loaded_from_disk = has_state;
                        recovery.active_slot = arena.active_slot();
                        recovery.generation = arena.generation();
                        return Ok(Self {
                            arena,
                            path: Some(path),
                            schema_version,
                            config,
                            recovery,
                            saves_since_compact: 0,
                            compaction_count: 0,
                        });
                    }
                    Err(StateRootInvalid::SchemaMismatch) => {
                        recovery.schema_mismatch_reset = true;
                        drop(arena);
                    }
                    Err(StateRootInvalid::Corrupt) => {
                        recovery.recovered_from_corrupt_arena = true;
                        drop(arena);
                    }
                },
                Err(_) => {
                    recovery.recovered_from_corrupt_arena = true;
                }
            }
            let _ = fs::remove_file(&path);
        }

        let arena = MmapOffsetArena::create(&path, config.initial_capacity)?;
        recovery.active_slot = arena.active_slot();
        recovery.generation = arena.generation();
        Ok(Self {
            arena,
            path: Some(path),
            schema_version,
            config,
            recovery,
            saves_since_compact: 0,
            compaction_count: 0,
        })
    }

    /// Creates an in-memory [`NativeStateStore`] (zero filesystem I/O; ideal for `wasm32`/tests).
    pub fn new_in_memory(
        schema_version: u32,
        config: StateStoreConfig,
    ) -> Result<Self, InstantBootError> {
        let arena = MmapOffsetArena::new_in_memory(config.initial_capacity)?;
        let recovery = StateRecoveryStats {
            active_slot: arena.active_slot(),
            generation: arena.generation(),
            ..Default::default()
        };
        Ok(Self {
            arena,
            path: None,
            schema_version,
            config,
            recovery,
            saves_since_compact: 0,
            compaction_count: 0,
        })
    }

    #[inline]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[inline]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[inline]
    pub fn recovery_stats(&self) -> StateRecoveryStats {
        self.recovery
    }

    #[inline]
    pub fn generation(&self) -> u64 {
        self.arena.generation()
    }

    #[inline]
    pub fn active_slot(&self) -> usize {
        self.arena.active_slot()
    }

    #[inline]
    pub fn used_bytes(&self) -> usize {
        self.arena.used_bytes()
    }

    #[inline]
    pub fn capacity_bytes(&self) -> usize {
        self.arena.capacity_bytes()
    }

    #[inline]
    pub fn compaction_count(&self) -> u64 {
        self.compaction_count
    }

    #[inline]
    pub fn arena(&self) -> &MmapOffsetArena {
        &self.arena
    }

    /// Zero-copy borrow of the latest committed state payload directly from `MmapOffsetArena`.
    pub fn load_bytes(&self) -> Result<Option<&[u8]>, InstantBootError> {
        let root_ptr = self.arena.root_ptr::<StateBlobRoot>();
        if root_ptr.is_null() {
            return Ok(None);
        }
        let root = self.arena.get(root_ptr)?;
        if root.schema_version != self.schema_version {
            return Ok(None);
        }
        let bytes = self.arena.slice(root.payload)?;
        if xxh64(bytes, 0) != root.payload_checksum {
            return Err(InstantBootError::Arena(ArenaError::CorruptHeader(
                "state payload XXH64 checksum mismatch",
            )));
        }
        Ok(Some(bytes))
    }

    /// Crash-atomically commits `state_bytes` into the inactive double-buffered header slot.
    pub fn save(&mut self, state_bytes: &[u8]) -> Result<StateSaveOutcome, InstantBootError> {
        let needed = state_bytes
            .len()
            .saturating_add(size_of::<StateBlobRoot>())
            .saturating_add(32);
        let remaining = self
            .arena
            .capacity_bytes()
            .saturating_sub(self.arena.used_bytes());
        let periodic = self.config.auto_compact_every > 0
            && self.saves_since_compact.saturating_add(1) >= self.config.auto_compact_every;

        if needed > remaining || periodic {
            self.compact_with_state(state_bytes)?;
            let root_ptr = self.arena.root_ptr::<StateBlobRoot>();
            let checksum = if root_ptr.is_null() {
                xxh64(state_bytes, 0)
            } else {
                self.arena.get(root_ptr)?.payload_checksum
            };
            return Ok(StateSaveOutcome {
                generation: self.arena.generation(),
                active_slot: self.arena.active_slot(),
                compacted: true,
                used_bytes: self.arena.used_bytes(),
                capacity_bytes: self.arena.capacity_bytes(),
                payload_checksum: checksum,
            });
        }

        let checksum = xxh64(state_bytes, 0);
        let payload = self.arena.alloc_slice_copy(state_bytes)?;
        let root_ptr = self.arena.alloc_val(StateBlobRoot {
            schema_version: self.schema_version,
            flags: 0,
            payload,
            payload_checksum: checksum,
        })?;
        let generation = if self.config.sync_on_save {
            self.arena.commit_root(root_ptr)?
        } else {
            self.arena.commit_root_buffered(root_ptr)?
        };
        self.saves_since_compact = self.saves_since_compact.saturating_add(1);

        Ok(StateSaveOutcome {
            generation,
            active_slot: self.arena.active_slot(),
            compacted: false,
            used_bytes: self.arena.used_bytes(),
            capacity_bytes: self.arena.capacity_bytes(),
            payload_checksum: checksum,
        })
    }

    /// Reclaims dead arena bytes via online compaction.
    pub fn compact(&mut self) -> Result<(usize, usize), InstantBootError> {
        let current = self.load_bytes()?.map(|b| b.to_vec());
        let before = self.arena.used_bytes();
        match current {
            Some(bytes) => self.compact_with_state(&bytes)?,
            None => self.reset()?,
        }
        Ok((before, self.arena.used_bytes()))
    }

    fn compact_with_state(&mut self, state_bytes: &[u8]) -> Result<(), InstantBootError> {
        let schema = self.schema_version;
        let checksum = xxh64(state_bytes, 0);
        let headroom = self
            .config
            .compact_headroom
            .max(state_bytes.len().saturating_mul(2))
            .max(4096);

        if self.path.is_some() {
            let min_needed = ARENA_DATA_START
                .saturating_add(state_bytes.len())
                .saturating_add(size_of::<StateBlobRoot>())
                .saturating_add(headroom);
            let extra = min_needed.saturating_sub(self.arena.used_bytes());
            let effective_headroom = headroom.max(extra);
            self.arena
                .compact_in_place(effective_headroom, |_old, new_arena| {
                    let payload = new_arena.alloc_slice_copy(state_bytes)?;
                    new_arena.alloc_val(StateBlobRoot {
                        schema_version: schema,
                        flags: 0,
                        payload,
                        payload_checksum: checksum,
                    })
                })?;
        } else {
            let next_cap = self
                .config
                .initial_capacity
                .max(state_bytes.len().saturating_mul(2).saturating_add(headroom));
            let mut next = MmapOffsetArena::new_in_memory(next_cap)?;
            let payload = next.alloc_slice_copy(state_bytes)?;
            let root = next.alloc_val(StateBlobRoot {
                schema_version: schema,
                flags: 0,
                payload,
                payload_checksum: checksum,
            })?;
            next.commit_root_buffered(root)?;
            self.arena = next;
        }
        self.saves_since_compact = 0;
        self.compaction_count = self.compaction_count.saturating_add(1);
        Ok(())
    }

    pub fn sync_all(&self) -> Result<(), InstantBootError> {
        self.arena.sync_all()?;
        Ok(())
    }

    /// Clears persisted state cleanly by committing a null root and compacting.
    pub fn reset(&mut self) -> Result<(), InstantBootError> {
        let null_root: OffsetPtr<StateBlobRoot> = OffsetPtr::NULL;
        if self.config.sync_on_save {
            self.arena.commit_root(null_root)?;
        } else {
            self.arena.commit_root_buffered(null_root)?;
        }
        if self.path.is_some() {
            self.arena
                .compact_in_place(self.config.compact_headroom, |_old, _new| {
                    Ok(OffsetPtr::<StateBlobRoot>::NULL)
                })?;
        } else {
            self.arena = MmapOffsetArena::new_in_memory(self.config.initial_capacity)?;
        }
        self.saves_since_compact = 0;
        Ok(())
    }
}

enum StateRootInvalid {
    SchemaMismatch,
    Corrupt,
}

fn validate_state_root(
    arena: &MmapOffsetArena,
    expected_schema: u32,
) -> Result<bool, StateRootInvalid> {
    let root_ptr = arena.root_ptr::<StateBlobRoot>();
    if root_ptr.is_null() {
        return Ok(false);
    }
    let root = arena
        .get(root_ptr)
        .map_err(|_| StateRootInvalid::Corrupt)?;
    if root.schema_version != expected_schema {
        return Err(StateRootInvalid::SchemaMismatch);
    }
    let bytes = arena
        .slice(root.payload)
        .map_err(|_| StateRootInvalid::Corrupt)?;
    if xxh64(bytes, 0) != root.payload_checksum {
        return Err(StateRootInvalid::Corrupt);
    }
    Ok(true)
}
