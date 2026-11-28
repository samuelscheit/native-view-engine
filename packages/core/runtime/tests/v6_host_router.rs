//! Runtime ABI v6 `RuntimeHostRouter` contract tests:
//! - Cross-thread `complete_host_request` / `push_host_event` / `publish_host_channel`
//! - Central `SERVICE_WINDOW::METHOD_WINDOW_REQUEST_FRAME -> WakeBridge`
//! - Staged `cdylib` reload (`load_app_module`) `bind_host(None)`-before-destroy + stale
//!   `owner_tag` drop isolation so Gen 2 never receives Gen 1's late completions.

#[path = "../../engine/tests/loader_common/mod.rs"]
mod loader_common;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, OnceLock};

use loader_common::{compile_test_cdylib, dylib_ext, SHARED_ABI_DEFS};
use nve_abi::{
    EventBatchBuf, FrameId, HostRequestId, HostServiceId, NodeId, NveBufferSlice,
    NveHostCallbacks, NveHostEventBatchRef, NveHostStatus, NveRuntimeApi, NveRuntimeConfig,
    NveRuntimeHandle, NveRuntimeStatus, NveSceneView, NveStrSlice, SurfaceId, TimestampUs,
    ValidatedEvents, METHOD_WINDOW_REQUEST_FRAME, NVE_CHANNEL_FLAG_RETAIN,
    NVE_RUNTIME_ABI_VERSION, SERVICE_WINDOW,
};
use nve_engine::{AppFrame, RuntimeApp, Viewport, WakeBridge};
use nve_host_services::{
    HostCallbackBridge, HostEvent, HostServiceProvider, InvokeResult, ResponseBuf, ServiceError,
};
use nve_runtime::{nve_runtime_get_interface, RuntimeBuilder, SurfaceRuntime};

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

struct SharedCallbackApp {
    slot: Arc<Mutex<Option<NveHostCallbacks>>>,
    received_batches: Arc<Mutex<Vec<Vec<u8>>>>,
    request_frame_on_render: Arc<AtomicBool>,
    request_frame_on_event: Arc<AtomicBool>,
    request_frame_on_bind: Arc<AtomicBool>,
    on_host_events: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl SharedCallbackApp {
    fn new(slot: Arc<Mutex<Option<NveHostCallbacks>>>) -> Self {
        Self {
            slot,
            received_batches: Arc::new(Mutex::new(Vec::new())),
            request_frame_on_render: Arc::new(AtomicBool::new(false)),
            request_frame_on_event: Arc::new(AtomicBool::new(false)),
            request_frame_on_bind: Arc::new(AtomicBool::new(false)),
            on_host_events: None,
        }
    }

    fn trigger_request_frame(&self) {
        if let Some(cb) = *self.slot.lock().expect("slot lock")
            && let Some(invoke) = cb.invoke
        {
            let mut out_len = 0usize;
            // SAFETY: Bound `cb` is active during owner-thread app callbacks.
            unsafe {
                let _ = invoke(
                    cb.host_ctx,
                    SERVICE_WINDOW,
                    METHOD_WINDOW_REQUEST_FRAME,
                    0,
                    NveBufferSlice::EMPTY,
                    std::ptr::null_mut(),
                    0,
                    &mut out_len,
                );
            }
        }
    }
}

impl RuntimeApp for SharedCallbackApp {
    fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
        if self.request_frame_on_render.swap(false, Ordering::AcqRel) {
            self.trigger_request_frame();
        }
        AppFrame::Clean
    }

    fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
        if self.request_frame_on_event.swap(false, Ordering::AcqRel) {
            self.trigger_request_frame();
            return true;
        }
        false
    }

    fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
        *self.slot.lock().expect("slot lock") = callbacks;
        if callbacks.is_some() && self.request_frame_on_bind.swap(false, Ordering::AcqRel) {
            self.trigger_request_frame();
        }
    }

    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        if !batch.is_empty() {
            self.received_batches
                .lock()
                .expect("batches lock")
                .push(batch.to_vec());
            if let Some(ref hook) = self.on_host_events {
                hook();
            }
        }
        false
    }
}

struct MockProvider {
    exec_count: AtomicUsize,
    channels: Mutex<Vec<(u32, bool, Vec<u8>)>>,
    pending_ids: Mutex<Vec<u64>>,
    panic_method: u32,
    reentrant_on_method: Option<(u32, Arc<dyn Fn() + Send + Sync>)>,
}

impl MockProvider {
    fn new() -> Self {
        Self {
            exec_count: AtomicUsize::new(0),
            channels: Mutex::new(Vec::new()),
            pending_ids: Mutex::new(Vec::new()),
            panic_method: 0,
            reentrant_on_method: None,
        }
    }
}

impl HostServiceProvider for MockProvider {
    fn invoke_raw_with_id(
        &self,
        _service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        if let Some((m, ref cb)) = self.reentrant_on_method
            && m == method_id
        {
            cb();
        }
        if self.panic_method != 0 && method_id == self.panic_method {
            #[expect(clippy::panic, reason = "synthetic provider panic test")]
            {
                panic!("synthetic provider panic");
            }
        }
        self.exec_count.fetch_add(1, Ordering::AcqRel);
        match method_id {
            1 => Ok(InvokeResult::Ready(ResponseBuf::from_slice_or_vec(request))),
            2 => {
                // Oversized 8 KiB response to trigger `BufferTooSmall` retry caching.
                let mut big = vec![0x5Au8; 8192];
                big[0] = request.first().copied().unwrap_or(0);
                Ok(InvokeResult::Ready(ResponseBuf::from_vec(big)))
            }
            3 => {
                self.pending_ids.lock().expect("pending lock").push(request_id);
                Ok(InvokeResult::Pending { request_id })
            }
            4 => Err(ServiceError::NotFound),
            _ => Err(ServiceError::Unsupported),
        }
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        self.channels
            .lock()
            .expect("channels lock")
            .push((channel_id, retain, payload.to_vec()));
        Ok(())
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        Vec::new()
    }
}

#[test]
fn host_router_concurrent_invoke_vs_detach_attach_callbacks_and_destroy_zero_uaf() {
    unsafe extern "C" fn fast_invoke(
        _ctx: *mut c_void,
        _service_id: u32,
        _method_id: u32,
        _host_req_id: u64,
        _request: NveBufferSlice,
        _out_buf: *mut u8,
        _out_cap: usize,
        out_len: *mut usize,
    ) -> i32 {
        if !out_len.is_null() {
            // SAFETY: Caller supplied writable `out_len`.
            unsafe { *out_len = 0 };
        }
        NveHostStatus::Ok.as_i32()
    }
    unsafe extern "C" fn fast_publish(
        _ctx: *mut c_void,
        _channel_id: u32,
        _flags: u32,
        _payload: NveBufferSlice,
    ) -> i32 {
        NveHostStatus::Ok.as_i32()
    }

    let slot = Arc::new(Mutex::new(None));
    let mut rt = RuntimeBuilder::new()
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .app(Box::new(SharedCallbackApp::new(Arc::clone(&slot))))
        .build();
    let host_cb = NveHostCallbacks {
        host_ctx: std::ptr::null_mut(),
        invoke: Some(fast_invoke),
        channel_publish: Some(fast_publish),
    };
    rt.set_host_callbacks(Some(host_cb)).expect("set callbacks");

    // Capture a stale copy of the first binding to keep hammering even after detach & destroy!
    let initial_cb = slot.lock().expect("slot lock").expect("bound callbacks");
    let stop = Arc::new(AtomicBool::new(false));
    let start_barrier = Arc::new(Barrier::new(5));
    let provider = Arc::new(MockProvider::new());

    let mut workers = Vec::new();
    for worker_id in 0..4usize {
        let slot_w = Arc::clone(&slot);
        let stop_w = Arc::clone(&stop);
        let barrier_w = Arc::clone(&start_barrier);
        workers.push(std::thread::spawn(move || {
            barrier_w.wait();
            let mut iters = 0usize;
            while !stop_w.load(Ordering::Acquire) || iters < 250 {
                let cb = (*slot_w.lock().expect("slot lock")).unwrap_or(initial_cb);
                let mut out_buf = [0u8; 32];
                let mut out_len = 0usize;
                if let Some(inv) = cb.invoke {
                    let req = [worker_id as u8, (iters & 0xFF) as u8];
                    // SAFETY: Generational `host_ctx` token is safe to call from any thread at any
                    // point, including during/after detach & runtime destroy.
                    unsafe {
                        let _ = inv(
                            cb.host_ctx,
                            0x3001,
                            1,
                            (iters as u64) + 1,
                            NveBufferSlice::from_slice(&req),
                            out_buf.as_mut_ptr(),
                            out_buf.len(),
                            &mut out_len,
                        );
                        let _ = inv(
                            cb.host_ctx,
                            SERVICE_WINDOW,
                            METHOD_WINDOW_REQUEST_FRAME,
                            0,
                            NveBufferSlice::EMPTY,
                            std::ptr::null_mut(),
                            0,
                            &mut out_len,
                        );
                    }
                }
                if let Some(pub_fn) = cb.channel_publish {
                    // SAFETY: Generational `host_ctx` token is safe concurrently with teardown.
                    unsafe {
                        let _ = pub_fn(cb.host_ctx, 5, 0, NveBufferSlice::from_slice(b"msg"));
                    }
                }
                iters += 1;
                if iters > 20_000 {
                    break;
                }
            }
        }));
    }

    start_barrier.wait();
    for round in 0..40 {
        match round % 4 {
            0 => rt.set_host_callbacks(Some(host_cb)).expect("set callbacks"),
            1 => rt
                .set_host_provider(Some(Arc::clone(&provider) as Arc<dyn HostServiceProvider>))
                .expect("set provider"),
            2 => rt.set_host_callbacks(None).expect("clear callbacks"),
            _ => {
                rt.detach_app().expect("detach");
                rt.attach_app(Box::new(SharedCallbackApp::new(Arc::clone(&slot))))
                    .expect("reattach");
            }
        }
        let _ = rt.tick(1.0 / 60.0).expect("tick");
    }

    // Destroy `rt` while worker threads are STILL hammering both live and stale bindings!
    drop(rt);
    stop.store(true, Ordering::Release);
    for w in workers {
        w.join().expect("worker must not panic or UAF");
    }

    // Post-destroy calls on the stale `initial_cb` MUST safely return `Cancelled`.
    let mut out_len = 0usize;
    // SAFETY: Stale generational token resolves to `None` -> `Cancelled` without dereferencing freed memory.
    let post_rc = unsafe {
        (initial_cb.invoke.expect("invoke"))(
            initial_cb.host_ctx,
            0x3001,
            1,
            1,
            NveBufferSlice::EMPTY,
            std::ptr::null_mut(),
            0,
            &mut out_len,
        )
    };
    assert_eq!(post_rc, NveHostStatus::Cancelled.as_i32());
}

#[test]
fn builder_unwind_in_install_wake_bridge_unbinds_before_drop_zero_uaf() {
    struct PanickingWakeApp {
        bound: Option<NveHostCallbacks>,
        unbound_in_drop: Arc<AtomicBool>,
        drop_invoke_status: Arc<Mutex<Option<i32>>>,
    }

    impl RuntimeApp for PanickingWakeApp {
        fn render_frame(&mut self, _frame_id: FrameId) -> AppFrame<'_> {
            AppFrame::Clean
        }
        fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
            false
        }
        fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
            if callbacks.is_none() {
                self.unbound_in_drop.store(true, Ordering::Release);
            } else {
                self.bound = callbacks;
            }
        }
        fn install_wake_bridge(&mut self, _wake: Arc<WakeBridge>) -> bool {
            panic!("synthetic install_wake_bridge panic");
        }
    }

    impl Drop for PanickingWakeApp {
        fn drop(&mut self) {
            if let Some(cb) = self.bound
                && let Some(inv) = cb.invoke
            {
                let mut out_len = 0usize;
                // SAFETY: `BuildAppGuard` deactivated and unbound `host_binding` before dropping `app`.
                let rc = unsafe {
                    inv(
                        cb.host_ctx,
                        0x3002,
                        1,
                        1,
                        NveBufferSlice::EMPTY,
                        std::ptr::null_mut(),
                        0,
                        &mut out_len,
                    )
                };
                *self.drop_invoke_status.lock().expect("status lock") = Some(rc);
            }
        }
    }

    let unbound = Arc::new(AtomicBool::new(false));
    let drop_rc = Arc::new(Mutex::new(None));
    let res = catch_unwind(AssertUnwindSafe(|| {
        let _rt = RuntimeBuilder::new()
            .app(Box::new(PanickingWakeApp {
                bound: None,
                unbound_in_drop: Arc::clone(&unbound),
                drop_invoke_status: Arc::clone(&drop_rc),
            }))
            .build();
    }));
    assert!(res.is_err(), "panic in install_wake_bridge must unwind");
    assert!(
        unbound.load(Ordering::Acquire),
        "BuildAppGuard must call bind_host(None) before dropping app on unwind"
    );
    assert_eq!(
        *drop_rc.lock().expect("status lock"),
        Some(NveHostStatus::Cancelled.as_i32()),
        "stale host_ctx invoke during app::drop must return Cancelled with zero UAF"
    );
}

#[test]
fn host_router_reentrant_set_callbacks_and_provider_no_deadlock() {
    struct ReentrantCtx {
        rt_ptr: *const SurfaceRuntime,
        reentered: AtomicBool,
    }
    // SAFETY: Accessed strictly on owner thread during this test.
    unsafe impl Send for ReentrantCtx {}
    // SAFETY: Accessed strictly on owner thread during this test.
    unsafe impl Sync for ReentrantCtx {}

    unsafe extern "C" fn reentrant_invoke(
        ctx: *mut c_void,
        _service_id: u32,
        _method_id: u32,
        _host_req_id: u64,
        _request: NveBufferSlice,
        _out_buf: *mut u8,
        _out_cap: usize,
        out_len: *mut usize,
    ) -> i32 {
        if !out_len.is_null() {
            // SAFETY: Valid writable pointer.
            unsafe { *out_len = 0 };
        }
        // SAFETY: `ctx` points to live `ReentrantCtx` on owner thread.
        let rctx = unsafe { &*ctx.cast::<ReentrantCtx>() };
        // SAFETY: `rt_ptr` points to live `SurfaceRuntime` on owner thread.
        let rt = unsafe { &*rctx.rt_ptr };
        // Re-entrantly clear callbacks & install a provider from INSIDE the C callback!
        rt.host_router().set_callbacks(None);
        rt.host_router().set_provider(Some(Arc::new(MockProvider::new())));
        rctx.reentered.store(true, Ordering::Release);
        NveHostStatus::Ok.as_i32()
    }

    let slot = Arc::new(Mutex::new(None));
    let mut rt = RuntimeBuilder::new()
        .app(Box::new(SharedCallbackApp::new(Arc::clone(&slot))))
        .build();
    let rctx = Box::new(ReentrantCtx {
        rt_ptr: &rt,
        reentered: AtomicBool::new(false),
    });
    rt.set_host_callbacks(Some(NveHostCallbacks {
        host_ctx: (&*rctx as *const ReentrantCtx).cast_mut().cast(),
        invoke: Some(reentrant_invoke),
        channel_publish: None,
    }))
    .expect("set callbacks");

    let cb = slot.lock().expect("slot lock").expect("bound callbacks");
    let mut out_len = 0usize;
    // SAFETY: Valid bound callbacks on owner thread.
    let rc = unsafe {
        (cb.invoke.expect("invoke"))(
            cb.host_ctx,
            0x3003,
            1,
            1,
            NveBufferSlice::EMPTY,
            std::ptr::null_mut(),
            0,
            &mut out_len,
        )
    };
    assert_eq!(rc, NveHostStatus::Ok.as_i32());
    assert!(rctx.reentered.load(Ordering::Acquire));

    // Also test re-entrant `set_provider(None)` from INSIDE `HostServiceProvider::invoke_raw_with_id`.
    let provider_reentered = Arc::new(AtomicBool::new(false));
    let pr_flag = Arc::clone(&provider_reentered);
    let router_addr = (rt.host_router() as *const nve_runtime::RuntimeHostRouter) as usize;
    let mut p = MockProvider::new();
    p.reentrant_on_method = Some((
        1,
        Arc::new(move || {
            // SAFETY: `rt` outlives this synchronous call on the owner thread.
            let router = unsafe { &*(router_addr as *const nve_runtime::RuntimeHostRouter) };
            router.set_provider(None);
            pr_flag.store(true, Ordering::Release);
        }),
    ));
    rt.set_host_provider(Some(Arc::new(p))).expect("set provider");
    // SAFETY: Valid bound callbacks on owner thread.
    let rc2 = unsafe {
        (cb.invoke.expect("invoke"))(
            cb.host_ctx,
            0x3003,
            1,
            2,
            NveBufferSlice::EMPTY,
            std::ptr::null_mut(),
            0,
            &mut out_len,
        )
    };
    assert_eq!(rc2, NveHostStatus::Ok.as_i32());
    assert!(provider_reentered.load(Ordering::Acquire));
}

#[test]
fn wake_deferral_proves_scene_acquire_succeeds_inside_wake_callback() {
    struct WakeProbe {
        api: NveRuntimeApi,
        handle: NveRuntimeHandle,
        wakes: AtomicUsize,
        busy_hits: AtomicUsize,
    }
    // SAFETY: `handle` is only probed via `scene_acquire`/`scene_release` on owner thread here.
    unsafe impl Send for WakeProbe {}
    // SAFETY: `handle` is only probed via `scene_acquire`/`scene_release` on owner thread here.
    unsafe impl Sync for WakeProbe {}

    unsafe extern "C" fn probing_wake(ctx: *mut c_void) {
        // SAFETY: `ctx` points to live `WakeProbe`.
        let p = unsafe { &*ctx.cast::<WakeProbe>() };
        p.wakes.fetch_add(1, Ordering::AcqRel);
        let mut view = NveSceneView::default();
        // SAFETY: Valid runtime handle; wake MUST run OUTSIDE `with_write` so `scene_acquire` returns OK!
        let rc = unsafe { (p.api.scene_acquire)(p.handle, &mut view) };
        if rc == NveRuntimeStatus::Busy as i32 {
            p.busy_hits.fetch_add(1, Ordering::AcqRel);
        } else if rc == OK {
            // SAFETY: Balanced with `scene_acquire`.
            unsafe {
                let _ = (p.api.scene_release)(p.handle);
            }
        }
    }

    let api = get_api();
    let cfg = NveRuntimeConfig {
        surface_id: 1,
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
        flags: 0,
        _reserved: 0,
    };
    // SAFETY: Valid stack config.
    let h = unsafe { (api.create)(&cfg) };
    assert!(!h.is_null());
    let probe = Box::new(WakeProbe {
        api,
        handle: h,
        wakes: AtomicUsize::new(0),
        busy_hits: AtomicUsize::new(0),
    });
    let probe_ptr = (&*probe as *const WakeProbe).cast_mut().cast::<c_void>();

    // SAFETY: Owner thread, `probe` outlives `(api.destroy)(h)`.
    unsafe {
        assert_eq!((api.set_wake_callback)(h, Some(probing_wake), probe_ptr), OK);
        let rt = &mut *(h.cast::<SurfaceRuntime>());
        let slot = Arc::new(Mutex::new(None));
        let mut app = SharedCallbackApp::new(Arc::clone(&slot));
        let on_render = Arc::clone(&app.request_frame_on_render);
        let on_event = Arc::clone(&app.request_frame_on_event);
        app.request_frame_on_bind.store(true, Ordering::Release);
        let h_addr = h as usize;
        app.on_host_events = Some(Arc::new(move || {
            // Synchronous push inside `dispatch_host_events` MUST defer wake until `with_write` drops!
            let _ = (api.push_host_event)(
                h_addr as NveRuntimeHandle,
                HostServiceId(0x4001),
                9,
                NveBufferSlice::from_slice(b"nested"),
            );
        }));

        // 1. Wake from `bind_host` during `attach_app`.
        rt.attach_app(Box::new(app)).expect("attach");
        assert!(probe.wakes.load(Ordering::Acquire) >= 1);
        assert_eq!(probe.busy_hits.load(Ordering::Acquire), 0);

        // 2. Wake from `render_frame` during `tick`.
        on_render.store(true, Ordering::Release);
        let mut changed = 0u32;
        assert_eq!((api.tick)(h, 1.0 / 60.0, &mut changed), OK);
        assert!(probe.wakes.load(Ordering::Acquire) >= 2);
        assert_eq!(probe.busy_hits.load(Ordering::Acquire), 0);

        // Consume Step 2's pending wake so Step 3's wake does not coalesce into it.
        assert_eq!((api.tick)(h, 1.0 / 60.0, &mut changed), OK);
        let before_dispatch_wakes = probe.wakes.load(Ordering::Acquire);

        // 3. Wake from `dispatch_events`.
        on_event.store(true, Ordering::Release);
        let mut ev = EventBatchBuf::new();
        ev.push_custom(SurfaceId::ROOT, NodeId::ROOT, TimestampUs(1), "ping", b"x");
        assert_eq!((api.dispatch_events)(h, ev.as_raw_batch(), &mut changed), OK);
        assert!(probe.wakes.load(Ordering::Acquire) > before_dispatch_wakes);
        assert_eq!(probe.busy_hits.load(Ordering::Acquire), 0);

        // 4. Wake from synchronous `push_host_event` inside `dispatch_host_events` during `tick`.
        assert_eq!(
            (api.push_host_event)(h, HostServiceId(0x4001), 1, NveBufferSlice::from_slice(b"go")),
            OK
        );
        let before_wakes = probe.wakes.load(Ordering::Acquire);
        assert_eq!((api.tick)(h, 1.0 / 60.0, &mut changed), OK);
        assert!(probe.wakes.load(Ordering::Acquire) > before_wakes);
        assert_eq!(
            probe.busy_hits.load(Ordering::Acquire),
            0,
            "deferred wake callbacks MUST NEVER observe SceneBusy"
        );

        assert_eq!((api.set_wake_callback)(h, None, std::ptr::null_mut()), OK);
        (api.destroy)(h);
    }
}

#[cfg(feature = "wire")]
#[test]
fn attach_app_replacing_pair_preserves_bind_host_request_frame_wake() {
    let mut rt = RuntimeBuilder::new().build();
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = Arc::clone(&wakes);
    rt.set_wake_callback(Arc::new(move || {
        w.fetch_add(1, Ordering::AcqRel);
    }));
    rt.pair_connect("ws://127.0.0.1:9/nve-wire", "tok")
        .expect("pair_connect");

    let slot = Arc::new(Mutex::new(None));
    let app = SharedCallbackApp::new(Arc::clone(&slot));
    app.request_frame_on_bind.store(true, Ordering::Release);
    let before = wakes.load(Ordering::Acquire);
    rt.attach_app(Box::new(app)).expect("attach over pair");
    assert!(
        rt.wake_bridge().is_pending(),
        "METHOD_WINDOW_REQUEST_FRAME emitted in bind_host must survive old_pair shutdown"
    );
    assert!(
        wakes.load(Ordering::Acquire) > before,
        "host wake callback must fire for bind_host request_frame after replacing pair"
    );
}

#[test]
fn host_router_provider_and_callbacks_retry_cache_single_execution_panic_and_channel_publish() {
    let slot = Arc::new(Mutex::new(None));
    let app = SharedCallbackApp::new(Arc::clone(&slot));
    let batches = Arc::clone(&app.received_batches);
    let mut rt = RuntimeBuilder::new().app(Box::new(app)).build();

    let cb = slot.lock().expect("slot lock").expect("bound callbacks");
    let invoke = cb.invoke.expect("invoke");
    let channel_publish = cb.channel_publish.expect("channel_publish");

    // 1. Malformed `(null, len>0)` slice rejected with `InvalidArg` on both entrypoints.
    let bad_slice = NveBufferSlice {
        ptr: std::ptr::null(),
        len: 8,
    };
    let mut out_len = 0usize;
    // SAFETY: Router validates `NveBufferSlice` before dereferencing.
    unsafe {
        assert_eq!(
            invoke(cb.host_ctx, 0x5001, 1, 1, bad_slice, std::ptr::null_mut(), 0, &mut out_len),
            NveHostStatus::InvalidArg.as_i32()
        );
        assert_eq!(
            channel_publish(cb.host_ctx, 1, 0, bad_slice),
            NveHostStatus::InvalidArg.as_i32()
        );
    }

    // 2. `HostBackend::Provider`: sync Ready, oversized BufferTooSmall single execution, async
    //    Pending, NotFound error, outbound channel_publish, and panic isolation (0 in-flight leak).
    let mut provider = MockProvider::new();
    provider.panic_method = 99;
    let provider = Arc::new(provider);
    rt.set_host_provider(Some(Arc::clone(&provider) as Arc<dyn HostServiceProvider>))
        .expect("set_host_provider");

    let mut small = [0u8; 16];
    // SAFETY: Valid buffers & active bound callbacks.
    unsafe {
        // Sync Ready
        assert_eq!(
            invoke(
                cb.host_ctx,
                0x5001,
                1,
                10,
                NveBufferSlice::from_slice(b"echo"),
                small.as_mut_ptr(),
                small.len(),
                &mut out_len,
            ),
            NveHostStatus::Ok.as_i32()
        );
        assert_eq!(&small[..out_len], b"echo");

        // Oversized BufferTooSmall -> retry executes handler EXACTLY ONCE!
        let before_execs = provider.exec_count.load(Ordering::Acquire);
        assert_eq!(
            invoke(
                cb.host_ctx,
                0x5001,
                2,
                11,
                NveBufferSlice::from_slice(&[0xAB]),
                small.as_mut_ptr(),
                small.len(),
                &mut out_len,
            ),
            NveHostStatus::BufferTooSmall.as_i32()
        );
        assert_eq!(out_len, 8192);
        let mut big = vec![0u8; out_len];
        assert_eq!(
            invoke(
                cb.host_ctx,
                0x5001,
                2,
                11,
                NveBufferSlice::from_slice(&[0xAB]),
                big.as_mut_ptr(),
                big.len(),
                &mut out_len,
            ),
            NveHostStatus::Ok.as_i32()
        );
        assert_eq!(out_len, 8192);
        assert_eq!(big[0], 0xAB);
        assert_eq!(
            provider.exec_count.load(Ordering::Acquire) - before_execs,
            1,
            "Provider oversized BufferTooSmall retry MUST execute handler exactly once"
        );

        // Async Pending + complete_host_request + tick drain
        assert_eq!(
            invoke(
                cb.host_ctx,
                0x5001,
                3,
                12,
                NveBufferSlice::EMPTY,
                std::ptr::null_mut(),
                0,
                &mut out_len,
            ),
            NveHostStatus::Pending.as_i32()
        );
        assert_eq!(rt.host_router().in_flight_count(), 1);
        let host_id = provider.pending_ids.lock().expect("pending lock").pop().expect("host id");
        assert!(rt
            .complete_host_request(HostRequestId(host_id), NveHostStatus::Ok, b"async-provider")
            .expect("complete"));
        let _ = rt.tick(1.0 / 60.0).expect("tick");
        assert_eq!(rt.host_router().in_flight_count(), 0);
        let b = batches.lock().expect("batches lock");
        let parsed = NveHostEventBatchRef::parse(b.last().expect("batch")).expect("valid NVHB");
        let rec = parsed.iter().next().expect("record");
        assert_eq!(rec.request_id, HostRequestId(12));
        assert_eq!(rec.payload, b"async-provider");
        drop(b);

        // Error mapping
        assert_eq!(
            invoke(cb.host_ctx, 0x5001, 4, 13, NveBufferSlice::EMPTY, std::ptr::null_mut(), 0, &mut out_len),
            NveHostStatus::NotFound.as_i32()
        );

        // Outbound channel_publish (both retain=false and retain=true)
        assert_eq!(
            channel_publish(cb.host_ctx, 7, 0, NveBufferSlice::from_slice(b"c0")),
            NveHostStatus::Ok.as_i32()
        );
        assert_eq!(
            channel_publish(cb.host_ctx, 8, NVE_CHANNEL_FLAG_RETAIN, NveBufferSlice::from_slice(b"c1")),
            NveHostStatus::Ok.as_i32()
        );
        assert_eq!(
            *provider.channels.lock().expect("channels lock"),
            vec![(7, false, b"c0".to_vec()), (8, true, b"c1".to_vec())]
        );

        // Panic inside Provider -> InternalError AND 0 in-flight slot leak!
        assert_eq!(
            invoke(cb.host_ctx, 0x5001, 99, 14, NveBufferSlice::EMPTY, std::ptr::null_mut(), 0, &mut out_len),
            NveHostStatus::InternalError.as_i32()
        );
        assert_eq!(
            rt.host_router().in_flight_count(),
            0,
            "panic inside Provider must release in-flight slot via InFlightGuard"
        );
    }

    // 3. `HostBackend::Callbacks` backed by real `HostCallbackBridge`: oversized BufferTooSmall
    //    retry MUST reuse `host_req_id` via `CallbackRetryIds` so bridge executes handler ONCE!
    let bridge = HostCallbackBridge::new(MockProvider::new());
    rt.set_host_callbacks(Some(bridge.callbacks()))
        .expect("set bridge callbacks");
    // SAFETY: `bridge` outlives `set_host_callbacks(None)` below.
    unsafe {
        let before_execs = bridge.provider().exec_count.load(Ordering::Acquire);
        assert_eq!(
            invoke(
                cb.host_ctx,
                0x5002,
                2,
                21,
                NveBufferSlice::from_slice(&[0xCD]),
                small.as_mut_ptr(),
                small.len(),
                &mut out_len,
            ),
            NveHostStatus::BufferTooSmall.as_i32()
        );
        assert_eq!(out_len, 8192);
        let mut big = vec![0u8; out_len];
        assert_eq!(
            invoke(
                cb.host_ctx,
                0x5002,
                2,
                21,
                NveBufferSlice::from_slice(&[0xCD]),
                big.as_mut_ptr(),
                big.len(),
                &mut out_len,
            ),
            NveHostStatus::Ok.as_i32()
        );
        assert_eq!(out_len, 8192);
        assert_eq!(big[0], 0xCD);
        assert_eq!(
            bridge.provider().exec_count.load(Ordering::Acquire) - before_execs,
            1,
            "Callbacks + HostCallbackBridge BufferTooSmall retry MUST execute handler exactly once"
        );

        // Outbound channel_publish through Callbacks backend
        assert_eq!(
            channel_publish(cb.host_ctx, 33, NVE_CHANNEL_FLAG_RETAIN, NveBufferSlice::from_slice(b"cb-ch")),
            NveHostStatus::Ok.as_i32()
        );
        assert_eq!(
            *bridge.provider().channels.lock().expect("channels lock"),
            vec![(33, true, b"cb-ch".to_vec())]
        );
    }

    // 4. Panic inside `HostCallbackBridge` (`Callbacks` backend) -> InternalError AND 0 in-flight
    //    slot leak!
    let mut panic_p = MockProvider::new();
    panic_p.panic_method = 99;
    let panic_bridge = HostCallbackBridge::new(panic_p);
    rt.set_host_callbacks(Some(panic_bridge.callbacks()))
        .expect("set panicking bridge callbacks");
    // SAFETY: `panic_bridge` outlives `set_host_callbacks(None)` below.
    unsafe {
        assert_eq!(
            invoke(cb.host_ctx, 0x5003, 99, 30, NveBufferSlice::EMPTY, std::ptr::null_mut(), 0, &mut out_len),
            NveHostStatus::InternalError.as_i32()
        );
    }
    assert_eq!(
        rt.host_router().in_flight_count(),
        0,
        "panic inside Callbacks backend must release in-flight slot"
    );
    rt.set_host_callbacks(None).expect("clear callbacks");
}

#[test]
fn host_router_batch_max_bytes_multi_tick_deferral() {
    let slot = Arc::new(Mutex::new(None));
    let app = SharedCallbackApp::new(Arc::clone(&slot));
    let batches = Arc::clone(&app.received_batches);
    let mut rt = RuntimeBuilder::new().app(Box::new(app)).build();

    // Push 5 × 1 MiB (= 5 MiB > 4 MiB `NVE_HOST_BATCH_MAX_BYTES`) so drain splits across >= 2 ticks.
    const PAYLOAD_LEN: usize = 1024 * 1024;
    for i in 0..5u32 {
        let payload = vec![(i as u8) + 1; PAYLOAD_LEN];
        rt.push_host_event(HostServiceId(0x6001), i, &payload)
            .expect("push large event");
    }

    let _ = rt.tick(1.0 / 60.0).expect("tick 1");
    let first_count: usize = batches
        .lock()
        .expect("batches lock")
        .iter()
        .map(|b| NveHostEventBatchRef::parse(b).expect("NVHB").count() as usize)
        .sum();
    assert!(
        (1..5).contains(&first_count),
        "tick 1 must partially drain capped at NVE_HOST_BATCH_MAX_BYTES, got {first_count}"
    );
    assert!(
        rt.wake_bridge().is_pending(),
        "partial drain (`more == true`) MUST schedule follow-up wake"
    );

    // Subsequent tick(s) drain remaining records completely in FIFO order.
    for _ in 0..4 {
        if !rt.wake_bridge().is_pending() {
            break;
        }
        let _ = rt.tick(1.0 / 60.0).expect("follow-up tick");
    }
    assert!(!rt.wake_bridge().is_pending(), "queue must be fully drained");

    let mut seen = Vec::new();
    for raw in batches.lock().expect("batches lock").iter() {
        let view = NveHostEventBatchRef::parse(raw).expect("valid NVHB");
        for rec in view.iter() {
            assert_eq!(rec.payload.len(), PAYLOAD_LEN);
            assert_eq!(rec.payload[0], (rec.method_or_event_or_channel as u8) + 1);
            seen.push(rec.method_or_event_or_channel);
        }
    }
    assert_eq!(seen, vec![0, 1, 2, 3, 4], "all deferred records delivered in FIFO order");
}
