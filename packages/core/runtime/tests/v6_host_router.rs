//! Runtime ABI v6 `RuntimeHostRouter` contract tests:
//! - Cross-thread `complete_host_request` / `push_host_event` / `publish_host_channel`
//! - Central `SERVICE_WINDOW::METHOD_WINDOW_REQUEST_FRAME -> WakeBridge`
//! - Staged `cdylib` reload (`load_app_module`) `bind_host(None)`-before-destroy + stale
//!   `owner_tag` drop isolation so Gen 2 never receives Gen 1's late completions.

#[path = "../../engine/tests/loader_common/mod.rs"]
mod loader_common;

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use loader_common::{compile_test_cdylib, dylib_ext, SHARED_ABI_DEFS};
use nve_abi::{
    HostRequestId, HostServiceId, NveBufferSlice, NveHostCallbacks, NveHostEventBatchRef,
    NveHostStatus, NveRuntimeApi, NveRuntimeConfig, NveRuntimeHandle, NveRuntimeStatus,
    NveStrSlice, NVE_CHANNEL_FLAG_RETAIN, NVE_RUNTIME_ABI_VERSION, SERVICE_WINDOW,
};
use nve_runtime::nve_runtime_get_interface;

const OK: i32 = NveRuntimeStatus::Ok as i32;

fn get_api() -> NveRuntimeApi {
    let ptr = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
    assert!(!ptr.is_null());
    // SAFETY: Static vtable for matching Runtime ABI v6.
    unsafe { *ptr }
}

fn str_slice(path: &Path) -> NveStrSlice {
    NveStrSlice::from_str(path.to_str().expect("UTF-8 test path"))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("nve-v6-host-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const HOST_CDYLIB_BODY: &str = r#"
const SERVICE_TEST: u32 = 0x1001;
const METHOD_ASYNC: u32 = 7;
const METHOD_UNBIND_MARKER: u32 = 99;

struct App {
    host: Option<NveHostCallbacks>,
    unbound_before_destroy: bool,
    log: Vec<u8>,
}

unsafe extern "C" fn create(_: u32) -> *mut c_void {
    Box::into_raw(Box::new(App { host: None, unbound_before_destroy: false, log: Vec::new() })).cast()
}

unsafe extern "C" fn destroy(h: *mut c_void) {
    let app = unsafe { Box::from_raw(h.cast::<App>()) };
    assert!(app.unbound_before_destroy, "bind_host(None) must run before destroy");
}

unsafe extern "C" fn render(_: *mut c_void, _: u64) -> NveBufferSlice {
    NveBufferSlice { ptr: std::ptr::null(), len: 0 }
}

unsafe extern "C" fn dispatch_events(_: *mut c_void, _: NveEventBatch) -> u32 { 0 }

unsafe extern "C" fn bind_host(h: *mut c_void, cb: *const NveHostCallbacks) {
    let app = unsafe { &mut *h.cast::<App>() };
    if cb.is_null() {
        if let Some(prev) = app.host.take()
            && let Some(invoke) = prev.invoke
        {
            let mut out_len = 0usize;
            let _ = unsafe {
                invoke(
                    prev.host_ctx,
                    SERVICE_TEST,
                    METHOD_UNBIND_MARKER,
                    99,
                    NveBufferSlice { ptr: std::ptr::null(), len: 0 },
                    std::ptr::null_mut(),
                    0,
                    &mut out_len,
                )
            };
        }
        app.unbound_before_destroy = true;
        return;
    }
    let callbacks = unsafe { *cb };
    app.host = Some(callbacks);
    if let Some(invoke) = callbacks.invoke {
        let mut out_len = 0usize;
        let _ = unsafe {
            invoke(
                callbacks.host_ctx,
                SERVICE_WINDOW,
                METHOD_WINDOW_REQUEST_FRAME,
                0,
                NveBufferSlice { ptr: std::ptr::null(), len: 0 },
                std::ptr::null_mut(),
                0,
                &mut out_len,
            )
        };
        let _ = unsafe {
            invoke(
                callbacks.host_ctx,
                SERVICE_TEST,
                METHOD_ASYNC,
                1,
                NveBufferSlice { ptr: GEN_TAG.as_ptr(), len: GEN_TAG.len() },
                std::ptr::null_mut(),
                0,
                &mut out_len,
            )
        };
    }
}

unsafe extern "C" fn dispatch_host_events(h: *mut c_void, batch: NveBufferSlice) -> u32 {
    let app = unsafe { &mut *h.cast::<App>() };
    if batch.len == 0 || batch.ptr.is_null() {
        return 0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(batch.ptr, batch.len) };
    app.log.extend_from_slice(GEN_TAG);
    app.log.push(b'|');
    app.log.extend_from_slice(bytes);
    1
}

unsafe extern "C" fn apply_hot_patch(_: *mut c_void, _: NveBufferSlice) -> u32 { 0 }
unsafe extern "C" fn snapshot_state(h: *mut c_void) -> NveBufferSlice {
    let app = unsafe { &*h.cast::<App>() };
    NveBufferSlice { ptr: app.log.as_ptr(), len: app.log.len() }
}
unsafe extern "C" fn restore_state(_: *mut c_void, _: NveBufferSlice) {}

static SURFACE: NveSurfaceApi = NveSurfaceApi { create, destroy, render, dispatch_events };
static DEV: NveDevApi = NveDevApi { apply_hot_patch, snapshot_state, restore_state };
static HOST: NveHostApi = NveHostApi { bind_host, dispatch_host_events };

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
    match id {
        1 => (&raw const SURFACE).cast(),
        4 => (&raw const DEV).cast(),
        5 => (&raw const HOST).cast(),
        _ => std::ptr::null(),
    }
}
"#;

struct HostCdylibFixtures {
    gen1: PathBuf,
    gen2: PathBuf,
}

fn host_fixtures() -> &'static HostCdylibFixtures {
    static FIX: OnceLock<HostCdylibFixtures> = OnceLock::new();
    FIX.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nve-runtime-v6-host-cdylib");
        std::fs::create_dir_all(&dir).expect("host fixture dir");
        let build = |name: &str, tag: &str| {
            let src = format!(
                "{SHARED_ABI_DEFS}\nconst SERVICE_WINDOW: u32 = {};\nconst METHOD_WINDOW_REQUEST_FRAME: u32 = {};\nconst GEN_TAG: &[u8] = b\"{tag}\";\n{HOST_CDYLIB_BODY}",
                nve_abi::SERVICE_WINDOW,
                nve_abi::METHOD_WINDOW_REQUEST_FRAME,
            );
            compile_test_cdylib(&dir, name, &src)
        };
        HostCdylibFixtures {
            gen1: build("nve_host_gen1", "GEN1"),
            gen2: build("nve_host_gen2", "GEN2"),
        }
    })
}

#[test]
fn host_router_cross_thread_ingress_request_frame_and_cdylib_reload_owner_tag_isolation() {
    struct HostProbe {
        wakes: AtomicUsize,
        window_invoke_hits: AtomicUsize,
        unbind_marker_hits: AtomicUsize,
        captured: Mutex<Vec<(String, u64)>>,
    }
    let probe = Arc::new(HostProbe {
        wakes: AtomicUsize::new(0),
        window_invoke_hits: AtomicUsize::new(0),
        unbind_marker_hits: AtomicUsize::new(0),
        captured: Mutex::new(Vec::new()),
    });

    unsafe extern "C" fn on_wake(ctx: *mut c_void) {
        // SAFETY: `ctx` points to `HostProbe` outliving the runtime.
        unsafe { (*ctx.cast::<HostProbe>()).wakes.fetch_add(1, Ordering::Release) };
    }
    unsafe extern "C" fn on_invoke(
        ctx: *mut c_void,
        service_id: u32,
        method_id: u32,
        host_req_id: u64,
        request: NveBufferSlice,
        _out_buf: *mut u8,
        _out_cap: usize,
        out_len: *mut usize,
    ) -> i32 {
        // SAFETY: `ctx` points to `HostProbe` outliving the runtime.
        let p = unsafe { &*ctx.cast::<HostProbe>() };
        if !out_len.is_null() {
            // SAFETY: Caller supplied writable `out_len`.
            unsafe { *out_len = 0 };
        }
        if service_id == SERVICE_WINDOW {
            p.window_invoke_hits.fetch_add(1, Ordering::Release);
            return NveHostStatus::Ok.as_i32();
        }
        if method_id == 99 {
            p.unbind_marker_hits.fetch_add(1, Ordering::Release);
            return NveHostStatus::Ok.as_i32();
        }
        // SAFETY: Valid slice for sync invoke call.
        let tag = String::from_utf8_lossy(unsafe { request.as_slice() }).into_owned();
        p.captured.lock().expect("captured lock").push((tag, host_req_id));
        NveHostStatus::Pending.as_i32()
    }

    let fx = host_fixtures();
    let dir = Scratch::new("reload");
    let module_path = dir.0.join(format!("libhost_app.{}", dylib_ext()));
    let api = get_api();
    let cfg = NveRuntimeConfig {
        surface_id: 1,
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Stack config outlives `create`.
    let h = unsafe { (api.create)(&cfg) };
    assert!(!h.is_null());
    let probe_ptr = Arc::as_ptr(&probe).cast_mut().cast::<c_void>();

    // SAFETY: Owner thread, `probe` outlives runtime destruction.
    unsafe {
        assert_eq!((api.set_wake_callback)(h, Some(on_wake), probe_ptr), OK);
        let cbs = NveHostCallbacks {
            host_ctx: probe_ptr,
            invoke: Some(on_invoke),
            channel_publish: None,
        };
        assert_eq!((api.set_host_callbacks)(h, &cbs), OK);

        // 1. Load Gen 1 -> triggers `METHOD_WINDOW_REQUEST_FRAME` centrally (wakes host, 0 hits
        //    on host `invoke`) and starts Pending async call (`caller_req_id = 1`).
        std::fs::copy(&fx.gen1, &module_path).expect("copy gen1 cdylib");
        assert_eq!((api.load_app_module)(h, str_slice(&module_path), 0), OK);
        assert!(probe.wakes.load(Ordering::Acquire) >= 1);
        assert_eq!(probe.window_invoke_hits.load(Ordering::Acquire), 0);

        // 2. Rebuild in place to Gen 2 and reload via `load_app_module`.
        //    Gen 1's `bind_host(None)` runs BEFORE `destroy` (asserted inside cdylib `destroy`),
        //    and its deactivated binding blocks calls (0 `unbind_marker_hits`).
        std::fs::copy(&fx.gen2, &module_path).expect("copy gen2 cdylib");
        assert_eq!((api.load_app_module)(h, str_slice(&module_path), 0), OK);
        assert_eq!(probe.unbind_marker_hits.load(Ordering::Acquire), 0);

        let captured = probe.captured.lock().expect("captured lock").clone();
        assert_eq!(captured.len(), 2, "expected Gen 1 & Gen 2 Pending requests: {captured:?}");
        assert_eq!(captured[0].0, "GEN1");
        assert_eq!(captured[1].0, "GEN2");
        let gen1_req = HostRequestId(captured[0].1);
        let gen2_req = HostRequestId(captured[1].1);
        assert_ne!(gen1_req, gen2_req);

        // 3. Complete Gen 1 (stale!), complete Gen 2 (fresh!), push event, and publish channel
        //    from a BACKGROUND worker thread!
        let h_addr = h as usize;
        std::thread::spawn(move || {
            let h_bg = h_addr as NveRuntimeHandle;
            let api_bg = get_api();
            assert_eq!(
                (api_bg.complete_host_request)(
                    h_bg,
                    gen1_req,
                    NveHostStatus::Ok.as_i32(),
                    NveBufferSlice::from_slice(b"STALE-GEN1"),
                ),
                OK
            );
            assert_eq!(
                (api_bg.complete_host_request)(
                    h_bg,
                    gen2_req,
                    NveHostStatus::Ok.as_i32(),
                    NveBufferSlice::from_slice(b"FRESH-GEN2"),
                ),
                OK
            );
            assert_eq!(
                (api_bg.push_host_event)(
                    h_bg,
                    HostServiceId(0x2002),
                    9,
                    NveBufferSlice::from_slice(b"PUSH-DATA"),
                ),
                OK
            );
            assert_eq!(
                (api_bg.publish_host_channel)(
                    h_bg,
                    77,
                    NVE_CHANNEL_FLAG_RETAIN,
                    NveBufferSlice::from_slice(b"CHAN-DATA"),
                ),
                OK
            );
        })
        .join()
        .expect("worker thread");

        // 4. Owner-thread tick drains canonical NVHB v1 batch into Gen 2.
        let mut changed = 0u32;
        assert_eq!((api.tick)(h, 0.016, &mut changed), OK);

        let mut snap = NveBufferSlice::EMPTY;
        assert_eq!((api.snapshot_state)(h, &mut snap), OK);
        let raw = snap.try_as_slice().unwrap_or(&[]);
        assert!(raw.starts_with(b"GEN2|"), "expected GEN2 log prefix");
        let nvhb = &raw[5..];
        assert!(
            !nvhb.windows(b"STALE-GEN1".len()).any(|w| w == b"STALE-GEN1"),
            "Gen 2 must NEVER receive Gen 1 stale response"
        );
        let view = NveHostEventBatchRef::parse(nvhb).expect("valid NVHB v1 batch");
        assert_eq!(view.count(), 3, "expected Response + Push + Channel");
        let records: Vec<_> = view.iter().collect();
        assert_eq!(records[0].request_id, HostRequestId(1));
        assert_eq!(records[0].payload, b"FRESH-GEN2");
        assert_eq!(records[1].service_id, HostServiceId(0x2002));
        assert_eq!(records[1].method_or_event_or_channel, 9);
        assert_eq!(records[1].payload, b"PUSH-DATA");
        assert_eq!(records[2].method_or_event_or_channel, 77);
        assert_eq!(records[2].payload, b"CHAN-DATA");

        (api.destroy)(h);
    }
}
