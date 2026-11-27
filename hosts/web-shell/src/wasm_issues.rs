//! Browser `wasm32` entrypoint (`start_issues_app`) and real-Chrome `FetchTransport` / CORS
//! probes for `examples/fullstack-issues` (`feature = "app-issues"`).

use std::cell::RefCell;
#[cfg(feature = "issues-probes")]
use std::future::Future;
#[cfg(feature = "issues-probes")]
use std::pin::Pin;
use std::rc::Rc;
#[cfg(feature = "issues-probes")]
use std::sync::Arc;
#[cfg(feature = "issues-probes")]
use std::task::{Context, Poll, Wake, Waker};

#[cfg(feature = "issues-probes")]
use bytes::Bytes;
use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::Viewport;
#[cfg(feature = "issues-probes")]
use nve_example_issues_api::{
    AddComment, AddCommentInput, ApiError, GetIssue, IssueFilter, IssueId, ListIssues,
};
use nve_example_issues_client::IssuesApp;
#[cfg(feature = "issues-probes")]
use nve_http_web::CredentialsMode;
use nve_http_web::{
    attach_browser_lifecycle, browser_sleep_ms, BrowserClock, FetchConfig, FetchTransport,
    LifecycleGuard,
};
use nve_query::{QueryClientConfig, Shared};
use nve_query_dioxus::{erased_query_client_with_clock, ErasedQueryClient, SleepHandle};
#[cfg(feature = "issues-probes")]
use nve_rpc::{
    call, Effect, ProcId, RawCall, RpcError, RpcStatus, Tag64, Transport, TransportErrorKind,
    CONTENT_TYPE_CBOR,
};
use wasm_bindgen::prelude::*;

use crate::session::{WebAppSession, DEFAULT_SURFACE_ID};
use crate::wasm_api::{start_surface_app_inner, WebAppController};

#[cfg(feature = "issues-probes")]
#[wasm_bindgen(inline_js = "export function nve_issues_deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
export function nve_issues_resolve(d, val) { d.resolve(val); }
export function nve_issues_reject(d, err) { d.reject(err); }
export function nve_issues_promise(d) { return d.promise; }
export function nve_issues_schedule_microtask(cb) { queueMicrotask(cb); }")]
extern "C" {
    fn nve_issues_deferred() -> JsValue;
    fn nve_issues_resolve(d: &JsValue, val: &str);
    fn nve_issues_reject(d: &JsValue, err: &str);
    fn nve_issues_promise(d: &JsValue) -> JsValue;
    fn nve_issues_schedule_microtask(cb: &JsValue);
}

#[derive(Clone)]
struct IssuesBootConfig {
    client: ErasedQueryClient,
}

#[component]
fn BrowserIssuesRoot() -> Element {
    let boot = dioxus_core::consume_context::<IssuesBootConfig>();
    let client_for_lifecycle = boot.client.clone();
    // Hold `LifecycleGuard` inside the root scope so `unmount()` detaches window listeners cleanly.
    let guard_slot: Rc<RefCell<Option<LifecycleGuard>>> = use_hook(move || {
        let focus_client = client_for_lifecycle.clone();
        let online_client = client_for_lifecycle;
        let guard = attach_browser_lifecycle(
            move || {
                let stale = focus_client.on_window_focus();
                focus_client.invalidate_handles(&stale);
            },
            move || {
                let stale = online_client.on_network_online();
                online_client.invalidate_handles(&stale);
            },
        );
        Rc::new(RefCell::new(Some(guard)))
    });
    use_drop(move || {
        let _ = guard_slot.borrow_mut().take();
    });
    rsx! {
        IssuesApp {
            client: boot.client,
            sleep: Some(SleepHandle::new(browser_sleep_ms)),
        }
    }
}

fn build_issues_session(
    surface_id: SurfaceId,
    backend: WebBackend,
    viewport: Viewport,
    api_base: &str,
) -> WebAppSession {
    let cfg = if api_base.trim().is_empty() {
        FetchConfig::same_origin()
    } else {
        FetchConfig::with_base_url(api_base)
    };
    let transport = FetchTransport::with_config(cfg);
    let client = erased_query_client_with_clock(
        transport,
        Shared::new(BrowserClock::new()),
        QueryClientConfig::default(),
    );
    let surface = DioxusSurface::new(surface_id, BrowserIssuesRoot)
        .with_context(IssuesBootConfig { client });
    WebAppSession::new(surface_id, surface, backend, viewport)
}

/// Mounts the cross-platform `examples/fullstack-issues` Dioxus app onto `#<mount_id>` backed by
/// browser `FetchTransport` (cookie auth by default) and `BrowserClock`.
#[wasm_bindgen]
pub fn start_issues_app(
    mount_id: &str,
    api_base: Option<String>,
) -> Result<WebAppController, JsValue> {
    let base = api_base.unwrap_or_default();
    start_surface_app_inner(
        "IssuesApp",
        DEFAULT_SURFACE_ID,
        mount_id,
        None,
        false,
        move |surface_id, backend, viewport| {
            build_issues_session(surface_id, backend, viewport, &base)
        },
    )
}

#[cfg(feature = "issues-probes")]
struct PendingPromise {
    deferred: JsValue,
    fut: Pin<Box<dyn Future<Output = Result<String, String>>>>,
    scheduled: bool,
}

#[cfg(feature = "issues-probes")]
enum PromiseEntry {
    Idle(PendingPromise),
    Polling { woken: bool },
}

#[cfg(feature = "issues-probes")]
thread_local! {
    static NEXT_PROMISE_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    static PENDING_PROMISES: RefCell<std::collections::BTreeMap<u64, PromiseEntry>> =
        const { RefCell::new(std::collections::BTreeMap::new()) };
}

#[cfg(feature = "issues-probes")]
struct PromiseWaker(u64);

#[cfg(feature = "issues-probes")]
fn schedule_promise_poll(id: u64) {
    let cb = Closure::once_into_js(move || {
        poll_promise_task(id);
    });
    nve_issues_schedule_microtask(&cb);
}

#[cfg(feature = "issues-probes")]
fn poll_promise_task(id: u64) {
    let entry = PENDING_PROMISES.with(|map| {
        let mut guard = map.borrow_mut();
        let slot = guard.get_mut(&id)?;
        let prev = std::mem::replace(slot, PromiseEntry::Polling { woken: false });
        match prev {
            PromiseEntry::Idle(mut item) => {
                item.scheduled = false;
                Some(item)
            }
            PromiseEntry::Polling { woken } => {
                *slot = PromiseEntry::Polling { woken };
                None
            }
        }
    });
    let Some(mut item) = entry else {
        return;
    };
    let waker = Waker::from(Arc::new(PromiseWaker(id)));
    let mut cx = Context::from_waker(&waker);
    match item.fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(val)) => {
            PENDING_PROMISES.with(|map| {
                map.borrow_mut().remove(&id);
            });
            nve_issues_resolve(&item.deferred, &val);
        }
        Poll::Ready(Err(err)) => {
            PENDING_PROMISES.with(|map| {
                map.borrow_mut().remove(&id);
            });
            nve_issues_reject(&item.deferred, &err);
        }
        Poll::Pending => {
            let woken_during_poll = PENDING_PROMISES.with(|map| {
                let mut guard = map.borrow_mut();
                let woken = matches!(guard.get(&id), Some(PromiseEntry::Polling { woken: true }));
                item.scheduled = woken;
                guard.insert(id, PromiseEntry::Idle(item));
                woken
            });
            if woken_during_poll {
                schedule_promise_poll(id);
            }
        }
    }
}

#[cfg(feature = "issues-probes")]
impl Wake for PromiseWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let id = self.0;
        let should_schedule = PENDING_PROMISES.with(|map| {
            let mut guard = map.borrow_mut();
            match guard.get_mut(&id) {
                Some(PromiseEntry::Polling { woken }) => {
                    *woken = true;
                    false
                }
                Some(PromiseEntry::Idle(item)) => {
                    if item.scheduled {
                        return false;
                    }
                    item.scheduled = true;
                    true
                }
                None => false,
            }
        });
        if should_schedule {
            schedule_promise_poll(id);
        }
    }
}

#[cfg(feature = "issues-probes")]
fn future_to_js_promise(fut: impl Future<Output = Result<String, String>> + 'static) -> JsValue {
    let deferred = nve_issues_deferred();
    let promise = nve_issues_promise(&deferred);
    let id = NEXT_PROMISE_ID.with(|c| {
        let next = c.get();
        c.set(next.wrapping_add(1).max(1));
        next
    });
    PENDING_PROMISES.with(|map| {
        map.borrow_mut().insert(
            id,
            PromiseEntry::Idle(PendingPromise {
                deferred,
                fut: Box::pin(fut),
                scheduled: false,
            }),
        );
    });
    poll_promise_task(id);
    promise
}

/// Real-browser probe exercising `FetchTransport` happy path, streaming `ReadableStream` cap,
/// `Content-Length` cap, `redirect: "manual"` 302 rejection, timeout, and abort-on-drop.
#[cfg(feature = "issues-probes")]
#[wasm_bindgen]
pub fn nve_probe_fetch_transport(probe_origin: String) -> JsValue {
    future_to_js_promise(async move {
        let base_cfg = || {
            FetchConfig::with_base_url(&probe_origin)
                .with_rpc_prefix("/probe/rpc")
                .with_credentials(CredentialsMode::Omit)
        };

        let make_call = |name: &'static str, body: Bytes| RawCall {
            proc_id: ProcId::of(name),
            name,
            effect: Effect::Query,
            content_type: CONTENT_TYPE_CBOR,
            body,
        };

        // 1. Happy path + Nve-Invalidate header parsing
        let ok_t = FetchTransport::with_config(base_cfg());
        let ok_reply = ok_t
            .call_raw(make_call("probe.v1.Test/Ok", Bytes::from_static(&[0x01])))
            .await
            .map_err(|e| format!("ok failed: {e}"))?;
        if ok_reply.http_status != 200 || ok_reply.body.as_ref() != [0x18, 0x2a] {
            return Err(format!("unexpected ok reply: {ok_reply:?}"));
        }
        if ok_reply.invalidated_tags != vec![Tag64(7), Tag64(42)] {
            return Err(format!("unexpected tags: {:?}", ok_reply.invalidated_tags));
        }

        // 2. Chunked ReadableStream cap -> ResponseTooLarge
        let stream_t = FetchTransport::with_config(base_cfg().with_max_response_bytes(2048));
        let stream_err = stream_t
            .call_raw(make_call("probe.v1.Test/StreamOversize", Bytes::new()))
            .await
            .err()
            .ok_or_else(|| "StreamOversize unexpectedly succeeded".to_owned())?;
        if stream_err.kind != TransportErrorKind::ResponseTooLarge {
            return Err(format!("expected ResponseTooLarge on stream cap, got {stream_err:?}"));
        }

        // 3. Content-Length pre-check -> ResponseTooLarge
        let cl_err = stream_t
            .call_raw(make_call("probe.v1.Test/ContentLengthOversize", Bytes::new()))
            .await
            .err()
            .ok_or_else(|| "ContentLengthOversize unexpectedly succeeded".to_owned())?;
        if cl_err.kind != TransportErrorKind::ResponseTooLarge {
            return Err(format!("expected ResponseTooLarge on Content-Length, got {cl_err:?}"));
        }

        // 4. Manual 302 redirect -> Redirect
        let redir_err = ok_t
            .call_raw(make_call("probe.v1.Test/Redirect", Bytes::new()))
            .await
            .err()
            .ok_or_else(|| "Redirect unexpectedly succeeded".to_owned())?;
        if redir_err.kind != TransportErrorKind::Redirect {
            return Err(format!("expected Redirect on 302, got {redir_err:?}"));
        }

        // 5. Timeout -> Timeout (distinct endpoint from drop probe so server verifies both aborts)
        let timeout_t = FetchTransport::with_config(base_cfg().with_timeout_ms(120));
        let timeout_err = timeout_t
            .call_raw(make_call("probe.v1.Test/SlowTimeout", Bytes::new()))
            .await
            .err()
            .ok_or_else(|| "SlowTimeout unexpectedly succeeded".to_owned())?;
        if timeout_err.kind != TransportErrorKind::Timeout {
            return Err(format!("expected Timeout, got {timeout_err:?}"));
        }

        // 6. Mid-flight Future drop triggers AbortController.abort() cleanly AFTER the request
        // reaches the server (sleeping briefly before drop guarantees the HTTP request is truly
        // in-flight on the server rather than cancelled before Chrome dispatches it).
        {
            let mut slow_fut = ok_t.call_raw(make_call("probe.v1.Test/SlowDrop", Bytes::new()));
            {
                let mut cx = Context::from_waker(Waker::noop());
                let first_poll = slow_fut.as_mut().poll(&mut cx);
                if !first_poll.is_pending() {
                    return Err("expected Pending on SlowDrop first poll".to_owned());
                }
            }
            browser_sleep_ms(80).await;
            drop(slow_fut);
        }
        browser_sleep_ms(150).await;

        Ok("OK:all-6-fetch-probes-passed".to_owned())
    })
}

/// Real-browser probe exercising cross-origin CORS preflight (`OPTIONS` + `POST /rpc/*` with
/// `Authorization: Bearer`) and verifying `Access-Control-Expose-Headers` exposes both
/// `Nve-Invalidate` and `Nve-Error-Kind`.
#[cfg(feature = "issues-probes")]
#[wasm_bindgen]
pub fn nve_probe_cross_origin_cors(issues_origin: String) -> JsValue {
    future_to_js_promise(async move {
        let cfg = FetchConfig::with_base_url(&issues_origin)
            .with_bearer_token("native-demo-bearer")
            .map_err(|e| e.to_string())?;
        let transport = FetchTransport::with_config(cfg);

        let list = call::<ListIssues>(&transport, &IssueFilter::default())
            .await
            .map_err(|e| format!("CORS ListIssues failed: {e}"))?;
        if list.value.len() < 3 {
            return Err(format!("expected >= 3 issues over CORS, got {}", list.value.len()));
        }

        let comment = call::<AddComment>(
            &transport,
            &AddCommentInput {
                issue_id: IssueId(1),
                body: "CORS preflight verified".into(),
            },
        )
        .await
        .map_err(|e| format!("CORS AddComment failed: {e}"))?;
        if comment.invalidated_tags.len() != 3 {
            return Err(format!(
                "expected 3 exposed Nve-Invalidate tags over CORS, got {:?}",
                comment.invalidated_tags
            ));
        }

        let missing = call::<GetIssue>(&transport, &IssueId(999)).await;
        match missing {
            Err(RpcError::Domain {
                status: RpcStatus::NotFound,
                error: ApiError::NotFound(IssueId(999)),
            }) => {}
            other => {
                return Err(format!(
                    "expected exposed Nve-Error-Kind:domain NotFound over CORS, got {other:?}"
                ));
            }
        }

        Ok(format!(
            "OK:cors-list={}-tags={}-domain-404-ok",
            list.value.len(),
            comment.invalidated_tags.len()
        ))
    })
}
