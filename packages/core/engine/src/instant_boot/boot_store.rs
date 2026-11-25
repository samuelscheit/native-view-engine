//! Coordinated [`NativeInstantBootStore`] combining bundled `frame0.nvereloc`,
//! returning-user `user_frame0.nvereloc`, and crash-atomic `state.nvemmap`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::reloc::{
    write_reloc_atomic, MmapReloc, RelocBuf, RelocError, RelocView, SceneRelocView, VerifyMode,
};

use super::{
    InstantBootError, NativeStateStore, StateRecoveryStats, StateSaveOutcome, StateStoreConfig,
    DEFAULT_STATE_ARENA_FILE, DEFAULT_USER_FRAME0_FILE,
};

/// Loaded instant-boot Frame-0 source returned by [`NativeInstantBootStore::load_frame0`].
pub enum InstantFrame0 {
    /// Returning-user cached Frame-0 mapped from `user_frame0.nvereloc`.
    UserCached(MmapReloc),
    /// Bundled cold-start default Frame-0 mapped from disk.
    BundledFile(MmapReloc),
    /// Bundled cold-start default Frame-0 held in a 64-byte-aligned [`RelocBuf`].
    BundledBytes(RelocBuf),
}

impl InstantFrame0 {
    #[inline]
    pub fn is_user_cached(&self) -> bool {
        matches!(self, Self::UserCached(_))
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::UserCached(m) | Self::BundledFile(m) => m.as_bytes(),
            Self::BundledBytes(b) => b.as_slice(),
        }
    }

    #[inline]
    pub fn reloc_view(&self, mode: VerifyMode) -> Result<RelocView<'_>, RelocError> {
        match self {
            Self::UserCached(m) | Self::BundledFile(m) => Ok(m.view()),
            Self::BundledBytes(b) => b.view(mode),
        }
    }

    #[inline]
    pub fn scene_view(&self, mode: VerifyMode) -> Result<SceneRelocView<'_>, RelocError> {
        SceneRelocView::from_bytes(self.as_bytes(), mode)
    }
}

/// Reusable Native Instant-Boot Store combining bundled `frame0.nvereloc`, per-user
/// `user_frame0.nvereloc`, and crash-atomic `state.nvemmap` ([`NativeStateStore`]).
pub struct NativeInstantBootStore {
    user_dir: PathBuf,
    user_frame0_path: PathBuf,
    default_frame0_path: Option<PathBuf>,
    default_frame0_bytes: Option<RelocBuf>,
    state_store: NativeStateStore,
    schema_version: u32,
}

impl NativeInstantBootStore {
    pub fn open(
        user_dir: impl AsRef<Path>,
        schema_version: u32,
        config: StateStoreConfig,
    ) -> Result<Self, InstantBootError> {
        let user_dir = user_dir.as_ref().to_path_buf();
        fs::create_dir_all(&user_dir)?;
        let state_path = user_dir.join(DEFAULT_STATE_ARENA_FILE);
        let user_frame0_path = user_dir.join(DEFAULT_USER_FRAME0_FILE);
        let state_store = NativeStateStore::open(&state_path, schema_version, config)?;

        let rec = state_store.recovery_stats();
        if (rec.schema_mismatch_reset || rec.recovered_from_corrupt_arena)
            && user_frame0_path.exists()
        {
            let _ = fs::remove_file(&user_frame0_path);
        }

        Ok(Self {
            user_dir,
            user_frame0_path,
            default_frame0_path: None,
            default_frame0_bytes: None,
            state_store,
            schema_version,
        })
    }

    pub fn with_default_frame0_path(mut self, path: impl AsRef<Path>) -> Self {
        self.default_frame0_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_default_frame0_bytes(mut self, bytes: &[u8]) -> Self {
        self.default_frame0_bytes = Some(RelocBuf::from_bytes(bytes));
        self
    }

    #[inline]
    pub fn user_dir(&self) -> &Path {
        &self.user_dir
    }

    #[inline]
    pub fn user_frame0_path(&self) -> &Path {
        &self.user_frame0_path
    }

    #[inline]
    pub fn state_arena_path(&self) -> Option<&Path> {
        self.state_store.path()
    }

    #[inline]
    pub fn state_store(&self) -> &NativeStateStore {
        &self.state_store
    }

    #[inline]
    pub fn state_store_mut(&mut self) -> &mut NativeStateStore {
        &mut self.state_store
    }

    #[inline]
    pub fn recovery_stats(&self) -> StateRecoveryStats {
        self.state_store.recovery_stats()
    }

    /// Loads the best available pre-rendered Frame-0 `NVERELOC` scene before live runtime boot.
    pub fn load_frame0(&self, mode: VerifyMode) -> Option<InstantFrame0> {
        if self.user_frame0_path.exists()
            && let Ok(mapped) =
                MmapReloc::open_with_schema(&self.user_frame0_path, self.schema_version, mode)
            && SceneRelocView::from_view(mapped.view()).is_ok()
        {
            return Some(InstantFrame0::UserCached(mapped));
        }

        if let Some(default_path) = &self.default_frame0_path
            && default_path.exists()
            && let Ok(mapped) =
                MmapReloc::open_with_schema(default_path, self.schema_version, mode)
            && SceneRelocView::from_view(mapped.view()).is_ok()
        {
            return Some(InstantFrame0::BundledFile(mapped));
        }

        if let Some(bytes) = &self.default_frame0_bytes
            && let Ok(view) = bytes.view(mode)
            && view.require_schema(self.schema_version).is_ok()
            && SceneRelocView::from_view(view).is_ok()
        {
            return Some(InstantFrame0::BundledBytes(bytes.clone()));
        }

        None
    }

    pub fn state_bytes(&self) -> Result<Option<&[u8]>, InstantBootError> {
        self.state_store.load_bytes()
    }

    pub fn save_state(&mut self, state_bytes: &[u8]) -> Result<StateSaveOutcome, InstantBootError> {
        self.state_store.save(state_bytes)
    }

    pub fn save_user_frame0(&mut self, scene_reloc_bytes: &[u8]) -> Result<(), InstantBootError> {
        let view = SceneRelocView::from_bytes(scene_reloc_bytes, VerifyMode::HeaderOnly)?;
        if view.schema_version() != self.schema_version {
            return Err(InstantBootError::Reloc(RelocError::SchemaVersionMismatch {
                expected: self.schema_version,
                actual: view.schema_version(),
            }));
        }
        write_reloc_atomic(&self.user_frame0_path, scene_reloc_bytes)?;
        Ok(())
    }

    pub fn save_checkpoint(
        &mut self,
        scene_reloc_bytes: &[u8],
        state_bytes: &[u8],
    ) -> Result<StateSaveOutcome, InstantBootError> {
        let outcome = self.save_state(state_bytes)?;
        self.state_store.sync_all()?;
        if !scene_reloc_bytes.is_empty() {
            self.save_user_frame0(scene_reloc_bytes)?;
        }
        Ok(outcome)
    }

    pub fn reset(&mut self) -> Result<(), InstantBootError> {
        self.state_store.reset()?;
        if self.user_frame0_path.exists() {
            fs::remove_file(&self.user_frame0_path)?;
        }
        Ok(())
    }
}
