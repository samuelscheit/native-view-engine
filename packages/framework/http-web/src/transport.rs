//! Browser [`FetchTransport`], [`BrowserClock`], and [`LifecycleGuard`] (`visibilitychange`,
//! `focus`, `online`).

use nve_query::Clock;
use nve_rpc::{BoxFuture, RawCall, RawReply, Transport, TransportError};

use crate::config::FetchConfig;

/// Minimal-WASM browser `fetch` RPC transport (`POST`, `application/cbor`, `redirect: "error"`,
/// streaming `ReadableStream` response cap, `AbortController` timeout + abort-on-drop).
#[derive(Debug, Clone)]
pub struct FetchTransport {
    config: FetchConfig,
}

impl FetchTransport {
    /// Creates a relative same-origin `FetchTransport` (`POST /rpc/<Procedure::NAME>`,
    /// `credentials: "same-origin"`).
    #[inline]
    pub fn same_origin() -> Self {
        Self {
            config: FetchConfig::same_origin(),
        }
    }

    #[inline]
    pub fn with_config(config: FetchConfig) -> Self {
        Self { config }
    }

    #[inline]
    pub fn config(&self) -> &FetchConfig {
        &self.config
    }
}

impl Default for FetchTransport {
    fn default() -> Self {
        Self::same_origin()
    }
}

impl Transport for FetchTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        #[cfg(target_arch = "wasm32")]
        {
            Box::pin(async move {
                let url = self.config.build_url(req.name);
                let auth = self.config.auth_header().unwrap_or("");
                crate::js_fetch::execute_js_fetch(
                    &url,
                    &req.body,
                    req.content_type,
                    self.config.credentials.as_js_str(),
                    auth,
                    self.config.timeout_ms,
                    self.config.max_response_bytes,
                )
                .await
            })
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = req;
            Box::pin(async {
                Err(TransportError::config(
                    "FetchTransport is only available on wasm32 browser targets",
                ))
            })
        }
    }
}

/// Monotonic browser clock backed by `performance.now()` on `wasm32` (and `SystemClock` on native).
#[derive(Debug, Default)]
pub struct BrowserClock {
    #[cfg(not(target_arch = "wasm32"))]
    inner: nve_query::SystemClock,
}

impl BrowserClock {
    #[inline]
    pub fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            inner: nve_query::SystemClock::new(),
        }
    }
}

impl Clock for BrowserClock {
    #[inline]
    fn now_ms(&self) -> u64 {
        #[cfg(target_arch = "wasm32")]
        {
            crate::js_fetch::perf_now_ms()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.now_ms()
        }
    }
}

/// RAII guard detaching browser `visibilitychange`, `focus`, and `online` listeners on drop.
pub struct LifecycleGuard {
    #[cfg(target_arch = "wasm32")]
    _inner: crate::js_fetch::JsLifecycleInner,
}

/// Attaches browser focus (`visibilitychange` + `focus`) and `online` listeners, invoking
/// `on_focus` and `on_online` until the returned [`LifecycleGuard`] is dropped.
pub fn attach_browser_lifecycle(
    on_focus: impl FnMut() + 'static,
    on_online: impl FnMut() + 'static,
) -> LifecycleGuard {
    #[cfg(target_arch = "wasm32")]
    {
        LifecycleGuard {
            _inner: crate::js_fetch::attach_js_lifecycle(Box::new(on_focus), Box::new(on_online)),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (on_focus, on_online);
        LifecycleGuard {}
    }
}
