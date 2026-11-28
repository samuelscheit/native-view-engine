//! Tier-2 `cdylib` hot-reload host-services re-binding tests verifying `NVE_IFACE_HOST = 5`
//! callbacks and direct `#[repr(C)]` `ThemeSnapshot` delivery stay non-dangling across reloads.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nve_abi::{FrameId, NodeId, SurfaceId};
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
        #[derive(Clone, Copy)]
        pub struct NveEventBatch {
            pub events: *const c_void,
            pub bytes: *const u8,
            pub event_count: u32,
            pub bytes_len: u32,
        }

        #[repr(C)]
        pub struct NveSurfaceApi {
            pub create: unsafe extern "C" fn(u32) -> *mut c_void,
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
            pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveEventBatch) -> u32,
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

        unsafe extern "C" fn dispatch_events(h: *mut c_void, ev: NveEventBatch) -> u32 {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            if ev.event_count > 0 {
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
            // Canonical NveHostEventBatch ("NVHB" v1) carrying a SERVICE_SYSTEM_ENV Push record.
            if payload.len >= 40 && !payload.ptr.is_null() {
                let bytes = unsafe { std::slice::from_raw_parts(payload.ptr, payload.len) };
                if &bytes[0..4] == b"NVHB"
                    && u16::from_le_bytes([bytes[4], bytes[5]]) == 1
                    && u16::from_le_bytes([bytes[6], bytes[7]]) >= 1
                    && bytes[8] == 2
                    && u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) == 0x0014
                    && u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) == 1
                    && u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]) == 12
                {
                    app.theme_updates += 1;
                    return 1;
                }
            }
            0
        }

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi {
            create, destroy, render, dispatch_events,
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

    let mut slot = HotAppSlot::load(&dylib, SurfaceId::ROOT).expect("load cdylib into HotAppSlot");
    assert!(slot.has_host_interface());
    assert!(slot.bind_host_services(Some(Arc::clone(&services))));

    // Gen 1 received initial #[repr(C)] ThemeSnapshot and invokes SERVICE_SYSTEM_ENV::GET_THEME live.
    assert_eq!(slot.render(FrameId(1)), b"f=1;bound=1;themes=1;state=1;scheme=1");
    let mut tick = nve_abi::EventBatchBuf::new();
    tick.push_press(SurfaceId::ROOT, NodeId::new(2, 0), nve_abi::TimestampUs(1), 0.0, 0.0);
    assert!(slot.dispatch_events(tick.as_validated()));

    // Update theme to Dark -> push typed SkiaHostEvent::ThemeChanged -> Tier-2 reload to Gen 2.
    let dark_snap = services.update_theme_from_winit(WinitTheme::Dark, 2.0);
    assert!(slot.dispatch_host_event(&SkiaHostEvent::ThemeChanged(dark_snap)));
    assert_eq!(slot.render(FrameId(2)), b"f=2;bound=1;themes=2;state=2;scheme=2");

    // Tier-2 reload preserves state=2, re-binds SkiaHostServices onto Gen 2 without dangling pointers.
    let gen2 = slot.reload(&dylib, SurfaceId::ROOT).expect("reload gen 2");
    assert_eq!(gen2, 2);
    assert_eq!(slot.render(FrameId(3)), b"f=3;bound=1;themes=1;state=2;scheme=2");
    assert!(slot.dispatch_events(tick.as_validated()));

    // Unbind cleanly clears callbacks (scheme becomes 0, bound=0).
    assert!(slot.bind_host_services(None));
    assert_eq!(slot.render(FrameId(4)), b"f=4;bound=0;themes=1;state=3;scheme=0");

    // Re-bind + drop slot safely without dangling pointers.
    assert!(slot.bind_host_services(Some(Arc::clone(&services))));
    drop(slot);
    assert_eq!(Arc::strong_count(&services), 1);
}

#[test]
fn skia_host_services_distinct_request_ids_take_async_panic_guard_and_rebind_cancel() {
    use nve_abi::SERVICE_CLIPBOARD;
    use nve_host_services::{
        CompletionSink, FfiHostServices, HandlerReply, HostServiceHandler, HostServiceProvider,
        HostServiceRegistry, InvokeResult, ServiceError,
    };

    struct PendingOrPanicHandler;
    impl HostServiceHandler for PendingOrPanicHandler {
        fn service_id(&self) -> u32 {
            SERVICE_CLIPBOARD
        }
        fn handle(
            &self,
            method_id: u32,
            _request_id: u64,
            _request: &[u8],
            _done: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            match method_id {
                1 => Ok(HandlerReply::Pending),
                9 => panic!("skia handler panic across extern C"),
                _ => Err(ServiceError::Unsupported),
            }
        }
    }

    let reg = Arc::new(HostServiceRegistry::new().with_handler(PendingOrPanicHandler));
    let svc = crate::host_services::SkiaHostServices::with_provider(
        Arc::clone(&reg) as Arc<dyn HostServiceProvider>
    );

    // invoke_raw MUST allocate distinct non-zero request IDs and forward take_async_response.
    let InvokeResult::Pending { request_id: id1 } =
        svc.invoke_raw(SERVICE_CLIPBOARD, 1, &[]).unwrap()
    else {
        panic!("expected Pending 1");
    };
    let InvokeResult::Pending { request_id: id2 } =
        svc.invoke_raw(SERVICE_CLIPBOARD, 1, &[]).unwrap()
    else {
        panic!("expected Pending 2");
    };
    assert_ne!(id1, 0);
    assert_ne!(id2, 0);
    assert_ne!(id1, id2);

    assert!(reg.completion_sink().complete_ok(id1, b"resp-1".to_vec()));
    assert!(reg.completion_sink().complete_ok(id2, b"resp-2".to_vec()));
    assert_eq!(svc.take_async_response(id1), Some(Ok(b"resp-1".to_vec())));
    assert_eq!(svc.take_async_response(id2), Some(Ok(b"resp-2".to_vec())));

    // Panicking handler over SkiaHostServices::callbacks() converts to InternalError (no unwind).
    let ffi = FfiHostServices::with_callbacks(svc.callbacks());
    assert!(matches!(
        ffi.invoke_sync(SERVICE_CLIPBOARD, 9, &[]),
        Err(ServiceError::InternalError(_))
    ));
    assert_eq!(reg.completion_sink().in_flight_count(), 0);

    // Pending request on host cancels cleanly via SkiaHostServices::cancel_pending().
    let InvokeResult::Pending { request_id: id3 } =
        svc.invoke_raw(SERVICE_CLIPBOARD, 1, &[]).unwrap()
    else {
        panic!("expected Pending 3");
    };
    assert_eq!(svc.cancel_pending(), 1);
    assert_eq!(
        svc.take_async_response(id3),
        Some(Err(ServiceError::Cancelled))
    );
}

#[test]
fn hot_app_slot_async_delivery_transactional_reload_no_double_bind_and_sibling_isolation() {
    use std::sync::atomic::AtomicUsize;

    use nve_abi::{ColorRgba, NveHostEventBatchBuilder, SERVICE_CLIPBOARD};
    use nve_engine::Viewport;
    use nve_host_services::{
        CompletionSink, HandlerReply, HostServiceHandler, HostServiceProvider, HostServiceRegistry,
        ServiceError,
    };

    use super::{DesktopPreviewOptions, DesktopSession, DesktopWindowHost};

    // Mock cdylib that initiates a Pending SERVICE_CLIPBOARD call (request_id = 1) inside
    // `bind_host`, counts how many times `bind_host` was invoked per instance, and records every
    // delivered Response record `(req_id, status, payload)` from `dispatch_host_events`.
    let dir = TempDirGuard::new("skia-async-reload");
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
        #[derive(Clone, Copy)]
        pub struct NveEventBatch {
            pub events: *const c_void,
            pub bytes: *const u8,
            pub event_count: u32,
            pub bytes_len: u32,
        }

        #[repr(C)]
        pub struct NveSurfaceApi {
            pub create: unsafe extern "C" fn(u32) -> *mut c_void,
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
            pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveEventBatch) -> u32,
        }

        #[repr(C)]
        pub struct NveHostApi {
            pub bind_host: unsafe extern "C" fn(*mut c_void, *const NveHostCallbacks),
            pub dispatch_host_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
        }

        struct AsyncCdylib {
            bind_calls: u32,
            resp_count: u32,
            cancelled_count: u32,
            last_payload: String,
            frame_buf: Vec<u8>,
        }

        unsafe extern "C" fn create(_: u32) -> *mut c_void {
            Box::into_raw(Box::new(AsyncCdylib {
                bind_calls: 0,
                resp_count: 0,
                cancelled_count: 0,
                last_payload: String::new(),
                frame_buf: Vec::new(),
            })) as *mut c_void
        }

        unsafe extern "C" fn destroy(h: *mut c_void) {
            if !h.is_null() {
                unsafe { drop(Box::from_raw(h as *mut AsyncCdylib)); }
            }
        }

        unsafe extern "C" fn render(h: *mut c_void, _: u64) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut AsyncCdylib) };
            app.frame_buf = format!(
                "binds={};resps={};cancels={};last={}",
                app.bind_calls, app.resp_count, app.cancelled_count, app.last_payload
            )
            .into_bytes();
            NveBufferSlice { ptr: app.frame_buf.as_ptr(), len: app.frame_buf.len() }
        }

        unsafe extern "C" fn dispatch_events(_: *mut c_void, _: NveEventBatch) -> u32 { 0 }

        unsafe extern "C" fn bind_host(h: *mut c_void, cb: *const NveHostCallbacks) {
            let app = unsafe { &mut *(h as *mut AsyncCdylib) };
            if cb.is_null() {
                return;
            }
            app.bind_calls += 1;
            let c = unsafe { *cb };
            if let Some(invoke) = c.invoke {
                let mut out_len = 0usize;
                // Always starts Pending call with caller request_id = 1 (simulates per-dylib static reset).
                let _ = unsafe {
                    invoke(
                        c.host_ctx,
                        0x0010,
                        1,
                        1,
                        NveBufferSlice { ptr: std::ptr::null(), len: 0 },
                        std::ptr::null_mut(),
                        0,
                        &mut out_len,
                    )
                };
            }
        }

        unsafe extern "C" fn dispatch_host_events(h: *mut c_void, payload: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut AsyncCdylib) };
            if payload.len < 8 || payload.ptr.is_null() {
                return 0;
            }
            let bytes = unsafe { std::slice::from_raw_parts(payload.ptr, payload.len) };
            if &bytes[0..4] != b"NVHB" {
                return 0;
            }
            let count = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
            let mut off = 8usize;
            let mut handled = 0u32;
            for _ in 0..count {
                if off + 32 > bytes.len() { break; }
                let kind = bytes[off];
                let status = i32::from_le_bytes([bytes[off + 4], bytes[off + 5], bytes[off + 6], bytes[off + 7]]);
                let plen = u32::from_le_bytes([bytes[off + 24], bytes[off + 25], bytes[off + 26], bytes[off + 27]]) as usize;
                let pstart = off + 32;
                let pend = pstart + plen;
                if pend > bytes.len() { break; }
                if kind == 1 {
                    app.resp_count += 1;
                    if status == -6 {
                        app.cancelled_count += 1;
                    } else if status == 0 {
                        app.last_payload = String::from_utf8_lossy(&bytes[pstart..pend]).into_owned();
                    }
                    handled = 1;
                }
                off = (pend + 7) & !7;
            }
            handled
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
    "#;
    let dylib = compile_test_cdylib(dir.path(), "skia_async_reload_app", cdylib_src);

    struct CapturePendingHandler {
        ids: std::sync::Mutex<Vec<u64>>,
    }
    impl HostServiceHandler for CapturePendingHandler {
        fn service_id(&self) -> u32 {
            SERVICE_CLIPBOARD
        }
        fn handle(
            &self,
            _: u32,
            request_id: u64,
            _: &[u8],
            _: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            self.ids.lock().unwrap().push(request_id);
            Ok(HandlerReply::Pending)
        }
    }

    let handler = Arc::new(CapturePendingHandler {
        ids: std::sync::Mutex::new(Vec::new()),
    });
    let reg = Arc::new(HostServiceRegistry::new().with_handler_arc(handler.clone()));
    let svc = crate::host_services::SkiaHostServices::with_provider(
        Arc::clone(&reg) as Arc<dyn HostServiceProvider>
    );
    let wakes = Arc::new(AtomicUsize::new(0));
    let wakes_c = Arc::clone(&wakes);
    svc.attach_wake_sink(Arc::new(move || {
        wakes_c.fetch_add(1, Ordering::SeqCst);
    }));

    // 1. Primary + secondary slots share one SkiaHostServices; both initiate request_id=1 on bind.
    let mut primary = HotAppSlot::load(&dylib, SurfaceId::ROOT).expect("load primary");
    assert!(primary.bind_host_services(Some(Arc::clone(&svc))));
    let mut secondary = HotAppSlot::load(&dylib, SurfaceId::new(2, 0)).expect("load secondary");
    assert!(secondary.bind_host_services(Some(Arc::clone(&svc))));
    assert_eq!(reg.completion_sink().in_flight_count(), 2);
    let (primary_host_id, secondary_host_id) = {
        let ids = handler.ids.lock().unwrap();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        (ids[0], ids[1])
    };

    // Dropping secondary discards ONLY secondary's pending request; primary stays alive!
    drop(secondary);
    assert_eq!(reg.completion_sink().in_flight_count(), 1);
    assert!(!reg.completion_sink().complete_ok(secondary_host_id, b"stale-sec".to_vec()));

    // Failed transactional reload leaves Gen 1 alive with its in-flight request completely intact!
    assert!(primary.reload(&dir.path().join("missing.dylib"), SurfaceId::ROOT).is_err());
    assert_eq!(reg.completion_sink().in_flight_count(), 1);

    // Complete primary's pending request -> wake fires -> DesktopWindowHost drains and delivers!
    assert!(reg.completion_sink().complete_ok(primary_host_id, b"gen1-ok".to_vec()));
    assert!(wakes.load(Ordering::SeqCst) >= 1);
    let session = DesktopSession::new(
        primary,
        SurfaceId::ROOT,
        Viewport { width: 320.0, height: 240.0, scale_factor: 1.0 },
        ColorRgba::BLACK,
        false,
    );
    let mut host = DesktopWindowHost::from_primary_session_with_host_services(
        DesktopPreviewOptions::default(),
        session,
        Arc::clone(&svc),
    );
    assert!(host.drain_host_service_batches());
    assert_eq!(
        host.session_mut().app_mut().render(FrameId(1)),
        b"binds=1;resps=1;cancels=0;last=gen1-ok"
    );

    // 2. Successful Tier-2 reload: bind_host called ONCE on Gen 2 (binds=1), Gen 1 pending
    //    discarded silently (no Cancelled delivered to Gen 2), and Gen 2's own Pending succeeds!
    assert!(host.session_mut().app_mut().bind_host_services(Some(Arc::clone(&svc))));
    let gen1_rebound_id = *handler.ids.lock().unwrap().last().unwrap();
    assert_eq!(reg.completion_sink().in_flight_count(), 1);

    let gen2 = host.session_mut().app_mut().reload(&dylib, SurfaceId::ROOT).expect("reload gen 2");
    assert_eq!(gen2, 2);
    // Gen 1's old request was discarded without queuing Cancelled; only Gen 2's new request lives.
    assert_eq!(reg.completion_sink().in_flight_count(), 1);
    assert_eq!(reg.completion_sink().queued_count(), 0);
    assert!(!reg.completion_sink().complete_ok(gen1_rebound_id, b"late-gen1".to_vec()));

    let gen2_host_id = *handler.ids.lock().unwrap().last().unwrap();
    assert_ne!(gen2_host_id, gen1_rebound_id);
    assert!(reg.completion_sink().complete_ok(gen2_host_id, b"gen2-clean".to_vec()));
    let mut b = NveHostEventBatchBuilder::new();
    let owner = host.session().host_owner_tag();
    assert_eq!(svc.drain_batch_for_owner_into(owner, &mut b), 1);
    assert!(host.session_mut().dispatch_host_batch(b.as_bytes()));
    assert_eq!(
        host.session_mut().app_mut().render(FrameId(2)),
        b"binds=1;resps=1;cancels=0;last=gen2-clean"
    );
}

#[test]
fn hot_app_reload_unbinds_before_destroy_snapshots_once_and_multi_window_broadcasts_reach_all() {
    use nve_abi::{
        NveHostEventBatchBuilder, NveHostEventBatchRef, NveHostEventKind, SERVICE_CLIPBOARD,
        SERVICE_SYSTEM_ENV,
    };
    use nve_host_services::{
        CompletionSink, HandlerReply, HostServiceHandler, HostServiceProvider, HostServiceRegistry,
        ServiceError,
    };

    // Test cdylib verifying:
    //  1. `destroy` ALWAYS observes `cb.is_none()` (unbind precedes destroy on reload AND Drop).
    //  2. `HotAppSlot::reload` invokes `snapshot_state` EXACTLY ONCE (no double snapshot).
    let dir = TempDirGuard::new("skia-unbind-snapshot");
    let marker_file = dir.path().join("lifecycle.log");
    let cdylib_src = format!(
        r#"
        use std::ffi::c_void;
        use std::io::Write;

        const LOG_PATH: &str = {log_path:?};

        fn append_log(line: &str) {{
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_PATH) {{
                let _ = writeln!(f, "{{line}}");
            }}
        }}

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveBufferSlice {{ pub ptr: *const u8, pub len: usize }}

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveHostCallbacks {{
            pub host_ctx: *mut c_void,
            pub invoke: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u64, NveBufferSlice, *mut u8, usize, *mut usize) -> i32>,
            pub channel_publish: Option<unsafe extern "C" fn(*mut c_void, u32, u32, NveBufferSlice) -> i32>,
        }}

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveEventBatch {{
            pub events: *const c_void,
            pub bytes: *const u8,
            pub event_count: u32,
            pub bytes_len: u32,
        }}

        #[repr(C)]
        pub struct NveSurfaceApi {{
            pub create: unsafe extern "C" fn(u32) -> *mut c_void,
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
            pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveEventBatch) -> u32,
        }}

        #[repr(C)]
        pub struct NveDevApi {{
            pub apply_hot_patch: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub snapshot_state: unsafe extern "C" fn(*mut c_void) -> NveBufferSlice,
            pub restore_state: unsafe extern "C" fn(*mut c_void, NveBufferSlice),
        }}

        #[repr(C)]
        pub struct NveHostApi {{
            pub bind_host: unsafe extern "C" fn(*mut c_void, *const NveHostCallbacks),
            pub dispatch_host_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
        }}

        struct ProbeCdylib {{
            cb: Option<NveHostCallbacks>,
            state_buf: Vec<u8>,
        }}

        unsafe extern "C" fn create(_: u32) -> *mut c_void {{
            Box::into_raw(Box::new(ProbeCdylib {{ cb: None, state_buf: vec![1, 2, 3, 4] }})) as *mut c_void
        }}

        unsafe extern "C" fn destroy(h: *mut c_void) {{
            if !h.is_null() {{
                let app = unsafe {{ Box::from_raw(h as *mut ProbeCdylib) }};
                append_log(if app.cb.is_none() {{ "destroy:unbound" }} else {{ "destroy:BOUND_BUG" }});
            }}
        }}

        unsafe extern "C" fn render(_: *mut c_void, _: u64) -> NveBufferSlice {{
            NveBufferSlice {{ ptr: std::ptr::null(), len: 0 }}
        }}
        unsafe extern "C" fn dispatch_events(_: *mut c_void, _: NveEventBatch) -> u32 {{ 0 }}
        unsafe extern "C" fn apply_hot_patch(_: *mut c_void, _: NveBufferSlice) -> u32 {{ 0 }}
        unsafe extern "C" fn snapshot_state(h: *mut c_void) -> NveBufferSlice {{
            let app = unsafe {{ &mut *(h as *mut ProbeCdylib) }};
            append_log("snapshot_state");
            NveBufferSlice {{ ptr: app.state_buf.as_ptr(), len: app.state_buf.len() }}
        }}
        unsafe extern "C" fn restore_state(_: *mut c_void, _: NveBufferSlice) {{}}
        unsafe extern "C" fn bind_host(h: *mut c_void, cb: *const NveHostCallbacks) {{
            let app = unsafe {{ &mut *(h as *mut ProbeCdylib) }};
            app.cb = if cb.is_null() {{ None }} else {{ Some(unsafe {{ *cb }}) }};
        }}
        unsafe extern "C" fn dispatch_host_events(_: *mut c_void, _: NveBufferSlice) -> u32 {{ 0 }}

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi {{ create, destroy, render, dispatch_events }};
        static DEV_API: NveDevApi = NveDevApi {{ apply_hot_patch, snapshot_state, restore_state }};
        static HOST_API: NveHostApi = NveHostApi {{ bind_host, dispatch_host_events }};

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {{
            match id {{
                1 => &SURFACE_API as *const NveSurfaceApi as *const c_void,
                4 => &DEV_API as *const NveDevApi as *const c_void,
                5 => &HOST_API as *const NveHostApi as *const c_void,
                _ => std::ptr::null(),
            }}
        }}
        "#,
        log_path = marker_file.to_string_lossy()
    );
    let dylib = compile_test_cdylib(dir.path(), "skia_unbind_snapshot_probe", &cdylib_src);
    let (_mem, services) = make_memory_services();
    let mut slot = HotAppSlot::load(&dylib, SurfaceId::ROOT).expect("load probe");
    assert!(slot.bind_host_services(Some(Arc::clone(&services))));
    let gen2 = slot.reload(&dylib, SurfaceId::ROOT).expect("reload probe");
    assert_eq!(gen2, 2);
    drop(slot);

    let log = std::fs::read_to_string(&marker_file).expect("read lifecycle log");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines.iter().filter(|&&l| l == "snapshot_state").count(),
        1,
        "HotAppSlot::reload must call snapshot_state exactly once, got log: {lines:?}"
    );
    assert!(
        !lines.contains(&"destroy:BOUND_BUG"),
        "HotAppInstance must call bind_host(None) BEFORE destroy on reload & drop, got: {lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|&&l| l == "destroy:unbound").count(),
        2,
        "both Gen 1 reload drop and Gen 2 final drop must observe unbound host"
    );

    // Multi-window SkiaHostServices broadcast vs owner-scoped drain isolation (owner_tag == 0
    // never steals nonzero owners' Responses; broadcasts drain separately for all windows).
    struct SimplePending;
    impl HostServiceHandler for SimplePending {
        fn service_id(&self) -> u32 {
            SERVICE_CLIPBOARD
        }
        fn handle(
            &self,
            _: u32,
            _: u64,
            _: &[u8],
            _: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            Ok(HandlerReply::Pending)
        }
    }
    let reg = Arc::new(HostServiceRegistry::new().with_handler(SimplePending));
    let svc = crate::host_services::SkiaHostServices::with_provider(
        Arc::clone(&reg) as Arc<dyn HostServiceProvider>,
    );
    let sink = reg.completion_sink();
    let h0 = sink.register_in_flight_for_owner(0, 1, SERVICE_CLIPBOARD, 1).unwrap();
    let h1 = sink.register_in_flight_for_owner(10, 1, SERVICE_CLIPBOARD, 1).unwrap();
    let h2 = sink.register_in_flight_for_owner(20, 1, SERVICE_CLIPBOARD, 1).unwrap();
    sink.push_event(SERVICE_SYSTEM_ENV, 1, b"dark".to_vec());
    sink.push_channel(3, false, b"msg".to_vec());
    assert!(sink.complete_ok(h0, b"w0".to_vec()));
    assert!(sink.complete_ok(h1, b"w1".to_vec()));
    assert!(sink.complete_ok(h2, b"w2".to_vec()));

    let mut bcast = NveHostEventBatchBuilder::new();
    assert_eq!(svc.drain_broadcasts_into(&mut bcast), 2);
    let bc_recs: Vec<_> = NveHostEventBatchRef::parse(bcast.as_bytes()).unwrap().iter().collect();
    assert_eq!(bc_recs[0].kind, NveHostEventKind::Push);
    assert_eq!(bc_recs[1].kind, NveHostEventKind::Channel);

    let mut w0_b = NveHostEventBatchBuilder::new();
    let mut w1_b = NveHostEventBatchBuilder::new();
    let mut w2_b = NveHostEventBatchBuilder::new();
    assert_eq!(svc.drain_batch_for_owner_into(0, &mut w0_b), 1);
    assert_eq!(svc.drain_batch_for_owner_into(10, &mut w1_b), 1);
    assert_eq!(svc.drain_batch_for_owner_into(20, &mut w2_b), 1);
    assert_eq!(NveHostEventBatchRef::parse(w0_b.as_bytes()).unwrap().iter().next().unwrap().payload, b"w0");
    assert_eq!(NveHostEventBatchRef::parse(w1_b.as_bytes()).unwrap().iter().next().unwrap().payload, b"w1");
    assert_eq!(NveHostEventBatchRef::parse(w2_b.as_bytes()).unwrap().iter().next().unwrap().payload, b"w2");
}
