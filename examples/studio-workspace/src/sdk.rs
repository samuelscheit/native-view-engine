//! Native platform capability integration (`nve-host-services` + `nve-clipboard` +
//! `nve-system-env`) for **Aether Studio Workspace**.
//!
//! Wires [`ClipboardClient`] (Markdown export + deep-link route URLs) and [`SystemEnvClient`]
//! (theme + locale sync) across both:
//! - Bound `NVE_IFACE_HOST = 5` native C ABI callbacks (`FfiHostServices`) when hosted, and
//! - Direct zero-serialization in-process [`MemoryHostServices`] fallback for headless/WASM/standalone
//!   execution & tests.

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::NveHostCallbacks;
use nve_clipboard::{ClipboardClient, MemoryClipboard};
use nve_host_services::{
    FfiHostServices, HostEvent, HostServiceProvider, InvokeResult, MemoryHostServices,
    ServiceError, TypedEventBus,
};
use nve_system_env::{
    ColorScheme, LocaleSnapshot, MemorySystemEnv, SystemEnvBridge, SystemEnvClient, SystemEnvEvent,
    ThemeSnapshot,
};

/// Synchronized host system environment snapshot (theme + locale) sourced via [`SystemEnvClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioSystemEnv {
    pub theme: ThemeSnapshot,
    pub locale: LocaleSnapshot,
}

impl Default for StudioSystemEnv {
    fn default() -> Self {
        Self {
            theme: ThemeSnapshot {
                color_scheme: Some(ColorScheme::Dark),
                ..ThemeSnapshot::default()
            },
            locale: LocaleSnapshot::default(),
        }
    }
}

/// Summary returned when applying or draining typed [`SystemEnvEvent`] updates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostEventSyncReport {
    pub processed_events: usize,
    pub updated_theme: Option<ThemeSnapshot>,
    pub updated_locale: Option<LocaleSnapshot>,
}

impl HostEventSyncReport {
    pub const fn env_changed(&self) -> bool {
        self.updated_theme.is_some() || self.updated_locale.is_some()
    }
}

/// Dual-mode host service facade for Aether Studio Workspace.
///
/// Automatically routes every [`ClipboardClient`] and [`SystemEnvClient`] call through bound
/// `NVE_IFACE_HOST = 5` callbacks (`FfiHostServices`) when hosted, and falls back seamlessly to
/// direct zero-serialization [`MemoryHostServices`] in headless/WASM/standalone runs.
pub struct StudioHostServices {
    ffi: FfiHostServices,
    fallback: Arc<dyn HostServiceProvider>,
    memory_fallback: Option<Arc<MemoryHostServices>>,
}

impl Default for StudioHostServices {
    fn default() -> Self {
        Self::new()
    }
}

impl StudioHostServices {
    /// Creates a [`StudioHostServices`] backed by a Studio-seeded [`MemoryHostServices`] fallback
    /// (dark theme + `en-US` locale) until a native host binds `NVE_IFACE_HOST = 5`.
    pub fn new() -> Self {
        let mem = Arc::new(MemoryHostServices::new());
        let default_env = StudioSystemEnv::default();
        mem.register(MemoryClipboard::new());
        mem.register(MemorySystemEnv::with_initial(
            default_env.theme,
            default_env.locale,
        ));
        Self::with_memory(mem)
    }

    /// Creates a [`StudioHostServices`] using an explicit [`MemoryHostServices`] fallback
    /// (ideal for deterministic tests inspecting clipboard & theme/locale sync).
    pub fn with_memory(memory: Arc<MemoryHostServices>) -> Self {
        let fallback: Arc<dyn HostServiceProvider> = memory.clone();
        Self {
            ffi: FfiHostServices::new(),
            fallback,
            memory_fallback: Some(memory),
        }
    }

    /// Creates a [`StudioHostServices`] using any custom [`HostServiceProvider`] fallback.
    pub fn with_fallback(fallback: Arc<dyn HostServiceProvider>) -> Self {
        Self {
            ffi: FfiHostServices::new(),
            fallback,
            memory_fallback: None,
        }
    }

    /// Binds or unbinds `NVE_IFACE_HOST = 5` native host callbacks.
    pub fn bind_host(&self, callbacks: Option<NveHostCallbacks>) {
        self.ffi.bind(callbacks);
    }

    /// Returns `true` when `NVE_IFACE_HOST = 5` native host callbacks are currently bound.
    pub fn is_ffi_bound(&self) -> bool {
        self.ffi.is_bound()
    }

    /// Returns the underlying [`MemoryHostServices`] fallback, if configured.
    pub fn memory_fallback(&self) -> Option<&Arc<MemoryHostServices>> {
        self.memory_fallback.as_ref()
    }

    /// Returns a typed [`ClipboardClient`] bound to this dual-mode provider.
    pub const fn clipboard(&self) -> ClipboardClient<'_, Self> {
        ClipboardClient::new(self)
    }

    /// Returns a typed [`SystemEnvClient`] bound to this dual-mode provider.
    pub const fn system_env(&self) -> SystemEnvClient<'_, Self> {
        SystemEnvClient::new(self)
    }

    /// Writes `text` to the active host or memory clipboard via [`ClipboardClient`].
    pub fn copy_text(&self, text: &str) -> Result<(), ServiceError> {
        self.clipboard().write_text(text)
    }

    /// Reads text back from the active host or memory clipboard via [`ClipboardClient`].
    pub fn read_clipboard_text(&self) -> Result<Option<String>, ServiceError> {
        self.clipboard().read_text()
    }

    /// Queries the live system theme and locale via [`SystemEnvClient`].
    pub fn fetch_system_env(&self) -> Result<StudioSystemEnv, ServiceError> {
        let client = self.system_env();
        let theme = client.get_theme()?;
        let locale = client.get_locale()?;
        Ok(StudioSystemEnv { theme, locale })
    }

    /// Applies direct typed [`SystemEnvEvent`]s and publishes them onto the active
    /// [`TypedEventBus`] with zero byte serialization.
    pub fn dispatch_system_env_events(&self, events: &[SystemEnvEvent]) -> HostEventSyncReport {
        let mut report = HostEventSyncReport {
            processed_events: events.len(),
            ..HostEventSyncReport::default()
        };
        let bus = self.typed_event_bus();
        for ev in events {
            match ev {
                SystemEnvEvent::ThemeChanged(theme) => {
                    report.updated_theme = Some(*theme);
                    if let Some(bus) = bus {
                        SystemEnvBridge::publish_theme(bus, *theme);
                    }
                }
                SystemEnvEvent::LocaleChanged(locale) => {
                    report.updated_locale = Some(locale.clone());
                    if let Some(bus) = bus {
                        SystemEnvBridge::publish_locale(bus, locale.clone());
                    }
                }
            }
        }
        report
    }

    /// Drains any queued typed [`SystemEnvEvent`]s from the active [`TypedEventBus`].
    pub fn drain_system_env_events(&self) -> HostEventSyncReport {
        let events = self.system_env().drain_events();
        let mut report = HostEventSyncReport {
            processed_events: events.len(),
            ..HostEventSyncReport::default()
        };
        for ev in events {
            match ev.as_ref() {
                SystemEnvEvent::ThemeChanged(theme) => report.updated_theme = Some(*theme),
                SystemEnvEvent::LocaleChanged(locale) => {
                    report.updated_locale = Some(locale.clone())
                }
            }
        }
        report
    }
}

impl HostServiceProvider for StudioHostServices {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        if self.ffi.is_bound() {
            self.ffi.invoke_raw(service_id, method_id, request)
        } else {
            self.fallback.invoke_raw(service_id, method_id, request)
        }
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        if self.ffi.is_bound() {
            self.ffi.channel_publish(channel_id, retain, payload)
        } else {
            self.fallback.channel_publish(channel_id, retain, payload)
        }
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        if self.ffi.is_bound() {
            self.ffi.poll_host_events()
        } else {
            self.fallback.poll_host_events()
        }
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        if self.ffi.is_bound() {
            self.ffi.typed_event_bus()
        } else {
            self.fallback.typed_event_bus()
        }
    }

    fn typed_service(
        &self,
        service_id: u32,
        method_id: u32,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        if self.ffi.is_bound() {
            None
        } else {
            self.fallback.typed_service(service_id, method_id, type_id)
        }
    }
}
