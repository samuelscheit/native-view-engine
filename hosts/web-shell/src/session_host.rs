//! Owned host-callback bridge slot & `NVE_IFACE_HOST = 5` binding methods for [`WebAppSession`].
//!
//! Enforces strict teardown ordering: `SessionApp::bind_host(None)` ALWAYS runs before the
//! pinned [`HostCallbackBridge`] is dropped, preventing any dangling `host_ctx` window.

#[cfg(feature = "web-host-services")]
use std::sync::Arc;

use nve_abi::NveHostCallbacks;
use nve_engine::{BackendError, FrameTiming};
use nve_host_services::{HostCallbackBridge, HostServiceProvider};

#[cfg(feature = "web-host-services")]
use crate::host_services::WebHostServices;
use crate::session::WebAppSession;

/// Defers `WebAppSession::unbind_host()` across microtask/macrotask boundaries when teardown
/// happens re-entrantly while `session` is mutably borrowed on the call stack.
#[cfg(target_arch = "wasm32")]
pub(crate) fn defer_session_unbind_host(
    session: std::rc::Rc<std::cell::RefCell<WebAppSession>>,
    attempt: u8,
) {
    const MAX_ATTEMPTS: u8 = 16;
    if attempt >= MAX_ATTEMPTS {
        return;
    }
    let owner_key = std::rc::Rc::as_ptr(&session) as usize;
    let task = move || {
        let unbound = if let Ok(mut guard) = session.try_borrow_mut() {
            guard.unbind_host();
            true
        } else {
            false
        };
        if !unbound {
            defer_session_unbind_host(session, attempt.saturating_add(1));
        }
    };
    if attempt < 4 {
        nve_backend_web::microtask::enqueue_microtask(owner_key, task);
    } else {
        nve_backend_web::microtask::enqueue_macrotask(owner_key, task);
    }
}

/// Type-erased pinned host-bridge owner held by [`WebAppSession`] so `Drop` unbinds the app
/// before releasing the underlying `HostCallbackBridge<P>` allocation.
pub(crate) trait OwnedHostBridge {
    fn cancel_before_drop(&mut self) {}
}

struct GenericBridgeOwner<P: HostServiceProvider> {
    _bridge: HostCallbackBridge<P>,
}

impl<P: HostServiceProvider> OwnedHostBridge for GenericBridgeOwner<P> {}

#[cfg(feature = "web-host-services")]
struct WebHostBridgeOwner {
    services: Arc<WebHostServices>,
    _bridge: HostCallbackBridge<Arc<WebHostServices>>,
}

#[cfg(feature = "web-host-services")]
impl OwnedHostBridge for WebHostBridgeOwner {
    fn cancel_before_drop(&mut self) {
        let _ = self.services.cancel_pending();
    }
}

impl WebAppSession {
    /// Binds or unbinds raw `NVE_IFACE_HOST = 5` C-ABI callbacks onto the active app.
    ///
    /// Returns `true` when binding mutated app state and requires a re-render. Unconditionally
    /// releases and cancels any previously owned bridge so swapping to raw callbacks never leaks.
    pub fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) -> bool {
        let dirty = self.app.bind_host(callbacks);
        if let Some(mut bridge) = self.host_bridge.take() {
            bridge.cancel_before_drop();
        }
        dirty
    }

    /// Binds an owned [`HostCallbackBridge`] onto this session via `NVE_IFACE_HOST = 5`,
    /// guaranteeing the app is unbound before the bridge drops.
    pub fn bind_host_bridge<P: HostServiceProvider + 'static>(
        &mut self,
        bridge: HostCallbackBridge<P>,
    ) -> bool {
        let callbacks = bridge.callbacks();
        // Bind new callbacks before dropping any prior bridge (zero dangling-pointer window).
        let dirty = self.app.bind_host(Some(callbacks));
        if let Some(mut old) = self
            .host_bridge
            .replace(Box::new(GenericBridgeOwner { _bridge: bridge }))
        {
            old.cancel_before_drop();
        }
        dirty
    }

    /// Binds a [`WebHostServices`] instance through [`HostCallbackBridge`] (`NVE_IFACE_HOST = 5`).
    #[cfg(feature = "web-host-services")]
    pub fn bind_web_host_services(&mut self, services: Arc<WebHostServices>) -> bool {
        let bridge = services.into_bridge();
        let callbacks = bridge.callbacks();
        let dirty = self.app.bind_host(Some(callbacks));
        if let Some(mut old) = self.host_bridge.replace(Box::new(WebHostBridgeOwner {
            services,
            _bridge: bridge,
        })) {
            old.cancel_before_drop();
        }
        dirty
    }

    /// Unbinds `NVE_IFACE_HOST = 5` from the app, cancels pending async host calls, and drops
    /// the owned bridge in safe order.
    pub fn unbind_host(&mut self) {
        let _ = self.app.bind_host(None);
        if let Some(mut bridge) = self.host_bridge.take() {
            bridge.cancel_before_drop();
        }
    }

    /// Direct access to the hosted Kitchen Sink module's `FfiHostServices` (tests only).
    #[cfg(all(test, feature = "app-kitchen-sink"))]
    pub(crate) fn kitchen_sink_host_services(&self) -> Option<&nve_host_services::FfiHostServices> {
        match &self.app {
            crate::session::SessionApp::KitchenSink(module) => Some(module.host_services()),
            _ => None,
        }
    }

    /// Dispatches a canonical `NveHostEventBatch` (`NVHB` v1) byte buffer to the app via
    /// `NVE_IFACE_HOST = 5`, rendering an incremental frame if the app updated state.
    pub fn dispatch_host_events(
        &mut self,
        batch: &[u8],
    ) -> Result<Option<FrameTiming>, BackendError> {
        if !self.app.dispatch_host_events(batch) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }
}
