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

use dioxus_core::Element;
use nve_abi::{
    HostRequestId, HostServiceId, NodeId, NveBufferSlice, NveHostCallbacks, NveHostStatus,
    NveMapEntry, NvePropsRef, NveValue, SurfaceId,
};
use nve_dioxus::prelude::*;
use nve_dioxus::{DioxusRuntimeApp, DioxusSurface, HostEventEmitter, HostProps};
use nve_engine::Viewport;
use nve_host_services::FfiHostServices;
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
