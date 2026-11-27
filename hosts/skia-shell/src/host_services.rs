//! Direct zero-serialization host-capability bridge for `hosts/skia-shell`.
//!
//! Combines `nve_host_services::HostServiceRegistry` with modular OS capability handlers
//! (`nve-clipboard`, `nve-filesystem`, `nve-secure-store`, `nve-linking`, `nve-system-env`,
//! `nve-haptics`), direct typed [`WindowCommand`] submission
//! ([`WindowServiceBackend`]), and zero-serialization [`TypedEventBus`] / [`SkiaHostEvent`]
//! delivery for live theme, locale, and multi-window lifecycle updates.

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, VecDeque};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{
    NveBufferSlice, NveHostCallbacks, NveHostStatus, METHOD_WINDOW_REQUEST_FRAME,
    NVE_CHANNEL_FLAG_RETAIN, SERVICE_SYSTEM_ENV, SERVICE_WINDOW,
};
use nve_clipboard::NativeClipboard;
use nve_filesystem::NativeFilesystem;
use nve_haptics::NativeHaptics;
use nve_host_services::{
    pack_typed, HostEvent, HostServiceProvider, HostServiceRegistry, InvokeResult, ServiceError,
    TypedEventBus,
};
use nve_interop::{NveChannelBus, WindowCommand, WindowHandleId, WindowLifecycleEvent};
use nve_linking::NativeLinking;
use nve_secure_store::NativeSecureStore;
use nve_system_env::{
    dispatch_system_env_ffi, ColorScheme, LocaleSnapshot, NativeSystemEnv, SystemEnvBackend,
    SystemEnvBridge, SystemEnvClient, ThemeSnapshot,
};
use winit::event_loop::EventLoopProxy;
use winit::window::Theme as WinitTheme;

use crate::window::HostWakeEvent;

/// Callback invoked whenever an app submits a typed [`WindowCommand`].
pub type WindowCommandSink = Arc<dyn Fn(WindowCommand) + Send + Sync>;

/// Direct typed host event delivered in-process to mounted desktop apps with zero serialization.
#[derive(Debug, Clone, PartialEq)]
pub enum SkiaHostEvent {
    ThemeChanged(ThemeSnapshot),
    LocaleChanged(LocaleSnapshot),
    SystemEnv {
        theme: ThemeSnapshot,
        locale: LocaleSnapshot,
    },
    WindowLifecycle(Vec<WindowLifecycleEvent>),
}

/// Direct typed in-process backend for [`SERVICE_WINDOW`] (`resolve_typed::<dyn WindowServiceBackend>`).
pub trait WindowServiceBackend: Send + Sync {
    fn submit_commands(&self, commands: &[WindowCommand]) -> usize;
}

/// Maps a `winit::window::Theme` to `nve_system_env`'s [`ColorScheme`].
pub const fn winit_theme_to_color_scheme(theme: WinitTheme) -> ColorScheme {
    match theme {
        WinitTheme::Light => ColorScheme::Light,
        WinitTheme::Dark => ColorScheme::Dark,
    }
}

fn scale_to_x100(scale_factor: f32) -> u16 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        (scale_factor * 100.0).round().clamp(1.0, f32::from(u16::MAX)) as u16
    } else {
        100
    }
}

/// Direct in-process [`SystemEnvBackend`] serving `SkiaHostServices`'s live winit-cached theme
/// and locale with zero wire serialization.
struct SkiaSystemEnvBackend {
    theme: Mutex<ThemeSnapshot>,
    locale: Mutex<LocaleSnapshot>,
}

impl SystemEnvBackend for SkiaSystemEnvBackend {
    fn get_theme(&self) -> Result<ThemeSnapshot, ServiceError> {
        Ok(*self.theme.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn get_locale(&self) -> Result<LocaleSnapshot, ServiceError> {
        Ok(self
            .locale
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone())
    }
}

/// Shared direct window-command queue & sink exposed via [`WindowServiceBackend`].
struct SkiaWindowBackend {
    window_sink: Mutex<Option<WindowCommandSink>>,
    queued_commands: Mutex<VecDeque<WindowCommand>>,
    pending_close_responses: Mutex<BTreeSet<WindowHandleId>>,
}

impl SkiaWindowBackend {
    fn new() -> Self {
        Self {
            window_sink: Mutex::new(None),
            queued_commands: Mutex::new(VecDeque::new()),
            pending_close_responses: Mutex::new(BTreeSet::new()),
        }
    }

    fn submit_iter(&self, commands: impl IntoIterator<Item = WindowCommand>) -> usize {
        let sink = self
            .window_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut count = 0usize;
        for cmd in commands {
            if let WindowCommand::RespondCloseRequest { handle, .. } = &cmd {
                self.pending_close_responses
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(*handle);
            }
            if let Some(sink) = &sink {
                sink(cmd);
            } else {
                self.queued_commands
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push_back(cmd);
            }
            count += 1;
        }
        count
    }
}

impl WindowServiceBackend for SkiaWindowBackend {
    fn submit_commands(&self, commands: &[WindowCommand]) -> usize {
        self.submit_iter(commands.iter().cloned())
    }
}

/// Composite host-capability service bound to loaded NVE app modules via `NVE_IFACE_HOST = 5`
/// (`NveHostApi::bind_host`) and directly in-process via [`HostServiceProvider`].
///
/// Heap-pinned inside an `Arc<SkiaHostServices>` so its `NveHostCallbacks::host_ctx` pointer
/// remains valid and immovable across Tier-2 `cdylib` reloads and secondary window spawns.
pub struct SkiaHostServices {
    inner: Arc<dyn HostServiceProvider>,
    channel_bus: Arc<Mutex<NveChannelBus>>,
    typed_events: Arc<TypedEventBus>,
    window_backend: Arc<SkiaWindowBackend>,
    window_backend_typed: (TypeId, Arc<dyn Any + Send + Sync>),
    cached_env: Arc<SkiaSystemEnvBackend>,
    cached_env_typed: (TypeId, Arc<dyn Any + Send + Sync>),
    wake_sink: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    pending_app_wake: AtomicBool,
}

impl SkiaHostServices {
    /// Creates a host service backed by live OS [`HostServiceRegistry`] with all Tier-1 native
    /// capability modules registered, seeding retained typed theme and locale snapshots onto
    /// the [`TypedEventBus`].
    pub fn new() -> Arc<Self> {
        let native = Arc::new(HostServiceRegistry::new());
        native.register(NativeClipboard::new());
        native.register(NativeFilesystem::new());
        native.register(NativeSecureStore::new());
        native.register(NativeLinking::new());
        native.register(NativeSystemEnv::new());
        native.register(NativeHaptics::new());
        let bus = native.channel_bus();
        Self::with_provider(native, bus)
    }

    /// Creates a host service backed by a custom [`HostServiceProvider`] (such as
    /// `nve_host_services::MemoryHostServices` in tests) and shared [`NveChannelBus`].
    pub fn with_provider(
        inner: Arc<dyn HostServiceProvider>,
        channel_bus: Arc<Mutex<NveChannelBus>>,
    ) -> Arc<Self> {
        let env_client = SystemEnvClient::new(&*inner);
        let initial_theme = env_client.get_theme().unwrap_or_default();
        let initial_locale = env_client.get_locale().unwrap_or_default();

        let typed_events = Arc::new(TypedEventBus::new());
        SystemEnvBridge::publish_theme(&typed_events, initial_theme);
        SystemEnvBridge::publish_locale(&typed_events, initial_locale.clone());

        let cached_env = Arc::new(SkiaSystemEnvBackend {
            theme: Mutex::new(initial_theme),
            locale: Mutex::new(initial_locale),
        });
        let cached_env_typed = pack_typed::<dyn SystemEnvBackend>(cached_env.clone());

        let window_backend = Arc::new(SkiaWindowBackend::new());
        let window_backend_typed = pack_typed::<dyn WindowServiceBackend>(window_backend.clone());

        Arc::new(Self {
            inner,
            channel_bus,
            typed_events,
            window_backend,
            window_backend_typed,
            cached_env,
            cached_env_typed,
            wake_sink: Mutex::new(None),
            pending_app_wake: AtomicBool::new(false),
        })
    }

    /// Returns a C-ABI [`NveHostCallbacks`] table whose `host_ctx` points to this `Arc`'s
    /// immovable heap allocation. Valid for as long as at least one `Arc<SkiaHostServices>`
    /// clone remains alive.
    pub fn callbacks(self: &Arc<Self>) -> NveHostCallbacks {
        NveHostCallbacks {
            host_ctx: Arc::as_ptr(self) as *mut c_void,
            invoke: Some(skia_host_invoke),
            channel_publish: Some(skia_host_channel_publish),
        }
    }

    /// Attaches a `winit` [`EventLoopProxy`] as the live window command sink and async frame wake
    /// sink, immediately flushing any commands buffered prior to event-loop startup.
    pub fn attach_event_loop_proxy(&self, proxy: EventLoopProxy<HostWakeEvent>) {
        let wake_proxy = proxy.clone();
        self.attach_wake_sink(Arc::new(move || {
            let _ = wake_proxy.send_event(HostWakeEvent::AppWake);
        }));
        self.attach_window_sink(Arc::new(move |cmd| {
            let _ = proxy.send_event(HostWakeEvent::WindowCommand(cmd));
        }));
    }

    /// Attaches an async frame wake callback invoked when an app calls
    /// `NveHostCallbacks::invoke(SERVICE_WINDOW, METHOD_WINDOW_REQUEST_FRAME)`.
    pub fn attach_wake_sink(&self, sink: Arc<dyn Fn() + Send + Sync>) {
        let fire_now = {
            let mut guard = self.wake_sink.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(Arc::clone(&sink));
            self.pending_app_wake.load(Ordering::Acquire)
        };
        if fire_now {
            sink();
        }
    }

    /// Marks an asynchronous frame wake pending and invokes the attached wake sink if present.
    pub fn request_frame_wake(&self) {
        self.pending_app_wake.store(true, Ordering::Release);
        let sink = self
            .wake_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(cb) = sink {
            cb();
        }
    }

    /// Consumes and returns whether an async app frame wake was requested via `SERVICE_WINDOW`.
    pub fn take_pending_app_wake(&self) -> bool {
        self.pending_app_wake.swap(false, Ordering::AcqRel)
    }

    /// Attaches a custom window command sink and flushes any previously queued commands in FIFO order.
    pub fn attach_window_sink(&self, sink: WindowCommandSink) {
        let pending: Vec<WindowCommand> = {
            let mut sink_guard = self
                .window_backend
                .window_sink
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *sink_guard = Some(Arc::clone(&sink));
            let mut q = self
                .window_backend
                .queued_commands
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            q.drain(..).collect()
        };
        for cmd in pending {
            sink(cmd);
        }
    }

    /// Detaches any installed window command sink (subsequent commands buffer in the internal queue).
    pub fn detach_window_sink(&self) {
        let mut sink_guard = self
            .window_backend
            .window_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *sink_guard = None;
    }

    /// Submits a single typed [`WindowCommand`] directly with zero serialization.
    pub fn submit_window_command(&self, cmd: WindowCommand) {
        let _ = self.window_backend.submit_iter(std::iter::once(cmd));
    }

    /// Submits a batch of typed [`WindowCommand`]s directly with zero serialization.
    pub fn submit_window_commands(&self, commands: &[WindowCommand]) -> usize {
        self.window_backend.submit_commands(commands)
    }

    /// Drains all buffered window commands (used by headless hosts and unit tests).
    pub fn drain_queued_window_commands(&self) -> Vec<WindowCommand> {
        self.window_backend
            .queued_commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// Returns `true` if an app queued `WindowCommand::RespondCloseRequest` for `handle`.
    pub fn has_pending_close_response(&self, handle: WindowHandleId) -> bool {
        self.window_backend
            .pending_close_responses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&handle)
    }

    /// Clears and returns whether `handle` had a pending `RespondCloseRequest` queued.
    pub fn take_pending_close_response(&self, handle: WindowHandleId) -> bool {
        self.window_backend
            .pending_close_responses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&handle)
    }

    pub fn channel_bus(&self) -> Arc<Mutex<NveChannelBus>> {
        Arc::clone(&self.channel_bus)
    }

    pub fn typed_events(&self) -> &Arc<TypedEventBus> {
        &self.typed_events
    }

    pub fn current_theme(&self) -> ThemeSnapshot {
        *self
            .cached_env
            .theme
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn current_locale(&self) -> LocaleSnapshot {
        self.cached_env
            .locale
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Builds an initial typed [`SkiaHostEvent::SystemEnv`] carrying the current theme & locale.
    pub fn initial_env_event(&self) -> SkiaHostEvent {
        SkiaHostEvent::SystemEnv {
            theme: self.current_theme(),
            locale: self.current_locale(),
        }
    }

    /// Updates cached theme state from a live `winit` `ThemeChanged` event, publishes a retained
    /// typed [`ThemeSnapshot`] onto [`TypedEventBus`], and returns the updated snapshot.
    pub fn update_theme_from_winit(&self, theme: WinitTheme, scale_factor: f32) -> ThemeSnapshot {
        let updated = {
            let mut guard = self
                .cached_env
                .theme
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            guard.color_scheme = winit_theme_to_color_scheme(theme);
            guard.scale_factor_x100 = scale_to_x100(scale_factor);
            *guard
        };
        SystemEnvBridge::publish_theme(&self.typed_events, updated);
        updated
    }

    /// Updates `scale_factor_x100` on the cached theme snapshot if it changed, publishing the
    /// typed snapshot onto [`TypedEventBus`].
    pub fn update_scale_factor(&self, scale_factor: f32) -> Option<ThemeSnapshot> {
        let next_x100 = scale_to_x100(scale_factor);
        let updated = {
            let mut guard = self
                .cached_env
                .theme
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if guard.scale_factor_x100 == next_x100 {
                return None;
            }
            guard.scale_factor_x100 = next_x100;
            *guard
        };
        SystemEnvBridge::publish_theme(&self.typed_events, updated);
        Some(updated)
    }

    /// Replaces the cached theme snapshot and publishes a retained typed [`ThemeSnapshot`].
    pub fn update_theme(&self, theme: ThemeSnapshot) -> ThemeSnapshot {
        {
            let mut guard = self
                .cached_env
                .theme
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *guard = theme;
        }
        SystemEnvBridge::publish_theme(&self.typed_events, theme);
        theme
    }

    /// Replaces the cached locale snapshot and publishes a retained typed [`LocaleSnapshot`].
    pub fn update_locale(&self, locale: LocaleSnapshot) -> LocaleSnapshot {
        {
            let mut guard = self
                .cached_env
                .locale
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *guard = locale.clone();
        }
        SystemEnvBridge::publish_locale(&self.typed_events, locale.clone());
        locale
    }
}

impl HostServiceProvider for SkiaHostServices {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        if service_id == SERVICE_WINDOW {
            if method_id == METHOD_WINDOW_REQUEST_FRAME {
                self.request_frame_wake();
                return Ok(InvokeResult::Ready(Vec::new().into()));
            }
            return Err(ServiceError::Unsupported);
        }
        if service_id == SERVICE_SYSTEM_ENV {
            return dispatch_system_env_ffi(&*self.cached_env, method_id, request)
                .map(|bytes| InvokeResult::Ready(bytes.into()));
        }
        self.inner.invoke_raw(service_id, method_id, request)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        self.inner.channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.inner.poll_host_events()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        Some(&self.typed_events)
    }

    fn typed_service(
        &self,
        service_id: u32,
        method_id: u32,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        if service_id == SERVICE_WINDOW {
            return (self.window_backend_typed.0 == type_id)
                .then(|| Arc::clone(&self.window_backend_typed.1));
        }
        if service_id == SERVICE_SYSTEM_ENV {
            return (self.cached_env_typed.0 == type_id)
                .then(|| Arc::clone(&self.cached_env_typed.1));
        }
        self.inner.typed_service(service_id, method_id, type_id)
    }
}

unsafe extern "C" fn skia_host_invoke(
    host_ctx: *mut c_void,
    service_id: u32,
    method_id: u32,
    _request_id: u64,
    request: NveBufferSlice,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    let services = unsafe { &*(host_ctx as *const SkiaHostServices) };
    let req_bytes = unsafe { request.as_slice() };
    match services.invoke_raw(service_id, method_id, req_bytes) {
        Ok(InvokeResult::Ready(bytes)) => {
            if !out_len.is_null() {
                unsafe { *out_len = bytes.len() };
            }
            if bytes.len() > out_cap {
                return NveHostStatus::BufferTooSmall.as_i32();
            }
            if !bytes.is_empty() {
                if out_buf.is_null() {
                    return NveHostStatus::InvalidArg.as_i32();
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
                }
            }
            NveHostStatus::Ok.as_i32()
        }
        Ok(InvokeResult::Pending { .. }) => {
            if !out_len.is_null() {
                unsafe { *out_len = 0 };
            }
            NveHostStatus::Pending.as_i32()
        }
        Err(err) => err.to_status().as_i32(),
    }
}

unsafe extern "C" fn skia_host_channel_publish(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    let services = unsafe { &*(host_ctx as *const SkiaHostServices) };
    let bytes = unsafe { payload.as_slice() };
    let retain = (flags & NVE_CHANNEL_FLAG_RETAIN) != 0;
    match services.channel_publish(channel_id, retain, bytes) {
        Ok(()) => NveHostStatus::Ok.as_i32(),
        Err(err) => err.to_status().as_i32(),
    }
}
