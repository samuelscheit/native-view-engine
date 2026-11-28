//! Tier-2 modular `nve_get_interface` `cdylib` hot-swap loader.
//!
//! Resolves the single `nve_get_interface` entry point once at load time, caches the small
//! `const` interface tables (`NveSurfaceApi`, plus optional `NveLayoutApi`, `NveComponentApi`,
//! `NveDevApi`), and exposes direct zero-cost calls + state-preserving generation reloads.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use libloading::Library;
use nve_abi::{
    NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveGestureDisposition,
    NveGetInterfaceFn, NveHostApi, NveHostCallbacks, NveIntrinsicSize, NveLayoutApi, NveMapEntry,
    NveSizeConstraints, NveSurfaceApi, SurfaceId, ValidatedEvents, NVE_IFACE_COMPONENT,
    NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};

static STAGED_COPY_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum LoaderError {
    /// The library could not be staged (missing / unreadable source, temp dir failure).
    Io(std::io::Error),
    /// The OS loader rejected the staged copy (`dlopen` / `LoadLibraryW`).
    Load(libloading::Error),
    /// The library loaded but exports no `nve_get_interface`.
    MissingEntry(libloading::Error),
    MissingSurfaceInterface,
    NullHandle,
    /// Loading in place (no staging root) a file this process already loaded in place: the OS
    /// loader would hand back the mapped image, silently running the old code.
    AlreadyLoadedInPlace(PathBuf),
}

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for LoaderError {}

/// Whether [`StagedDylib`] loads a private copy (desktop) or the file in place (mobile).
///
/// Android (API 29+ SELinux `execute_no_trans` / W^X) only executes code from the APK's
/// `nativeLibraryDir`, and `std::env::temp_dir()` there (`/data/local/tmp`) is not app-writable;
/// iOS only loads signed images from the bundle. Mobile hot reload therefore relies on the dev
/// tooling pushing a uniquely named (versioned) library per generation, or on an app-private,
/// loadable staging root in [`MODULE_STAGE_DIR_ENV`] (Android `Context.getCodeCacheDir()`).
pub const STAGES_PRIVATE_COPY: bool = !cfg!(any(target_os = "android", target_os = "ios"));

/// Environment variable naming the staging root (instead of `std::env::temp_dir()`); set, it
/// stages on every platform.
pub const MODULE_STAGE_DIR_ENV: &str = "NVE_MODULE_STAGE_DIR";

/// Where staged copies go: [`MODULE_STAGE_DIR_ENV`], else the temp dir when
/// [`STAGES_PRIVATE_COPY`]; `None` loads in place.
fn stage_root() -> Option<PathBuf> {
    match std::env::var_os(MODULE_STAGE_DIR_ENV).filter(|v| !v.is_empty()) {
        Some(root) => Some(PathBuf::from(root)),
        None => STAGES_PRIVATE_COPY.then(std::env::temp_dir),
    }
}

/// The private directory staged copies for `subdir` go to (`<root>/<subdir>-<euid>` on unix, so
/// users sharing `/tmp` never collide; `temp_dir()` is already per user on Windows); `None` when
/// loading in place.
pub fn staging_dir(subdir: &str) -> Option<PathBuf> {
    stage_root().map(|root| staging_dir_in(&root, subdir))
}

fn staging_dir_in(root: &Path, subdir: &str) -> PathBuf {
    #[cfg(unix)]
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    return root.join(format!("{subdir}-{}", unsafe { libc::geteuid() }));
    #[cfg(not(unix))]
    return root.join(subdir);
}

/// `(device, inode)` of every file loaded in place by this process.
static IN_PLACE_FILES: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());

/// Exclusive claim on a file loaded in place, released (after the unload) on drop.
struct InPlaceClaim((u64, u64));

impl InPlaceClaim {
    fn new(path: &Path) -> Result<Self, LoaderError> {
        let meta = std::fs::metadata(path).map_err(LoaderError::Io)?;
        #[cfg(unix)]
        let id = {
            use std::os::unix::fs::MetadataExt;
            (meta.dev(), meta.ino())
        };
        // In-place loading only happens on Android / iOS (unix); the claim is moot elsewhere.
        #[cfg(not(unix))]
        let id = {
            let _ = meta;
            (0, IN_PLACE_FILES_UNIQUE.fetch_add(1, Ordering::Relaxed))
        };
        let mut loaded = IN_PLACE_FILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if loaded.contains(&id) {
            return Err(LoaderError::AlreadyLoadedInPlace(path.to_path_buf()));
        }
        loaded.push(id);
        Ok(Self(id))
    }
}

#[cfg(not(unix))]
static IN_PLACE_FILES_UNIQUE: AtomicU64 = AtomicU64::new(0);

impl Drop for InPlaceClaim {
    fn drop(&mut self) {
        let mut loaded = IN_PLACE_FILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(at) = loaded.iter().position(|&id| id == self.0) {
            loaded.swap_remove(at);
        }
    }
}

/// Canonical RAII dynamic-library loader. On desktop it stages a unique copy in a private
/// (`0700`, per-user, exclusively created) staging directory before calling `dlopen` (a rebuilt
/// file at the same path always runs new code; no Windows DLL locks) and unloads before removing
/// the copy. On Android / iOS it loads the file in place ([`STAGES_PRIVATE_COPY`]) unless
/// [`MODULE_STAGE_DIR_ENV`] names a staging root, refusing a file it already loaded in place.
pub struct StagedDylib {
    lib: Option<Library>,
    get_interface: NveGetInterfaceFn,
    loaded_path: PathBuf,
    /// `true` when `loaded_path` is our private copy (removed after unloading).
    owns_copy: bool,
    /// Held while loaded in place; dropped after `lib`.
    _in_place: Option<InPlaceClaim>,
}

impl StagedDylib {
    pub fn load(dylib_path: &Path, subdir: &str, label: &str) -> Result<Self, LoaderError> {
        let (loaded_path, owns_copy, in_place) = match stage_root() {
            Some(root) => (stage_private_copy(&root, dylib_path, subdir, label)?, true, None),
            None => (dylib_path.to_path_buf(), false, Some(InPlaceClaim::new(dylib_path)?)),
        };
        let discard = |path: &Path| {
            if owns_copy {
                let _ = std::fs::remove_file(path);
            }
        };
        // SAFETY: Loading an NVE app `cdylib` via `libloading`.
        let lib = match unsafe { Library::new(&loaded_path) } {
            Ok(lib) => lib,
            Err(err) => {
                discard(&loaded_path);
                return Err(LoaderError::Load(err));
            }
        };
        // SAFETY: `nve_get_interface` conforms to the `NveGetInterfaceFn` C ABI signature.
        let get_interface: NveGetInterfaceFn = match unsafe {
            lib.get::<NveGetInterfaceFn>(b"nve_get_interface")
        } {
            Ok(sym) => *sym,
            Err(err) => {
                drop(lib);
                discard(&loaded_path);
                return Err(LoaderError::MissingEntry(err));
            }
        };
        Ok(Self {
            lib: Some(lib),
            get_interface,
            loaded_path,
            owns_copy,
            _in_place: in_place,
        })
    }

    pub fn get_interface(&self) -> NveGetInterfaceFn {
        self.get_interface
    }

    /// The file the OS loaded: the private staged copy on desktop, the source path on mobile.
    pub fn loaded_copy_path(&self) -> &Path {
        &self.loaded_path
    }
}

impl Drop for StagedDylib {
    fn drop(&mut self) {
        // Unload the dynamic library before removing the temporary copy so Windows
        // releases the OS file lock prior to deletion.
        drop(self.lib.take());
        if self.owns_copy {
            let _ = std::fs::remove_file(&self.loaded_path);
        }
    }
}

/// Copies `dylib_path` into [`staging_dir_in`] under a unique name. The directory is private
/// (created `0700`; an existing one must
/// be a real directory owned by the effective user, tightened to `0700`) and the copy is created
/// exclusively without following symlinks, so no other user can swap the code between copy and
/// `dlopen`.
fn stage_private_copy(
    root: &Path,
    dylib_path: &Path,
    subdir: &str,
    label: &str,
) -> Result<PathBuf, LoaderError> {
    /// Fresh names tried when a stale copy (a crashed process with a recycled pid) holds one.
    const MAX_NAME_ATTEMPTS: u32 = 16;
    let ext = dylib_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or(std::env::consts::DLL_EXTENSION);
    let stem = dylib_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("nve_app");
    let dir = ensure_private_dir(staging_dir_in(root, subdir)).map_err(LoaderError::Io)?;
    let mut src = std::fs::File::open(dylib_path).map_err(LoaderError::Io)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700).custom_flags(libc::O_NOFOLLOW);
    }
    let mut attempt = 0u32;
    let (copy, mut dst) = loop {
        let seq = STAGED_COPY_SEQ.fetch_add(1, Ordering::Relaxed);
        let copy = dir.join(format!("{stem}_{label}_{}_{seq}.{ext}", std::process::id()));
        match options.open(&copy) {
            Ok(file) => break (copy, file),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && attempt + 1 < MAX_NAME_ATTEMPTS => {
                attempt += 1;
            }
            Err(err) => return Err(LoaderError::Io(err)),
        }
    };
    if let Err(err) = std::io::copy(&mut src, &mut dst).and_then(|_| dst.sync_all()) {
        drop(dst);
        let _ = std::fs::remove_file(&copy);
        return Err(LoaderError::Io(err));
    }
    Ok(copy)
}

fn ensure_private_dir(dir: PathBuf) -> std::io::Result<PathBuf> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(&dir) {
        Ok(()) => return Ok(dir),
        Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => return Err(err),
        Err(_) => {}
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.file_type().is_dir() {
        return Err(std::io::Error::other(format!(
            "staging path {} is not a directory",
            dir.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        let euid = unsafe { libc::geteuid() };
        if meta.uid() != euid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("staging dir {} is owned by uid {}, not {euid}", dir.display(), meta.uid()),
            ));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    Ok(dir)
}

/// Summary returned after a successful transactional [`HotAppInstance::reload`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadOutcome {
    pub previous_generation: u64,
    pub generation: u64,
    pub preserved_state_bytes: usize,
}

pub struct HotAppInstance {
    staged: Option<StagedDylib>,
    surface: NveSurfaceApi,
    layout: Option<NveLayoutApi>,
    component: Option<NveComponentApi>,
    dev: Option<NveDevApi>,
    host: Option<NveHostApi>,
    bound_host: Option<NveHostCallbacks>,
    handle: NveAppHandle,
    surface_id: SurfaceId,
    generation: u64,
}

impl HotAppInstance {
    /// Loads a compiled app `cdylib` via a unique staged filename so `dlopen` never returns
    /// a stale cached image (and Windows never hits DLL lock collisions during hot-swap),
    /// resolves `nve_get_interface`, and optionally restores state.
    pub fn load(
        dylib_path: &Path,
        surface_id: SurfaceId,
        generation: u64,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, LoaderError> {
        let staged = StagedDylib::load(dylib_path, "nve-hotswap", &format!("gen{generation}"))?;
        let get_iface = staged.get_interface();

        // SAFETY: `get_iface` returns static, properly aligned NVE C-ABI vtable pointers (or null)
        // that remain valid while `staged` keeps the dylib loaded; `surface.create` / `restore_state`
        // obey the NVE surface/dev C ABI contract.
        let (surface, layout, component, dev, host, handle) = unsafe {
            let surface_ptr = get_iface(NVE_IFACE_SURFACE) as *const NveSurfaceApi;
            if surface_ptr.is_null() {
                return Err(LoaderError::MissingSurfaceInterface);
            }
            let surface = *surface_ptr;

            let layout_ptr = get_iface(NVE_IFACE_LAYOUT) as *const NveLayoutApi;
            let layout = (!layout_ptr.is_null()).then(|| *layout_ptr);

            let component_ptr = get_iface(NVE_IFACE_COMPONENT) as *const NveComponentApi;
            let component = (!component_ptr.is_null()).then(|| *component_ptr);

            let dev_ptr = get_iface(NVE_IFACE_DEV) as *const NveDevApi;
            let dev = (!dev_ptr.is_null()).then(|| *dev_ptr);

            let host_ptr = get_iface(NVE_IFACE_HOST) as *const NveHostApi;
            let host = (!host_ptr.is_null()).then(|| *host_ptr);

            let handle = (surface.create)(surface_id);
            if handle.is_null() {
                return Err(LoaderError::NullHandle);
            }
            if let (Some(dev_api), Some(state)) = (dev, prior_state)
                && !state.is_empty()
            {
                (dev_api.restore_state)(handle, NveBufferSlice::from_slice(state));
            }
            (surface, layout, component, dev, host, handle)
        };

        Ok(Self {
            staged: Some(staged),
            surface,
            layout,
            component,
            dev,
            host,
            bound_host: None,
            handle,
            surface_id,
            generation,
        })
    }

    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn loaded_copy_path(&self) -> &Path {
        self.staged
            .as_ref()
            .map(StagedDylib::loaded_copy_path)
            .unwrap_or_else(|| Path::new(""))
    }

    /// Renders a frame and borrows the binary `NVE2` wire frame slice directly from the loaded
    /// app module's internal frame buffer (zero heap allocation / zero `.to_vec()` copy per frame).
    ///
    /// The borrowed slice remains valid until the next `&mut self` call on this [`HotAppInstance`].
    pub fn render(&mut self, frame_id: nve_abi::FrameId) -> &[u8] {
        // SAFETY: `self.handle` is live while `self` owns it; returned buffer stays valid until next `&mut self` call.
        unsafe { (self.surface.render)(self.handle, frame_id).as_slice() }
    }

    /// Forwards a validated zero-copy event batch directly across the C FFI boundary.
    pub fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        // SAFETY: `self.handle` is live and `events` stays valid for the synchronous FFI call.
        unsafe { (self.surface.dispatch_events)(self.handle, events.as_raw_batch()) != 0 }
    }

    pub fn measure(&mut self, constraints: NveSizeConstraints) -> Option<NveIntrinsicSize> {
        self.layout.map(|api| {
            // SAFETY: `self.handle` is live and `api` was resolved from the same loaded dylib.
            unsafe { (api.measure)(self.handle, constraints) }
        })
    }

    pub fn hit_test(&mut self, x: f32, y: f32, pointer_phase: u8) -> NveGestureDisposition {
        match self.layout {
            Some(api) => {
                // SAFETY: `self.handle` is live and `api` was resolved from the same loaded dylib.
                NveGestureDisposition::from_u8(unsafe {
                    (api.hit_test)(self.handle, x, y, pointer_phase)
                })
            }
            None => NveGestureDisposition::PassThrough,
        }
    }

    pub fn set_props(&mut self, entries: &[NveMapEntry]) -> bool {
        match self.component {
            // SAFETY: `self.handle` is live and `entries` stays valid for the synchronous FFI call.
            Some(api) => unsafe {
                (api.set_props)(
                    self.handle,
                    entries.as_ptr(),
                    entries.len().min(u32::MAX as usize) as u32,
                ) != 0
            },
            None => false,
        }
    }

    /// Drains outbound component events as an ingress-validated zero-copy [`ValidatedEvents`].
    pub fn drain_emitted_events(&mut self) -> ValidatedEvents<'_> {
        match self.component {
            // SAFETY: `self.handle` is live; returned batch borrows from the module's buffer until next `&mut self` call.
            Some(api) => unsafe {
                ValidatedEvents::from_raw((api.drain_emitted_events)(self.handle))
            },
            None => ValidatedEvents::EMPTY,
        }
    }

    /// Forwards one binary `NVP1` hot-patch frame (as received from `nve dev`) to the module.
    pub fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        match self.dev {
            // SAFETY: `self.handle` is live and `patch_nvp1` stays valid for the synchronous FFI call.
            Some(api) => unsafe {
                (api.apply_hot_patch)(self.handle, NveBufferSlice::from_slice(patch_nvp1)) != 0
            },
            None => false,
        }
    }

    /// Captures serialized app state and borrows the slice directly from the loaded app module's
    /// internal snapshot buffer without copying.
    pub fn snapshot_state(&mut self) -> &[u8] {
        match self.dev {
            // SAFETY: `self.handle` is live; returned slice stays valid until the next `&mut self` call.
            Some(api) => unsafe { (api.snapshot_state)(self.handle).as_slice() },
            None => &[],
        }
    }

    pub fn has_host_interface(&self) -> bool {
        self.host.is_some()
    }

    /// Binds (or unbinds when `None`) host capability callbacks via `NVE_IFACE_HOST = 5`.
    /// Returns `true` if the loaded module exposes `NveHostApi`.
    pub fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) -> bool {
        self.bound_host = callbacks;
        match self.host {
            Some(api) => {
                let ptr = callbacks
                    .as_ref()
                    .map_or(std::ptr::null(), |cb| cb as *const NveHostCallbacks);
                // SAFETY: `self.handle` is live and `ptr` is null or points to the valid `NveHostCallbacks` copy.
                unsafe { (api.bind_host)(self.handle, ptr) };
                true
            }
            None => false,
        }
    }

    /// Dispatches a host-event payload buffer via `NveHostApi::dispatch_host_events`.
    pub fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        match self.host {
            // SAFETY: `self.handle` is live and `batch` stays valid for the synchronous FFI call.
            Some(api) => unsafe {
                (api.dispatch_host_events)(self.handle, NveBufferSlice::from_slice(batch)) != 0
            },
            None => false,
        }
    }

    /// Transactionally hot-swaps this live instance to a freshly compiled dylib while preserving
    /// app state.
    ///
    /// Snapshots state from the live app, then stages (under a unique filename to avoid Windows
    /// DLL lock collisions), loads, validates (`nve_get_interface` + `NVE_IFACE_SURFACE`),
    /// instantiates, and restores state on the candidate dylib BEFORE swapping out and unloading
    /// the existing instance. If any step fails, `self` stays 100% alive and untouched.
    pub fn reload(&mut self, new_dylib_path: &Path) -> Result<ReloadOutcome, LoaderError> {
        let surface_id = self.surface_id;
        let previous_generation = self.generation;
        let next_gen = previous_generation + 1;
        let bound_host = self.bound_host;

        let (mut candidate, preserved_state_bytes) = {
            let saved = self.snapshot_state();
            let len = saved.len();
            let prior = (!saved.is_empty()).then_some(saved);
            (Self::load(new_dylib_path, surface_id, next_gen, prior)?, len)
        };
        if let Some(cb) = bound_host {
            candidate.bind_host(Some(cb));
        }
        let old = std::mem::replace(self, candidate);
        drop(old);
        Ok(ReloadOutcome {
            previous_generation,
            generation: next_gen,
            preserved_state_bytes,
        })
    }
}

impl Drop for HotAppInstance {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: `self.handle` was created by `self.surface.create` and is destroyed once before unloading `staged`.
            unsafe { (self.surface.destroy)(self.handle) };
            self.handle = std::ptr::null_mut();
        }
        drop(self.staged.take());
    }
}

#[cfg(all(test, unix))]
mod staging_tests {
    use super::{InPlaceClaim, LoaderError, ensure_private_dir, stage_private_copy, staging_dir_in};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn copies_stage_in_a_per_user_private_dir() {
        let root = scratch("root");
        std::fs::create_dir(&root).expect("test fs op");
        let src = root.join("libapp.so");
        std::fs::write(&src, b"code").expect("test fs op");
        let copy = stage_private_copy(&root, &src, "mods", "g1").expect("stage");
        let dir = staging_dir_in(&root, "mods");
        // SAFETY: `geteuid` has no preconditions.
        assert_eq!(dir, root.join(format!("mods-{}", unsafe { libc::geteuid() })));
        assert_eq!(copy.parent(), Some(dir.as_path()));
        assert_eq!(std::fs::read(&copy).expect("test fs op"), b"code");
        assert_eq!(mode(&dir), 0o700);
        std::fs::remove_dir_all(&root).expect("test fs op");
    }

    #[test]
    fn a_file_loaded_in_place_is_claimed_until_unloaded() {
        let dir = scratch("claim");
        std::fs::create_dir(&dir).expect("test fs op");
        let (file, alias) = (dir.join("libapp.so"), dir.join("libapp-v2.so"));
        std::fs::write(&file, b"code").expect("test fs op");
        std::fs::hard_link(&file, &alias).expect("test fs op");
        let claim = InPlaceClaim::new(&file).expect("first load");
        for again in [&file, &alias] {
            assert!(
                matches!(InPlaceClaim::new(again), Err(LoaderError::AlreadyLoadedInPlace(_))),
                "the same inode (also through another name) would map the old image"
            );
        }
        drop(claim);
        drop(InPlaceClaim::new(&alias).expect("released on unload"));
        std::fs::remove_dir_all(&dir).expect("test fs op");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nve-loader-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&dir);
        dir
    }

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::symlink_metadata(path).expect("test fs op").permissions().mode() & 0o777
    }

    #[test]
    fn staging_dir_is_created_private() {
        let dir = scratch("fresh");
        ensure_private_dir(dir.clone()).expect("create");
        assert_eq!(mode(&dir), 0o700);
        ensure_private_dir(dir.clone()).expect("reuse");
        std::fs::remove_dir_all(&dir).expect("test fs op");
    }

    #[test]
    fn an_own_shared_staging_dir_is_tightened() {
        let dir = scratch("loose");
        std::fs::create_dir(&dir).expect("test fs op");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).expect("test fs op");
        ensure_private_dir(dir.clone()).expect("own dir");
        assert_eq!(mode(&dir), 0o700, "group / other access removed");
        std::fs::remove_dir_all(&dir).expect("test fs op");
    }

    #[test]
    fn symlinked_or_file_staging_paths_are_rejected() {
        let target = scratch("target");
        std::fs::create_dir(&target).expect("test fs op");
        let link = scratch("link");
        std::os::unix::fs::symlink(&target, &link).expect("test fs op");
        assert!(ensure_private_dir(link.clone()).is_err(), "a symlink is never followed");
        std::fs::remove_file(&link).expect("test fs op");
        std::fs::write(&link, b"x").expect("test fs op");
        assert!(ensure_private_dir(link.clone()).is_err(), "a file is not a directory");
        std::fs::remove_file(&link).expect("test fs op");
        std::fs::remove_dir_all(&target).expect("test fs op");
    }
}
