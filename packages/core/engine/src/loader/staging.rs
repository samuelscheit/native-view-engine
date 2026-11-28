//! Private-copy dynamic library staging and digest-validated in-place load claims.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use libloading::Library;
use nve_abi::{NveGetInterfaceFn, Xxh64Hasher};

use super::LoaderError;

static STAGED_COPY_SEQ: AtomicU64 = AtomicU64::new(1);

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

/// `(file_len, xxh64)` of a dylib's raw bytes read directly from its open file descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileDigest {
    len: u64,
    xxh64: u64,
}

fn hash_open_file(file: &mut std::fs::File) -> std::io::Result<FileDigest> {
    let mut hasher = Xxh64Hasher::new(0);
    let mut buf = [0u8; 16 * 1024];
    let mut len = 0u64;
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                len = len.saturating_add(n as u64);
                hasher.update(&buf[..n]);
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(FileDigest { len, xxh64: hasher.digest() })
}

struct InPlaceEntry {
    /// `(device, inode)` from `fstat` on the open fd on Unix (`None` on non-Unix).
    inode: Option<(u64, u64)>,
    /// Canonical (or absolute/fallback) path resolved before opening and passed to `Library::new`.
    resolved_path: PathBuf,
    digest: FileDigest,
    /// Unsealed in-flight claims sharing this exact `(inode, resolved_path, digest)` entry.
    pending: usize,
    /// `true` once `Library::new` mapped this image (kept for process lifetime).
    sealed: bool,
}

/// Files claimed for in-place loading in this process, keyed by `(dev, ino)` and resolved path
/// together with `(file_len, xxh64)`. Re-opening an unchanged bundled library (Activity recreation,
/// multi-surface open, `detachApp()` -> `attachApp()`) is allowed; attempting to reload an inode
/// or resolved path whose bytes changed since an earlier load is rejected because `RTLD_NODELETE`
/// platforms (iOS/macOS dyld, Android Bionic, musl) would silently execute the old mapped image.
static IN_PLACE_FILES: Mutex<Vec<InPlaceEntry>> = Mutex::new(Vec::new());

/// Reservation for an in-place load. Dropping before [`Self::seal`] rolls it back (a failed
/// `dlopen` never mapped the image); after [`Self::seal`] the recorded digest stays sealed for the
/// lifetime of the process so only unchanged bytes can re-open that inode or resolved path.
struct InPlaceClaim {
    inode: Option<(u64, u64)>,
    resolved_path: PathBuf,
    digest: FileDigest,
    sealed: bool,
}

impl InPlaceClaim {
    fn new(path: &Path) -> Result<Self, LoaderError> {
        let resolved_path = std::fs::canonicalize(path)
            .or_else(|_| std::path::absolute(path))
            .unwrap_or_else(|_| path.to_path_buf());
        let mut file = std::fs::File::open(&resolved_path).map_err(LoaderError::Io)?;
        let meta = file.metadata().map_err(LoaderError::Io)?;
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            Some((meta.dev(), meta.ino()))
        };
        #[cfg(not(unix))]
        let inode = {
            let _ = meta;
            None
        };
        let digest = hash_open_file(&mut file).map_err(LoaderError::Io)?;

        let mut loaded = IN_PLACE_FILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for entry in loaded.iter() {
            let same_inode = inode.is_some() && entry.inode == inode;
            let same_path = entry.resolved_path == resolved_path;
            if (same_inode || same_path) && entry.digest != digest {
                return Err(LoaderError::AlreadyLoadedInPlace(resolved_path));
            }
        }
        if let Some(existing) = loaded.iter_mut().find(|e| {
            e.inode == inode && e.resolved_path == resolved_path && e.digest == digest
        }) {
            existing.pending += 1;
        } else {
            loaded.push(InPlaceEntry {
                inode,
                resolved_path: resolved_path.clone(),
                digest,
                pending: 1,
                sealed: false,
            });
        }
        Ok(Self { inode, resolved_path, digest, sealed: false })
    }

    fn resolved_path(&self) -> &Path {
        &self.resolved_path
    }

    /// Seals the recorded digest for the lifetime of the process once `dlopen` mapped the file.
    fn seal(&mut self) {
        if self.sealed {
            return;
        }
        self.sealed = true;
        let mut loaded = IN_PLACE_FILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = loaded.iter_mut().find(|e| {
            e.inode == self.inode
                && e.resolved_path == self.resolved_path
                && e.digest == self.digest
        }) {
            entry.sealed = true;
            entry.pending = entry.pending.saturating_sub(1);
        }
    }
}

impl Drop for InPlaceClaim {
    fn drop(&mut self) {
        if self.sealed {
            return;
        }
        let mut loaded = IN_PLACE_FILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(at) = loaded.iter().position(|e| {
            e.inode == self.inode
                && e.resolved_path == self.resolved_path
                && e.digest == self.digest
        }) {
            loaded[at].pending = loaded[at].pending.saturating_sub(1);
            if !loaded[at].sealed && loaded[at].pending == 0 {
                loaded.swap_remove(at);
            }
        }
    }
}

/// Canonical RAII dynamic-library loader. On desktop it stages a unique copy in a private
/// (`0700`, per-user, exclusively created) staging directory before calling `dlopen` (a rebuilt
/// file at the same path always runs new code; no Windows DLL locks) and unloads before removing
/// the copy. On Android / iOS it loads the file in place ([`STAGES_PRIVATE_COPY`]) unless
/// [`MODULE_STAGE_DIR_ENV`] names a staging root, permitting unchanged bundle re-opens while
/// refusing reloads whose bytes changed on disk.
pub struct StagedDylib {
    lib: Option<Library>,
    get_interface: NveGetInterfaceFn,
    loaded_path: PathBuf,
    /// `true` when `loaded_path` is our private copy (removed after unloading).
    owns_copy: bool,
    /// Digest-validated claim sealed once `dlopen` maps an in-place file.
    _in_place: Option<InPlaceClaim>,
}

impl StagedDylib {
    pub fn load(dylib_path: &Path, subdir: &str, label: &str) -> Result<Self, LoaderError> {
        let root = stage_root();
        Self::load_with_root(dylib_path, root.as_deref(), subdir, label)
    }

    fn load_with_root(
        dylib_path: &Path,
        root: Option<&Path>,
        subdir: &str,
        label: &str,
    ) -> Result<Self, LoaderError> {
        let (loaded_path, owns_copy, mut in_place) = match root {
            Some(root) => (stage_private_copy(root, dylib_path, subdir, label)?, true, None),
            None => {
                let claim = InPlaceClaim::new(dylib_path)?;
                (claim.resolved_path().to_path_buf(), false, Some(claim))
            }
        };
        let discard = |path: &Path| {
            if owns_copy {
                let _ = std::fs::remove_file(path);
            }
        };
        // SAFETY: Loading an NVE app `cdylib` via `libloading`.
        let lib = match unsafe { Library::new(&loaded_path) } {
            Ok(lib) => {
                // The OS loader has mapped this image; seal the digest-based claim for this
                // process even if symbol lookup fails or this handle is later dropped (`RTLD_NODELETE`).
                if let Some(claim) = in_place.as_mut() {
                    claim.seal();
                }
                lib
            }
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

    /// The file the OS loaded: the private staged copy on desktop, the resolved source path on mobile.
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

/// Copies `dylib_path` into [`staging_dir_in`] under a unique name.
///
/// On Unix, the directory is opened once with `O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`, verified
/// via `fstat` (`S_ISDIR`, `st_uid == geteuid()`), tightened via `fchmod(fd, 0o700)`, and the
/// copy is created exclusively via `openat(dir_fd, name, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW
/// | O_CLOEXEC, 0o600)`, closing all path-based TOCTOU symlink races.
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
    let dir = staging_dir_in(root, subdir);
    let mut src = std::fs::File::open(dylib_path).map_err(LoaderError::Io)?;

    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd};

        let dir_fd = open_private_dir(&dir).map_err(LoaderError::Io)?;
        let mut attempt = 0u32;
        let (file_name, c_name, mut dst) = loop {
            let seq = STAGED_COPY_SEQ.fetch_add(1, Ordering::Relaxed);
            let file_name = format!("{stem}_{label}_{}_{seq}.{ext}", std::process::id());
            if file_name.contains('/') {
                return Err(LoaderError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("staged library filename contains path separator: {file_name}"),
                )));
            }
            let c_name = CString::new(file_name.as_str()).map_err(|_| {
                LoaderError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "staged library filename contains interior NUL",
                ))
            })?;
            // SAFETY: `dir_fd` is a verified private directory fd and `c_name` is a single
            // NUL-terminated entry name; `mode` is promoted to `c_uint` for C variadic `openat`.
            let fd = unsafe {
                libc::openat(
                    dir_fd.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    0o600 as libc::c_uint,
                )
            };
            if fd >= 0 {
                // SAFETY: `fd` is a freshly opened, exclusive file descriptor owned by this call.
                let file = unsafe { std::fs::File::from_raw_fd(fd) };
                break (file_name, c_name, file);
            }
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::AlreadyExists && attempt + 1 < MAX_NAME_ATTEMPTS {
                attempt += 1;
            } else {
                return Err(LoaderError::Io(err));
            }
        };
        if let Err(err) = std::io::copy(&mut src, &mut dst).and_then(|_| dst.sync_all()) {
            drop(dst);
            // SAFETY: `dir_fd` and `c_name` refer to the entry just created above.
            let _ = unsafe { libc::unlinkat(dir_fd.as_raw_fd(), c_name.as_ptr(), 0) };
            return Err(LoaderError::Io(err));
        }
        Ok(dir.join(file_name))
    }

    #[cfg(not(unix))]
    {
        let dir = ensure_private_dir_non_unix(dir).map_err(LoaderError::Io)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
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
}

/// Atomically creates (if absent), opens without following symlinks, verifies (`S_ISDIR` and
/// `st_uid == geteuid()`), and `fchmod(0o700)`s `dir`, returning the pinned directory fd so
/// subsequent `openat` calls cannot race a symlink swap.
#[cfg(unix)]
fn open_private_dir(dir: &Path) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let c_dir = CString::new(dir.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "staging dir contains interior NUL")
    })?;
    // SAFETY: `c_dir` is a valid NUL-terminated path; `EEXIST` is handled below via `open`.
    if unsafe { libc::mkdir(c_dir.as_ptr(), 0o700) } != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(err);
        }
    }
    // SAFETY: `c_dir` is NUL-terminated; `O_DIRECTORY | O_NOFOLLOW` refuses symlinks and files.
    let fd = unsafe {
        libc::open(
            c_dir.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a valid newly opened directory descriptor.
    let dir_fd = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `dir_fd` is live and `st` points to writable `libc::stat` storage.
    if unsafe { libc::fstat(dir_fd.as_raw_fd(), st.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fstat` succeeded and initialized `st`.
    let st = unsafe { st.assume_init() };
    if (st.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        return Err(std::io::Error::other(format!(
            "staging path {} is not a directory",
            dir.display()
        )));
    }
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    if st.st_uid != euid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("staging dir {} is owned by uid {}, not {euid}", dir.display(), st.st_uid),
        ));
    }
    // SAFETY: `dir_fd` is a verified directory owned by the effective user.
    if unsafe { libc::fchmod(dir_fd.as_raw_fd(), 0o700) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(dir_fd)
}

#[cfg(not(unix))]
fn ensure_private_dir_non_unix(dir: PathBuf) -> std::io::Result<PathBuf> {
    match std::fs::DirBuilder::new().create(&dir) {
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
    Ok(dir)
}

#[cfg(all(test, unix))]
mod staging_tests {
    use super::{
        InPlaceClaim, LoaderError, StagedDylib, open_private_dir, stage_private_copy,
        staging_dir_in,
    };
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn copies_stage_in_a_per_user_private_dir_with_0600_mode() {
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
        assert_eq!(mode(&copy), 0o600, "staged copy created 0o600 via openat");
        std::fs::remove_dir_all(&root).expect("test fs op");
    }

    #[test]
    fn in_place_claim_allows_unchanged_reopens_and_rejects_modified_reloads() {
        let dir = scratch("claim");
        std::fs::create_dir(&dir).expect("test fs op");
        let (file, alias, atomic_tmp, replacement) = (
            dir.join("libapp.so"),
            dir.join("libapp-alias.so"),
            dir.join("libapp.so.tmp"),
            dir.join("libapp-v2.so"),
        );
        std::fs::write(&file, b"bundle-v1").expect("test fs op");
        std::fs::hard_link(&file, &alias).expect("test fs op");

        // (a) Unchanged bundle re-opens succeed both concurrently (multi-surface) and after drop
        //     (Activity recreation / detachApp -> attachApp), including through hard-link aliases.
        let mut first = InPlaceClaim::new(&file).expect("initial in-place claim");
        let mut concurrent = InPlaceClaim::new(&file).expect("concurrent unchanged claim");
        first.seal();
        let mut concurrent_alias = InPlaceClaim::new(&alias).expect("unchanged hard-link alias");
        concurrent.seal();
        concurrent_alias.seal();
        drop(first);
        drop(concurrent);
        drop(concurrent_alias);

        let mut after_drop = InPlaceClaim::new(&file).expect("re-open unchanged file after drop");
        after_drop.seal();
        drop(after_drop);

        // (b) Modifying the in-place file on disk (same inode, changed bytes — even same length)
        //     fails with `AlreadyLoadedInPlace` on both the original path and its hard-link alias.
        std::fs::write(&file, b"bundle-v2").expect("test fs op");
        for again in [&file, &alias] {
            assert!(
                matches!(InPlaceClaim::new(again), Err(LoaderError::AlreadyLoadedInPlace(_))),
                "modified inode must be rejected after sealed load"
            );
        }

        // Atomic rename replacement at the same canonical path (new inode, changed bytes) is also
        // rejected because `dlopen` caches mapped images by path / install name.
        std::fs::write(&atomic_tmp, b"bundle-v3-longer").expect("test fs op");
        std::fs::rename(&atomic_tmp, &file).expect("test fs op");
        assert!(
            matches!(InPlaceClaim::new(&file), Err(LoaderError::AlreadyLoadedInPlace(_))),
            "atomic replacement with changed bytes at same canonical path must be rejected"
        );

        // A freshly created versioned file (distinct inode & canonical path) still loads cleanly.
        std::fs::write(&replacement, b"bundle-v3-longer").expect("test fs op");
        let mut fresh = InPlaceClaim::new(&replacement).expect("distinct versioned file loads");
        fresh.seal();
        drop(fresh);
        std::fs::remove_dir_all(&dir).expect("test fs op");
    }

    #[test]
    fn in_place_claim_rolls_back_on_failed_library_load() {
        let dir = scratch("claim-rollback");
        std::fs::create_dir(&dir).expect("test fs op");
        let file = dir.join("libbroken.so");
        std::fs::write(&file, b"not-a-valid-dylib-v1").expect("test fs op");

        // Unsealed claim rolls back on drop so modified bytes can subsequently be claimed.
        let unsealed = InPlaceClaim::new(&file).expect("unsealed claim");
        drop(unsealed);
        std::fs::write(&file, b"not-a-valid-dylib-v2").expect("test fs op");
        let unsealed2 = InPlaceClaim::new(&file).expect("retry after unsealed rollback");
        drop(unsealed2);

        // Real `StagedDylib::load_with_root(..., None, ...)` failing inside `Library::new` must
        // roll back its `InPlaceClaim` cleanly so a subsequent load of new bytes at the same path
        // is NOT rejected with `AlreadyLoadedInPlace`.
        assert!(matches!(
            StagedDylib::load_with_root(&file, None, "mods", "g1"),
            Err(LoaderError::Load(_))
        ));
        std::fs::write(&file, b"still-not-a-dylib-but-different-bytes").expect("test fs op");
        let mut retry = InPlaceClaim::new(&file).expect("failed Library::new rolled back claim");
        retry.seal();
        drop(retry);
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
        drop(open_private_dir(&dir).expect("create"));
        assert_eq!(mode(&dir), 0o700);
        drop(open_private_dir(&dir).expect("reuse"));
        std::fs::remove_dir_all(&dir).expect("test fs op");
    }

    #[test]
    fn an_own_shared_staging_dir_is_tightened_via_fchmod() {
        let dir = scratch("loose");
        std::fs::create_dir(&dir).expect("test fs op");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).expect("test fs op");
        drop(open_private_dir(&dir).expect("own dir"));
        assert_eq!(mode(&dir), 0o700, "group / other access removed via fchmod");
        std::fs::remove_dir_all(&dir).expect("test fs op");
    }

    #[test]
    fn symlinked_or_file_staging_paths_are_rejected() {
        let target = scratch("target");
        std::fs::create_dir(&target).expect("test fs op");
        let link = scratch("link");
        std::os::unix::fs::symlink(&target, &link).expect("test fs op");
        assert!(open_private_dir(&link).is_err(), "a symlink is never followed");
        std::fs::remove_file(&link).expect("test fs op");
        std::fs::write(&link, b"x").expect("test fs op");
        assert!(open_private_dir(&link).is_err(), "a file is not a directory");
        std::fs::remove_file(&link).expect("test fs op");
        std::fs::remove_dir_all(&target).expect("test fs op");
    }

    #[test]
    fn in_place_symlink_repoint_loads_new_target_via_canonical_path() {
        let dir = scratch("symlink");
        std::fs::create_dir(&dir).expect("test fs op");
        let build = |name: &str, tag: usize| {
            let (src, out) = (dir.join(format!("{name}.rs")), dir.join(format!("lib{name}.{}", std::env::consts::DLL_EXTENSION)));
            let code = format!("#[unsafe(no_mangle)] pub extern \"C\" fn nve_get_interface(_: u32) -> *const core::ffi::c_void {{ {tag} as *const _ }}");
            std::fs::write(&src, code).expect("test fs op");
            let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
            let args = ["--crate-type=cdylib", "--edition=2024", "-C", "opt-level=0", "-o"];
            assert!(std::process::Command::new(rustc).args(args).arg(&out).arg(&src).status().expect("rustc").success());
            out
        };
        let (v1, v2, link) = (build("sym_v1", 1), build("sym_v2", 2), dir.join("current.so"));
        std::os::unix::fs::symlink(&v1, &link).expect("test fs op");
        let d1 = StagedDylib::load_with_root(&link, None, "mods", "g1").expect("load v1 via symlink");
        assert_eq!(d1.loaded_copy_path(), std::fs::canonicalize(&v1).expect("canon v1"));
        // SAFETY: Test fixture returns its integer tag without dereferencing.
        assert_eq!(unsafe { (d1.get_interface())(nve_abi::InterfaceId(0)) as usize }, 1);
        std::fs::remove_file(&link).expect("test fs op");
        std::os::unix::fs::symlink(&v2, &link).expect("test fs op");
        let d2 = StagedDylib::load_with_root(&link, None, "mods", "g2").expect("load v2 via repointed symlink");
        assert_eq!(d2.loaded_copy_path(), std::fs::canonicalize(&v2).expect("canon v2"));
        // SAFETY: Test fixture returns its integer tag without dereferencing.
        assert_eq!(unsafe { (d2.get_interface())(nve_abi::InterfaceId(0)) as usize }, 2);
        drop((d1, d2));
        std::fs::write(&v1, b"overwritten-v1").expect("test fs op");
        assert!(matches!(StagedDylib::load_with_root(&v1, None, "mods", "g3"), Err(LoaderError::AlreadyLoadedInPlace(_))));
        std::fs::remove_dir_all(&dir).expect("test fs op");
    }
}
