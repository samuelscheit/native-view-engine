//! `HotAppInstance` (`AppLoader`) integration tests:
//! - Error paths (missing file, invalid dylib, missing symbol, null surface interface)
//! - Optional interface fallback, full dylib lifecycle, and state-preserving hot-swap
//! - `NVE_IFACE_HOST` rebinding across hot-swap
//! - Zero-copy pointer-identity borrowing across `render`, `drain_emitted_events`, and `snapshot_state`

mod loader_common;

use loader_common::{compile_test_cdylib, TempDirGuard, SHARED_ABI_DEFS};
use nve_abi::{
    FrameId, NodeId, NveDirectEvent, NveGestureDisposition, NveIntrinsicSize, NveMapEntry,
    NveSizeConstraints, NveValue, SurfaceId,
};
use nve_engine::loader::{HotAppInstance, LoaderError};
use nve_engine::HotPatchMsg;

#[test]
fn loader_reports_io_error_for_missing_dylib_and_load_error_for_invalid_binary() {
    let dir = TempDirGuard::new("loader-errors");

    let missing = dir.path().join("does_not_exist.dylib");
    let err = HotAppInstance::load(&missing, SurfaceId::ROOT, 1, None)
        .err()
        .expect("missing dylib must fail");
    assert!(matches!(err, LoaderError::Io(_)), "got {err:?}");
    assert!(!err.to_string().is_empty());

    let bogus = dir.path().join("not_a_dylib.dylib");
    std::fs::write(&bogus, b"definitely not a Mach-O / ELF / PE library").unwrap();
    let err = HotAppInstance::load(&bogus, SurfaceId::ROOT, 1, None)
        .err()
        .expect("corrupt binary must fail");
    assert!(matches!(err, LoaderError::Load(_)), "got {err:?}");
}

#[test]
fn loader_rejects_dylib_with_missing_symbol_or_null_surface_interface() {
    let dir = TempDirGuard::new("loader-symbols");

    let no_sym_path = compile_test_cdylib(
        dir.path(),
        "no_sym_app",
        r#"
        #[unsafe(no_mangle)]
        pub extern "C" fn unrelated_symbol() -> u32 { 42 }
        "#,
    );
    let err = HotAppInstance::load(&no_sym_path, SurfaceId::ROOT, 1, None)
        .err()
        .expect("missing symbol must fail");
    assert!(matches!(err, LoaderError::Load(_)), "got {err:?}");

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
    let err = HotAppInstance::load(&null_surface_path, SurfaceId::ROOT, 1, None)
        .err()
        .expect("null surface interface must fail");
    assert!(matches!(err, LoaderError::MissingSurfaceInterface), "got {err:?}");
}

#[test]
fn loader_handles_surface_only_module_with_optional_interfaces_absent() {
    let dir = TempDirGuard::new("loader-surface-only");
    let src = format!(
        "{SHARED_ABI_DEFS}\n{}",
        r#"
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
        unsafe extern "C" fn dispatch_events(_h: *mut c_void, ev: NveEventBatch) -> u32 {
            u32::from(ev.count > 0)
        }

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi {
            create,
            destroy,
            render,
            dispatch_events,
        };

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            if id == 1 {
                &SURFACE_API as *const NveSurfaceApi as *const c_void
            } else {
                std::ptr::null()
            }
        }
        "#
    );
    let path = compile_test_cdylib(dir.path(), "surface_only_app", &src);

    let mut inst = HotAppInstance::load(&path, SurfaceId::new(5, 0), 1, Some(b"ignored")).expect("load surface-only");
    assert_eq!(inst.generation(), 1);
    assert_eq!(inst.render(FrameId(9)), 9u64.to_le_bytes());
    let tap = [NveDirectEvent::press(SurfaceId::new(5, 0), NodeId::new(2, 0), 10, 1.0, 2.0)];
    assert!(inst.dispatch_events(&tap));
    assert!(!inst.dispatch_events(&[]));

    assert_eq!(inst.measure(NveSizeConstraints::loose(300.0, 400.0)), None);
    assert_eq!(inst.hit_test(10.0, 20.0, 1), NveGestureDisposition::PassThrough);
    assert!(!inst.set_props(&[]));
    assert!(inst.drain_emitted_events().is_empty());
    assert!(!inst.apply_hot_patch(b"NVP1"));
    assert!(inst.snapshot_state().is_empty());
}

#[test]
fn loader_valid_dylib_full_lifecycle_and_state_preserving_hotswap() {
    let dir = TempDirGuard::new("loader-hotswap");
    let cdylib_src = format!(
        "{SHARED_ABI_DEFS}\n{}",
        r#"
        struct FullApp {
            counter: u32,
            render_buf: Vec<u8>,
            state_buf: Vec<u8>,
            emitted_buf: Vec<NveDirectEvent>,
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
        unsafe extern "C" fn dispatch_events(h: *mut c_void, ev: NveEventBatch) -> u32 {
            let app = unsafe { &mut *(h as *mut FullApp) };
            if ev.count > 0 {
                app.counter += 1;
                app.emitted_buf = vec![NveDirectEvent {
                    surface_id: 1,
                    node_id: app.counter,
                    timestamp_us: 99,
                    kind: 1,
                    payload_tag: 1,
                    modifiers: 0,
                    event_flags: 0,
                    key_code: 0,
                    name: NveStrSlice { ptr: std::ptr::null(), len: 0 },
                    x: 1.0,
                    y: 2.0,
                    width: 0.0,
                    height: 0.0,
                    text: NveStrSlice { ptr: std::ptr::null(), len: 0 },
                    value: NveValue { tag: 0, _pad: 0, payload: [0, 0] },
                }];
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
        unsafe extern "C" fn set_props(h: *mut c_void, entries: *const NveMapEntry, count: u32) -> u32 {
            let app = unsafe { &mut *(h as *mut FullApp) };
            if !entries.is_null() && count > 0 {
                let first = unsafe { &*entries };
                if first.value.tag == 2 {
                    app.counter = first.value.payload[0] as u32;
                    return 1;
                }
            }
            0
        }
        unsafe extern "C" fn drain_emitted_events(h: *mut c_void) -> NveEventBatch {
            let app = unsafe { &mut *(h as *mut FullApp) };
            NveEventBatch { ptr: app.emitted_buf.as_ptr(), count: app.emitted_buf.len() as u32, _pad: 0 }
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

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi { create, destroy, render, dispatch_events };
        static LAYOUT_API: NveLayoutApi = NveLayoutApi { measure, hit_test };
        static COMPONENT_API: NveComponentApi = NveComponentApi { set_props, drain_emitted_events };
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
        "#
    );

    let dylib_path = compile_test_cdylib(dir.path(), "full_hotswap_app", &cdylib_src);

    let mut inst = HotAppInstance::load(&dylib_path, SurfaceId::ROOT, 1, None).expect("load gen 1");
    let gen1_copy = inst.loaded_copy_path().to_path_buf();
    assert!(gen1_copy.exists());
    assert_eq!(inst.generation(), 1);
    assert_eq!(inst.render(FrameId(1)), b"frame=1;counter=0");

    assert_eq!(
        inst.measure(NveSizeConstraints::loose(500.0, 500.0)),
        Some(NveIntrinsicSize { width: 250.0, height: 120.0 })
    );
    assert_eq!(inst.hit_test(10.0, 5.0, 1), NveGestureDisposition::Claim);
    assert_eq!(inst.hit_test(-1.0, 5.0, 1), NveGestureDisposition::DeferToParent);

    let props = [NveMapEntry::new("counter", NveValue::i64(5))];
    assert!(inst.set_props(&props));
    let tap = [NveDirectEvent::press(SurfaceId::ROOT, NodeId::new(2, 0), 10, 1.0, 2.0)];
    assert!(inst.dispatch_events(&tap));
    let emitted = inst.drain_emitted_events();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted.get(0).unwrap().node_id(), NodeId::new(6, 0));

    let patch = HotPatchMsg::DevStatus { message: "fixture".into(), latency_ms: 0 }
        .to_bytes()
        .unwrap();
    assert!(inst.apply_hot_patch(&patch));
    assert_eq!(inst.render(FrameId(2)), b"frame=2;counter=16");
    assert_eq!(inst.snapshot_state(), 16u32.to_le_bytes());

    let outcome2 = inst.reload(&dylib_path).expect("reload gen 2");
    assert_eq!(outcome2.previous_generation, 1);
    assert_eq!(outcome2.generation, 2);
    assert_eq!(outcome2.preserved_state_bytes, 4);
    assert!(!gen1_copy.exists());
    let gen2_copy = inst.loaded_copy_path().to_path_buf();
    assert_ne!(gen2_copy, gen1_copy);
    assert!(gen2_copy.exists());
    assert_eq!(inst.generation(), 2);
    assert_eq!(inst.render(FrameId(3)), b"frame=3;counter=16");
    assert!(inst.dispatch_events(&tap));
    assert_eq!(inst.render(FrameId(4)), b"frame=4;counter=17");

    // Transactional failure safety across missing/corrupt/bad-symbol/no-surface/null-handle.
    assert!(matches!(inst.reload(&dir.path().join("missing.dylib")), Err(LoaderError::Io(_))));
    let corrupt_path = dir.path().join("corrupt_gen3.dylib");
    std::fs::write(&corrupt_path, b"not a valid shared object").unwrap();
    assert!(matches!(inst.reload(&corrupt_path), Err(LoaderError::Load(_))));

    let no_sym_path = compile_test_cdylib(
        dir.path(),
        "reload_no_sym",
        "#[unsafe(no_mangle)] pub extern \"C\" fn not_nve_get_interface() -> u32 { 1 }",
    );
    assert!(matches!(inst.reload(&no_sym_path), Err(LoaderError::Load(_))));

    let no_surface_path = compile_test_cdylib(
        dir.path(),
        "reload_no_surface",
        "use std::ffi::c_void; #[unsafe(no_mangle)] pub unsafe extern \"C\" fn nve_get_interface(_: u32) -> *const c_void { std::ptr::null() }",
    );
    assert!(matches!(inst.reload(&no_surface_path), Err(LoaderError::MissingSurfaceInterface)));

    let null_handle_src = format!(
        "{SHARED_ABI_DEFS}\n{}",
        r#"
        unsafe extern "C" fn create(_: u32) -> *mut c_void { std::ptr::null_mut() }
        unsafe extern "C" fn destroy(_: *mut c_void) {}
        unsafe extern "C" fn render(_: *mut c_void, _: u64) -> NveBufferSlice {
            NveBufferSlice { ptr: std::ptr::null(), len: 0 }
        }
        unsafe extern "C" fn dispatch(_: *mut c_void, _: NveEventBatch) -> u32 { 0 }
        static SURFACE: NveSurfaceApi = NveSurfaceApi { create, destroy, render, dispatch_events: dispatch };
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            if id == 1 { &SURFACE as *const _ as *const c_void } else { std::ptr::null() }
        }
        "#
    );
    let null_handle_path = compile_test_cdylib(dir.path(), "reload_null_handle", &null_handle_src);
    assert!(matches!(inst.reload(&null_handle_path), Err(LoaderError::NullHandle)));
    assert_eq!(inst.generation(), 2);
    assert_eq!(inst.snapshot_state(), 17u32.to_le_bytes());

    let outcome3 = inst.reload(&dylib_path).expect("reload gen 3");
    assert_eq!(outcome3.generation, 3);
    assert!(!gen2_copy.exists());
    let gen3_copy = inst.loaded_copy_path().to_path_buf();
    assert_eq!(inst.render(FrameId(5)), b"frame=5;counter=17");
    drop(inst);
    assert!(!gen3_copy.exists());
}
