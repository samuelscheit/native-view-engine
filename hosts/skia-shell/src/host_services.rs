//! Zero-JSON `NVE_IFACE_HOST = 5` host-capability bridge for `hosts/skia-shell`.
//!
//! Combines `nve_sdk::NativeHostServices` (clipboard, filesystem, secure store, linking,
//! system theme/locale, notifications, haptics, `NveChannelBus`) with `SERVICE_WINDOW = 0x0002`
//! multi-window command interception (`NVW1`) and `NVH1` live lifecycle / theme (`NVT2`) /
//! locale (`NVL1`) push encoding.

use std::collections::{BTreeSet, VecDeque};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use nve_abi::{
    NveBufferSlice, NveHostCallbacks, NveHostEventBatch, NveHostStatus, NVE_CHANNEL_FLAG_RETAIN,
    SERVICE_SYSTEM_ENV, SERVICE_WINDOW,
};
use nve_interop::{
    decode_window_commands, encode_window_events, NveChannelBus, WindowCommand, WindowHandleId,
    WindowLifecycleEvent, CHANNEL_ID_LOCALE, CHANNEL_ID_THEME,
};
use nve_sdk::system_env::{
    ColorScheme, LocaleSnapshot, SystemEnvBridge, SystemEnvClient, ThemeSnapshot, METHOD_GET_LOCALE,
    METHOD_GET_THEME,
};
use nve_sdk::{HostEvent, HostServiceProvider, InvokeResult, NativeHostServices, SdkError};
use winit::event_loop::EventLoopProxy;
use winit::window::Theme as WinitTheme;

use crate::window::HostWakeEvent;

/// `SERVICE_WINDOW = 0x0002` method ID for dispatching an `NVW1` binary [`WindowCommand`] batch.
pub const METHOD_WINDOW_DISPATCH_BATCH: u32 = 1;

/// `SERVICE_WINDOW = 0x0002` `NVH1` `HostPushEvent` event ID carrying an `NVW1` binary
/// [`WindowLifecycleEvent`] batch.
pub const EVENT_WINDOW_LIFECYCLE: u32 = 1;

/// Callback invoked whenever an app submits a decoded [`WindowCommand`] over `SERVICE_WINDOW`.
pub type WindowCommandSink = Arc<dyn Fn(WindowCommand) + Send + Sync>;

/// Maps a `winit::window::Theme` to `nve_sdk`'s `NVT2` [`ColorScheme`].
pub const fn winit_theme_to_color_scheme(theme: WinitTheme) -> ColorScheme {
    match theme {
        WinitTheme::Light => ColorScheme::Light,
        WinitTheme::Dark => ColorScheme::Dark,
    }
}

/// Encodes a retained 16-byte `NVT2` theme update as a binary `NVH1` `ChannelMessage` batch
/// on `CHANNEL_ID_THEME = 1`.
pub fn encode_theme_channel_batch(theme: &ThemeSnapshot) -> Vec<u8> {
    let mut batch = NveHostEventBatch::new();
    batch.push_channel_message(
        CHANNEL_ID_THEME,
        NVE_CHANNEL_FLAG_RETAIN,
        theme.encode_nvt2().to_vec(),
    );
    batch.encode()
}

/// Encodes a retained binary `NVL1` locale update as a binary `NVH1` `ChannelMessage` batch
/// on `CHANNEL_ID_LOCALE = 2`.
pub fn encode_locale_channel_batch(locale: &LocaleSnapshot) -> Vec<u8> {
    let mut batch = NveHostEventBatch::new();
    if let Ok(nvl1) = locale.encode_nvl1() {
        batch.push_channel_message(CHANNEL_ID_LOCALE, NVE_CHANNEL_FLAG_RETAIN, nvl1);
    }
    batch.encode()
}

/// Encodes initial retained theme (`NVT2` on `CHANNEL_ID_THEME`) and locale (`NVL1` on
/// `CHANNEL_ID_LOCALE`) into a single `NVH1` batch dispatched when a surface binds or reloads.
pub fn encode_system_env_batch(theme: &ThemeSnapshot, locale: &LocaleSnapshot) -> Vec<u8> {
    let mut batch = NveHostEventBatch::new();
    batch.push_channel_message(
        CHANNEL_ID_THEME,
        NVE_CHANNEL_FLAG_RETAIN,
        theme.encode_nvt2().to_vec(),
    );
    if let Ok(nvl1) = locale.encode_nvl1() {
        batch.push_channel_message(CHANNEL_ID_LOCALE, NVE_CHANNEL_FLAG_RETAIN, nvl1);
    }
    batch.encode()
}

/// Encodes [`WindowLifecycleEvent`]s as an `NVW1` binary payload wrapped in an `NVH1`
/// `HostPushEvent` for `SERVICE_WINDOW = 0x0002`.
pub fn encode_window_lifecycle_batch(events: &[WindowLifecycleEvent]) -> Vec<u8> {
    let mut batch = NveHostEventBatch::new();
    batch.push_host_event(
        SERVICE_WINDOW,
        EVENT_WINDOW_LIFECYCLE,
        encode_window_events(events),
    );
    batch.encode()
}

fn scale_to_x100(scale_factor: f32) -> u16 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        (scale_factor * 100.0).round().clamp(1.0, f32::from(u16::MAX)) as u16
    } else {
        100
    }
}

/// Composite zero-JSON host-capability service bound to loaded NVE app modules via
/// `NVE_IFACE_HOST = 5` (`NveHostApi::bind_host`).
///
/// Heap-pinned inside an `Arc<SkiaHostServices>` so its `NveHostCallbacks::host_ctx` pointer
/// remains valid and immovable across Tier-2 `cdylib` reloads and secondary window spawns.
pub struct SkiaHostServices {
    inner: Arc<dyn HostServiceProvider>,
    channel_bus: Arc<Mutex<NveChannelBus>>,
    window_sink: Mutex<Option<WindowCommandSink>>,
    queued_commands: Mutex<VecDeque<WindowCommand>>,
    pending_close_responses: Mutex<BTreeSet<WindowHandleId>>,
    current_theme: Mutex<ThemeSnapshot>,
    current_locale: Mutex<LocaleSnapshot>,
}

impl SkiaHostServices {
    /// Creates a host service backed by live OS [`NativeHostServices`], seeding retained
    /// `NVT2` theme and `NVL1` locale snapshots onto the shared [`NveChannelBus`].
    pub fn new() -> Arc<Self> {
        let native = Arc::new(NativeHostServices::new());
        let bus = native.channel_bus();
        Self::with_provider(native, bus)
    }

    /// Creates a host service backed by a custom [`HostServiceProvider`] (such as
    /// `nve_sdk::MemoryHostServices` in tests) and shared [`NveChannelBus`].
    pub fn with_provider(
        inner: Arc<dyn HostServiceProvider>,
        channel_bus: Arc<Mutex<NveChannelBus>>,
    ) -> Arc<Self> {
        let env_client = SystemEnvClient::new(&*inner);
        let initial_theme = env_client.get_theme().unwrap_or_default();
        let initial_locale = env_client.get_locale().unwrap_or_default();

        {
            let mut bus = channel_bus.lock().unwrap_or_else(|e| e.into_inner());
            let _ = SystemEnvBridge::publish_theme(&mut bus, &initial_theme);
            let _ = SystemEnvBridge::publish_locale(&mut bus, &initial_locale);
        }

        Arc::new(Self {
            inner,
            channel_bus,
            window_sink: Mutex::new(None),
            queued_commands: Mutex::new(VecDeque::new()),
            pending_close_responses: Mutex::new(BTreeSet::new()),
            current_theme: Mutex::new(initial_theme),
            current_locale: Mutex::new(initial_locale),
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

    /// Attaches a `winit` [`EventLoopProxy`] as the live `SERVICE_WINDOW` command sink and
    /// immediately flushes any commands buffered prior to event-loop startup.
    pub fn attach_event_loop_proxy(&self, proxy: EventLoopProxy<HostWakeEvent>) {
        self.attach_window_sink(Arc::new(move |cmd| {
            let _ = proxy.send_event(HostWakeEvent::WindowCommand(cmd));
        }));
    }

    /// Attaches a custom `SERVICE_WINDOW` command sink and flushes any previously queued
    /// commands in FIFO order.
    pub fn attach_window_sink(&self, sink: WindowCommandSink) {
        let pending: Vec<WindowCommand> = {
            let mut sink_guard = self.window_sink.lock().unwrap_or_else(|e| e.into_inner());
            *sink_guard = Some(Arc::clone(&sink));
            let mut q = self.queued_commands.lock().unwrap_or_else(|e| e.into_inner());
            q.drain(..).collect()
        };
        for cmd in pending {
            sink(cmd);
        }
    }

    /// Detaches any installed window command sink (subsequent `SERVICE_WINDOW` commands buffer
    /// in the internal queue).
    pub fn detach_window_sink(&self) {
        let mut sink_guard = self.window_sink.lock().unwrap_or_else(|e| e.into_inner());
        *sink_guard = None;
    }

    /// Drains all buffered `SERVICE_WINDOW` commands (used by headless hosts and unit tests).
    pub fn drain_queued_window_commands(&self) -> Vec<WindowCommand> {
        self.queued_commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    /// Returns `true` if an app queued `WindowCommand::RespondCloseRequest` for `handle`.
    pub fn has_pending_close_response(&self, handle: WindowHandleId) -> bool {
        self.pending_close_responses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&handle)
    }

    /// Clears and returns whether `handle` had a pending `RespondCloseRequest` queued.
    pub fn take_pending_close_response(&self, handle: WindowHandleId) -> bool {
        self.pending_close_responses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&handle)
    }

    pub fn channel_bus(&self) -> Arc<Mutex<NveChannelBus>> {
        Arc::clone(&self.channel_bus)
    }

    pub fn current_theme(&self) -> ThemeSnapshot {
        *self.current_theme.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn current_locale(&self) -> LocaleSnapshot {
        self.current_locale
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Encodes the current retained theme (`NVT2`) and locale (`NVL1`) into an `NVH1` batch.
    pub fn encode_initial_env_batch(&self) -> Vec<u8> {
        let theme = self.current_theme();
        let locale = self.current_locale();
        encode_system_env_batch(&theme, &locale)
    }

    /// Updates cached theme state from a live `winit` `ThemeChanged` event, publishes retained
    /// `NVT2` onto `NveChannelBus`, and returns `(ThemeSnapshot, nvh1_batch)`.
    pub fn update_theme_from_winit(
        &self,
        theme: WinitTheme,
        scale_factor: f32,
    ) -> (ThemeSnapshot, Vec<u8>) {
        let updated = {
            let mut guard = self.current_theme.lock().unwrap_or_else(|e| e.into_inner());
            guard.color_scheme = winit_theme_to_color_scheme(theme);
            guard.scale_factor_x100 = scale_to_x100(scale_factor);
            *guard
        };
        let batch = self.publish_theme_snapshot(&updated);
        (updated, batch)
    }

    /// Updates `scale_factor_x100` on the cached theme snapshot if it changed, returning the
    /// updated snapshot and encoded `NVH1` `NVT2` batch.
    pub fn update_scale_factor(&self, scale_factor: f32) -> Option<(ThemeSnapshot, Vec<u8>)> {
        let next_x100 = scale_to_x100(scale_factor);
        let updated = {
            let mut guard = self.current_theme.lock().unwrap_or_else(|e| e.into_inner());
            if guard.scale_factor_x100 == next_x100 {
                return None;
            }
            guard.scale_factor_x100 = next_x100;
            *guard
        };
        let batch = self.publish_theme_snapshot(&updated);
        Some((updated, batch))
    }

    /// Replaces the cached theme snapshot, publishes retained `NVT2` onto `NveChannelBus`, and
    /// returns the encoded `NVH1` `ChannelMessage` batch.
    pub fn update_theme(&self, theme: ThemeSnapshot) -> Vec<u8> {
        {
            let mut guard = self.current_theme.lock().unwrap_or_else(|e| e.into_inner());
            *guard = theme;
        }
        self.publish_theme_snapshot(&theme)
    }

    /// Replaces the cached locale snapshot, publishes retained `NVL1` onto `NveChannelBus`, and
    /// returns the encoded `NVH1` `ChannelMessage` batch.
    pub fn update_locale(&self, locale: LocaleSnapshot) -> Vec<u8> {
        {
            let mut guard = self.current_locale.lock().unwrap_or_else(|e| e.into_inner());
            *guard = locale.clone();
        }
        {
            let mut bus = self.channel_bus.lock().unwrap_or_else(|e| e.into_inner());
            let _ = SystemEnvBridge::publish_locale(&mut bus, &locale);
        }
        encode_locale_channel_batch(&locale)
    }

    fn publish_theme_snapshot(&self, theme: &ThemeSnapshot) -> Vec<u8> {
        {
            let mut bus = self.channel_bus.lock().unwrap_or_else(|e| e.into_inner());
            let _ = SystemEnvBridge::publish_theme(&mut bus, theme);
        }
        encode_theme_channel_batch(theme)
    }

    fn dispatch_window_service(
        &self,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError> {
        if method_id != METHOD_WINDOW_DISPATCH_BATCH {
            return Err(SdkError::Unsupported);
        }
        let commands =
            decode_window_commands(request).map_err(|e| SdkError::InvalidArg(e.to_string()))?;
        let count = commands.len() as u32;
        let sink = self
            .window_sink
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

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
        }
        Ok(InvokeResult::Ready(count.to_le_bytes().to_vec()))
    }
}

impl HostServiceProvider for SkiaHostServices {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError> {
        if service_id == SERVICE_WINDOW {
            return self.dispatch_window_service(method_id, request);
        }
        if service_id == SERVICE_SYSTEM_ENV {
            match method_id {
                METHOD_GET_THEME => {
                    return Ok(InvokeResult::Ready(
                        self.current_theme().encode_nvt2().to_vec(),
                    ));
                }
                METHOD_GET_LOCALE => {
                    return Ok(InvokeResult::Ready(self.current_locale().encode_nvl1()?));
                }
                _ => {}
            }
        }
        self.inner.invoke_raw(service_id, method_id, request)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), SdkError> {
        self.inner.channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.inner.poll_host_events()
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
