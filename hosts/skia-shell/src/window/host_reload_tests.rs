//! Tier-2 `cdylib` hot-reload host-services re-binding tests verifying `NVE_IFACE_HOST = 5`
//! callbacks and direct `#[repr(C)]` `ThemeSnapshot` delivery stay non-dangling across reloads.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use winit::window::Theme as WinitTheme;

use super::host_services_tests::make_memory_services;
use super::{DesktopApp, HotAppSlot};
use crate::host_services::SkiaHostEvent;

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nve_skia_host_reload_{prefix}_{}_{seq}",
            std::process::id()
        ));
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

fn compile_test_cdylib(dir: &Path, name: &str, source: &str) -> PathBuf {
    let src_path = dir.join(format!("{name}.rs"));
    std::fs::write(&src_path, source).expect("write test cdylib source");
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    let out_path = dir.join(format!("lib{name}.{ext}"));
    let status = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2024")
        .arg(&src_path)
        .arg("-o")
        .arg(&out_path)
        .status()
        .expect("invoke rustc for test cdylib");
    assert!(status.success(), "rustc failed compiling {name}");
    out_path
}

#[test]
fn hot_app_slot_binds_host_services_rebinds_across_tier2_reload_without_dangling_pointers() {
    let dir = TempDirGuard::new("skia-host-reload");
    let cdylib_src = r#"
        use std::ffi::c_void;

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveBufferSlice { pub ptr: *const u8, pub len: usize }

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveHostCallbacks {
            pub host_ctx: *mut c_void,
            pub invoke: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u64, NveBufferSlice, *mut u8, usize, *mut usize) -> i32>,
            pub channel_publish: Option<unsafe extern "C" fn(*mut c_void, u32, u32, NveBufferSlice) -> i32>,
        }

        #[repr(C)]
        pub struct NveSurfaceApi {
            pub create: unsafe extern "C" fn(u32) -> *mut c_void,
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
            pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub dispatch_events_direct: Option<unsafe extern "C" fn(*mut c_void, *const c_void, u32) -> u32>,
        }

        #[repr(C)]
        pub struct NveDevApi {
            pub apply_hot_patch: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub snapshot_state: unsafe extern "C" fn(*mut c_void) -> NveBufferSlice,
            pub restore_state: unsafe extern "C" fn(*mut c_void, NveBufferSlice),
        }

        #[repr(C)]
        pub struct NveHostApi {
            pub bind_host: unsafe extern "C" fn(*mut c_void, *const NveHostCallbacks),
            pub dispatch_host_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
        }

        struct LiveHostCdylib {
            cb: Option<NveHostCallbacks>,
            theme_updates: u32,
            reloads_preserved: u32,
            last_theme_scheme: u8,
            frame_buf: Vec<u8>,
            state_buf: Vec<u8>,
        }

        unsafe extern "C" fn create(_: u32) -> *mut c_void {
            Box::into_raw(Box::new(LiveHostCdylib {
                cb: None,
                theme_updates: 0,
                reloads_preserved: 1,
                last_theme_scheme: 0,
                frame_buf: Vec::new(),
                state_buf: Vec::new(),
            })) as *mut c_void
        }

        unsafe extern "C" fn destroy(h: *mut c_void) {
            if !h.is_null() {
                unsafe { drop(Box::from_raw(h as *mut LiveHostCdylib)); }
            }
        }

        unsafe extern "C" fn render(h: *mut c_void, frame_id: u64) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            // Invoke SERVICE_SYSTEM_ENV::METHOD_GET_THEME (0x0014, 1) returning raw 12-byte
            // #[repr(C)] ThemeSnapshot on every render to prove host_ctx stays non-dangling!
            let mut scheme = 0u8;
            if let Some(cb) = app.cb
                && let Some(invoke) = cb.invoke
            {
                let mut out = [0u8; 32];
                let mut out_len = 0usize;
                let st = unsafe {
                    invoke(
                        cb.host_ctx,
                        0x0014,
                        1,
                        frame_id,
                        NveBufferSlice { ptr: std::ptr::null(), len: 0 },
                        out.as_mut_ptr(),
                        out.len(),
                        &mut out_len,
                    )
                };
                if st == 0 && out_len == 12 {
                    scheme = out[0];
                }
            }
            app.last_theme_scheme = scheme;
            app.frame_buf = format!(
                "f={frame_id};bound={};themes={};state={};scheme={}",
                u8::from(app.cb.is_some()),
                app.theme_updates,
                app.reloads_preserved,
                app.last_theme_scheme,
            )
            .into_bytes();
            NveBufferSlice { ptr: app.frame_buf.as_ptr(), len: app.frame_buf.len() }
        }

        unsafe extern "C" fn dispatch_events(h: *mut c_void, ev: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            if ev.len > 0 {
                app.reloads_preserved += 1;
                return 1;
            }
            0
        }

        unsafe extern "C" fn apply_hot_patch(_: *mut c_void, _: NveBufferSlice) -> u32 { 0 }

        unsafe extern "C" fn snapshot_state(h: *mut c_void) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            app.state_buf = app.reloads_preserved.to_le_bytes().to_vec();
            NveBufferSlice { ptr: app.state_buf.as_ptr(), len: app.state_buf.len() }
        }

        unsafe extern "C" fn restore_state(h: *mut c_void, s: NveBufferSlice) {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            if s.len == 4 && !s.ptr.is_null() {
                let bytes = unsafe { std::slice::from_raw_parts(s.ptr, 4) };
                app.reloads_preserved = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            }
        }

        unsafe extern "C" fn bind_host(h: *mut c_void, cb: *const NveHostCallbacks) {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            app.cb = if cb.is_null() { None } else { Some(unsafe { *cb }) };
        }

        unsafe extern "C" fn dispatch_host_events(h: *mut c_void, payload: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            // Direct 12-byte #[repr(C)] ThemeSnapshot payload (zero binary batch codecs).
            if payload.len == 12 && !payload.ptr.is_null() {
                app.theme_updates += 1;
                return 1;
            }
            0
        }

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi {
            create, destroy, render, dispatch_events, dispatch_events_direct: None,
        };
        static DEV_API: NveDevApi = NveDevApi { apply_hot_patch, snapshot_state, restore_state };
        static HOST_API: NveHostApi = NveHostApi { bind_host, dispatch_host_events };

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            match id {
                1 => &SURFACE_API as *const NveSurfaceApi as *const c_void,
                4 => &DEV_API as *const NveDevApi as *const c_void,
                5 => &HOST_API as *const NveHostApi as *const c_void,
                _ => std::ptr::null(),
            }
        }
    "#;

    let dylib = compile_test_cdylib(dir.path(), "skia_host_reload_app", cdylib_src);
    let (_mem, services) = make_memory_services();

    let mut slot = HotAppSlot::load(&dylib, 1).expect("load cdylib into HotAppSlot");
    assert!(slot.has_host_interface());
    assert!(slot.bind_host_services(Some(Arc::clone(&services))));

    // Gen 1 received initial #[repr(C)] ThemeSnapshot and invokes SERVICE_SYSTEM_ENV::GET_THEME live.
    assert_eq!(slot.render(1), b"f=1;bound=1;themes=1;state=1;scheme=1");
    assert!(slot.dispatch_events(b"tick"));

    // Update theme to Dark -> push typed SkiaHostEvent::ThemeChanged -> Tier-2 reload to Gen 2.
    let dark_snap = services.update_theme_from_winit(WinitTheme::Dark, 2.0);
    assert!(slot.dispatch_host_event(&SkiaHostEvent::ThemeChanged(dark_snap)));
    assert_eq!(slot.render(2), b"f=2;bound=1;themes=2;state=2;scheme=2");

    // Tier-2 reload preserves state=2, re-binds SkiaHostServices onto Gen 2 without dangling pointers.
    let gen2 = slot.reload(&dylib, 1).expect("reload gen 2");
    assert_eq!(gen2, 2);
    assert_eq!(slot.render(3), b"f=3;bound=1;themes=1;state=2;scheme=2");
    assert!(slot.dispatch_events(b"tick"));

    // Unbind cleanly clears callbacks (scheme becomes 0, bound=0).
    assert!(slot.bind_host_services(None));
    assert_eq!(slot.render(4), b"f=4;bound=0;themes=1;state=3;scheme=0");

    // Re-bind + drop slot safely without dangling pointers.
    assert!(slot.bind_host_services(Some(Arc::clone(&services))));
    drop(slot);
    assert_eq!(Arc::strong_count(&services), 1);
}
