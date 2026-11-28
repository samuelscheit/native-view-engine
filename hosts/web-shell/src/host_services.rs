//! Thin `hosts/web-shell` composition wiring `nve-clipboard-web`, `nve-system-env-web`,
//! `nve-linking-web`, `nve-haptics-web`, and `NveChannelBus` through [`HostCallbackBridge`]
//! (`NVE_IFACE_HOST = 5`) with microtask-deferred `NveHostEventBatch` flushing backed by
//! `nve_backend_web::microtask`.

use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex};

use nve_clipboard_web::WebClipboard;
use nve_haptics_web::WebHaptics;
use nve_host_services::{
    CompletionSink, HostCallbackBridge, HostEvent, HostServiceProvider, HostServiceRegistry,
    InvokeResult, ServiceError, TypedEventBus,
};
use nve_interop::NveChannelBus;
use nve_linking_web::WebLinking;
use nve_system_env_web::{WebSystemEnv, WebThemeWatcher};

/// Composite browser host-capability service bound to web-hosted NVE apps via
/// [`HostCallbackBridge<Arc<WebHostServices>>`] (`NVE_IFACE_HOST = 5`).
pub struct WebHostServices {
    registry: HostServiceRegistry,
    clipboard: Arc<WebClipboard>,
    system_env: Arc<WebSystemEnv>,
}

impl WebHostServices {
    /// Constructs a live browser [`WebHostServices`] registering clipboard, system-env,
    /// linking, haptics, and `NveChannelBus`.
    pub fn new() -> Arc<Self> {
        Self::with_components(Arc::new(WebClipboard::new()), Arc::new(WebSystemEnv::new()))
    }

    /// Constructs a deterministic headless [`WebHostServices`] for native host unit tests.
    pub fn headless(clipboard: Arc<WebClipboard>, system_env: Arc<WebSystemEnv>) -> Arc<Self> {
        Self::with_components(clipboard, system_env)
    }

    fn with_components(clipboard: Arc<WebClipboard>, system_env: Arc<WebSystemEnv>) -> Arc<Self> {
        let registry = HostServiceRegistry::new();
        registry.register_arc(clipboard.clone());
        registry.register_arc(system_env.clone());
        registry.register(WebLinking::new());
        registry.register(WebHaptics::new());
        Arc::new(Self {
            registry,
            clipboard,
            system_env,
        })
    }

    /// Builds a pinned C-ABI [`HostCallbackBridge`] backed by this `Arc<WebHostServices>`.
    pub fn into_bridge(self: &Arc<Self>) -> HostCallbackBridge<Arc<Self>> {
        HostCallbackBridge::new(Arc::clone(self))
    }

    pub fn clipboard(&self) -> &Arc<WebClipboard> {
        &self.clipboard
    }

    pub fn system_env(&self) -> &Arc<WebSystemEnv> {
        &self.system_env
    }

    pub fn channel_bus(&self) -> Arc<Mutex<NveChannelBus>> {
        self.registry.channel_bus()
    }

    pub fn completion_sink(&self) -> &CompletionSink {
        self.registry.completion_sink()
    }

    /// Installs a live [`WebThemeWatcher`] feeding `SERVICE_SYSTEM_ENV` `Push` records into
    /// this service's [`CompletionSink`].
    pub fn install_theme_watcher(&self) -> WebThemeWatcher {
        WebThemeWatcher::install(
            Arc::clone(&self.system_env),
            self.registry.completion_sink().clone(),
        )
    }

    /// Cancels all in-flight async requests and drops pending browser Promise closures.
    pub fn cancel_pending(&self) -> usize {
        let cancelled = self.registry.cancel_pending();
        self.clipboard.cancel_pending();
        cancelled
    }

    /// Drains all queued async `Response`, `Push`, and `Channel` records into a canonical
    /// `NveHostEventBatch` (`NVHB` v1) byte buffer.
    pub fn drain_host_event_batch(&self) -> Option<Vec<u8>> {
        WebClipboard::reap_settled_closures();
        self.registry.drain_host_event_batch()
    }
}

impl HostServiceProvider for WebHostServices {
    fn invoke_raw_with_id(
        &self,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        self.registry
            .invoke_raw_with_id(service_id, method_id, request_id, request)
    }

    fn invoke_raw_with_id_for_owner(
        &self,
        owner_tag: u64,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        self.registry.invoke_raw_with_id_for_owner(
            owner_tag,
            service_id,
            method_id,
            request_id,
            request,
        )
    }

    fn typed_service(
        &self,
        service_id: u32,
        method_id: u32,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        self.registry.typed_service(service_id, method_id, type_id)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        self.registry.channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.registry.poll_host_events()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        self.registry.typed_event_bus()
    }

    fn host_completion_sink(&self) -> Option<&CompletionSink> {
        Some(self.registry.completion_sink())
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        self.registry.take_async_response(request_id)
    }

    fn cancel_pending(&self) -> usize {
        self.cancel_pending()
    }

    fn discard_pending(&self) -> usize {
        let discarded = self.registry.discard_pending();
        self.clipboard.cancel_pending();
        discarded
    }

    fn discard_pending_for_owner(&self, owner_tag: u64) -> usize {
        let discarded = self.registry.discard_pending_for_owner(owner_tag);
        self.clipboard.cancel_pending_for_owner(owner_tag);
        discarded
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod wasm_drain {
    use std::cell::{Cell, RefCell};
    use std::rc::{Rc, Weak};
    use std::sync::Arc;

    use nve_abi::NveHostEventBatchBuilder;
    #[cfg(feature = "dev")]
    use nve_backend_web::microtask::debug_microtask_counts;
    use nve_backend_web::microtask::{
        cancel_owner_microtasks, enqueue_macrotask, enqueue_microtask,
    };
    use nve_system_env_web::WebThemeWatcher;

    use super::WebHostServices;
    use crate::session::WebAppSession;

    /// Microtask retries before yielding to macrotasks when `WebAppSession` is momentarily borrowed.
    const MICROTASK_BORROW_RETRY_LIMIT: u8 = 4;
    /// Hard retry cap preventing unbounded spinning if a session borrow is permanently stuck.
    const MAX_BORROW_RETRIES: u8 = 16;

    type WakeTrampoline = Box<dyn Fn(bool)>;

    thread_local! {
        static WAKE_TRAMPOLINES: RefCell<Vec<(usize, WakeTrampoline)>> =
            const { RefCell::new(Vec::new()) };
    }

    #[cfg(feature = "dev")]
    pub(crate) fn debug_drain_counts() -> (usize, usize, usize) {
        let (rust_tasks, js_tasks) = debug_microtask_counts();
        (
            rust_tasks,
            WAKE_TRAMPOLINES.with(|l| l.borrow().len()),
            js_tasks,
        )
    }

    /// Owns the live browser theme watcher, reusable batch builder, and microtask drain guard for
    /// a mounted surface.
    pub(crate) struct WebHostRuntimeGuard {
        services: Arc<WebHostServices>,
        _theme_watcher: WebThemeWatcher,
        active: Rc<Cell<bool>>,
    }

    impl WebHostRuntimeGuard {
        #[cfg(feature = "dev")]
        pub(crate) fn cached_scale_factor_x100(&self) -> u16 {
            self.services.system_env().current_theme().scale_factor_x100
        }

        #[cfg(feature = "dev")]
        pub(crate) fn debug_start_clipboard_read(&self) -> u64 {
            use nve_host_services::{HostServiceProvider, InvokeResult};
            match self
                .services
                .invoke_raw(nve_abi::SERVICE_CLIPBOARD, 1, &[])
            {
                Ok(InvokeResult::Pending { request_id }) => request_id,
                _ => 0,
            }
        }

        #[cfg(feature = "dev")]
        pub(crate) fn debug_set_clipboard_shadow(&self, text: Option<String>) {
            self.services.clipboard().set_shadow_text(text);
        }

        /// Starts a clipboard read without letting `wasm_drain` consume it, or returns sync text.
        #[cfg(feature = "dev")]
        pub(crate) fn debug_clipboard_read_probe(&self) -> String {
            use nve_clipboard_web::decode_read_text_response;
            use nve_host_services::{HostServiceProvider, InvokeResult};
            match self
                .services
                .invoke_raw(nve_abi::SERVICE_CLIPBOARD, 1, &[])
            {
                Ok(InvokeResult::Ready(bytes)) => match decode_read_text_response(&bytes) {
                    Ok(Some(t)) => format!("sync:{t}"),
                    Ok(None) => "sync-none".to_owned(),
                    Err(e) => format!("decode-err:{e}"),
                },
                Ok(InvokeResult::Pending { request_id }) => {
                    // Pause auto-drain so the E2E probe can inspect the resolved status directly.
                    self.active.set(false);
                    format!("pending:{request_id}")
                }
                Err(err) => format!("err:{}", err.to_status().as_i32()),
            }
        }

        #[cfg(feature = "dev")]
        pub(crate) fn debug_take_clipboard_result(&self, request_id: u64) -> String {
            use nve_clipboard_web::decode_read_text_response;
            match self
                .services
                .completion_sink()
                .take_completed_response(request_id)
            {
                None => "none".to_owned(),
                Some(Ok(bytes)) => match decode_read_text_response(&bytes) {
                    Ok(Some(t)) => format!("ok:{t}"),
                    Ok(None) => "ok-none".to_owned(),
                    Err(e) => format!("decode-err:{e}"),
                },
                Some(Err(err)) => format!("err:{}", err.to_status().as_i32()),
            }
        }

        #[cfg(feature = "dev")]
        pub(crate) fn debug_arm_reentrant_unmount_on_ready(&self, surface_id: nve_abi::SurfaceId) {
            self.services.completion_sink().set_on_ready(Some(Arc::new(move || {
                crate::wasm_api::teardown_surface_reentrant(surface_id);
            })));
        }

        pub(crate) fn install(
            services: Arc<WebHostServices>,
            session: Weak<RefCell<WebAppSession>>,
        ) -> Self {
            let active = Rc::new(Cell::new(true));
            let scheduled = Rc::new(Cell::new(false));
            let borrow_retries = Rc::new(Cell::new(0u8));
            let batch_builder = Rc::new(RefCell::new(NveHostEventBatchBuilder::with_capacity(
                1024,
            )));
            let theme_watcher = services.install_theme_watcher();
            let owner_key = Arc::as_ptr(&services) as usize;

            let active_for_wake = Rc::clone(&active);
            let scheduled_for_wake = Rc::clone(&scheduled);
            let retries_for_wake = Rc::clone(&borrow_retries);
            let services_for_wake = Arc::clone(&services);
            let builder_for_wake = Rc::clone(&batch_builder);

            let schedule_drain = move |use_macrotask: bool| {
                if !active_for_wake.get() || scheduled_for_wake.replace(true) {
                    return;
                }
                let active_inner = Rc::clone(&active_for_wake);
                let scheduled_inner = Rc::clone(&scheduled_for_wake);
                let retries_inner = Rc::clone(&retries_for_wake);
                let services_inner = Arc::clone(&services_for_wake);
                let builder_inner = Rc::clone(&builder_for_wake);
                let session_inner = session.clone();
                let run = move || {
                    scheduled_inner.set(false);
                    if !active_inner.get() {
                        return;
                    }
                    let Some(session_rc) = session_inner.upgrade() else {
                        return;
                    };
                    match session_rc.try_borrow_mut() {
                        Ok(mut guard) => {
                            retries_inner.set(0);
                            let mut builder = builder_inner.borrow_mut();
                            builder.clear();
                            if let Ok(written) = services_inner
                                .completion_sink()
                                .drain_batch_into(&mut builder)
                                && written > 0
                            {
                                let _ = guard.dispatch_host_events(builder.as_bytes());
                            }
                            drop(builder);
                            if services_inner.completion_sink().queued_count() > 0 {
                                invoke_wake_trampoline(owner_key, false);
                            }
                        }
                        Err(_) => {
                            let next = retries_inner.get().saturating_add(1);
                            retries_inner.set(next);
                            if next < MAX_BORROW_RETRIES {
                                let yield_macro = next >= MICROTASK_BORROW_RETRY_LIMIT;
                                invoke_wake_trampoline(owner_key, yield_macro);
                            }
                        }
                    }
                };
                if use_macrotask {
                    enqueue_macrotask(owner_key, run);
                } else {
                    enqueue_microtask(owner_key, run);
                }
            };

            WAKE_TRAMPOLINES.with(|list| {
                let mut guard = list.borrow_mut();
                guard.retain(|(k, _)| *k != owner_key);
                guard.push((owner_key, Box::new(schedule_drain)));
            });
            services.completion_sink().set_on_ready(Some(Arc::new(move || {
                invoke_wake_trampoline(owner_key, false);
            })));

            Self {
                services,
                _theme_watcher: theme_watcher,
                active,
            }
        }
    }

    impl Drop for WebHostRuntimeGuard {
        fn drop(&mut self) {
            self.active.set(false);
            let owner_key = Arc::as_ptr(&self.services) as usize;
            self.services.completion_sink().set_on_ready(None);
            WAKE_TRAMPOLINES.with(|list| {
                list.borrow_mut().retain(|(k, _)| *k != owner_key);
            });
            cancel_owner_microtasks(owner_key);
            let _ = self.services.cancel_pending();
        }
    }

    fn invoke_wake_trampoline(owner_key: usize, use_macrotask: bool) {
        WAKE_TRAMPOLINES.with(|list| {
            if let Some((_, f)) = list.borrow().iter().find(|(k, _)| *k == owner_key) {
                f(use_macrotask);
            }
        });
    }
}
