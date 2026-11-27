//! App files on disk: `cdylib` app modules loaded from private staged copies (so a rebuilt file
//! always runs fresh code and never stays locked), and crash-atomic [`StateFile`] persistence of
//! the attached app's `NVE_IFACE_DEV` state. Native only; the C ABI reports `Unsupported` on wasm.

use std::path::Path;

use nve_engine::loader::{LoaderError, StagedDylib};
use nve_engine::{CapabilityUnsupported, PersistenceError, StateFile, MAX_STATE_FILE_BYTES};

use crate::app::{CAbiAppAdapter, RuntimeApp};
use crate::builder::RuntimeError;
use crate::hooks::with_app;
use crate::surface::SurfaceRuntime;

/// `std::env::temp_dir()` subdirectory holding staged module copies (each removed on unload).
const MODULE_STAGING_DIR: &str = "nve-runtime-modules";

/// The loader failure as a runtime error naming the module `path` (hosts show it to developers).
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
    }
}

fn persistence_error(e: PersistenceError) -> RuntimeError {
    RuntimeError::Io(format!("state file: {e}"))
}

impl SurfaceRuntime {
    /// Owner-thread, unpoisoned precondition of every file operation, checked before any file
    /// is touched (staged, quarantined, rotated, or deleted).
    fn check_file_op(&self) -> Result<(), RuntimeError> {
        self.check_not_poisoned()?;
        self.check_owner_thread()
    }

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
}
