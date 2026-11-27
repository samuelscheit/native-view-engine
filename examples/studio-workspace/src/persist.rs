//! Crash-atomic local-first binary persistence (`StateFile` + `Frame0File`) for **Aether Workspace**.
//!
//! On native targets, [`WorkspaceStore`] wraps [`nve_engine::StateFile`] (`state.nve`, atomic
//! temp-file + rename + XXH64 checksum) and [`nve_engine::Frame0File`] (`frame0.nve2`).
//! On `wasm32`, [`WorkspaceStore`] compiles to a zero-cost no-op stub with identical API.

use std::fmt;

use nve_engine::{DEFAULT_FRAME0_FILE_NAME, DEFAULT_STATE_FILE_NAME, PersistenceError};
pub use nve_engine::{
    StateRecoveryStats as PersistRecoveryStats, StateSaveOutcome as PersistOutcome,
};

use crate::state::WORKSPACE_STATE_BIN_VERSION;

/// Opt-in environment variable enabling local-first persistence for `StudioWorkspaceModule`.
pub const STUDIO_PERSIST_ENV_VAR: &str = "NVE_STUDIO_DATA_DIR";

/// Crash-atomic `StateFile` name inside a workspace data directory.
pub const STATE_FILE_NAME: &str = DEFAULT_STATE_FILE_NAME;

/// Per-user Frame-0 `NVE2` keyframe file name inside a workspace data directory.
pub const USER_FRAME0_FILE_NAME: &str = DEFAULT_FRAME0_FILE_NAME;

/// Application schema version embedded in Studio `state.nve` and `frame0.nve2` headers.
pub const STUDIO_SNAPSHOT_SCHEMA_VERSION: u32 = WORKSPACE_STATE_BIN_VERSION as u32;

/// Durability configuration for [`WorkspaceStore`] (durable `sync_on_save: true` by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistConfig {
    pub sync_on_save: bool,
}

impl Default for PersistConfig {
    fn default() -> Self {
        Self { sync_on_save: true }
    }
}

/// Persistence error surfaced by [`WorkspaceStore`] operations.
#[derive(Debug)]
pub enum PersistError {
    Io(std::io::Error),
    Persistence(PersistenceError),
    CorruptState(&'static str),
}

impl fmt::Display for PersistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "persistence I/O error: {err}"),
            Self::Persistence(err) => write!(f, "state file error: {err}"),
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

impl From<PersistenceError> for PersistError {
    fn from(value: PersistenceError) -> Self {
        Self::Persistence(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::path::{Path, PathBuf};

    use nve_engine::{Frame0File, Frame0Footer, Frame0Meta, Frame0Miss, StateFile};
    use nve_store::NodeStore;

    use crate::state::WorkspaceState;

    use super::{
        PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats, STATE_FILE_NAME,
        STUDIO_PERSIST_ENV_VAR, STUDIO_SNAPSHOT_SCHEMA_VERSION, USER_FRAME0_FILE_NAME,
    };

    /// Crash-atomic local-first binary store backed by [`StateFile`] (`state.nve`) and
    /// [`Frame0File`] (`frame0.nve2`).
    pub struct WorkspaceStore {
        dir: PathBuf,
        state_file: StateFile,
        frame0_path: PathBuf,
        initial_state: WorkspaceState,
        recovery: PersistRecoveryStats,
    }

    impl WorkspaceStore {
        pub fn open(dir: impl AsRef<Path>) -> Result<Self, PersistError> {
            Self::open_with_config(dir, PersistConfig::default())
        }

        pub fn open_from_env() -> Option<Result<Self, PersistError>> {
            let raw = std::env::var_os(STUDIO_PERSIST_ENV_VAR)?;
            if raw.is_empty() {
                return None;
            }
            Some(Self::open(PathBuf::from(raw)))
        }

        pub fn open_with_config(
            dir: impl AsRef<Path>,
            config: PersistConfig,
        ) -> Result<Self, PersistError> {
            let dir = dir.as_ref().to_path_buf();
            std::fs::create_dir_all(&dir)?;
            let state_path = dir.join(STATE_FILE_NAME);
            let frame0_path = dir.join(USER_FRAME0_FILE_NAME);

            let mut state_file = StateFile::open_with_sync(
                &state_path,
                STUDIO_SNAPSHOT_SCHEMA_VERSION,
                config.sync_on_save,
            )?;
            let mut recovery = state_file.recovery_stats();

            let initial_state = match state_file.load()? {
                Some(bytes) => match WorkspaceState::from_bytes(&bytes) {
                    Some(decoded) => decoded,
                    None => {
                        state_file.quarantine_corrupt()?;
                        recovery = state_file.recovery_stats();
                        let default_state = WorkspaceState::default();
                        let _ = state_file.save(&default_state.to_bytes())?;
                        default_state
                    }
                },
                None => WorkspaceState::default(),
            };

            Ok(Self {
                dir,
                state_file,
                frame0_path,
                initial_state,
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
        pub fn state_file_path(&self) -> Option<&Path> {
            self.state_file.path()
        }

        #[inline]
        pub fn user_frame0_path(&self) -> &Path {
            &self.frame0_path
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
            self.state_file.generation()
        }

        /// Crash-atomically persists `state` into `state.nve`.
        pub fn save(&mut self, state: &WorkspaceState) -> Result<PersistOutcome, PersistError> {
            Ok(self.state_file.save(&state.to_bytes())?)
        }

        /// Crash-atomically persists both `state` (`state.nve`) and a Frame-0 `NVE2` keyframe (`frame0.nve2`).
        pub fn save_checkpoint(
            &mut self,
            state: &WorkspaceState,
            store: &NodeStore,
            meta: Frame0Meta,
        ) -> Result<(PersistOutcome, Frame0Footer), PersistError> {
            let outcome = self.save(state)?;
            let footer = Frame0File::save(&self.frame0_path, store, meta)?;
            Ok((outcome, footer))
        }

        /// Reads and validates the cached `frame0.nve2` keyframe if present.
        pub fn load_frame0(
            &self,
            expected: Frame0Meta,
        ) -> Result<(Frame0Footer, Vec<u8>), Frame0Miss> {
            Frame0File::read_and_validate(&self.frame0_path, expected)
        }

        /// Decodes the latest committed `WorkspaceState` from `StateFile`'s validated cache.
        pub fn load_state(&self) -> Result<WorkspaceState, PersistError> {
            let Some(bytes) = self.state_file.load()? else {
                return Ok(WorkspaceState::default());
            };
            WorkspaceState::from_bytes(&bytes)
                .ok_or(PersistError::CorruptState("NWS1 v3 decode failed"))
        }

        /// Resets persisted state and deletes any cached `frame0.nve2`.
        pub fn reset(&mut self) -> Result<(), PersistError> {
            self.state_file.reset()?;
            match std::fs::remove_file(&self.frame0_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(PersistError::Io(e)),
            }
            self.initial_state = WorkspaceState::default();
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::path::{Path, PathBuf};

    use nve_engine::{Frame0Footer, Frame0Meta, Frame0Miss};
    use nve_store::NodeStore;

    use crate::state::WorkspaceState;

    use super::{
        PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats, STATE_FILE_NAME,
        USER_FRAME0_FILE_NAME,
    };

    /// Zero-cost no-op persistence stub for `wasm32` targets.
    pub struct WorkspaceStore {
        dir: PathBuf,
        state_file_path: PathBuf,
        frame0_path: PathBuf,
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
                state_file_path: dir.join(STATE_FILE_NAME),
                frame0_path: dir.join(USER_FRAME0_FILE_NAME),
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
        pub fn state_file_path(&self) -> Option<&Path> {
            Some(&self.state_file_path)
        }
        #[inline]
        pub fn user_frame0_path(&self) -> &Path {
            &self.frame0_path
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
        pub fn save(&mut self, _state: &WorkspaceState) -> Result<PersistOutcome, PersistError> {
            Ok(PersistOutcome {
                generation: 0,
                byte_len: 0,
                payload_checksum: 0,
            })
        }

        #[inline]
        pub fn save_checkpoint(
            &mut self,
            _state: &WorkspaceState,
            _store: &NodeStore,
            meta: Frame0Meta,
        ) -> Result<(PersistOutcome, Frame0Footer), PersistError> {
            Ok((
                PersistOutcome {
                    generation: 0,
                    byte_len: 0,
                    payload_checksum: 0,
                },
                Frame0Footer {
                    node_count: 0,
                    flags: 0,
                    table_schema_hash: meta.table_schema_hash,
                    app_build_id: meta.app_build_id,
                    env_hash: meta.env_hash,
                    payload_xxh64: 0,
                },
            ))
        }

        #[inline]
        pub fn load_frame0(
            &self,
            _expected: Frame0Meta,
        ) -> Result<(Frame0Footer, Vec<u8>), Frame0Miss> {
            Err(Frame0Miss::NotFound)
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
