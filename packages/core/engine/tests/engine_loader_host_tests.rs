mod loader_common;

use std::ffi::c_void;

use loader_common::{compile_test_cdylib, TempDirGuard, SHARED_ABI_DEFS};
use nve_abi::{FrameId, NveHostCallbacks, SurfaceId};
use nve_engine::loader::HotAppInstance;

#[test]
fn loader_resolves_host_interface_and_rebinds_across_hotswap() {
    let dir = TempDirGuard::new("loader-host-iface");
    let cdylib_src = format!(
        "{SHARED_ABI_DEFS}\n{}",
        r#"
        struct HostDylibApp { host_ctx: usize, event_bytes: usize, buf: Vec<u8> }

        unsafe extern "C" fn create(_: u32) -> *mut c_void {
            Box::into_raw(Box::new(HostDylibApp { host_ctx: 0, event_bytes: 0, buf: Vec::new() })) as *mut c_void
        }
        unsafe extern "C" fn destroy(h: *mut c_void) {
            if !h.is_null() { unsafe { drop(Box::from_raw(h as *mut HostDylibApp)); } }
        }
        unsafe extern "C" fn render(h: *mut c_void, _: u64) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut HostDylibApp) };
            app.buf = format!("ctx={};ev={}", app.host_ctx, app.event_bytes).into_bytes();
            NveBufferSlice { ptr: app.buf.as_ptr(), len: app.buf.len() }
        }
        unsafe extern "C" fn dispatch_events(_: *mut c_void, _: NveEventBatch) -> u32 { 0 }
        unsafe extern "C" fn bind_host(h: *mut c_void, cb: *const NveHostCallbacks) {
            let app = unsafe { &mut *(h as *mut HostDylibApp) };
            app.host_ctx = if cb.is_null() { 0 } else { unsafe { (*cb).host_ctx as usize } };
        }
        unsafe extern "C" fn dispatch_host_events(h: *mut c_void, batch: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut HostDylibApp) };
            app.event_bytes += batch.len;
            u32::from(batch.len > 0)
        }

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi { create, destroy, render, dispatch_events };
        static HOST_API: NveHostApi = NveHostApi { bind_host, dispatch_host_events };

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            match id {
                1 => &SURFACE_API as *const NveSurfaceApi as *const c_void,
                5 => &HOST_API as *const NveHostApi as *const c_void,
                _ => std::ptr::null(),
            }
        }
        "#
    );
    let dylib = compile_test_cdylib(dir.path(), "host_iface_app", &cdylib_src);
    let mut inst = HotAppInstance::load(&dylib, SurfaceId::ROOT, 1, None).expect("load host-capable dylib");
    assert!(inst.has_host_interface());

    let cb = NveHostCallbacks {
        host_ctx: 0xBEEFusize as *mut c_void,
        invoke: None,
        channel_publish: None,
    };
    assert!(inst.bind_host(Some(cb)));
    assert!(inst.dispatch_host_events(b"hostpayload"));
    assert_eq!(inst.render(FrameId(1)), format!("ctx={};ev=11", 0xBEEFusize).as_bytes());

    let outcome = inst.reload(&dylib).expect("reload gen 2");
    assert_eq!(outcome.generation, 2);
    assert_eq!(inst.render(FrameId(2)), format!("ctx={};ev=0", 0xBEEFusize).as_bytes());
}

#[test]
fn loader_zero_copy_borrows_dylib_buffers_directly_and_handles_null_or_empty_slices() {
    let dir = TempDirGuard::new("loader-zerocopy-ptr");
    let cdylib_src = format!(
        "{SHARED_ABI_DEFS}\n{}",
        r#"
        struct ZeroCopyApp {
            frame_buf: Box<[u8; 16]>,
            emitted_buf: Box<[NveDirectEvent; 1]>,
            state_buf: Box<[u8; 16]>,
            mode: u64,
        }

        unsafe extern "C" fn create(_: u32) -> *mut c_void {
            let mut app = Box::new(ZeroCopyApp {
                frame_buf: Box::new([0u8; 16]),
                emitted_buf: Box::new([NveDirectEvent {
                    surface_id: 1,
                    node_id: 2,
                    timestamp_us: 0,
                    kind: 1,
                    payload_tag: 1,
                    modifiers: 0,
                    event_flags: 0,
                    key_code: 0,
                    name: NveStrSlice { ptr: std::ptr::null(), len: 0 },
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                    text: NveStrSlice { ptr: std::ptr::null(), len: 0 },
                    value: NveValue { tag: 0, _pad: 0, payload: [0, 0] },
                }]),
                state_buf: Box::new([0u8; 16]),
                mode: 0,
            });
            let f_addr = app.frame_buf.as_ptr() as usize;
            app.frame_buf[0..8].copy_from_slice(&(f_addr as u64).to_le_bytes());
            let e_addr = app.emitted_buf.as_ptr() as usize;
            app.emitted_buf[0].timestamp_us = e_addr as u64;
            let s_addr = app.state_buf.as_ptr() as usize;
            app.state_buf[0..8].copy_from_slice(&(s_addr as u64).to_le_bytes());
            Box::into_raw(app) as *mut c_void
        }

        unsafe extern "C" fn destroy(h: *mut c_void) {
            if !h.is_null() { unsafe { drop(Box::from_raw(h as *mut ZeroCopyApp)); } }
        }

        unsafe extern "C" fn render(h: *mut c_void, frame_id: u64) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut ZeroCopyApp) };
            app.mode = frame_id;
            match frame_id {
                0 => NveBufferSlice { ptr: std::ptr::null(), len: 0 },
                99 => NveBufferSlice { ptr: std::ptr::null(), len: 128 },
                100 => NveBufferSlice { ptr: app.frame_buf.as_ptr(), len: 0 },
                _ => {
                    app.frame_buf[8..16].copy_from_slice(&frame_id.to_le_bytes());
                    NveBufferSlice { ptr: app.frame_buf.as_ptr(), len: 16 }
                }
            }
        }

        unsafe extern "C" fn dispatch_events(_: *mut c_void, _: NveEventBatch) -> u32 { 0 }
        unsafe extern "C" fn set_props(_: *mut c_void, _: *const NveMapEntry, _: u32) -> u32 { 0 }

        unsafe extern "C" fn drain_emitted_events(h: *mut c_void) -> NveEventBatch {
            let app = unsafe { &mut *(h as *mut ZeroCopyApp) };
            match app.mode {
                0 => NveEventBatch { ptr: std::ptr::null(), count: 0, _pad: 0 },
                99 => NveEventBatch { ptr: std::ptr::null(), count: 64, _pad: 0 },
                100 => NveEventBatch { ptr: app.emitted_buf.as_ptr(), count: 0, _pad: 0 },
                _ => NveEventBatch { ptr: app.emitted_buf.as_ptr(), count: 1, _pad: 0 },
            }
        }

        unsafe extern "C" fn apply_hot_patch(_: *mut c_void, _: NveBufferSlice) -> u32 { 0 }

        unsafe extern "C" fn snapshot_state(h: *mut c_void) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut ZeroCopyApp) };
            match app.mode {
                0 => NveBufferSlice { ptr: std::ptr::null(), len: 0 },
                99 => NveBufferSlice { ptr: std::ptr::null(), len: 32 },
                100 => NveBufferSlice { ptr: app.state_buf.as_ptr(), len: 0 },
                _ => NveBufferSlice { ptr: app.state_buf.as_ptr(), len: 16 },
            }
        }

        unsafe extern "C" fn restore_state(_: *mut c_void, _: NveBufferSlice) {}

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi { create, destroy, render, dispatch_events };
        static COMPONENT_API: NveComponentApi = NveComponentApi { set_props, drain_emitted_events };
        static DEV_API: NveDevApi = NveDevApi { apply_hot_patch, snapshot_state, restore_state };

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            match id {
                1 => &SURFACE_API as *const NveSurfaceApi as *const c_void,
                3 => &COMPONENT_API as *const NveComponentApi as *const c_void,
                4 => &DEV_API as *const NveDevApi as *const c_void,
                _ => std::ptr::null(),
            }
        }
        "#
    );

    let dylib = compile_test_cdylib(dir.path(), "zerocopy_ptr_app", &cdylib_src);
    let mut inst = HotAppInstance::load(&dylib, SurfaceId::ROOT, 1, None).expect("load zerocopy dylib");

    let frame = inst.render(FrameId(7));
    assert_eq!(frame.len(), 16);
    assert_eq!(frame.as_ptr() as usize, u64::from_le_bytes(frame[0..8].try_into().unwrap()) as usize);
    assert_eq!(u64::from_le_bytes(frame[8..16].try_into().unwrap()), 7);

    let emitted = inst.drain_emitted_events();
    assert_eq!(emitted.len(), 1);
    assert_eq!(
        emitted.as_slice().as_ptr() as usize,
        emitted.get(0).unwrap().timestamp_us() as usize
    );

    let state = inst.snapshot_state();
    assert_eq!(state.len(), 16);
    assert_eq!(state.as_ptr() as usize, u64::from_le_bytes(state[0..8].try_into().unwrap()) as usize);

    for edge_frame in [0u64, 99, 100] {
        assert!(inst.render(FrameId(edge_frame)).is_empty());
        assert!(inst.drain_emitted_events().is_empty());
        assert!(inst.snapshot_state().is_empty());
    }
}
