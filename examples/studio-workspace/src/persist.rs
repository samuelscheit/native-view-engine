//! Crash-safe local-first binary persistence (`MmapOffsetArena` + `NVERELOC`) for **Aether Workspace**.
//!
//! On native targets, [`WorkspaceStore`] wraps [`nve_engine::NativeInstantBootStore`] combining:
//! - Double-buffered crash-atomic `state.nvemmap` (`MmapOffsetArena`, Slots A/B + XXH64 checksums),
//! - Optional returning-user pre-rendered Frame-0 cache (`user_frame0.nvereloc`),
//! - Automatic online compaction, torn-header recovery, schema-version invalidation, and
//!   corrupt-file recovery to default state.
//!
//! On `wasm32`, [`WorkspaceStore`] compiles to a zero-cost no-op stub with identical API.

use std::fmt;

use nve_engine::{
    DEFAULT_STATE_ARENA_CAPACITY, DEFAULT_STATE_ARENA_FILE, DEFAULT_STATE_COMPACT_HEADROOM,
    DEFAULT_USER_FRAME0_FILE, StateStoreConfig,
};

use crate::state::WORKSPACE_STATE_BIN_VERSION;

/// Opt-in environment variable enabling local-first persistence for `StudioWorkspaceModule`.
pub const STUDIO_PERSIST_ENV_VAR: &str = "NVE_STUDIO_DATA_DIR";

/// Crash-atomic `MmapOffsetArena` state file name inside a workspace data directory.
pub const STATE_ARENA_FILE_NAME: &str = DEFAULT_STATE_ARENA_FILE;

/// Per-user pre-rendered Frame-0 `NVERELOC` scene file name inside a workspace data directory.
pub const USER_FRAME0_FILE_NAME: &str = DEFAULT_USER_FRAME0_FILE;

/// Default automatic online compaction interval (saves between compactions).
pub const DEFAULT_COMPACT_INTERVAL: u64 = 32;

/// Application schema version embedded in Studio `state.nvemmap` and `frame0.nvereloc` headers.
pub const STUDIO_SNAPSHOT_SCHEMA_VERSION: u32 = WORKSPACE_STATE_BIN_VERSION as u32;

/// Compaction and durability configuration for [`WorkspaceStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistConfig {
    pub initial_capacity: usize,
    pub compact_headroom: usize,
    pub auto_compact_every: u64,
    pub sync_on_save: bool,
}

impl Default for PersistConfig {
    fn default() -> Self {
        Self {
            initial_capacity: DEFAULT_STATE_ARENA_CAPACITY,
            compact_headroom: DEFAULT_STATE_COMPACT_HEADROOM,
            auto_compact_every: DEFAULT_COMPACT_INTERVAL,
            sync_on_save: false,
        }
    }
}

impl PersistConfig {
    pub const fn to_state_store_config(self) -> StateStoreConfig {
        StateStoreConfig {
            initial_capacity: self.initial_capacity,
            compact_headroom: self.compact_headroom,
            sync_on_save: self.sync_on_save,
            auto_compact_every: self.auto_compact_every,
        }
    }
}

/// Diagnostic summary returned by [`WorkspaceStore::recovery_stats`] after opening a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PersistRecoveryStats {
    pub loaded_from_disk: bool,
    pub schema_mismatch_reset: bool,
    pub recovered_from_corrupt_arena: bool,
    pub active_slot: usize,
    pub generation: u64,
}

/// Outcome returned by [`WorkspaceStore::save`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistOutcome {
    pub generation: u64,
    pub active_slot: usize,
    pub compacted: bool,
    pub used_bytes: usize,
    pub capacity_bytes: usize,
    pub payload_checksum: u64,
}

/// Persistence error surfaced by [`WorkspaceStore`] operations.
#[derive(Debug)]
pub enum PersistError {
    Io(std::io::Error),
    InstantBoot(nve_engine::InstantBootError),
    CorruptState(&'static str),
}

impl fmt::Display for PersistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "persistence I/O error: {err}"),
            Self::InstantBoot(err) => write!(f, "instant-boot store error: {err}"),
            Self::CorruptState(msg) => write!(f, "corrupt workspace state: {msg}"),
        }
    }
}

impl std::error::Error for PersistError {}

impl From<std::io::Error> for PersistError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<nve_engine::InstantBootError> for PersistError {
    fn from(value: nve_engine::InstantBootError) -> Self {
        Self::InstantBoot(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::path::{Path, PathBuf};

    use nve_engine::{InstantFrame0, NativeInstantBootStore, VerifyMode};

    use crate::state::WorkspaceState;

    use super::{
        PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats, STUDIO_PERSIST_ENV_VAR,
        STUDIO_SNAPSHOT_SCHEMA_VERSION,
    };

    /// Crash-safe local-first binary store backed by [`NativeInstantBootStore`]
    /// (`MmapOffsetArena` double-buffered `state.nvemmap` + atomic `user_frame0.nvereloc`).
    pub struct WorkspaceStore {
        boot_store: NativeInstantBootStore,
        initial_state: WorkspaceState,
        recovery: PersistRecoveryStats,
    }

    impl WorkspaceStore {
        /// Opens or initializes a local-first workspace store at `dir` using default thresholds.
        pub fn open(dir: impl AsRef<Path>) -> Result<Self, PersistError> {
            Self::open_with_config(dir, PersistConfig::default())
        }

        /// Opens a store when [`STUDIO_PERSIST_ENV_VAR`] (`NVE_STUDIO_DATA_DIR`) is set and
        /// non-empty; returns `None` when opt-in persistence is disabled.
        pub fn open_from_env() -> Option<Result<Self, PersistError>> {
            let raw = std::env::var_os(STUDIO_PERSIST_ENV_VAR)?;
            if raw.is_empty() {
                return None;
            }
            Some(Self::open(PathBuf::from(raw)))
        }

        /// Opens or initializes a local-first workspace store with explicit compaction & durability.
        pub fn open_with_config(
            dir: impl AsRef<Path>,
            config: PersistConfig,
        ) -> Result<Self, PersistError> {
            let mut boot_store = NativeInstantBootStore::open(
                dir,
                STUDIO_SNAPSHOT_SCHEMA_VERSION,
                config.to_state_store_config(),
            )?;
            let raw_rec = boot_store.recovery_stats();
            let mut recovery = PersistRecoveryStats {
                loaded_from_disk: raw_rec.loaded_from_disk,
                schema_mismatch_reset: raw_rec.schema_mismatch_reset,
                recovered_from_corrupt_arena: raw_rec.recovered_from_corrupt_arena,
                active_slot: raw_rec.active_slot,
                generation: raw_rec.generation,
            };

            let initial_state = match boot_store.state_bytes() {
                Ok(Some(bytes)) => match WorkspaceState::from_bytes(bytes) {
                    Some(decoded) => decoded,
                    None => {
                        recovery.loaded_from_disk = false;
                        recovery.recovered_from_corrupt_arena = true;
                        let default_state = WorkspaceState::default();
                        boot_store.reset()?;
                        let _ = boot_store.save_state(&default_state.to_bytes())?;
                        default_state
                    }
                },
                Ok(None) => WorkspaceState::default(),
                Err(_) => {
                    recovery.loaded_from_disk = false;
                    recovery.recovered_from_corrupt_arena = true;
                    let default_state = WorkspaceState::default();
                    boot_store.reset()?;
                    let _ = boot_store.save_state(&default_state.to_bytes())?;
                    default_state
                }
            };

            Ok(Self {
                boot_store,
                initial_state,
                recovery,
            })
        }

        /// Attaches a bundled pre-baked `frame0.nvereloc` path for 0-latency cold first launch.
        pub fn with_default_frame0_path(mut self, path: impl AsRef<Path>) -> Self {
            self.boot_store = self.boot_store.with_default_frame0_path(path);
            self
        }

        /// Attaches embedded pre-baked `frame0.nvereloc` bytes for 0-latency cold first launch.
        pub fn with_default_frame0_bytes(mut self, bytes: &[u8]) -> Self {
            self.boot_store = self.boot_store.with_default_frame0_bytes(bytes);
            self
        }

        #[inline]
        pub fn is_enabled(&self) -> bool {
            true
        }

        #[inline]
        pub fn dir(&self) -> &Path {
            self.boot_store.user_dir()
        }

        #[inline]
        pub fn state_arena_path(&self) -> Option<&Path> {
            self.boot_store.state_arena_path()
        }

        #[inline]
        pub fn user_frame0_path(&self) -> &Path {
            self.boot_store.user_frame0_path()
        }

        #[inline]
        pub fn boot_store(&self) -> &NativeInstantBootStore {
            &self.boot_store
        }

        #[inline]
        pub fn initial_state(&self) -> &WorkspaceState {
            &self.initial_state
        }

        #[inline]
        pub fn recovery_stats(&self) -> PersistRecoveryStats {
            self.recovery
        }

        #[inline]
        pub fn generation(&self) -> u64 {
            self.boot_store.state_store().generation()
        }

        #[inline]
        pub fn active_slot(&self) -> usize {
            self.boot_store.state_store().active_slot()
        }

        #[inline]
        pub fn used_bytes(&self) -> usize {
            self.boot_store.state_store().used_bytes()
        }

        #[inline]
        pub fn compaction_count(&self) -> u64 {
            self.boot_store.state_store().compaction_count()
        }

        /// Loads the best available pre-rendered Frame-0 `NVERELOC` scene (`user_frame0.nvereloc`
        /// or bundled `frame0.nvereloc`).
        pub fn load_instant_frame0(&self, mode: VerifyMode) -> Option<InstantFrame0> {
            self.boot_store.load_frame0(mode)
        }

        /// Crash-atomically commits `state` into double-buffered `state.nvemmap` (`MmapOffsetArena`),
        /// automatically compacting when thresholds or capacity limits are reached.
        pub fn save(&mut self, state: &WorkspaceState) -> Result<PersistOutcome, PersistError> {
            let state_bytes = state.to_bytes();
            let out = self.boot_store.save_state(&state_bytes)?;
            Ok(PersistOutcome {
                generation: out.generation,
                active_slot: out.active_slot,
                compacted: out.compacted,
                used_bytes: out.used_bytes,
                capacity_bytes: out.capacity_bytes,
                payload_checksum: out.payload_checksum,
            })
        }

        /// Persists both `state` (into `state.nvemmap`) and a pre-rendered `SceneRelocView` buffer
        /// (into `user_frame0.nvereloc`) and forces online compaction.
        pub fn save_checkpoint(
            &mut self,
            state: &WorkspaceState,
            scene_reloc_bytes: &[u8],
        ) -> Result<PersistOutcome, PersistError> {
            let state_bytes = state.to_bytes();
            let out = self
                .boot_store
                .save_checkpoint(scene_reloc_bytes, &state_bytes)?;
            Ok(PersistOutcome {
                generation: out.generation,
                active_slot: out.active_slot,
                compacted: out.compacted,
                used_bytes: out.used_bytes,
                capacity_bytes: out.capacity_bytes,
                payload_checksum: out.payload_checksum,
            })
        }

        /// Explicitly compacts `state.nvemmap` in place, reclaiming dead arena bytes.
        pub fn compact(&mut self) -> Result<(usize, usize), PersistError> {
            Ok(self.boot_store.state_store_mut().compact()?)
        }

        /// Re-reads and validates the latest committed `WorkspaceState` from `state.nvemmap`.
        pub fn load_state(&self) -> Result<WorkspaceState, PersistError> {
            let Some(bytes) = self.boot_store.state_bytes()? else {
                return Ok(WorkspaceState::default());
            };
            WorkspaceState::from_bytes(bytes)
                .ok_or(PersistError::CorruptState("NWS1 v3 decode failed"))
        }

        /// Resets persisted state and deletes any cached `user_frame0.nvereloc`.
        pub fn reset(&mut self) -> Result<(), PersistError> {
            self.boot_store.reset()?;
            self.initial_state = WorkspaceState::default();
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::path::{Path, PathBuf};

    use nve_engine::{InstantFrame0, VerifyMode};

    use crate::state::WorkspaceState;

    use super::{
        PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats, STATE_ARENA_FILE_NAME,
        USER_FRAME0_FILE_NAME,
    };

    /// Zero-cost no-op persistence stub for `wasm32` targets.
    pub struct WorkspaceStore {
        dir: PathBuf,
        state_arena_path: PathBuf,
        user_frame0_path: PathBuf,
        initial_state: WorkspaceState,
    }

    impl WorkspaceStore {
        pub fn open(dir: impl AsRef<Path>) -> Result<Self, PersistError> {
            Self::open_with_config(dir, PersistConfig::default())
        }

        pub fn open_from_env() -> Option<Result<Self, PersistError>> {
            None
        }

        pub fn open_with_config(
            dir: impl AsRef<Path>,
            _config: PersistConfig,
        ) -> Result<Self, PersistError> {
            let dir = dir.as_ref().to_path_buf();
            Ok(Self {
                state_arena_path: dir.join(STATE_ARENA_FILE_NAME),
                user_frame0_path: dir.join(USER_FRAME0_FILE_NAME),
                dir,
                initial_state: WorkspaceState::default(),
            })
        }

        pub fn with_default_frame0_path(self, _path: impl AsRef<Path>) -> Self {
            self
        }

        pub fn with_default_frame0_bytes(self, _bytes: &[u8]) -> Self {
            self
        }

        #[inline]
        pub fn is_enabled(&self) -> bool {
            false
        }

        #[inline]
        pub fn dir(&self) -> &Path {
            &self.dir
        }

        #[inline]
        pub fn state_arena_path(&self) -> Option<&Path> {
            Some(&self.state_arena_path)
        }

        #[inline]
        pub fn user_frame0_path(&self) -> &Path {
            &self.user_frame0_path
        }

        #[inline]
        pub fn initial_state(&self) -> &WorkspaceState {
            &self.initial_state
        }

        #[inline]
        pub fn recovery_stats(&self) -> PersistRecoveryStats {
            PersistRecoveryStats::default()
        }

        #[inline]
        pub fn generation(&self) -> u64 {
            0
        }

        #[inline]
        pub fn active_slot(&self) -> usize {
            0
        }

        #[inline]
        pub fn used_bytes(&self) -> usize {
            0
        }

        #[inline]
        pub fn compaction_count(&self) -> u64 {
            0
        }

        #[inline]
        pub fn load_instant_frame0(&self, _mode: VerifyMode) -> Option<InstantFrame0> {
            None
        }

        #[inline]
        pub fn save(&mut self, _state: &WorkspaceState) -> Result<PersistOutcome, PersistError> {
            Ok(PersistOutcome {
                generation: 0,
                active_slot: 0,
                compacted: false,
                used_bytes: 0,
                capacity_bytes: 0,
                payload_checksum: 0,
            })
        }

        #[inline]
        pub fn save_checkpoint(
            &mut self,
            _state: &WorkspaceState,
            _scene_reloc_bytes: &[u8],
        ) -> Result<PersistOutcome, PersistError> {
            Ok(PersistOutcome {
                generation: 0,
                active_slot: 0,
                compacted: false,
                used_bytes: 0,
                capacity_bytes: 0,
                payload_checksum: 0,
            })
        }

        #[inline]
        pub fn compact(&mut self) -> Result<(usize, usize), PersistError> {
            Ok((0, 0))
        }

        #[inline]
        pub fn load_state(&self) -> Result<WorkspaceState, PersistError> {
            Ok(self.initial_state.clone())
        }

        #[inline]
        pub fn reset(&mut self) -> Result<(), PersistError> {
            self.initial_state = WorkspaceState::default();
            Ok(())
        }
    }
}

pub use imp::WorkspaceStore;
