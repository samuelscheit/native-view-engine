//! Deep unit & integration tests for `nve-engine`:
//! - `HotPatchMsg` protocol serialization, deserialization, error paths, and C ABI routing
//! - `HotAppInstance` (`AppLoader`) error paths (missing file, invalid dylib, missing symbol,
//!   null surface interface), optional interface fallback, full dylib load, and state-preserving hot-swap
//! - `FrameProfiler` span recording, ring eviction, percentiles, and Chrome Trace JSON output
//! - `SurfaceRunner` frame sequencing and error propagation
//! - `NveApp` default implementations and `export_nve_app!` C ABI null-safety

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use nve_abi::{
    CommandBufferBuilder, DecodeError, FrameId, NativeEvent, NodeId, NveBufferSlice,
    NveComponentApi, NveDevApi, NveDirectEvent, NveEventBatch, NveEventBatchRef,
    NveGestureDisposition, NveIntrinsicSize, NveLayoutApi, NveMapEntry, NvePropsRef,
    NveSizeConstraints, NveSurfaceApi, NveValue, SurfaceId, NVE_IFACE_COMPONENT, NVE_IFACE_DEV,
    NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
use nve_engine::loader::{HotAppInstance, LoaderError};
use nve_engine::{
    BackendApplyStats, BackendError, FrameProfiler, FrameTiming, HotPatchMsg, LayoutRect, NveApp,
    RenderBackend, SurfaceRunner, Viewport,
};

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "nve-engine-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dylib_ext() -> &'static str {
    if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    }
}

fn compile_test_cdylib(dir: &Path, name: &str, rust_source: &str) -> PathBuf {
    let src_path = dir.join(format!("{name}.rs"));
    let out_path = dir.join(format!("lib{name}.{}", dylib_ext()));
    std::fs::write(&src_path, rust_source).expect("write test cdylib source");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let output = ProcessCommand::new(rustc)
        .args([
            "--crate-type=cdylib",
            "--edition=2024",
            "-C",
            "opt-level=0",
            "-o",
        ])
        .arg(&out_path)
        .arg(&src_path)
        .output()
        .expect("invoke rustc for test cdylib");
    assert!(
        output.status.success(),
        "rustc failed for {name}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    out_path
}

// ============================================================================
// 1. Hot-reload protocol serialization / deserialization & patch routing
// ============================================================================

#[test]
fn hot_patch_msg_roundtrip_all_variants_and_wire_tags() {
    let cases = vec![
        (
            HotPatchMsg::TemplatePatch {
                generation: 42,
                template_id: "card::header".into(),
                attr_updates: vec![
                    ("opacity".into(), "0.85".into()),
                    ("background-color".into(), "#0f172a".into()),
                ],
                text_updates: vec![(0, "Hello 🚀".into()), (3, "".into())],
                target_test_id: None,
                node_path: vec![],
                text_replacements: vec![],
            },
            "template_patch",
        ),
        (
            HotPatchMsg::TemplatePatch {
                generation: 0,
                template_id: "".into(),
                attr_updates: vec![],
                text_updates: vec![],
                target_test_id: None,
                node_path: vec![],
                text_replacements: vec![],
            },
            "template_patch",
        ),
        (
            HotPatchMsg::DylibReload {
                generation: u64::MAX,
                dylib_path: "/tmp/libnve_app.dylib".into(),
                build_ms: 215,
            },
            "dylib_reload",
        ),
        (
            HotPatchMsg::DevStatus {
                message: "Hot swap ready (gen 7)".into(),
                latency_ms: 12,
            },
            "dev_status",
        ),
    ];

    for (msg, expected_tag) in cases {
        let json = msg.to_json();
        let raw: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(raw["type"].as_str(), Some(expected_tag));

        let decoded = HotPatchMsg::from_json(&json).expect("roundtrip decode");
        assert_eq!(decoded, msg);
    }
}

#[test]
fn hot_patch_msg_rejects_invalid_and_malformed_json() {
    let invalid_payloads = [
        "",
        "{",
        "null",
        r#"{"type":"unknown_variant","generation":1}"#,
        r#"{"type":"template_patch","generation":1}"#, // missing fields
        r#"{"type":"dylib_reload","generation":"not_a_number","dylib_path":"x","build_ms":1}"#,
        r#"{"type":"dev_status","message":123,"latency_ms":5}"#,
    ];

    for raw in invalid_payloads {
        assert!(
            HotPatchMsg::from_json(raw).is_err(),
            "expected error for malformed HotPatchMsg: {raw}"
        );
    }
}

// Local NveApp exported via `export_nve_app!` for testing patch routing and C ABI null safety.
struct RoutingTestApp {
    surface_id: SurfaceId,
    patches_received: Vec<HotPatchMsg>,
    state: Vec<u8>,
    frame_buf: Vec<u8>,
    emitted: Vec<u8>,
}

impl NveApp for RoutingTestApp {
    fn create(surface_id: SurfaceId) -> Self {
        let mut b = CommandBufferBuilder::new(surface_id, 1);
        b.commit(true);
        Self {
            surface_id,
            patches_received: Vec::new(),
            state: b"initial".to_vec(),
            frame_buf: b.into_bytes(),
            emitted: vec![1, 2, 3],
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let mut b = CommandBufferBuilder::new(self.surface_id, frame_id);
        b.commit(true);
        self.frame_buf = b.into_bytes();
        &self.frame_buf
    }

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        !encoded_events.is_empty()
    }

    fn set_props(&mut self, props_json: &[u8]) -> bool {
        !props_json.is_empty()
    }

    fn set_props_direct(&mut self, props: NvePropsRef<'_>) -> bool {
        props.get("count").and_then(|v| v.as_i64()).is_some()
    }

    fn dispatch_events_direct(&mut self, events: NveEventBatchRef<'_>) -> bool {
        events
            .iter()
            .any(|ev| ev.name() == "onPing" && ev.value().get("ok").and_then(|v| v.as_bool()) == Some(true))
    }

    fn drain_emitted_events(&mut self) -> &[u8] {
        &self.emitted
    }

    fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
        match HotPatchMsg::from_json(patch_json) {
            Ok(msg @ HotPatchMsg::TemplatePatch { .. }) => {
                self.patches_received.push(msg);
                true
            }
            _ => false,
        }
    }

    fn snapshot_state(&mut self) -> &[u8] {
        &self.state
    }

    fn restore_state(&mut self, state: &[u8]) {
        self.state = state.to_vec();
    }
}

nve_engine::export_nve_app!(RoutingTestApp, symbol = nve_engine_test_get_interface);

unsafe extern "C" {
    fn nve_engine_test_get_interface(interface_id: u32) -> *const c_void;
}

#[test]
fn export_nve_app_routes_hot_patches_handles_nulls_and_defaults() {
    // Unknown interface IDs return NULL.
    unsafe {
        assert!(nve_engine_test_get_interface(0).is_null());
        assert!(nve_engine_test_get_interface(999).is_null());
    }

    let surface = unsafe { *(nve_engine_test_get_interface(NVE_IFACE_SURFACE) as *const NveSurfaceApi) };
    let layout = unsafe { *(nve_engine_test_get_interface(NVE_IFACE_LAYOUT) as *const NveLayoutApi) };
    let component = unsafe { *(nve_engine_test_get_interface(NVE_IFACE_COMPONENT) as *const NveComponentApi) };
    let dev = unsafe { *(nve_engine_test_get_interface(NVE_IFACE_DEV) as *const NveDevApi) };

    // Null handle guards across every exported C ABI function must never dereference or panic.
    let null = std::ptr::null_mut();
    unsafe {
        (surface.destroy)(null);
        assert_eq!((surface.render)(null, 1).len, 0);
        assert_eq!((surface.dispatch_events)(null, NveBufferSlice::EMPTY), 0);
        assert_eq!(
            (surface.dispatch_events_direct.unwrap())(null, NveEventBatch::EMPTY),
            0
        );
        assert_eq!(
            (layout.measure)(null, NveSizeConstraints::loose(100.0, 200.0)),
            NveIntrinsicSize::default()
        );
        assert_eq!((layout.hit_test)(null, 10.0, 20.0, 1), 2);
        assert_eq!((component.set_props)(null, NveBufferSlice::EMPTY), 0);
        assert_eq!(
            (component.set_props_direct.unwrap())(null, std::ptr::null(), 0),
            0
        );
        assert_eq!((component.drain_emitted_events)(null).len, 0);
        assert_eq!((dev.apply_hot_patch)(null, NveBufferSlice::EMPTY), 0);
        assert_eq!((dev.snapshot_state)(null).len, 0);
        (dev.restore_state)(null, NveBufferSlice::EMPTY);
    }

    // Live handle exercises patch routing, invalid UTF-8 rejection, default measure/hit_test, state snapshot.
    let handle = unsafe { (surface.create)(7) };
    assert!(!handle.is_null());

    // Direct zero-copy event dispatch & props update via C ABI
    let payload_entries = [NveMapEntry::new("ok", NveValue::bool(true))];
    let direct_events = [NveDirectEvent::custom(
        7,
        42,
        1234,
        "onPing",
        NveValue::map(&payload_entries),
    )];
    let prop_entries = [NveMapEntry::new("count", NveValue::i64(99))];
    unsafe {
        assert_eq!(
            (surface.dispatch_events_direct.unwrap())(
                handle,
                NveEventBatch::from_slice(&direct_events)
            ),
            1
        );
        assert_eq!(
            (component.set_props_direct.unwrap())(
                handle,
                prop_entries.as_ptr(),
                prop_entries.len() as u32
            ),
            1
        );
    }

    // Default NveApp::measure handles finite and infinite constraints properly.
    let finite_size = unsafe {
        (layout.measure)(
            handle,
            NveSizeConstraints {
                min_w: 50.0,
                min_h: 30.0,
                max_w: 300.0,
                max_h: 500.0,
            },
        )
    };
    assert_eq!(finite_size, NveIntrinsicSize { width: 300.0, height: 500.0 });

    let inf_size = unsafe {
        (layout.measure)(
            handle,
            NveSizeConstraints {
                min_w: 64.0,
                min_h: 48.0,
                max_w: f32::INFINITY,
                max_h: f32::INFINITY,
            },
        )
    };
    assert_eq!(inf_size, NveIntrinsicSize { width: 64.0, height: 48.0 });
    assert_eq!(
        NveGestureDisposition::from_u8(unsafe { (layout.hit_test)(handle, 0.0, 0.0, 1) }),
        NveGestureDisposition::PassThrough
    );

    // Valid TemplatePatch JSON routes cleanly -> returns 1.
    let valid_patch = HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "root".into(),
        attr_updates: vec![("opacity".into(), "0.6".into())],
        text_updates: vec![],
        target_test_id: None,
        node_path: vec![],
        text_replacements: vec![],
    }
    .to_json();
    assert_eq!(
        unsafe { (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(valid_patch.as_bytes())) },
        1
    );

    // Non-template patch returns 0.
    let status_patch = HotPatchMsg::DevStatus {
        message: "ping".into(),
        latency_ms: 1,
    }
    .to_json();
    assert_eq!(
        unsafe { (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(status_patch.as_bytes())) },
        0
    );

    // Invalid UTF-8 bytes safely rejected with 0.
    let bad_utf8 = [0xFFu8, 0xFE, 0x80];
    assert_eq!(
        unsafe { (dev.apply_hot_patch)(handle, NveBufferSlice::from_slice(&bad_utf8)) },
        0
    );

    // State snapshot & restore roundtrip.
    let new_state = b"saved-v2";
    unsafe { (dev.restore_state)(handle, NveBufferSlice::from_slice(new_state)) };
    let snapped = unsafe { (dev.snapshot_state)(handle).as_slice().to_vec() };
    assert_eq!(snapped, new_state);

    unsafe { (surface.destroy)(handle) };
}

// ============================================================================
// 2. AppLoader (`HotAppInstance`) error paths & valid load / hot-swap
// ============================================================================

#[test]
fn loader_reports_io_error_for_missing_dylib_and_load_error_for_invalid_binary() {
    let dir = TempDirGuard::new("loader-errors");

    let missing = dir.path().join("does_not_exist.dylib");
    let err = HotAppInstance::load(&missing, 1, 1, None)
        .err()
        .expect("missing dylib must fail");
    assert!(matches!(err, LoaderError::Io(_)), "got {err:?}");
    assert!(!err.to_string().is_empty());

    let bogus = dir.path().join("not_a_dylib.dylib");
    std::fs::write(&bogus, b"definitely not a Mach-O / ELF / PE library").unwrap();
    let err = HotAppInstance::load(&bogus, 1, 1, None)
        .err()
        .expect("corrupt binary must fail");
    assert!(matches!(err, LoaderError::Load(_)), "got {err:?}");
}

#[test]
fn loader_rejects_dylib_with_missing_symbol_or_null_surface_interface() {
    let dir = TempDirGuard::new("loader-symbols");

    // 1. Valid cdylib with no `nve_get_interface` symbol.
    let no_sym_path = compile_test_cdylib(
        dir.path(),
        "no_sym_app",
        r#"
        #[unsafe(no_mangle)]
        pub extern "C" fn unrelated_symbol() -> u32 { 42 }
        "#,
    );
    let err = HotAppInstance::load(&no_sym_path, 1, 1, None)
        .err()
        .expect("missing symbol must fail");
    assert!(matches!(err, LoaderError::Load(_)), "got {err:?}");

    // 2. Valid cdylib exporting `nve_get_interface` returning NULL for NVE_IFACE_SURFACE.
    let null_surface_path = compile_test_cdylib(
        dir.path(),
        "null_surface_app",
        r#"
        use std::ffi::c_void;
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(_id: u32) -> *const c_void {
            std::ptr::null()
        }
        "#,
    );
    let err = HotAppInstance::load(&null_surface_path, 1, 1, None)
        .err()
        .expect("null surface interface must fail");
    assert!(
        matches!(err, LoaderError::MissingSurfaceInterface),
        "got {err:?}"
    );
}

#[test]
fn loader_handles_surface_only_module_with_optional_interfaces_absent() {
    let dir = TempDirGuard::new("loader-surface-only");
    let path = compile_test_cdylib(
        dir.path(),
        "surface_only_app",
        r#"
        use std::ffi::c_void;

        #[repr(C)]
        pub struct NveBufferSlice { pub ptr: *const u8, pub len: usize }

        #[repr(C)]
        pub struct NveSurfaceApi {
            pub create: unsafe extern "C" fn(u32) -> *mut c_void,
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
            pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub dispatch_events_direct: Option<unsafe extern "C" fn(*mut c_void, *const c_void, u32) -> u32>,
        }

        struct MinimalApp { buf: Vec<u8> }

        unsafe extern "C" fn create(surface_id: u32) -> *mut c_void {
            Box::into_raw(Box::new(MinimalApp { buf: surface_id.to_le_bytes().to_vec() })) as *mut c_void
        }
        unsafe extern "C" fn destroy(h: *mut c_void) {
            if !h.is_null() { unsafe { drop(Box::from_raw(h as *mut MinimalApp)); } }
        }
        unsafe extern "C" fn render(h: *mut c_void, frame_id: u64) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut MinimalApp) };
            app.buf = frame_id.to_le_bytes().to_vec();
            NveBufferSlice { ptr: app.buf.as_ptr(), len: app.buf.len() }
        }
        unsafe extern "C" fn dispatch_events(_h: *mut c_void, ev: NveBufferSlice) -> u32 {
            u32::from(ev.len > 0)
        }

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi {
            create,
            destroy,
            render,
            dispatch_events,
            dispatch_events_direct: None,
        };

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            if id == 1 {
                &SURFACE_API as *const NveSurfaceApi as *const c_void
            } else {
                std::ptr::null()
            }
        }
        "#,
    );

    let mut inst = HotAppInstance::load(&path, 5, 1, Some(b"ignored")).expect("load surface-only");
    assert_eq!(inst.generation(), 1);
    assert_eq!(inst.render(9), 9u64.to_le_bytes());
    assert!(inst.dispatch_events(b"ev"));
    assert!(!inst.dispatch_events(b""));

    // All optional interfaces are NULL -> clean fallback defaults.
    assert_eq!(inst.measure(NveSizeConstraints::loose(300.0, 400.0)), None);
    assert_eq!(
        inst.hit_test(10.0, 20.0, 1),
        NveGestureDisposition::PassThrough
    );
    assert!(!inst.set_props(b"{}"));
    assert!(inst.drain_emitted_events().is_empty());
    assert!(!inst.apply_hot_patch("{}"));
    assert!(inst.snapshot_state().is_empty());
}

#[test]
fn loader_valid_dylib_full_lifecycle_and_state_preserving_hotswap() {
    let dir = TempDirGuard::new("loader-hotswap");
    let cdylib_src = r#"
        use std::ffi::c_void;

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveBufferSlice { pub ptr: *const u8, pub len: usize }

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveSizeConstraints { pub min_w: f32, pub min_h: f32, pub max_w: f32, pub max_h: f32 }

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveIntrinsicSize { pub width: f32, pub height: f32 }

        #[repr(C)]
        pub struct NveSurfaceApi {
            pub create: unsafe extern "C" fn(u32) -> *mut c_void,
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
            pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub dispatch_events_direct: Option<unsafe extern "C" fn(*mut c_void, *const c_void, u32) -> u32>,
        }

        #[repr(C)]
        pub struct NveLayoutApi {
            pub measure: unsafe extern "C" fn(*mut c_void, NveSizeConstraints) -> NveIntrinsicSize,
            pub hit_test: unsafe extern "C" fn(*mut c_void, f32, f32, u8) -> u8,
        }

        #[repr(C)]
        pub struct NveComponentApi {
            pub set_props: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub drain_emitted_events: unsafe extern "C" fn(*mut c_void) -> NveBufferSlice,
            pub set_props_direct: Option<unsafe extern "C" fn(*mut c_void, *const c_void, u32) -> u32>,
        }

        #[repr(C)]
        pub struct NveDevApi {
            pub apply_hot_patch: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub snapshot_state: unsafe extern "C" fn(*mut c_void) -> NveBufferSlice,
            pub restore_state: unsafe extern "C" fn(*mut c_void, NveBufferSlice),
        }

        struct FullApp {
            counter: u32,
            render_buf: Vec<u8>,
            state_buf: Vec<u8>,
            emitted_buf: Vec<u8>,
        }

        unsafe extern "C" fn create(_surface_id: u32) -> *mut c_void {
            Box::into_raw(Box::new(FullApp {
                counter: 0,
                render_buf: Vec::new(),
                state_buf: Vec::new(),
                emitted_buf: Vec::new(),
            })) as *mut c_void
        }
        unsafe extern "C" fn destroy(h: *mut c_void) {
            if !h.is_null() { unsafe { drop(Box::from_raw(h as *mut FullApp)); } }
        }
        unsafe extern "C" fn render(h: *mut c_void, frame_id: u64) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut FullApp) };
            app.render_buf = format!("frame={frame_id};counter={}", app.counter).into_bytes();
            NveBufferSlice { ptr: app.render_buf.as_ptr(), len: app.render_buf.len() }
        }
        unsafe extern "C" fn dispatch_events(h: *mut c_void, ev: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut FullApp) };
            if ev.len > 0 {
                app.counter += 1;
                app.emitted_buf = vec![0xEE, app.counter as u8];
                1
            } else {
                0
            }
        }
        unsafe extern "C" fn measure(_h: *mut c_void, c: NveSizeConstraints) -> NveIntrinsicSize {
            NveIntrinsicSize { width: c.max_w.min(250.0), height: c.max_h.min(120.0) }
        }
        unsafe extern "C" fn hit_test(_h: *mut c_void, x: f32, _y: f32, _phase: u8) -> u8 {
            if x >= 0.0 { 1 } else { 3 }
        }
        unsafe extern "C" fn set_props(h: *mut c_void, props: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut FullApp) };
            if props.len == 4 {
                let s = unsafe { std::slice::from_raw_parts(props.ptr, 4) };
                app.counter = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                1
            } else {
                0
            }
        }
        unsafe extern "C" fn drain_emitted_events(h: *mut c_void) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut FullApp) };
            NveBufferSlice { ptr: app.emitted_buf.as_ptr(), len: app.emitted_buf.len() }
        }
        unsafe extern "C" fn apply_hot_patch(h: *mut c_void, patch: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut FullApp) };
            if patch.len > 0 {
                app.counter += 10;
                1
            } else {
                0
            }
        }
        unsafe extern "C" fn snapshot_state(h: *mut c_void) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut FullApp) };
            app.state_buf = app.counter.to_le_bytes().to_vec();
            NveBufferSlice { ptr: app.state_buf.as_ptr(), len: app.state_buf.len() }
        }
        unsafe extern "C" fn restore_state(h: *mut c_void, state: NveBufferSlice) {
            let app = unsafe { &mut *(h as *mut FullApp) };
            if state.len == 4 {
                let s = unsafe { std::slice::from_raw_parts(state.ptr, 4) };
                app.counter = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            }
        }

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi {
            create,
            destroy,
            render,
            dispatch_events,
            dispatch_events_direct: None,
        };
        static LAYOUT_API: NveLayoutApi = NveLayoutApi { measure, hit_test };
        static COMPONENT_API: NveComponentApi = NveComponentApi {
            set_props,
            drain_emitted_events,
            set_props_direct: None,
        };
        static DEV_API: NveDevApi = NveDevApi { apply_hot_patch, snapshot_state, restore_state };

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            match id {
                1 => &SURFACE_API as *const NveSurfaceApi as *const c_void,
                2 => &LAYOUT_API as *const NveLayoutApi as *const c_void,
                3 => &COMPONENT_API as *const NveComponentApi as *const c_void,
                4 => &DEV_API as *const NveDevApi as *const c_void,
                _ => std::ptr::null(),
            }
        }
    "#;

    let dylib_path = compile_test_cdylib(dir.path(), "full_hotswap_app", cdylib_src);

    let mut inst = HotAppInstance::load(&dylib_path, 1, 1, None).expect("load gen 1");
    let gen1_copy = inst.loaded_copy_path().to_path_buf();
    assert!(gen1_copy.exists(), "gen 1 temp copy must exist while loaded");
    assert_eq!(inst.generation(), 1);
    assert_eq!(inst.render(1), b"frame=1;counter=0");

    assert_eq!(
        inst.measure(NveSizeConstraints::loose(500.0, 500.0)),
        Some(NveIntrinsicSize {
            width: 250.0,
            height: 120.0
        })
    );
    assert_eq!(inst.hit_test(10.0, 5.0, 1), NveGestureDisposition::Claim);
    assert_eq!(
        inst.hit_test(-1.0, 5.0, 1),
        NveGestureDisposition::DeferToParent
    );

    assert!(inst.set_props(&5u32.to_le_bytes()));
    assert!(inst.dispatch_events(b"tap"));
    assert_eq!(inst.drain_emitted_events(), vec![0xEE, 6]);
    assert!(inst.apply_hot_patch(r#"{"type":"template_patch"}"#));
    assert_eq!(inst.render(2), b"frame=2;counter=16");
    assert_eq!(inst.snapshot_state(), 16u32.to_le_bytes());

    // Hot-swap gen 1 -> gen 2 -> gen 3 preserving state across reloads and cleaning up old copies.
    let outcome2 = inst.reload(&dylib_path).expect("reload gen 2");
    assert_eq!(outcome2.previous_generation, 1);
    assert_eq!(outcome2.generation, 2);
    assert_eq!(outcome2.preserved_state_bytes, 4);
    assert!(
        !gen1_copy.exists(),
        "gen 1 temp copy must be deleted on drop after unloading library"
    );
    let gen2_copy = inst.loaded_copy_path().to_path_buf();
    assert_ne!(gen2_copy, gen1_copy);
    assert!(gen2_copy.exists(), "gen 2 temp copy must exist while loaded");
    assert_eq!(inst.generation(), 2);
    assert_eq!(inst.render(3), b"frame=3;counter=16");
    assert!(inst.dispatch_events(b"tap2"));
    assert_eq!(inst.render(4), b"frame=4;counter=17");

    // Transactional failure safety: missing file, corrupt binary, missing symbol, missing
    // NVE_IFACE_SURFACE, and null handle must ALL fail while keeping gen 2 live and untouched!
    let missing_path = dir.path().join("missing_gen3.dylib");
    assert!(matches!(
        inst.reload(&missing_path),
        Err(LoaderError::Io(_))
    ));
    assert_eq!(inst.generation(), 2);
    assert!(gen2_copy.exists());
    assert_eq!(inst.render(40), b"frame=40;counter=17");

    let corrupt_path = dir.path().join("corrupt_gen3.dylib");
    std::fs::write(&corrupt_path, b"not a valid shared object").unwrap();
    assert!(matches!(
        inst.reload(&corrupt_path),
        Err(LoaderError::Load(_))
    ));
    assert_eq!(inst.generation(), 2);
    assert_eq!(inst.render(41), b"frame=41;counter=17");

    let no_sym_path = compile_test_cdylib(
        dir.path(),
        "reload_no_sym",
        r#"
        #[unsafe(no_mangle)]
        pub extern "C" fn not_nve_get_interface() -> u32 { 1 }
        "#,
    );
    assert!(matches!(
        inst.reload(&no_sym_path),
        Err(LoaderError::Load(_))
    ));
    assert_eq!(inst.generation(), 2);
    assert_eq!(inst.render(42), b"frame=42;counter=17");

    let no_surface_path = compile_test_cdylib(
        dir.path(),
        "reload_no_surface",
        r#"
        use std::ffi::c_void;
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(_id: u32) -> *const c_void {
            std::ptr::null()
        }
        "#,
    );
    assert!(matches!(
        inst.reload(&no_surface_path),
        Err(LoaderError::MissingSurfaceInterface)
    ));
    assert_eq!(inst.generation(), 2);
    assert_eq!(inst.render(43), b"frame=43;counter=17");

    let null_handle_path = compile_test_cdylib(
        dir.path(),
        "reload_null_handle",
        r#"
        use std::ffi::c_void;
        #[repr(C)]
        pub struct NveBufferSlice { pub ptr: *const u8, pub len: usize }
        #[repr(C)]
        pub struct NveSurfaceApi {
            pub create: unsafe extern "C" fn(u32) -> *mut c_void,
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
            pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub dispatch_events_direct: Option<unsafe extern "C" fn(*mut c_void, *const c_void, u32) -> u32>,
        }
        unsafe extern "C" fn create(_: u32) -> *mut c_void { std::ptr::null_mut() }
        unsafe extern "C" fn destroy(_: *mut c_void) {}
        unsafe extern "C" fn render(_: *mut c_void, _: u64) -> NveBufferSlice {
            NveBufferSlice { ptr: std::ptr::null(), len: 0 }
        }
        unsafe extern "C" fn dispatch(_: *mut c_void, _: NveBufferSlice) -> u32 { 0 }
        static SURFACE: NveSurfaceApi = NveSurfaceApi {
            create, destroy, render, dispatch_events: dispatch, dispatch_events_direct: None
        };
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            if id == 1 { &SURFACE as *const _ as *const c_void } else { std::ptr::null() }
        }
        "#,
    );
    assert!(matches!(
        inst.reload(&null_handle_path),
        Err(LoaderError::NullHandle)
    ));
    assert_eq!(inst.generation(), 2);
    assert_eq!(inst.snapshot_state(), 17u32.to_le_bytes());
    assert_eq!(inst.render(44), b"frame=44;counter=17");

    // Subsequent valid reload to gen 3 still succeeds cleanly and preserves counter == 17!
    let outcome3 = inst.reload(&dylib_path).expect("reload gen 3");
    assert_eq!(outcome3.previous_generation, 2);
    assert_eq!(outcome3.generation, 3);
    assert_eq!(outcome3.preserved_state_bytes, 4);
    assert!(
        !gen2_copy.exists(),
        "gen 2 temp copy must be deleted on drop after unloading library"
    );
    let gen3_copy = inst.loaded_copy_path().to_path_buf();
    assert!(gen3_copy.exists(), "gen 3 temp copy must exist while loaded");
    assert_eq!(inst.generation(), 3);
    assert_eq!(inst.render(5), b"frame=5;counter=17");

    drop(inst);
    assert!(
        !gen3_copy.exists(),
        "gen 3 temp copy must be deleted when HotAppInstance is dropped"
    );
}

// ============================================================================
// 3. FrameProfiler & SurfaceRunner tests
// ============================================================================

#[test]
fn profiler_empty_capacity_eviction_and_percentiles() {
    let empty = FrameProfiler::new(8);
    assert_eq!(empty.p50_total_us(), 0);
    assert_eq!(empty.p99_total_us(), 0);
    let parsed_empty: serde_json::Value =
        serde_json::from_str(&empty.to_chrome_trace_json()).unwrap();
    assert_eq!(parsed_empty.as_array().unwrap().len(), 0);

    // Capacity 0 clamps safely to 1.
    let mut cap_zero = FrameProfiler::new(0);
    cap_zero.record(FrameTiming {
        frame_id: 1,
        reconcile_us: 10,
        ..Default::default()
    });
    cap_zero.record(FrameTiming {
        frame_id: 2,
        reconcile_us: 25,
        ..Default::default()
    });
    assert_eq!(cap_zero.frames().len(), 1);
    assert_eq!(cap_zero.frames()[0].frame_id, 2);

    // Ring buffer evicts oldest frames FIFO and computes accurate p50/p99 on unsorted totals.
    let mut profiler = FrameProfiler::new(5);
    let totals = [100u64, 500, 10, 40, 20, 30, 1000];
    for (i, total) in totals.iter().enumerate() {
        profiler.record(FrameTiming {
            frame_id: (i + 1) as u64,
            op_count: 5,
            mutation_count: 2,
            reconcile_us: *total,
            ..Default::default()
        });
    }
    // Last 5 recorded frames have totals [10, 40, 20, 30, 1000] -> sorted [10, 20, 30, 40, 1000].
    assert_eq!(profiler.frames().len(), 5);
    assert_eq!(profiler.frames()[0].frame_id, 3);
    assert_eq!(profiler.frames()[4].frame_id, 7);
    assert_eq!(profiler.p50_total_us(), 30);
    assert_eq!(profiler.p99_total_us(), 1000);
}

#[test]
fn profiler_chrome_trace_json_emits_valid_spans_and_monotonic_timestamps() {
    let mut profiler = FrameProfiler::new(16);
    let f1 = FrameTiming {
        frame_id: 1,
        op_count: 10,
        mutation_count: 4,
        reconcile_us: 15,
        encode_us: 5,
        decode_us: 7,
        layout_us: 40,
        diff_us: 8,
        mount_us: 12,
    };
    assert_eq!(f1.total_us(), 87);
    let f2 = FrameTiming {
        frame_id: 2,
        op_count: 2,
        mutation_count: 1,
        reconcile_us: 0,
        encode_us: 0,
        decode_us: 0,
        layout_us: 0,
        diff_us: 0,
        mount_us: 0,
    };
    profiler.record(f1);
    profiler.record(f2);

    let json = profiler.to_chrome_trace_json();
    let arr: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid trace JSON");
    assert_eq!(arr.len(), 12);

    let expected_stages = [
        ("rust.reconcile", 1u64, 15u64),
        ("abi.encode", 1, 5),
        ("backend.decode", 2, 7),
        ("backend.layout", 2, 40),
        ("backend.diff", 2, 8),
        ("backend.mount", 3, 12),
    ];

    let mut prev_ts = 0u64;
    for (idx, (name, tid, dur)) in expected_stages.iter().enumerate() {
        let ev = &arr[idx];
        assert_eq!(ev["name"].as_str(), Some(*name));
        assert_eq!(ev["cat"].as_str(), Some("nve.frame"));
        assert_eq!(ev["ph"].as_str(), Some("X"));
        assert_eq!(ev["pid"].as_u64(), Some(1));
        assert_eq!(ev["tid"].as_u64(), Some(*tid));
        assert_eq!(ev["dur"].as_u64(), Some(*dur));
        let ts = ev["ts"].as_u64().unwrap();
        if idx > 0 {
            assert!(ts > prev_ts, "timestamps must advance monotonically");
        }
        prev_ts = ts;
    }

    // Frame 2 starts after frame 1 + 100us gap, and zero-dur spans still advance by 1us.
    let f2_start = arr[6]["ts"].as_u64().unwrap();
    assert_eq!(f2_start, 87 + 100);
    for (offset, ev) in arr[7..12].iter().enumerate() {
        let ts = ev["ts"].as_u64().unwrap();
        assert_eq!(ts, f2_start + (offset + 1) as u64);
    }
}

struct StubBackend {
    viewport: Viewport,
    fail_next: bool,
}

impl RenderBackend for StubBackend {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn set_viewport(&mut self, _surface_id: SurfaceId, viewport: Viewport) {
        self.viewport = viewport;
    }

    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError> {
        if self.fail_next {
            self.fail_next = false;
            return Err(BackendError::Decode(DecodeError::Truncated));
        }
        Ok(BackendApplyStats {
            op_count: command_bytes.len() as u32,
            mutation_count: 3,
            decode_us: 11,
            layout_us: 22,
            diff_us: 5,
            mount_us: 9,
        })
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        Vec::new()
    }

    fn layout_of(&self, _node_id: NodeId) -> Option<LayoutRect> {
        None
    }
}

#[test]
fn surface_runner_sequences_frames_records_profiler_and_propagates_errors() {
    let vp = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 2.0,
    };
    let mut runner = SurfaceRunner::new(
        9,
        vp,
        StubBackend {
            viewport: Viewport::default(),
            fail_next: false,
        },
    );
    assert_eq!(runner.surface_id(), 9);
    assert_eq!(runner.backend().viewport, vp);

    let f1 = runner.next_frame_id();
    let f2 = runner.next_frame_id();
    assert_eq!((f1, f2), (1, 2));

    let t1 = runner
        .submit_recorded_frame(f1, 13, 4, &[1, 2, 3, 4])
        .expect("frame 1 ok");
    assert_eq!(t1.total_us(), 13 + 4 + 11 + 22 + 5 + 9);
    assert_eq!(runner.profiler().frames().len(), 1);

    // Failed frame propagates BackendError and does NOT pollute profiler history.
    runner.backend_mut().fail_next = true;
    let err = runner.submit_recorded_frame(f2, 1, 1, &[]).unwrap_err();
    assert!(matches!(err, BackendError::Decode(DecodeError::Truncated)));
    assert_eq!(runner.profiler().frames().len(), 1);
}
