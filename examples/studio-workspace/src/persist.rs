//! Crash-safe local-first binary persistence (`NVE_SNAP` + `.nvewal`) for **Aether Workspace**.
//!
//! On native targets, [`WorkspaceStore`] pairs `nve_engine::MmapSnapshot` +
//! `write_snapshot_atomic` with `SnapshotWal` (`.nvewal`) for O(1) incremental appends,
//! automatic compaction once the WAL exceeds byte/record thresholds, torn-tail recovery,
//! and corrupt-file fallback to default state.
//!
//! On `wasm32`, [`WorkspaceStore`] compiles to a zero-cost no-op stub with identical API.

use std::fmt;

use nve_engine::WalDurability;

use crate::state::WORKSPACE_STATE_BIN_VERSION;

/// Opt-in environment variable enabling local-first persistence for `StudioWorkspaceModule`.
pub const STUDIO_PERSIST_ENV_VAR: &str = "NVE_STUDIO_DATA_DIR";

/// Base `NVE_SNAP` snapshot file name inside a workspace data directory.
pub const SNAPSHOT_FILE_NAME: &str = "workspace.nvesnap";

/// Append-only `NVE_WAL1` write-ahead log file name inside a workspace data directory.
pub const WAL_FILE_NAME: &str = "workspace.nvewal";

/// Default WAL record count threshold triggering automatic compaction into `workspace.nvesnap`.
pub const DEFAULT_WAL_COMPACT_MAX_RECORDS: u64 = 32;

/// Default WAL byte size threshold triggering automatic compaction into `workspace.nvesnap`.
pub const DEFAULT_WAL_COMPACT_MAX_BYTES: u64 = 256 * 1024;

/// Application schema version embedded in `NVE_SNAP` and `NVE_WAL1` headers.
pub const STUDIO_SNAPSHOT_SCHEMA_VERSION: u32 = WORKSPACE_STATE_BIN_VERSION as u32;

/// Compaction and durability configuration for [`WorkspaceStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistConfig {
    pub max_wal_bytes: u64,
    pub max_wal_records: u64,
    pub durability: WalDurability,
}

impl Default for PersistConfig {
    fn default() -> Self {
        Self {
            max_wal_bytes: DEFAULT_WAL_COMPACT_MAX_BYTES,
            max_wal_records: DEFAULT_WAL_COMPACT_MAX_RECORDS,
            durability: WalDurability::Buffered,
        }
    }
}

/// Diagnostic summary returned by [`WorkspaceStore::recovery_stats`] after opening a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PersistRecoveryStats {
    pub loaded_from_disk: bool,
    pub replayed_wal_records: usize,
    pub truncated_torn_wal_bytes: u64,
    pub recovered_from_corrupt_snapshot: bool,
    pub recovered_from_corrupt_wal: bool,
}

/// Outcome returned by [`WorkspaceStore::save`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistOutcome {
    pub wal_seq: u64,
    pub compacted: bool,
    pub wal_record_count: u64,
    pub wal_file_len: u64,
    pub snapshot_checksum: u64,
}

/// Persistence error surfaced by [`WorkspaceStore`] operations.
#[derive(Debug)]
pub enum PersistError {
    Io(std::io::Error),
    Snapshot(nve_engine::SnapshotError),
    Wal(nve_engine::WalError),
    CorruptState(&'static str),
}

impl fmt::Display for PersistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "persistence I/O error: {err}"),
            Self::Snapshot(err) => write!(f, "snapshot error: {err}"),
            Self::Wal(err) => write!(f, "WAL error: {err}"),
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

impl From<nve_engine::SnapshotError> for PersistError {
    fn from(value: nve_engine::SnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

impl From<nve_engine::WalError> for PersistError {
    fn from(value: nve_engine::WalError) -> Self {
        Self::Wal(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::fs;
    use std::path::{Path, PathBuf};

    use nve_engine::{write_snapshot_atomic, MmapSnapshot, SnapshotWal, WalRecordKind};

    use crate::state::WorkspaceState;

    use super::{
        PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats, SNAPSHOT_FILE_NAME,
        STUDIO_PERSIST_ENV_VAR, STUDIO_SNAPSHOT_SCHEMA_VERSION, WAL_FILE_NAME,
    };

    const SNAPSHOT_META_TAG: &[u8] = b"nve-studio-nws1-v2";

    /// Crash-safe local-first binary store backed by `MmapSnapshot` + `write_snapshot_atomic`
    /// + `SnapshotWal` (`.nvewal`).
    pub struct WorkspaceStore {
        dir: PathBuf,
        snapshot_path: PathBuf,
        wal_path: PathBuf,
        wal: SnapshotWal,
        config: PersistConfig,
        base_checksum: u64,
        compaction_count: u64,
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
            let dir = dir.as_ref().to_path_buf();
            fs::create_dir_all(&dir)?;
            let snapshot_path = dir.join(SNAPSHOT_FILE_NAME);
            let wal_path = dir.join(WAL_FILE_NAME);

            let mut recovery = PersistRecoveryStats::default();

            if snapshot_path.exists() {
                match load_base_snapshot(&snapshot_path) {
                    Ok((base_state, base_checksum)) => {
                        recovery.loaded_from_disk = true;
                        let (wal, state) = open_and_replay_wal(
                            &wal_path,
                            base_checksum,
                            base_state,
                            config.durability,
                            &mut recovery,
                        )?;
                        return Ok(Self {
                            dir,
                            snapshot_path,
                            wal_path,
                            wal,
                            config,
                            base_checksum,
                            compaction_count: 0,
                            initial_state: state,
                            recovery,
                        });
                    }
                    Err(_) => {
                        // Corrupt or incompatible base snapshot: cleanly recover to default state.
                        recovery.recovered_from_corrupt_snapshot = true;
                    }
                }
            }

            let default_state = WorkspaceState::default();
            let state_bytes = default_state.to_bytes();
            let header = write_snapshot_atomic(
                &snapshot_path,
                STUDIO_SNAPSHOT_SCHEMA_VERSION,
                0,
                &[],
                &state_bytes,
                SNAPSHOT_META_TAG,
            )?;
            let wal = SnapshotWal::create_fresh(
                &wal_path,
                STUDIO_SNAPSHOT_SCHEMA_VERSION,
                header.checksum_xxh64,
                config.durability,
            )?;

            Ok(Self {
                dir,
                snapshot_path,
                wal_path,
                wal,
                config,
                base_checksum: header.checksum_xxh64,
                compaction_count: 0,
                initial_state: default_state,
                recovery,
            })
        }

        #[inline]
        pub fn is_enabled(&self) -> bool {
            true
        }

        #[inline]
        pub fn dir(&self) -> &Path {
            &self.dir
        }

        #[inline]
        pub fn snapshot_path(&self) -> &Path {
            &self.snapshot_path
        }

        #[inline]
        pub fn wal_path(&self) -> &Path {
            &self.wal_path
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
        pub fn wal_record_count(&self) -> u64 {
            self.wal.record_count()
        }

        #[inline]
        pub fn wal_file_len(&self) -> u64 {
            self.wal.file_len()
        }

        #[inline]
        pub fn base_snapshot_checksum(&self) -> u64 {
            self.base_checksum
        }

        #[inline]
        pub fn compaction_count(&self) -> u64 {
            self.compaction_count
        }

        /// Appends `state` incrementally to `.nvewal` and automatically compacts into
        /// `workspace.nvesnap` when either byte or record threshold is reached.
        pub fn save(&mut self, state: &WorkspaceState) -> Result<PersistOutcome, PersistError> {
            let state_bytes = state.to_bytes();
            let wal_seq = self
                .wal
                .append_delta(WalRecordKind::StateDelta, &state_bytes)?;
            let mut compacted = false;

            if self
                .wal
                .should_compact(self.config.max_wal_bytes, self.config.max_wal_records)
            {
                let header = self.wal.compact(
                    &self.snapshot_path,
                    STUDIO_SNAPSHOT_SCHEMA_VERSION,
                    &[],
                    &state_bytes,
                    SNAPSHOT_META_TAG,
                )?;
                self.base_checksum = header.checksum_xxh64;
                self.compaction_count = self.compaction_count.saturating_add(1);
                compacted = true;
            }

            Ok(PersistOutcome {
                wal_seq,
                compacted,
                wal_record_count: self.wal.record_count(),
                wal_file_len: self.wal.file_len(),
                snapshot_checksum: self.base_checksum,
            })
        }

        /// Crash-safely compacts `state` (with optional Frame-0 `CommandBuffer` bytes) into a
        /// fresh atomic `NVE_SNAP` file and rotates `.nvewal` to an empty header.
        pub fn compact(
            &mut self,
            state: &WorkspaceState,
            frame0_bytes: &[u8],
        ) -> Result<u64, PersistError> {
            let state_bytes = state.to_bytes();
            let header = self.wal.compact(
                &self.snapshot_path,
                STUDIO_SNAPSHOT_SCHEMA_VERSION,
                frame0_bytes,
                &state_bytes,
                SNAPSHOT_META_TAG,
            )?;
            self.base_checksum = header.checksum_xxh64;
            self.compaction_count = self.compaction_count.saturating_add(1);
            Ok(self.base_checksum)
        }

        /// Re-reads the persisted base `MmapSnapshot` and replays `.nvewal` to verify disk state.
        pub fn load_state(&self) -> Result<WorkspaceState, PersistError> {
            let (mut state, _) = load_base_snapshot(&self.snapshot_path)?;
            self.wal.replay_into(|rec| -> Result<(), &'static str> {
                if rec.kind == WalRecordKind::StateDelta {
                    let decoded = WorkspaceState::from_bytes(rec.payload)
                        .ok_or("invalid NWS1 v2 state payload in WAL record")?;
                    state = decoded;
                }
                Ok(())
            })?;
            Ok(state)
        }
    }

    fn load_base_snapshot(path: &Path) -> Result<(WorkspaceState, u64), PersistError> {
        let snap = MmapSnapshot::open_with_schema(path, STUDIO_SNAPSHOT_SCHEMA_VERSION)?;
        let decoded = WorkspaceState::from_bytes(snap.state_bytes())
            .ok_or(PersistError::CorruptState("NWS1 v2 decode failed"))?;
        Ok((decoded, snap.checksum_xxh64()))
    }

    fn open_and_replay_wal(
        wal_path: &Path,
        base_checksum: u64,
        base_state: WorkspaceState,
        durability: nve_engine::WalDurability,
        recovery: &mut PersistRecoveryStats,
    ) -> Result<(SnapshotWal, WorkspaceState), PersistError> {
        match SnapshotWal::open_or_create_with_durability(
            wal_path,
            STUDIO_SNAPSHOT_SCHEMA_VERSION,
            base_checksum,
            durability,
        ) {
            Ok(wal) => {
                recovery.truncated_torn_wal_bytes = wal.recovered_truncated_bytes();
                let mut latest = base_state.clone();
                let mut replayed = 0usize;
                let replay_res = wal.replay_into(|rec| -> Result<(), &'static str> {
                    if rec.kind == WalRecordKind::StateDelta {
                        let decoded = WorkspaceState::from_bytes(rec.payload)
                            .ok_or("corrupt NWS1 v2 payload inside WAL")?;
                        latest = decoded;
                        replayed += 1;
                    }
                    Ok(())
                });
                match replay_res {
                    Ok(_) => {
                        recovery.replayed_wal_records = replayed;
                        Ok((wal, latest))
                    }
                    Err(_) => {
                        recovery.recovered_from_corrupt_wal = true;
                        let fresh = SnapshotWal::create_fresh(
                            wal_path,
                            STUDIO_SNAPSHOT_SCHEMA_VERSION,
                            base_checksum,
                            durability,
                        )?;
                        Ok((fresh, base_state))
                    }
                }
            }
            Err(_) => {
                recovery.recovered_from_corrupt_wal = true;
                let fresh = SnapshotWal::create_fresh(
                    wal_path,
                    STUDIO_SNAPSHOT_SCHEMA_VERSION,
                    base_checksum,
                    durability,
                )?;
                Ok((fresh, base_state))
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::path::{Path, PathBuf};

    use crate::state::WorkspaceState;

    use super::{
        PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats, SNAPSHOT_FILE_NAME,
        WAL_FILE_NAME,
    };

    /// Zero-cost no-op persistence stub for `wasm32` targets.
    pub struct WorkspaceStore {
        dir: PathBuf,
        snapshot_path: PathBuf,
        wal_path: PathBuf,
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
                snapshot_path: dir.join(SNAPSHOT_FILE_NAME),
                wal_path: dir.join(WAL_FILE_NAME),
                dir,
                initial_state: WorkspaceState::default(),
            })
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
        pub fn snapshot_path(&self) -> &Path {
            &self.snapshot_path
        }

        #[inline]
        pub fn wal_path(&self) -> &Path {
            &self.wal_path
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
        pub fn wal_record_count(&self) -> u64 {
            0
        }

        #[inline]
        pub fn wal_file_len(&self) -> u64 {
            0
        }

        #[inline]
        pub fn base_snapshot_checksum(&self) -> u64 {
            0
        }

        #[inline]
        pub fn compaction_count(&self) -> u64 {
            0
        }

        #[inline]
        pub fn save(&mut self, _state: &WorkspaceState) -> Result<PersistOutcome, PersistError> {
            Ok(PersistOutcome {
                wal_seq: 0,
                compacted: false,
                wal_record_count: 0,
                wal_file_len: 0,
                snapshot_checksum: 0,
            })
        }

        #[inline]
        pub fn compact(
            &mut self,
            _state: &WorkspaceState,
            _frame0_bytes: &[u8],
        ) -> Result<u64, PersistError> {
            Ok(0)
        }

        #[inline]
        pub fn load_state(&self) -> Result<WorkspaceState, PersistError> {
            Ok(self.initial_state.clone())
        }
    }
}

pub use imp::WorkspaceStore;
