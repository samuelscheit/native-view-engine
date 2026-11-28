//! End-to-end tests for [`DioxusRuntimeApp`] hosted inside [`nve_runtime::SurfaceRuntime`]:
//! - `set_props` (`HostProps` in `ScopeId::ROOT` context + dirty re-render)
//! - `drain_emitted_events` (`HostEventEmitter` custom outbound events)
//! - `supports_state` / `snapshot_state` / `restore_state` + `save_state_file` / `restore_state_file`
//! - `bind_host` / `dispatch_host_events` (`Arc<FfiHostServices>` sync + async `Pending` via
//!   `RuntimeHostRouter` + cross-thread `complete_host_request` + `push_host_event`)

#![cfg(not(target_arch = "wasm32"))]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dioxus_core::{Element, ScopeId};
use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_abi::{
    HostRequestId, HostServiceId, NodeId, NveBufferSlice, NveHostCallbacks, NveHostStatus,
    NveMapEntry, NvePropsRef, NveValue, SurfaceId,
};
use nve_dioxus::prelude::*;
use nve_dioxus::{DioxusRuntimeApp, DioxusSurface, HostEventEmitter, HostProps};
use nve_engine::{RuntimeApp, Viewport};
use nve_host_services::{FfiHostServices, HostPushEvent, ServiceError};
use nve_runtime::{RuntimeBuilder, SceneReadGuard};

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("nve-dioxus-v6-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_text_slab(guard: &SceneReadGuard<'_>) -> String {
    let view = guard.view();
    assert!(!view.text_slab.is_null());
    // SAFETY: `guard` holds the scene reader lock for `text_slab_len` bytes.
    let bytes =
        unsafe { std::slice::from_raw_parts(view.text_slab, view.text_slab_len as usize) };
    std::str::from_utf8(bytes).expect("UTF-8 text slab").to_owned()
}

#[derive(Clone)]
struct AppShared {
    persisted_count: Arc<Mutex<i32>>,
    async_reply: Arc<Mutex<String>>,
    pushed_events: Arc<Mutex<Vec<String>>>,
    started_async: Arc<std::sync::atomic::AtomicBool>,
}

fn full_v6_dioxus_app() -> Element {
    let props = consume_context::<HostProps>();
    let emitter = consume_context::<HostEventEmitter>();
    let host = consume_context::<Arc<FfiHostServices>>();
    let shared = consume_context::<AppShared>();

    let title = props.get_str("title").unwrap_or_else(|| "default".to_owned());
    let prop_num = props.get_i64("n").unwrap_or(0);
    let count = *shared.persisted_count.lock().expect("count lock");
    let reply = shared.async_reply.lock().expect("reply lock").clone();
    let pushes = shared.pushed_events.lock().expect("pushes lock").join(",");

    if prop_num == 7 {
        emitter.emit_custom(NodeId::ROOT, "on_seven", b"lucky");
    }

    if !shared.started_async.swap(true, Ordering::AcqRel) && host.is_bound() {
        let reply_slot = Arc::clone(&shared.async_reply);
        let _ = host.invoke_with_callback(0x5501, 3, b"ask", move |res| {
            let text = match res {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(err) => format!("err:{err:?}"),
            };
            *reply_slot.lock().expect("reply lock") = text;
        });
    }

    rsx! {
        view {
            style: "width: 300px; height: 100px; flex-direction: column;",
            text { "title={title} n={prop_num} count={count} reply={reply} push={pushes}" }
        }
    }
}

struct MockHostCtx {
    pending_req_id: AtomicU64,
}

unsafe extern "C" fn mock_host_invoke(
    ctx: *mut c_void,
    _service_id: u32,
    _method_id: u32,
    host_req_id: u64,
    _request: NveBufferSlice,
    _out_buf: *mut u8,
    _out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if !out_len.is_null() {
        // SAFETY: Caller passed writable `out_len`.
        unsafe { *out_len = 0 };
    }
    // SAFETY: `ctx` points to `MockHostCtx` alive until `set_host_callbacks(None)`.
    let state = unsafe { &*ctx.cast::<MockHostCtx>() };
    state.pending_req_id.store(host_req_id, Ordering::Release);
    NveHostStatus::Pending.as_i32()
}

#[test]
fn dioxus_runtime_app_props_events_state_files_and_host_services_end_to_end() {
    let dir = ScratchDir::new("e2e");
    let state_path = dir.0.join("dioxus.nvestate");
    let host_ctx = Arc::new(MockHostCtx {
        pending_req_id: AtomicU64::new(0),
    });

    let shared1 = AppShared {
        persisted_count: Arc::new(Mutex::new(42)),
        async_reply: Arc::new(Mutex::new("waiting".to_owned())),
        pushed_events: Arc::new(Mutex::new(Vec::new())),
        started_async: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let mut surface1 =
        DioxusSurface::new_with_context(SurfaceId::ROOT, full_v6_dioxus_app, shared1.clone());
    {
        let c_read = Arc::clone(&shared1.persisted_count);
        let c_write = Arc::clone(&shared1.persisted_count);
        surface1.signal_registry_mut().register_slot(
            "App",
            "count",
            move || *c_read.lock().expect("count lock"),
            move |v: i32| *c_write.lock().expect("count lock") = v,
        );
    }

    let app1 = DioxusRuntimeApp::new(surface1);
    let host_svc1 = Arc::clone(app1.host_services());
    let push_sink = Arc::clone(&shared1.pushed_events);
    let _sub = host_svc1.typed_events().subscribe::<nve_host_services::HostPushEvent, _>(
        false,
        move |ev| {
            if ev.service_id == 0x5502 && ev.event_id == 11 {
                push_sink
                    .lock()
                    .expect("push sink lock")
                    .push(String::from_utf8_lossy(&ev.payload).into_owned());
            }
        },
    );

    let mut rt = RuntimeBuilder::new()
        .surface_id(SurfaceId::ROOT)
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .build();
    rt.set_host_callbacks(Some(NveHostCallbacks {
        host_ctx: Arc::as_ptr(&host_ctx).cast_mut().cast(),
        invoke: Some(mock_host_invoke),
        channel_publish: None,
    }))
    .expect("set_host_callbacks");
    rt.attach_app(Box::new(app1)).expect("attach app1");

    // 1. Initial tick mounts app, fires async Pending host call via RuntimeHostRouter.
    assert!(rt.tick(1.0 / 60.0).expect("tick 1"));
    let req_id = host_ctx.pending_req_id.load(Ordering::Acquire);
    assert_ne!(req_id, 0, "FfiHostServices must have invoked Pending host request");
    {
        let g = rt.acquire_scene().expect("acquire 1");
        let text = read_text_slab(&g);
        assert!(text.contains("title=default n=0 count=42 reply=waiting"), "text=`{text}`");
    }

    // 2. `set_props` updates `HostProps` -> re-renders & emits custom outbound event `on_seven`.
    let prop_entries = [
        NveMapEntry::new("title", NveValue::from_str("hello")),
        NveMapEntry::new("n", NveValue::i64(7)),
    ];
    // SAFETY: `prop_entries` outlives `set_props`.
    let props_ref = unsafe { NvePropsRef::from_slice(&prop_entries) };
    assert!(rt.set_props(props_ref).expect("set_props"));
    // Setting identical props is a no-op (`false`).
    assert!(!rt.set_props(props_ref).expect("set_props unchanged"));
    assert!(rt.tick(1.0 / 60.0).expect("tick after set_props"));
    {
        let g = rt.acquire_scene().expect("acquire after props");
        let text = read_text_slab(&g);
        assert!(text.contains("title=hello n=7 count=42"), "text=`{text}`");
    }
    {
        let emitted = rt.drain_emitted_events().expect("drain emitted");
        assert_eq!(emitted.len(), 1);
        let ev = emitted.iter().next().expect("event 0");
        assert_eq!(ev.node(), NodeId::ROOT);
        let raw = emitted.bytes();
        assert!(raw.windows(b"lucky".len()).any(|w| w == b"lucky"));
    }
    assert!(rt.drain_emitted_events().expect("drain empty").is_empty());

    // 3. Cross-thread `complete_host_request` + `push_host_event` -> drained in `tick` -> re-renders!
    let router = rt.host_router();
    std::thread::scope(|s| {
        s.spawn(|| {
            assert!(router.complete_request(
                HostRequestId(req_id),
                NveHostStatus::Ok,
                b"pong-99"
            ));
            router.push_event(HostServiceId(0x5502), 11, b"alert-ok");
        });
    });
    assert!(rt.tick(1.0 / 60.0).expect("tick after host completion"));
    {
        let g = rt.acquire_scene().expect("acquire after host reply");
        let text = read_text_slab(&g);
        assert!(text.contains("reply=pong-99"), "text=`{text}`");
        assert!(text.contains("push=alert-ok"), "text=`{text}`");
    }

    // 4. Persist state to disk (`save_state_file`), attach a brand-new `DioxusRuntimeApp` with
    //    `count = 0`, and restore from disk (`restore_state_file`) -> recovers `count = 42`!
    rt.save_state_file(&state_path, 1).expect("save_state_file");
    assert!(state_path.exists());

    let shared2 = AppShared {
        persisted_count: Arc::new(Mutex::new(0)),
        async_reply: Arc::new(Mutex::new("fresh".to_owned())),
        pushed_events: Arc::new(Mutex::new(Vec::new())),
        started_async: Arc::new(std::sync::atomic::AtomicBool::new(true)),
    };
    let mut surface2 =
        DioxusSurface::new_with_context(SurfaceId::ROOT, full_v6_dioxus_app, shared2.clone());
    {
        let c_read = Arc::clone(&shared2.persisted_count);
        let c_write = Arc::clone(&shared2.persisted_count);
        surface2.signal_registry_mut().register_slot(
            "App",
            "count",
            move || *c_read.lock().expect("count lock"),
            move |v: i32| *c_write.lock().expect("count lock") = v,
        );
    }
    rt.attach_app(Box::new(DioxusRuntimeApp::new(surface2)))
        .expect("attach app2");
    rt.restore_state_file(&state_path, 1)
        .expect("restore_state_file");
    assert_eq!(*shared2.persisted_count.lock().expect("count lock"), 42);
    assert!(rt.tick(1.0 / 60.0).expect("tick app2"));
    {
        let g = rt.acquire_scene().expect("acquire app2");
        let text = read_text_slab(&g);
        assert!(text.contains("count=42"), "restored text=`{text}`");
    }

    rt.set_host_callbacks(None).expect("clear host callbacks");
}

#[derive(Clone)]
struct RealSignalCtx {
    persisted_count: Signal<i32>,
    cancel_hits: Arc<std::sync::atomic::AtomicUsize>,
}

fn real_signals_v6_app() -> Element {
    let ctx = consume_context::<RealSignalCtx>();
    let host = consume_context::<Arc<FfiHostServices>>();
    struct OwnerSignal<T: 'static>(Signal<T>);
    impl<T: 'static> OwnerSignal<T> {
        fn get(&self) -> Signal<T> {
            self.0
        }
    }
    // SAFETY: `DioxusRuntimeApp::dispatch_host_events` always invokes `TypedEventBus` subscribers
    // strictly on the owner thread inside `ScopeId::ROOT`.
    unsafe impl<T: 'static> Send for OwnerSignal<T> {}
    // SAFETY: Same owner-thread invariant as above.
    unsafe impl<T: 'static> Sync for OwnerSignal<T> {}

    let reply_sig = use_signal(|| "waiting".to_owned());
    let reply_owner = OwnerSignal(reply_sig);
    let push_sig = use_signal(|| "none".to_owned());
    let push_owner = OwnerSignal(push_sig);
    let cancel_sig = use_signal(|| 0i32);
    let cancel_owner = OwnerSignal(cancel_sig);

    let cancel_hits = Arc::clone(&ctx.cancel_hits);
    let _sub = dioxus_core::use_hook(move || {
        let sub = host
            .typed_events()
            .subscribe::<HostPushEvent, _>(false, move |ev| {
                if ev.service_id == 0x5512 && ev.event_id == 22 {
                    let mut sig = push_owner.get();
                    sig.set(String::from_utf8_lossy(&ev.payload).into_owned());
                }
            });
        if host.is_bound() {
            let _ = host.invoke_with_callback(0x5511, 1, b"ok-req", move |res| {
                let text = match res {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(err) => format!("err:{err:?}"),
                };
                let mut sig = reply_owner.get();
                sig.set(text);
            });
            let _ = host.invoke_with_callback(0x5511, 2, b"cancel-req", move |res| {
                if matches!(res, Err(ServiceError::Cancelled)) {
                    let mut sig = cancel_owner.get();
                    sig += 1;
                    cancel_hits.fetch_add(1, Ordering::Release);
                }
            });
        }
        sub
    });

    let count = *ctx.persisted_count.read();
    let reply = reply_sig.read().clone();
    let push = push_sig.read().clone();
    let cancelled = *cancel_sig.read();
    rsx! {
        view {
            style: "width: 320px; height: 100px; flex-direction: column;",
            text { "count={count} reply={reply} push={push} cancelled={cancelled}" }
        }
    }
}

struct MultiReqHostCtx {
    ok_req_id: AtomicU64,
    cancel_req_id: AtomicU64,
}

unsafe extern "C" fn multi_req_mock_invoke(
    ctx: *mut c_void,
    _service_id: u32,
    method_id: u32,
    host_req_id: u64,
    _request: NveBufferSlice,
    _out_buf: *mut u8,
    _out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if !out_len.is_null() {
        // SAFETY: Caller passed writable `out_len`.
        unsafe { *out_len = 0 };
    }
    // SAFETY: `ctx` points to `MultiReqHostCtx` alive until callbacks are cleared.
    let state = unsafe { &*ctx.cast::<MultiReqHostCtx>() };
    if method_id == 1 {
        state.ok_req_id.store(host_req_id, Ordering::Release);
    } else if method_id == 2 {
        state.cancel_req_id.store(host_req_id, Ordering::Release);
    }
    NveHostStatus::Pending.as_i32()
}

#[test]
fn dioxus_runtime_app_real_signals_callbacks_cancel_pre_mount_and_corrupt_restore() {
    let dir = ScratchDir::new("real-signals");
    let state_path = dir.0.join("signals.nvestate");
    let host_ctx = Arc::new(MultiReqHostCtx {
        ok_req_id: AtomicU64::new(0),
        cancel_req_id: AtomicU64::new(0),
    });
    let cancel_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut surface1 = DioxusSurface::new(SurfaceId::ROOT, real_signals_v6_app);
    let count_sig1 = surface1.in_root_scope(|| {
        let sig = Signal::new_in_scope(42i32, ScopeId::ROOT);
        dioxus_core::provide_context(RealSignalCtx {
            persisted_count: sig,
            cancel_hits: Arc::clone(&cancel_hits),
        });
        sig
    });
    surface1
        .signal_registry_mut()
        .register_signal("App", "count", count_sig1);

    let mut rt = RuntimeBuilder::new()
        .surface_id(SurfaceId::ROOT)
        .viewport(Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        })
        .build();
    rt.set_host_callbacks(Some(NveHostCallbacks {
        host_ctx: Arc::as_ptr(&host_ctx).cast_mut().cast(),
        invoke: Some(multi_req_mock_invoke),
        channel_publish: None,
    }))
    .expect("set_host_callbacks");
    rt.attach_app(Box::new(DioxusRuntimeApp::new(surface1)))
        .expect("attach app1");

    // 1. Mount fires both async calls; complete #1 and push event -> mutates real `use_signal`s
    //    inside `dispatch_host_events` with zero `NoCurrentRuntime` panic!
    assert!(rt.tick(1.0 / 60.0).expect("tick 1"));
    let ok_req = host_ctx.ok_req_id.load(Ordering::Acquire);
    let cancel_req = host_ctx.cancel_req_id.load(Ordering::Acquire);
    assert!(ok_req != 0 && cancel_req != 0 && ok_req != cancel_req);

    let router = rt.host_router();
    std::thread::scope(|s| {
        s.spawn(|| {
            assert!(router.complete_request(
                HostRequestId(ok_req),
                NveHostStatus::Ok,
                b"real-signal-reply",
            ));
            router.push_event(HostServiceId(0x5512), 22, b"real-signal-push");
        });
    });
    assert!(rt.tick(1.0 / 60.0).expect("tick 2"));
    {
        let g = rt.acquire_scene().expect("acquire after host events");
        let text = read_text_slab(&g);
        assert!(
            text.contains("count=42 reply=real-signal-reply push=real-signal-push"),
            "real Signal mutations must render cleanly: `{text}`"
        );
    }

    // 2. `save_state_file` snapshots live `register_signal` inside `ScopeId::ROOT` without panic!
    rt.save_state_file(&state_path, 1).expect("save_state_file");
    assert!(state_path.exists());

    // 3. `detach_app` calls `bind_host(None)` -> cancels in-flight #2 -> mutates `cancel_sig`
    //    (a live `use_signal`) cleanly without `NoCurrentRuntime` panic or poisoning `rt`!
    rt.detach_app().expect("detach_app must not poison runtime");
    assert_eq!(cancel_hits.load(Ordering::Acquire), 1);

    // 4. Pre-mount `restore_state_file` into a fresh `DioxusRuntimeApp` (before first `tick()`)
    //    hydrates live `register_signal` (`0 -> 42`) cleanly!
    let mut surface2 = DioxusSurface::new(SurfaceId::ROOT, real_signals_v6_app);
    let count_sig2 = surface2.in_root_scope(|| {
        let sig = Signal::new_in_scope(0i32, ScopeId::ROOT);
        dioxus_core::provide_context(RealSignalCtx {
            persisted_count: sig,
            cancel_hits: Arc::clone(&cancel_hits),
        });
        sig
    });
    surface2
        .signal_registry_mut()
        .register_signal("App", "count", count_sig2);
    rt.attach_app(Box::new(DioxusRuntimeApp::new(surface2)))
        .expect("attach app2");
    rt.restore_state_file(&state_path, 1)
        .expect("pre-mount restore_state_file");
    assert!(rt.tick(1.0 / 60.0).expect("first tick after pre-mount restore"));
    {
        let g = rt.acquire_scene().expect("acquire app2");
        let text = read_text_slab(&g);
        assert!(text.contains("count=42"), "pre-mount hydrated signal=`{text}`");
    }

    // 5. Corrupt `restore_state` MUST NOT panic, poison `rt`, or clobber live signal state!
    rt.restore_state(b"not-a-valid-nve-sig1-archive")
        .expect("corrupt restore_state must not poison runtime");
    let _ = rt.tick(1.0 / 60.0).expect("tick after corrupt restore");
    {
        let g = rt.acquire_scene().expect("acquire after corrupt restore");
        let text = read_text_slab(&g);
        assert!(text.contains("count=42"), "corrupt restore must keep live signal: `{text}`");
    }

    // 6. Direct `DioxusRuntimeApp::last_state_error` records decode errors and clears on success.
    let mut surface3 = DioxusSurface::new(SurfaceId::ROOT, real_signals_v6_app);
    let count_sig3 = surface3.in_root_scope(|| {
        let sig = Signal::new_in_scope(7i32, ScopeId::ROOT);
        dioxus_core::provide_context(RealSignalCtx {
            persisted_count: sig,
            cancel_hits: Arc::clone(&cancel_hits),
        });
        sig
    });
    surface3
        .signal_registry_mut()
        .register_signal("App", "count", count_sig3);
    let mut standalone = DioxusRuntimeApp::new(surface3);
    let valid_snap = standalone
        .snapshot_state()
        .expect("snapshot_state")
        .to_vec();
    assert!(standalone.last_state_error().is_none());
    standalone
        .restore_state(b"corrupt-bytes")
        .expect("restore_state corrupt");
    assert!(
        standalone.last_state_error().is_some(),
        "corrupt restore_state must record last_state_error"
    );
    standalone
        .restore_state(&valid_snap)
        .expect("restore_state valid");
    assert!(
        standalone.last_state_error().is_none(),
        "valid restore_state must clear last_state_error"
    );

    rt.set_host_callbacks(None).expect("clear host callbacks");
}

#[component]
fn PropsChild() -> Element {
    let props = consume_context::<HostProps>();
    let badge = props.get_str("badge").unwrap_or_else(|| "init".to_owned());
    let level = props.get_i64("level").unwrap_or(0);
    rsx! {
        text { "badge={badge} level={level}" }
    }
}

fn parent_ignoring_host_props() -> Element {
    // Parent intentionally NEVER reads `HostProps`; `PropsChild` takes zero props so Dioxus
    // memoizes it unless `HostProps`'s internal reactive signal marks `PropsChild`'s scope dirty.
    rsx! {
        view {
            style: "width: 240px; height: 60px;",
            PropsChild {}
        }
    }
}

#[test]
fn dioxus_runtime_app_child_component_reacts_to_host_props_without_parent_read() {
    let surface = DioxusSurface::new(SurfaceId::ROOT, parent_ignoring_host_props);
    let mut rt = RuntimeBuilder::new()
        .surface_id(SurfaceId::ROOT)
        .viewport(Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        })
        .app(Box::new(DioxusRuntimeApp::new(surface)))
        .build();

    assert!(rt.tick(1.0 / 60.0).expect("initial mount"));
    {
        let g = rt.acquire_scene().expect("acquire initial");
        assert!(read_text_slab(&g).contains("badge=init level=0"));
    }

    let entries = [
        NveMapEntry::new("badge", NveValue::from_str("gold")),
        NveMapEntry::new("level", NveValue::i64(9)),
    ];
    // SAFETY: `entries` outlives `set_props`.
    let props_ref = unsafe { NvePropsRef::from_slice(&entries) };
    assert!(rt.set_props(props_ref).expect("set_props"));
    assert!(rt.tick(1.0 / 60.0).expect("tick after set_props"));
    {
        let g = rt.acquire_scene().expect("acquire updated child");
        let text = read_text_slab(&g);
        assert!(
            text.contains("badge=gold level=9"),
            "memoized child component must react to HostProps update: `{text}`"
        );
    }
}
