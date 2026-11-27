//! Compact `inline_js` Fetch + `ReadableStream` byte-cap bridge, `AbortOnDrop` Promise future,
//! `performance.now()` clock, and `visibilitychange` / `focus` / `online` listeners for `wasm32`.

use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::rc::Rc;

use bytes::Bytes;
use js_sys::{Array, Function, Reflect, Uint8Array};
use nve_rpc::{classify_http_reply, RawReply, TransportError};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
export function nveStartFetch(url, body, ct, creds, auth, timeoutMs, maxBytes, onDone) {
  const reqBody = body.slice();
  const ac = new AbortController();
  let timedOut = false;
  const timer = timeoutMs > 0 ? setTimeout(() => { timedOut = true; ac.abort("timeout"); }, timeoutMs) : 0;
  const headers = { "content-type": ct, "accept": ct };
  if (auth) headers["authorization"] = auth;
  const promise = (async () => {
    try {
      const res = await globalThis.fetch(url, {
        method: "POST", body: reqBody, headers, credentials: creds,
        redirect: "error", cache: "no-store", signal: ac.signal
      });
      const cl = res.headers.get("content-length");
      if (cl !== null && Number(cl) > maxBytes) {
        try { await res.body?.cancel(); } catch (_) {}
        throw { k: "too_large", m: "response content-length exceeded cap" };
      }
      let out;
      if (res.body && typeof res.body.getReader === "function") {
        const reader = res.body.getReader();
        const chunks = [];
        let total = 0;
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          if (!value || value.byteLength === 0) continue;
          total += value.byteLength;
          if (total > maxBytes) {
            try { await reader.cancel(); } catch (_) {}
            throw { k: "too_large", m: "response body exceeded max_response_bytes" };
          }
          chunks.push(value);
        }
        if (chunks.length === 0) out = new Uint8Array(0);
        else if (chunks.length === 1) out = chunks[0];
        else {
          out = new Uint8Array(total);
          let off = 0;
          for (const c of chunks) { out.set(c, off); off += c.byteLength; }
        }
      } else {
        const ab = await res.arrayBuffer();
        if (ab.byteLength > maxBytes) throw { k: "too_large", m: "response body exceeded max_response_bytes" };
        out = new Uint8Array(ab);
      }
      return [
        res.status,
        res.headers.get("nve-error-kind") || "",
        res.headers.get("nve-invalidate") || "",
        out
      ];
    } catch (e) {
      if (e && typeof e === "object" && typeof e.k === "string") throw e;
      if (timedOut) throw { k: "timeout", m: "fetch timed out" };
      if (ac.signal.aborted) throw { k: "cancelled", m: "fetch aborted" };
      if (globalThis.navigator && globalThis.navigator.onLine === false)
        throw { k: "offline", m: "browser is offline" };
      const msg = (e && e.message) ? String(e.message) : String(e);
      if (msg.toLowerCase().includes("redirect")) throw { k: "redirect", m: msg };
      throw { k: "network", m: msg };
    } finally {
      if (timer) clearTimeout(timer);
    }
  })();
  promise.then(
    (v) => onDone(true, v),
    (e) => onDone(false, e)
  );
  return () => { try { ac.abort("cancelled"); } catch (_) {} };
}

export function nvePerfNowMs() {
  return (globalThis.performance && typeof globalThis.performance.now === "function")
    ? globalThis.performance.now()
    : Date.now();
}

export function nveAttachLifecycle(onFocus, onOnline) {
  const doc = globalThis.document;
  const win = globalThis.window;
  const vis = () => { if (doc && doc.visibilityState === "visible") onFocus(); };
  if (doc && doc.addEventListener) doc.addEventListener("visibilitychange", vis);
  if (win && win.addEventListener) {
    win.addEventListener("focus", onFocus);
    win.addEventListener("online", onOnline);
  }
  return () => {
    if (doc && doc.removeEventListener) doc.removeEventListener("visibilitychange", vis);
    if (win && win.removeEventListener) {
      win.removeEventListener("focus", onFocus);
      win.removeEventListener("online", onOnline);
    }
  };
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = nveStartFetch)]
    fn nve_start_fetch(
        url: &str,
        body: &[u8],
        ct: &str,
        creds: &str,
        auth: &str,
        timeout_ms: u32,
        max_bytes: f64,
        on_done: &JsValue,
    ) -> Function;

    #[wasm_bindgen(js_name = nvePerfNowMs)]
    fn nve_perf_now_ms() -> f64;

    #[wasm_bindgen(js_name = nveAttachLifecycle)]
    fn nve_attach_lifecycle(on_focus: &JsValue, on_online: &JsValue) -> Function;
}

struct PromiseState {
    result: Option<Result<JsValue, JsValue>>,
    waker: Option<Waker>,
}

struct FetchPromiseFuture {
    state: Rc<RefCell<PromiseState>>,
    abort_fn: Option<Function>,
}

impl Drop for FetchPromiseFuture {
    fn drop(&mut self) {
        if let Some(abort) = self.abort_fn.take() {
            let _ = abort.call0(&JsValue::NULL);
        }
    }
}

impl Future for FetchPromiseFuture {
    type Output = Result<JsValue, JsValue>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut st = self.state.borrow_mut();
        if let Some(res) = st.result.take() {
            drop(st);
            self.abort_fn = None;
            return Poll::Ready(res);
        }
        st.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

pub(crate) async fn execute_js_fetch(
    url: &str,
    body: &[u8],
    content_type: &str,
    credentials: &str,
    auth_header: &str,
    timeout_ms: u32,
    max_response_bytes: usize,
) -> Result<RawReply, TransportError> {
    let state = Rc::new(RefCell::new(PromiseState {
        result: None,
        waker: None,
    }));

    let done_state = Rc::clone(&state);
    let on_done = Closure::once_into_js(move |ok: bool, val: JsValue| {
        let waker = {
            let mut st = done_state.borrow_mut();
            st.result = Some(if ok { Ok(val) } else { Err(val) });
            st.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    });

    let abort_fn = nve_start_fetch(
        url,
        body,
        content_type,
        credentials,
        auth_header,
        timeout_ms,
        max_response_bytes as f64,
        &on_done,
    );

    let fut = FetchPromiseFuture {
        state,
        abort_fn: Some(abort_fn),
    };

    match fut.await {
        Ok(val) => {
            let arr = Array::from(&val);
            let status = arr.get(0).as_f64().unwrap_or(500.0) as u16;
            let err_kind_str = arr.get(1).as_string().unwrap_or_default();
            let inv_str = arr.get(2).as_string().unwrap_or_default();
            let u8_arr = Uint8Array::new(&arr.get(3));
            let body_vec = u8_arr.to_vec();

            let err_kind = if err_kind_str.is_empty() {
                None
            } else {
                Some(err_kind_str.as_bytes())
            };
            let inv_slice = if inv_str.is_empty() {
                None
            } else {
                Some(inv_str.as_bytes())
            };
            Ok(classify_http_reply(
                status,
                err_kind,
                inv_slice,
                Bytes::from(body_vec),
            ))
        }
        Err(err) => Err(map_js_error(&err)),
    }
}

fn map_js_error(err: &JsValue) -> TransportError {
    let kind = Reflect::get(err, &JsValue::from_str("k"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    let msg = Reflect::get(err, &JsValue::from_str("m"))
        .ok()
        .and_then(|v| v.as_string())
        .or_else(|| err.as_string())
        .unwrap_or_else(|| "browser fetch failed".to_owned());

    match kind.as_str() {
        "timeout" => TransportError::timeout(msg),
        "cancelled" => TransportError::cancelled(msg),
        "offline" => TransportError::offline(msg),
        "too_large" => TransportError::response_too_large(msg),
        "redirect" => TransportError::redirect(msg),
        _ => TransportError::network(msg),
    }
}

#[inline]
pub(crate) fn perf_now_ms() -> u64 {
    let raw = nve_perf_now_ms();
    if raw.is_finite() && raw >= 0.0 {
        raw as u64
    } else {
        0
    }
}

pub(crate) struct JsLifecycleInner {
    detach: Function,
    _on_focus: Closure<dyn FnMut()>,
    _on_online: Closure<dyn FnMut()>,
}

impl Drop for JsLifecycleInner {
    fn drop(&mut self) {
        let _ = self.detach.call0(&JsValue::NULL);
    }
}

pub(crate) fn attach_js_lifecycle(
    on_focus: Box<dyn FnMut()>,
    on_online: Box<dyn FnMut()>,
) -> JsLifecycleInner {
    let focus_cb = Closure::wrap(on_focus);
    let online_cb = Closure::wrap(on_online);
    let detach = nve_attach_lifecycle(focus_cb.as_ref(), online_cb.as_ref());
    JsLifecycleInner {
        detach,
        _on_focus: focus_cb,
        _on_online: online_cb,
    }
}
