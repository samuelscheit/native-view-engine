//! App files through the `NveRuntimeApi` C vtable: `load_app_module` stages a private copy of a
//! real (rustc-built) app `cdylib`, so reloading a file rebuilt in place runs the NEW code with the
//! carried state, failed loads keep the running app, and staged copies are deleted on unload; and
//! `save_state_file` / `restore_state_file` / `clear_state_file` persist app state as the engine's
//! `StateFile` (round trip, checksum quarantine, schema rotation, newer-schema refusal).

#[path = "../../engine/tests/loader_common/mod.rs"]
mod loader_common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use loader_common::{compile_test_cdylib, dylib_ext, SHARED_ABI_DEFS};
use nve_abi::{
    NveBufferSlice, NveRuntimeApi, NveRuntimeConfig, NveRuntimeHandle, NveSceneView, NveRuntimeStatus,
    NveStrSlice, NVE_RUNTIME_ABI_VERSION,
};
use nve_engine::StateFile;
use nve_engine::loader::staging_dir;
use nve_runtime::{MODULE_STAGING_DIR, nve_runtime_get_interface};

const OK: i32 = NveRuntimeStatus::Ok as i32;
const INVALID: i32 = NveRuntimeStatus::InvalidArg as i32;
const UNSUPPORTED: i32 = NveRuntimeStatus::Unsupported as i32;
const IO: i32 = NveRuntimeStatus::Io as i32;
const NOT_FOUND: i32 = NveRuntimeStatus::NotFound as i32;
const BUSY: i32 = NveRuntimeStatus::Busy as i32;

/// A stateful app whose snapshot is `<VERSION>:<state>` (empty without state); `restore_state`
/// keeps what follows the first `:`, so a snapshot of any version restores into any other.
const APP_BODY: &str = r#"
struct App { state: Vec<u8>, snap: Vec<u8> }
unsafe extern "C" fn create(_: u32) -> *mut c_void {
    Box::into_raw(Box::new(App { state: Vec::new(), snap: Vec::new() })).cast()
}
unsafe extern "C" fn destroy(h: *mut c_void) { drop(unsafe { Box::from_raw(h.cast::<App>()) }); }
unsafe extern "C" fn render(_: *mut c_void, _: u64) -> NveBufferSlice {
    NveBufferSlice { ptr: std::ptr::null(), len: 0 }
}
unsafe extern "C" fn dispatch_events(_: *mut c_void, _: NveEventBatch) -> u32 { 0 }
unsafe extern "C" fn apply_hot_patch(_: *mut c_void, _: NveBufferSlice) -> u32 { 0 }
unsafe extern "C" fn snapshot_state(h: *mut c_void) -> NveBufferSlice {
    let app = unsafe { &mut *h.cast::<App>() };
    if app.state.is_empty() { return NveBufferSlice { ptr: std::ptr::null(), len: 0 }; }
    app.snap = [VERSION, b":", &app.state].concat();
    NveBufferSlice { ptr: app.snap.as_ptr(), len: app.snap.len() }
}
unsafe extern "C" fn restore_state(h: *mut c_void, s: NveBufferSlice) {
    let app = unsafe { &mut *h.cast::<App>() };
    let bytes = if s.len == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(s.ptr, s.len) } };
    let at = bytes.iter().position(|&b| b == b':').map_or(0, |i| i + 1);
    app.state = bytes[at..].to_vec();
}
static SURFACE: NveSurfaceApi = NveSurfaceApi { create, destroy, render, dispatch_events };
static DEV: NveDevApi = NveDevApi { apply_hot_patch, snapshot_state, restore_state };
#[repr(C)]
struct RawSurface([*const c_void; 4]);
unsafe impl Sync for RawSurface {}
static PARTIAL: RawSurface = RawSurface([
    create as *const c_void, destroy as *const c_void, render as *const c_void, std::ptr::null(),
]);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
    match (id, VARIANT) {
        (1, "partial") => (&raw const PARTIAL).cast(),
        (1, "full") => (&raw const SURFACE).cast(),
        (4, "full") => (&raw const DEV).cast(),
        _ => std::ptr::null(),
    }
}
"#;

struct Fixtures {
    v1: PathBuf,
    v2: PathBuf,
    no_surface: PathBuf,
    partial_surface: PathBuf,
    no_entry: PathBuf,
}

/// Builds every fixture `cdylib` once per test binary.
fn fixtures() -> &'static Fixtures {
    static FIXTURES: OnceLock<Fixtures> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nve-runtime-app-files");
        std::fs::create_dir_all(&dir).expect("fixture dir");
        let app = |name: &str, version: &str, variant: &str| {
            let src = format!(
                "{SHARED_ABI_DEFS}\nconst VERSION: &[u8] = b\"{version}\";\nconst VARIANT: &str = \"{variant}\";\n{APP_BODY}"
            );
            compile_test_cdylib(&dir, name, &src)
        };
        Fixtures {
            v1: app("nve_app_v1", "v1", "full"),
            v2: app("nve_app_v2", "v2", "full"),
            no_surface: app("nve_app_no_surface", "v1", "none"),
            partial_surface: app("nve_app_partial_surface", "v1", "partial"),
            no_entry: compile_test_cdylib(
                &dir,
                "nve_app_no_entry",
                "#[unsafe(no_mangle)] pub extern \"C\" fn not_nve_get_interface() -> u32 { 1 }",
            ),
        }
    })
}

/// A unique scratch directory removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("nve-runtime-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One runtime surface driven through the C vtable on the test thread.
struct Rt {
    api: NveRuntimeApi,
    h: NveRuntimeHandle,
}

impl Rt {
    fn new() -> Self {
        let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
        assert!(!ptr.is_null());
        // SAFETY: Static vtable returned for the matching ABI version.
        let api = unsafe { *ptr };
        // SAFETY: NULL config selects defaults; the handle is destroyed in `Drop` on this thread.
        let h = unsafe { (api.create)(std::ptr::null::<NveRuntimeConfig>()) };
        assert!(!h.is_null());
        Self { api, h }
    }

    fn load(&self, path: &Path, carry_state: bool) -> i32 {
        let path = path.to_str().expect("UTF-8 test path");
        // SAFETY: Live handle on its owner thread; `path` outlives the call.
        unsafe { (self.api.load_app_module)(self.h, NveStrSlice::from_str(path), u32::from(carry_state)) }
    }

    fn snapshot(&self) -> Result<Vec<u8>, i32> {
        let mut out = NveBufferSlice::EMPTY;
        // SAFETY: Live handle; the borrowed slice is copied before the next call.
        let rc = unsafe { (self.api.snapshot_state)(self.h, &mut out) };
        if rc != OK {
            return Err(rc);
        }
        // SAFETY: An `Ok` snapshot is readable for `len` bytes until the next call.
        Ok(unsafe { out.try_as_slice() }.unwrap_or(&[]).to_vec())
    }

    fn restore(&self, state: &[u8]) -> i32 {
        // SAFETY: Live handle; `state` outlives the call.
        unsafe { (self.api.restore_state)(self.h, NveBufferSlice::from_slice(state)) }
    }

    fn save_file(&self, path: &Path, schema: u32) -> i32 {
        // SAFETY: Live handle; the path outlives the call.
        unsafe { (self.api.save_state_file)(self.h, str_slice(path), schema) }
    }

    fn restore_file(&self, path: &Path, schema: u32) -> i32 {
        // SAFETY: Live handle; the path outlives the call.
        unsafe { (self.api.restore_state_file)(self.h, str_slice(path), schema) }
    }

    fn clear_file(&self, path: &Path) -> i32 {
        // SAFETY: Live handle; the path outlives the call.
        unsafe { (self.api.clear_state_file)(self.h, str_slice(path)) }
    }

    fn detach(&self) -> i32 {
        // SAFETY: Live handle on its owner thread.
        unsafe { (self.api.attach_app_interface)(self.h, None) }
    }

    fn last_error(&self) -> String {
        let mut buf = vec![0u8; 512];
        // SAFETY: `buf` is writable for its length.
        let len = unsafe { (self.api.last_error)(buf.as_mut_ptr(), buf.len()) };
        buf.truncate(len.min(buf.len() - 1));
        String::from_utf8(buf).expect("UTF-8 last_error")
    }
}

impl Drop for Rt {
    fn drop(&mut self) {
        // SAFETY: Live handle destroyed once on its owner thread.
        unsafe { (self.api.destroy)(self.h) };
    }
}

fn str_slice(path: &Path) -> NveStrSlice {
    NveStrSlice::from_str(path.to_str().expect("UTF-8 test path"))
}

/// This process's staged copies (`<staging dir>/<stem>_<label>_<pid>_<seq>.<ext>`) of modules
/// whose file stem is `stem`.
fn staged_copies(stem: &str) -> usize {
    let prefix = format!("{stem}_");
    let pid = format!("_{}_", std::process::id());
    let dir = staging_dir(MODULE_STAGING_DIR).expect("desktop tests stage copies");
    std::fs::read_dir(dir)
        .map(|dir| {
            dir.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(&prefix) && name.contains(&pid))
                .count()
        })
        .unwrap_or(0)
}

fn stem(path: &Path) -> String {
    path.file_stem().expect("module path has a file stem").to_string_lossy().into_owned()
}

#[test]
fn reloading_a_module_rebuilt_at_the_same_path_runs_the_new_code_with_carried_state() {
    let fx = fixtures();
    let dir = Scratch::new("rebuilt-module");
    let app = dir.0.join(format!("librebuilt_app_{}.{}", std::process::id(), dylib_ext()));
    let app_stem = stem(&app);
    let build = |fixture: &Path| {
        std::fs::copy(fixture, &app).expect("copy fixture over the app path");
    };
    let rt = Rt::new();

    build(&fx.v1);
    assert_eq!(rt.load(&app, false), OK, "{}", rt.last_error());
    assert_eq!(staged_copies(&app_stem), 1);
    assert_eq!(rt.snapshot(), Ok(Vec::new()), "fresh app has no state");
    assert_eq!(rt.restore(b"seed:abc"), OK);
    assert_eq!(rt.snapshot().unwrap(), b"v1:abc");

    // Rebuild in place: same path, new code. The reload must not reuse the cached v1 image.
    build(&fx.v2);
    assert_eq!(rt.load(&app, true), OK, "{}", rt.last_error());
    assert_eq!(rt.snapshot().unwrap(), b"v2:abc", "new code, carried state");
    assert_eq!(staged_copies(&app_stem), 1, "the v1 copy is unloaded and deleted");

    // Back to v1 without carrying state: fresh v1 app.
    build(&fx.v1);
    assert_eq!(rt.load(&app, false), OK);
    assert_eq!(rt.snapshot(), Ok(Vec::new()));
    assert_eq!(rt.restore(b"x:q"), OK);
    assert_eq!(rt.snapshot().unwrap(), b"v1:q");

    // Carrying from an app without state starts the new one fresh.
    assert_eq!(rt.restore(b""), OK);
    build(&fx.v2);
    assert_eq!(rt.load(&app, true), OK);
    assert_eq!(rt.snapshot(), Ok(Vec::new()));

    // Detaching unloads the module and deletes its staged copy; so does destroying the runtime.
    assert_eq!(rt.detach(), OK);
    assert_eq!(staged_copies(&app_stem), 0);
    assert_eq!(rt.load(&app, false), OK);
    assert_eq!(staged_copies(&app_stem), 1);
    drop(rt);
    assert_eq!(staged_copies(&app_stem), 0);
}

#[test]
fn failed_module_loads_keep_the_running_app_and_leave_no_staged_copy() {
    let fx = fixtures();
    let dir = Scratch::new("failed-loads");
    let rt = Rt::new();
    assert_eq!(rt.load(&fx.v1, false), OK);
    assert_eq!(rt.restore(b"s:live"), OK);

    let garbage = dir.0.join(format!("libgarbage.{}", dylib_ext()));
    std::fs::write(&garbage, b"not a shared library").unwrap();
    let cases: [(&Path, i32, &str); 6] = [
        (&dir.0.join("missing.so"), IO, "cannot stage"),
        (Path::new(""), IO, "cannot stage"),
        (&garbage, IO, "cannot load"),
        (&fx.no_entry, NOT_FOUND, "nve_get_interface"),
        (&fx.no_surface, UNSUPPORTED, "NVE_IFACE_SURFACE"),
        (&fx.partial_surface, UNSUPPORTED, "NVE_IFACE_SURFACE"),
    ];
    for (path, status, message) in cases {
        for carry in [false, true] {
            assert_eq!(rt.load(path, carry), status, "{path:?}");
            let err = rt.last_error();
            assert!(err.contains(message), "{path:?}: `{err}`");
            assert_eq!(rt.snapshot().unwrap(), b"v1:live", "{path:?} must not replace the app");
            if !path.as_os_str().is_empty() {
                assert_eq!(staged_copies(&stem(path)), 0, "{path:?}");
            }
        }
    }

    // Invalid arguments are refused before anything is staged.
    let bad_utf8 = [0xFFu8, 0xFE];
    let bad = NveStrSlice { ptr: bad_utf8.as_ptr(), len: bad_utf8.len() };
    let null_with_len = NveStrSlice { ptr: std::ptr::null(), len: 4 };
    // SAFETY: Both slices are rejected by validation; null handles are guarded.
    unsafe {
        assert_eq!((rt.api.load_app_module)(rt.h, bad, 0), INVALID);
        assert_eq!((rt.api.load_app_module)(rt.h, null_with_len, 0), INVALID);
        let v1 = str_slice(&fx.v1);
        assert_eq!((rt.api.load_app_module)(std::ptr::null_mut(), v1, 0), INVALID);
    }
    let handle = rt.h as usize;
    let v1 = fx.v1.clone();
    let off_thread = std::thread::spawn(move || {
        // SAFETY: The runtime rejects non-owner threads before touching any state.
        unsafe { (rt_api().load_app_module)(handle as NveRuntimeHandle, str_slice(&v1), 0) }
    })
    .join()
    .unwrap();
    assert_eq!(off_thread, INVALID);
    assert_eq!(staged_copies(&stem(&fx.v1)), 1, "only the running module is staged");

    // A held scene refuses the load before anything is staged, loaded or created.
    let mut view = NveSceneView::default();
    // SAFETY: `rt.h` is live; the scene is released below on this thread.
    assert_eq!(unsafe { (rt.api.scene_acquire)(rt.h, &mut view) }, OK);
    assert_eq!(rt.load(&fx.v2, true), BUSY);
    assert_eq!(staged_copies(&stem(&fx.v2)), 0, "nothing staged while busy");
    // SAFETY: Releases the scene acquired above.
    assert_eq!(unsafe { (rt.api.scene_release)(rt.h) }, OK);
    assert_eq!(rt.snapshot().unwrap(), b"v1:live");
}

fn rt_api() -> NveRuntimeApi {
    // SAFETY: Static vtable returned for the matching ABI version.
    unsafe { *nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION) }
}

#[test]
fn state_files_round_trip_quarantine_corruption_and_rotate_older_schemas() {
    let fx = fixtures();
    let dir = Scratch::new("state-files");
    let path = dir.0.join("nested").join("app.nvestate");
    let sibling = |suffix: &str| dir.0.join("nested").join(format!("app.nvestate.{suffix}"));
    let rt = Rt::new();

    assert_eq!(rt.save_file(&path, 7), UNSUPPORTED, "no app");
    assert_eq!(rt.load(&fx.v1, false), OK);
    assert_eq!(rt.restore_file(&path, 7), NOT_FOUND, "no file yet");
    assert_eq!(rt.save_file(&path, 7), NOT_FOUND, "app without state");
    assert!(!path.exists(), "nothing written for an empty state");

    assert_eq!(rt.restore(b"s:abc"), OK);
    assert_eq!(rt.save_file(&path, 7), OK, "{}", rt.last_error());
    let saved = std::fs::read(&path).unwrap();
    assert_eq!(StateFile::decode_envelope(&saved, 7).unwrap(), b"v1:abc");
    let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
    assert_eq!(entries.len(), 1, "atomic write leaves no temp file");

    assert_eq!(rt.restore(b"s:zzz"), OK);
    assert_eq!(rt.restore_file(&path, 7), OK);
    assert_eq!(rt.snapshot().unwrap(), b"v1:abc");

    // Written by a newer host: refused and kept in place.
    assert_eq!(rt.restore_file(&path, 6), IO);
    assert!(rt.last_error().contains("schema"));
    assert_eq!(std::fs::read(&path).unwrap(), saved);

    // A flipped payload bit fails the checksum: quarantined, the app keeps its state.
    let mut flipped = saved.clone();
    *flipped.last_mut().unwrap() ^= 1;
    std::fs::write(&path, &flipped).unwrap();
    assert_eq!(rt.restore(b"s:keep"), OK);
    assert_eq!(rt.restore_file(&path, 7), NOT_FOUND);
    assert!(rt.last_error().contains("corrupt"), "{}", rt.last_error());
    assert!(!path.exists());
    assert_eq!(std::fs::read(sibling("corrupt")).unwrap(), flipped);
    assert_eq!(rt.snapshot().unwrap(), b"v1:keep");

    // An older schema is rotated aside, never misread: on restore and when saving over it.
    std::fs::write(&path, &saved).unwrap();
    assert_eq!(rt.restore_file(&path, 8), NOT_FOUND);
    assert!(rt.last_error().contains("older schema"), "{}", rt.last_error());
    assert!(!path.exists());
    assert_eq!(std::fs::read(sibling("v7")).unwrap(), saved);
    std::fs::remove_file(sibling("v7")).unwrap();
    std::fs::write(&path, &saved).unwrap();
    assert_eq!(rt.save_file(&path, 9), OK);
    assert_eq!(std::fs::read(sibling("v7")).unwrap(), saved);
    assert_eq!(StateFile::decode_envelope(&std::fs::read(&path).unwrap(), 9).unwrap(), b"v1:keep");

    // Restoring into a reloaded module of another version: the file carries app state across.
    assert_eq!(rt.load(&fx.v2, false), OK);
    assert_eq!(rt.restore_file(&path, 9), OK);
    assert_eq!(rt.snapshot().unwrap(), b"v2:keep");

    // Cold start: delete, idempotently.
    assert_eq!(rt.clear_file(&path), OK);
    assert!(!path.exists());
    assert_eq!(rt.clear_file(&path), OK);
    assert_eq!(rt.restore_file(&path, 9), NOT_FOUND);

    // Path errors are I/O errors; a detached runtime has no state to save or restore into.
    assert_eq!(rt.save_file(Path::new(""), 1), IO);
    assert_eq!(rt.save_file(&dir.0, 1), IO, "a directory is not a state file");
    assert_eq!(rt.detach(), OK);
    std::fs::write(&path, &saved).unwrap();
    assert_eq!(rt.save_file(&path, 7), UNSUPPORTED);
    assert_eq!(rt.restore_file(&path, 7), UNSUPPORTED);
    // SAFETY: Null handles are guarded by every entry point.
    unsafe {
        let p = str_slice(&path);
        assert_eq!((rt.api.save_state_file)(std::ptr::null_mut(), p, 1), INVALID);
        assert_eq!((rt.api.restore_state_file)(std::ptr::null_mut(), p, 1), INVALID);
        assert_eq!((rt.api.clear_state_file)(std::ptr::null_mut(), p), INVALID);
    }
}

/// Every file in `dir` with its bytes (sorted), to prove a call left the disk untouched.
fn disk(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn state_calls_that_cannot_complete_never_touch_the_disk() {
    let fx = fixtures();
    let dir = Scratch::new("state-preconditions");
    let path = dir.0.join("app.nvestate");
    let missing_dir = dir.0.join("never-created").join("app.nvestate");
    let rt = Rt::new();
    assert_eq!(rt.load(&fx.v1, false), OK);
    assert_eq!(rt.restore(b"s:abc"), OK);
    assert_eq!(rt.save_file(&path, 7), OK);
    let valid = std::fs::read(&path).unwrap();
    let mut corrupt = valid.clone();
    *corrupt.last_mut().unwrap() ^= 1;

    // `(file contents, schema the caller asks for)`: a valid file, a corrupt one (would be
    // quarantined), and an older-schema one (would be rotated).
    let cases: [(&[u8], u32); 3] = [(&valid, 7), (&corrupt, 7), (&valid, 8)];

    // Busy: a held scene blocks every state call BEFORE the file is opened.
    for (bytes, schema) in cases {
        std::fs::write(&path, bytes).unwrap();
        let before = disk(&dir.0);
        let mut view = NveSceneView::default();
        // SAFETY: `rt.h` is live; the scene is released below on this thread.
        assert_eq!(unsafe { (rt.api.scene_acquire)(rt.h, &mut view) }, OK);
        assert_eq!(rt.restore_file(&path, schema), BUSY, "restore while a scene is held");
        assert_eq!(rt.save_file(&path, schema), BUSY, "save while a scene is held");
        assert_eq!(rt.save_file(&missing_dir, schema), BUSY);
        // SAFETY: Releases the scene acquired above.
        assert_eq!(unsafe { (rt.api.scene_release)(rt.h) }, OK);
        assert_eq!(disk(&dir.0), before, "busy calls left the files in place (schema {schema})");
    }

    // No app: restore is unsupported for missing, corrupt, and older-schema files alike.
    assert_eq!(rt.detach(), OK);
    for (bytes, schema) in cases {
        std::fs::write(&path, bytes).unwrap();
        let before = disk(&dir.0);
        assert_eq!(rt.restore_file(&path, schema), UNSUPPORTED);
        assert_eq!(rt.save_file(&path, schema), UNSUPPORTED);
        assert_eq!(rt.restore_file(&missing_dir, schema), UNSUPPORTED);
        assert_eq!(disk(&dir.0), before, "no-app calls left the files in place (schema {schema})");
    }
    assert!(!missing_dir.parent().unwrap().exists(), "no directory is created for a call that fails");

    // A path without a file name is an I/O error for every file operation, never "already absent".
    assert_eq!(rt.clear_file(Path::new("")), IO);
    assert_eq!(rt.clear_file(Path::new("/")), IO);
}
