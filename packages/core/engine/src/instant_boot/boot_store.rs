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

#[cfg(not(target_arch = "wasm32"))]
fn is_truthy_env(name: &str) -> bool {
    let Ok(val) = std::env::var(name) else {
        return false;
    };
    let trimmed = val.trim();
    trimmed == "1"
        || trimmed.eq_ignore_ascii_case("true")
        || trimmed.eq_ignore_ascii_case("yes")
        || trimmed.eq_ignore_ascii_case("on")
}

/// Returns `true` when `NVE_SNAPSHOT_DISABLE` is set to a truthy value (`1`, `true`, `yes`, `on`).
pub fn is_snapshot_disabled() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        true
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        is_truthy_env("NVE_SNAPSHOT_DISABLE")
    }
}

/// Returns `true` when `NVE_RESET_STATE` is set to a truthy value (`1`, `true`, `yes`, `on`).
pub fn is_snapshot_reset_requested() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        is_truthy_env("NVE_RESET_STATE")
    }
}

/// Removes cached `user_frame0.nvereloc` and `state.nvemmap` inside `dir`, if present.
pub fn reset_instant_boot_dir(dir: &Path) {
    let _ = fs::remove_file(dir.join(DEFAULT_USER_FRAME0_FILE));
    let _ = fs::remove_file(dir.join(DEFAULT_STATE_ARENA_FILE));
}

#[cfg(not(target_arch = "wasm32"))]
fn canonicalize_app_identity(app_identity: &str) -> String {
    let trimmed = app_identity.trim();
    if trimmed.is_empty() {
        return "nve-app".to_string();
    }
    let candidate = Path::new(trimmed);
    if candidate.exists()
        && let Ok(canon) = fs::canonicalize(candidate)
    {
        return canon.to_string_lossy().into_owned();
    }
    trimmed.to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_app_slug(app_identity: &str) -> String {
    let trimmed = app_identity.trim();
    if trimmed.is_empty() {
        return "nve-app".to_string();
    }
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let stem = Path::new(base)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(base);
    let mut slug = String::with_capacity(stem.len().min(48));
    let mut prev_dash = false;
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "nve-app".to_string()
    } else {
        slug
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn default_instant_boot_cache_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME")
            && !home.trim().is_empty()
        {
            return PathBuf::from(home)
                .join("Library")
                .join("Caches")
                .join("dev.nve")
                .join("instant-boot");
        }
        std::env::temp_dir().join("nve").join("instant-boot")
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA")
            && !local_app_data.trim().is_empty()
        {
            return PathBuf::from(local_app_data)
                .join("nve")
                .join("instant-boot");
        }
        if let Ok(temp) = std::env::var("TEMP")
            && !temp.trim().is_empty()
        {
            return PathBuf::from(temp).join("nve").join("instant-boot");
        }
        std::env::temp_dir().join("nve").join("instant-boot")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME")
            && !xdg.trim().is_empty()
        {
            return PathBuf::from(xdg).join("nve").join("instant-boot");
        }
        if let Ok(home) = std::env::var("HOME")
            && !home.trim().is_empty()
        {
            return PathBuf::from(home)
                .join(".cache")
                .join("nve")
                .join("instant-boot");
        }
        std::env::temp_dir().join("nve").join("instant-boot")
    }
}

/// Resolves the deterministic per-app default [`NativeInstantBootStore`] cache directory for
/// `app_identity` (`<cache-root>/<slug>-<hash8>`), matching C++ `resolve_default_snapshot_path`.
///
/// Honors environment overrides:
/// - `NVE_SNAPSHOT_DISABLE=1` -> returns `None` (disabled).
/// - `NVE_RESET_STATE=1` -> clears existing `user_frame0.nvereloc` and `state.nvemmap` in the
///   resolved directory before returning `Some(dir)`.
pub fn default_instant_boot_dir(app_identity: &str) -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app_identity;
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if is_snapshot_disabled() {
            return None;
        }
        let canonical = canonicalize_app_identity(app_identity);
        let slug = sanitize_app_slug(app_identity);
        let digest = nve_abi::xxh64(canonical.as_bytes(), 0);
        let hash8 = format!("{:08x}", (digest & 0xFFFF_FFFF) as u32);
        let dir = default_instant_boot_cache_root().join(format!("{slug}-{hash8}"));
        if is_snapshot_reset_requested() {
            reset_instant_boot_dir(&dir);
        }
        Some(dir)
    }
}

/// Checks for a pre-baked cold-start sidecar Frame-0 `NVERELOC` file alongside `dylib_path`:
/// 1. `<dylib-dir>/<stem>.frame0.nvereloc`
/// 2. `<dylib-dir>/<stem-without-lib-prefix>.frame0.nvereloc` (when `<stem>` starts with `lib`)
/// 3. `<dylib-dir>/frame0.nvereloc`
pub fn resolve_sidecar_frame0_path(dylib_path: &Path) -> Option<PathBuf> {
    let dir = dylib_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if let Some(stem) = dylib_path.file_stem().and_then(|s| s.to_str())
        && !stem.is_empty()
    {
        let by_stem = dir.join(format!("{stem}.frame0.nvereloc"));
        if by_stem.is_file() {
            return Some(by_stem);
        }
        if let Some(stripped) = stem.strip_prefix("lib")
            && !stripped.is_empty()
        {
            let without_lib = dir.join(format!("{stripped}.frame0.nvereloc"));
            if without_lib.is_file() {
                return Some(without_lib);
            }
        }
    }
    let generic = dir.join("frame0.nvereloc");
    if generic.is_file() {
        return Some(generic);
    }
    None
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod resolver_tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            let saved = keys
                .iter()
                .map(|&k| (k, std::env::var(k).ok()))
                .collect();
            Self { saved }
        }

        fn set(&self, key: &str, val: Option<&str>) {
            // SAFETY: Guarded by `ENV_LOCK` so no concurrent test mutates env vars.
            unsafe {
                match val {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, orig) in &self.saved {
                // SAFETY: Guarded by `ENV_LOCK`.
                unsafe {
                    match orig {
                        Some(v) => std::env::set_var(k, v),
                        None => std::env::remove_var(k),
                    }
                }
            }
        }
    }

    #[test]
    fn default_instant_boot_dir_is_deterministic_unique_and_honors_env_overrides() {
        let _lock = ENV_LOCK.lock().unwrap();
        let guard = EnvGuard::new(&["NVE_SNAPSHOT_DISABLE", "NVE_RESET_STATE"]);
        guard.set("NVE_SNAPSHOT_DISABLE", None);
        guard.set("NVE_RESET_STATE", None);

        let a1 = default_instant_boot_dir("/apps/one/libdemo.dylib").expect("enabled");
        let a2 = default_instant_boot_dir("/apps/one/libdemo.dylib").expect("enabled");
        let b = default_instant_boot_dir("/apps/two/libdemo.dylib").expect("enabled");
        assert_eq!(a1, a2);
        assert_ne!(a1, b, "same basename in different dirs must not collide");
        assert!(
            a1.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("libdemo-")
        );

        // NVE_SNAPSHOT_DISABLE=1 disables default instant-boot cleanly.
        guard.set("NVE_SNAPSHOT_DISABLE", Some("1"));
        assert!(default_instant_boot_dir("/apps/one/libdemo.dylib").is_none());
        guard.set("NVE_SNAPSHOT_DISABLE", None);

        // NVE_RESET_STATE=1 deletes cached user_frame0.nvereloc and state.nvemmap.
        let unique_id = format!("/tmp/nve-test-reset-{}/libdemo.dylib", std::process::id());
        let dir = default_instant_boot_dir(&unique_id).expect("enabled");
        fs::create_dir_all(&dir).unwrap();
        let f0 = dir.join(DEFAULT_USER_FRAME0_FILE);
        let st = dir.join(DEFAULT_STATE_ARENA_FILE);
        fs::write(&f0, b"cached-frame0").unwrap();
        fs::write(&st, b"cached-state").unwrap();
        assert!(f0.exists() && st.exists());

        guard.set("NVE_RESET_STATE", Some("1"));
        let reset_dir = default_instant_boot_dir(&unique_id).expect("still returns dir");
        assert_eq!(reset_dir, dir);
        assert!(!f0.exists(), "user_frame0.nvereloc should be deleted on reset");
        assert!(!st.exists(), "state.nvemmap should be deleted on reset");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_sidecar_frame0_path_checks_stem_and_generic() {
        let dir = std::env::temp_dir().join(format!(
            "nve_sidecar_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dylib = dir.join("libmy_app.dylib");
        fs::write(&dylib, b"dummy").unwrap();
        assert!(resolve_sidecar_frame0_path(&dylib).is_none());

        let generic = dir.join("frame0.nvereloc");
        fs::write(&generic, b"generic").unwrap();
        assert_eq!(resolve_sidecar_frame0_path(&dylib), Some(generic.clone()));

        let by_stem = dir.join("libmy_app.frame0.nvereloc");
        fs::write(&by_stem, b"stem").unwrap();
        assert_eq!(resolve_sidecar_frame0_path(&dylib), Some(by_stem));

        let _ = fs::remove_dir_all(&dir);
    }
}
