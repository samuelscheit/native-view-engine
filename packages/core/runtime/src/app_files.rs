//! App files on disk & Frame-0 keyframe boot: `cdylib` app modules loaded from private staged
//! copies, crash-atomic [`StateFile`] persistence of `NVE_IFACE_DEV` state, and zero-`unsafe`
//! [`Frame0File`] save/boot (disk on native; `boot_frame0_bytes` on all targets including wasm32).

use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use nve_engine::loader::{LoaderError, StagedDylib};
#[cfg(not(target_arch = "wasm32"))]
use nve_engine::{CapabilityUnsupported, PersistenceError, StateFile, MAX_STATE_FILE_BYTES};
use nve_engine::{Frame0File, Frame0Meta, Frame0Miss};
use nve_tables::tables::core_tables_schema_hash;

#[cfg(not(target_arch = "wasm32"))]
use crate::app::{CAbiAppAdapter, RuntimeApp};
use crate::builder::RuntimeError;
#[cfg(not(target_arch = "wasm32"))]
use crate::hooks::with_app;
use crate::surface::SurfaceRuntime;

/// Staging-root subdirectory (per user, see `StagedDylib`) holding staged module copies (each
/// removed on unload).
#[cfg(not(target_arch = "wasm32"))]
pub const MODULE_STAGING_DIR: &str = "nve-runtime-modules";

/// The loader failure as a runtime error naming the module `path` (hosts show it to developers).
#[cfg(not(target_arch = "wasm32"))]
fn loader_error(e: LoaderError, path: &Path) -> RuntimeError {
    let path = path.display();
    match e {
        LoaderError::Io(e) => RuntimeError::Io(format!("cannot stage app module {path}: {e}")),
        LoaderError::Load(e) => RuntimeError::Io(format!("cannot load app module {path}: {e}")),
        LoaderError::MissingEntry(_) => {
            RuntimeError::NotFound("app module exports no `nve_get_interface`")
        }
        LoaderError::MissingSurfaceInterface | LoaderError::NullHandle => {
            RuntimeError::Unsupported("app module exports no usable NVE_IFACE_SURFACE")
        }
        LoaderError::AlreadyLoadedInPlace(_) => RuntimeError::Io(format!(
            "app module {path} is already loaded in place; push a versioned file or set {}",
            nve_engine::loader::MODULE_STAGE_DIR_ENV
        )),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn persistence_error(e: PersistenceError) -> RuntimeError {
    RuntimeError::Io(format!("state file: {e}"))
}

fn frame0_miss_error(miss: Frame0Miss) -> RuntimeError {
    match miss {
        Frame0Miss::NotFound => RuntimeError::NotFound("frame0.nve2 not found"),
        Frame0Miss::Io(e) => RuntimeError::Io(format!("frame0.nve2 I/O error: {e}")),
        Frame0Miss::FlagsMismatch(_) => RuntimeError::NotFound("frame0.nve2 unsupported flags"),
        Frame0Miss::Corrupt(msg) => RuntimeError::NotFound(msg),
        Frame0Miss::Backend(e) => RuntimeError::Io(format!("frame0.nve2 backend error: {e}")),
        Frame0Miss::SchemaMismatch { .. } => {
            RuntimeError::NotFound("frame0.nve2 table schema hash mismatch")
        }
        Frame0Miss::BuildIdMismatch { .. } => {
            RuntimeError::NotFound("frame0.nve2 app_build_id mismatch")
        }
        Frame0Miss::EnvHashMismatch { .. } => {
            RuntimeError::NotFound("frame0.nve2 env_hash mismatch")
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn should_remove_frame0_file(miss: &Frame0Miss) -> bool {
    matches!(
        miss,
        Frame0Miss::Corrupt(_)
            | Frame0Miss::FlagsMismatch(_)
            | Frame0Miss::SchemaMismatch { .. }
            | Frame0Miss::BuildIdMismatch { .. }
            | Frame0Miss::EnvHashMismatch { .. }
    )
}

impl SurfaceRuntime {
    /// Owner-thread, unpoisoned precondition of every file operation, checked before any file
    /// is touched (staged, quarantined, rotated, or deleted).
    fn check_file_op(&self) -> Result<(), RuntimeError> {
        self.check_not_poisoned()?;
        self.check_owner_thread()
    }

    /// Validates an in-memory `frame0.nve2` envelope buffer against `core_tables_schema_hash()`,
    /// `app_build_id`, and `env_hash`, applies it to the replica store, lays out, and publishes
    /// Frame-0 immediately (supported on all targets including `wasm32`).
    pub fn boot_frame0_bytes(
        &mut self,
        envelope: &[u8],
        app_build_id: u64,
        env_hash: u64,
    ) -> Result<(), RuntimeError> {
        self.boot_frame0_bytes_shared(envelope, app_build_id, env_hash)
    }

    pub(crate) fn boot_frame0_bytes_shared(
        &self,
        envelope: &[u8],
        app_build_id: u64,
        env_hash: u64,
    ) -> Result<(), RuntimeError> {
        self.check_file_op()?;
        self.with_write(|inner| {
            if inner.app.is_some() {
                return Err(RuntimeError::InvalidArg(
                    "cannot boot frame0 while an app is attached",
                ));
            }
            if inner.pair.is_some() {
                return Err(RuntimeError::InvalidArg(
                    "cannot boot frame0 while a pair session is attached",
                ));
            }
            Ok(())
        })?;
        let (footer, payload) = Frame0File::read_and_validate_bytes(
            envelope,
            core_tables_schema_hash(),
            app_build_id,
            env_hash,
        )
        .map_err(frame0_miss_error)?;
        self.with_write(|inner| inner.boot_validated_frame0(payload, footer.node_count as usize))?;
        self.intrinsic_size.notify();
        self.wake.trigger();
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    const NO_FILES: RuntimeError = RuntimeError::Unsupported("app files need a native filesystem");

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn load_app_module_shared(&self, _: &Path, _: bool) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn save_state_file_shared(&self, _: &Path, _: u32) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn restore_state_file_shared(&self, _: &Path, _: u32) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn clear_state_file_shared(&self, _: &Path) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn save_frame0_file_shared(
        &self,
        _: &Path,
        _: u64,
        _: u64,
    ) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn boot_frame0_file_shared(
        &self,
        _: &Path,
        _: u64,
        _: u64,
    ) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SurfaceRuntime {
    /// [`Self::check_file_op`] plus an attached app with state support and no scene held
    /// (`SceneBusy`): a state file is never opened (created, rotated, quarantined) for a call
    /// that cannot complete.
    fn check_state_op(&self, capability: &'static str) -> Result<(), RuntimeError> {
        self.check_file_op()?;
        self.with_write(|inner| {
            with_app(inner, capability, |app| {
                if app.supports_state() { Ok(()) } else { Err(CapabilityUnsupported) }
            })
        })
    }

    /// Loads the app `cdylib` at `path` from a private staged copy and attaches it, replacing the
    /// running app or pairing; the previous module unloads after its app is destroyed.
    ///
    /// With `carry_state`, the running app's state snapshot is restored into the new app before
    /// it goes live (a Tier-2 hot reload). Any failure leaves the running app untouched.
    pub fn load_app_module(&mut self, path: &Path, carry_state: bool) -> Result<(), RuntimeError> {
        self.load_app_module_shared(path, carry_state)
    }

    pub(crate) fn load_app_module_shared(
        &self,
        path: &Path,
        carry_state: bool,
    ) -> Result<(), RuntimeError> {
        self.check_file_op()?;
        // `SceneBusy` before staging, `dlopen` and `create`: a held scene would refuse the
        // attach only after the module's initializers and app constructor already ran.
        self.with_write(|_| Ok(()))?;
        let label = format!("surface{}", self.surface_id);
        let module = StagedDylib::load(path, MODULE_STAGING_DIR, &label).map_err(|e| loader_error(e, path))?;
        let mut app = CAbiAppAdapter::from_staged_dylib(module, self.surface_id)?;
        if carry_state
            && let Some(state) = self.app_state_copy()?
            && !state.is_empty()
        {
            // A new module without `NVE_IFACE_DEV` starts fresh.
            let _ = app.restore_state(&state);
        }
        self.attach_app_shared(Box::new(app))
    }

    /// Owned copy of the attached app's state snapshot (`None` without an app or `NVE_IFACE_DEV`).
    fn app_state_copy(&self) -> Result<Option<Vec<u8>>, RuntimeError> {
        self.with_write(|inner| {
            Ok(inner
                .app
                .as_deref_mut()
                .and_then(|app| app.snapshot_state().ok())
                .map(<[u8]>::to_vec))
        })
    }

    /// Persists the app's state snapshot to `path` as an atomic, checksummed [`StateFile`]
    /// tagged `schema_version` (an existing older-schema file is first rotated to `<file>.v<n>`,
    /// a corrupt one quarantined). `NotFound` when the app has no state; nothing is written then.
    pub fn save_state_file(&mut self, path: &Path, schema_version: u32) -> Result<(), RuntimeError> {
        self.save_state_file_shared(path, schema_version)
    }

    pub(crate) fn save_state_file_shared(
        &self,
        path: &Path,
        schema_version: u32,
    ) -> Result<(), RuntimeError> {
        self.check_file_op()?;
        let state = self.with_write(|inner| {
            with_app(inner, "app does not support state snapshots (NVE_IFACE_DEV)", |app| {
                app.snapshot_state().map(<[u8]>::to_vec)
            })
        })?;
        if state.is_empty() {
            return Err(RuntimeError::NotFound("app has no state to save"));
        }
        if state.len() > MAX_STATE_FILE_BYTES {
            return Err(RuntimeError::InvalidArg("app state exceeds MAX_STATE_FILE_BYTES"));
        }
        let mut file = StateFile::open(path, schema_version).map_err(persistence_error)?;
        file.save(&state).map_err(persistence_error)?;
        Ok(())
    }

    /// Restores a [`Self::save_state_file`] file into the attached app and wakes the host.
    ///
    /// `NotFound` when there is no usable file: none exists, it was corrupt (quarantined to
    /// `<file>.corrupt`), or it had an older schema (rotated to `<file>.v<n>`). A newer schema
    /// is an `Io` error and leaves the file in place.
    pub fn restore_state_file(&mut self, path: &Path, schema_version: u32) -> Result<(), RuntimeError> {
        self.restore_state_file_shared(path, schema_version)
    }

    pub(crate) fn restore_state_file_shared(
        &self,
        path: &Path,
        schema_version: u32,
    ) -> Result<(), RuntimeError> {
        self.check_state_op("app does not support state restore (NVE_IFACE_DEV)")?;
        let file = StateFile::open(path, schema_version).map_err(persistence_error)?;
        let Some(state) = file.load().map_err(persistence_error)? else {
            let recovery = file.recovery_stats();
            return Err(RuntimeError::NotFound(if recovery.recovered_from_corrupt {
                "state file was corrupt and moved to <file>.corrupt"
            } else if recovery.rotated_from_older_schema.is_some() {
                "state file had an older schema and was moved to <file>.v<schema>"
            } else {
                "no state file"
            }));
        };
        self.restore_state_shared(&state)
    }

    /// Deletes the state file at `path` (a cold start); succeeds when it is already absent.
    pub fn clear_state_file(&mut self, path: &Path) -> Result<(), RuntimeError> {
        self.clear_state_file_shared(path)
    }

    pub(crate) fn clear_state_file_shared(&self, path: &Path) -> Result<(), RuntimeError> {
        self.check_file_op()?;
        StateFile::remove(path).map_err(persistence_error)
    }

    /// Atomically persists the active `NodeStore` as a Frame-0 `frame0.nve2` file tagged with
    /// `core_tables_schema_hash()`, `app_build_id`, and `env_hash`. Returns `NotFound` if the
    /// surface currently has zero live nodes.
    pub fn save_frame0_file(
        &mut self,
        path: &Path,
        app_build_id: u64,
        env_hash: u64,
    ) -> Result<(), RuntimeError> {
        self.save_frame0_file_shared(path, app_build_id, env_hash)
    }

    pub(crate) fn save_frame0_file_shared(
        &self,
        path: &Path,
        app_build_id: u64,
        env_hash: u64,
    ) -> Result<(), RuntimeError> {
        self.check_file_op()?;
        let meta = Frame0Meta {
            table_schema_hash: core_tables_schema_hash(),
            app_build_id,
            env_hash,
        };
        let encoded = self.with_write(|inner| {
            if inner.state.frame_id.0 == 0 {
                return Err(RuntimeError::NotFound("surface has no committed frame to save"));
            }
            let store = inner.active_store();
            if store.stats().live_nodes == 0 {
                return Err(RuntimeError::NotFound("surface has no live nodes to save"));
            }
            Frame0File::encode(store, meta).map_err(|e| match e {
                PersistenceError::PayloadTooLarge(_) => {
                    RuntimeError::InvalidArg("frame0 exceeds MAX_FRAME0_FILE_BYTES")
                }
                other => RuntimeError::Io(format!("frame0 encode failed: {other}")),
            })
        })?;
        Frame0File::save_bytes(path, &encoded)
            .map_err(|e| RuntimeError::Io(format!("frame0 save failed: {e}")))
    }

    /// Validates and boots a persisted `frame0.nve2` keyframe from `path` into `replica_store`,
    /// commits layout, and publishes Frame-0 immediately. Corrupt or mismatched files (schema,
    /// build id, env hash, flags, or wire decode failure) are automatically removed and reported
    /// as `NotFound`.
    pub fn boot_frame0_file(
        &mut self,
        path: &Path,
        app_build_id: u64,
        env_hash: u64,
    ) -> Result<(), RuntimeError> {
        self.boot_frame0_file_shared(path, app_build_id, env_hash)
    }

    pub(crate) fn boot_frame0_file_shared(
        &self,
        path: &Path,
        app_build_id: u64,
        env_hash: u64,
    ) -> Result<(), RuntimeError> {
        self.check_file_op()?;
        // Verify surface preconditions (`SceneBusy`, no app/pair attached) before touching disk.
        self.with_write(|inner| {
            if inner.app.is_some() {
                return Err(RuntimeError::InvalidArg(
                    "cannot boot frame0 while an app is attached",
                ));
            }
            if inner.pair.is_some() {
                return Err(RuntimeError::InvalidArg(
                    "cannot boot frame0 while a pair session is attached",
                ));
            }
            Ok(())
        })?;
        let meta = Frame0Meta {
            table_schema_hash: core_tables_schema_hash(),
            app_build_id,
            env_hash,
        };
        let (footer, payload) = match Frame0File::read_and_validate(path, meta) {
            Ok(v) => v,
            Err(miss) => {
                if should_remove_frame0_file(&miss) {
                    let _ = std::fs::remove_file(path);
                }
                return Err(frame0_miss_error(miss));
            }
        };
        if let Err(err) =
            self.with_write(|inner| inner.boot_validated_frame0(&payload, footer.node_count as usize))
        {
            if matches!(err, RuntimeError::NotFound(_)) {
                let _ = std::fs::remove_file(path);
            }
            return Err(err);
        }
        self.intrinsic_size.notify();
        self.wake.trigger();
        Ok(())
    }
}
