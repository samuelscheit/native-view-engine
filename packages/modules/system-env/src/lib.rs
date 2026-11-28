//! Live System Theme ([`ThemeSnapshot`]) and Locale ([`LocaleSnapshot`]) capability
//! (`SERVICE_SYSTEM_ENV = 0x0014`) with direct zero-serialization in-process dispatch
//! ([`SystemEnvBackend`]) and direct typed event delivery ([`SystemEnvEvent`] / [`SystemEnvBridge`]).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod native;

use std::any::{Any, TypeId};
use std::mem::{offset_of, size_of};
use std::sync::{Arc, Mutex};

pub use nve_abi::{ColorScheme, ContrastLevel, SERVICE_SYSTEM_ENV};
use nve_host_services::{
    pack_typed, resolve_typed, ByteReader, ByteWriter, CompletionSink, HandlerReply,
    HostServiceHandler, HostServiceProvider, ServiceError, TypedEventBus, TypedEventSubscription,
    MAX_PAYLOAD_BYTES,
};

pub use native::{parse_locale_tag, NativeSystemEnv};

pub const METHOD_GET_THEME: u32 = 1;
pub const METHOD_GET_LOCALE: u32 = 2;

pub const EVENT_THEME_CHANGED: u32 = 1;
pub const EVENT_LOCALE_CHANGED: u32 = 2;

/// Exact C-ABI POD size in bytes of [`ThemeSnapshot`].
pub const THEME_SNAPSHOT_REPR_C_LEN: usize = 12;

/// Direct `#[repr(C)]` system theme & display accessibility snapshot.
///
/// `color_scheme: Option<ColorScheme>` uses niche optimization (`size_of == 1`: `None = 0`,
/// `Some(Light) = 1`, `Some(Dark) = 2`), preserving the exact 12-byte C-ABI layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThemeSnapshot {
    pub color_scheme: Option<ColorScheme>,
    pub contrast: ContrastLevel,
    pub reduced_motion: bool,
    pub reduced_transparency: bool,
    pub accent_rgba: u32,
    pub font_scale_x1000: u16,
    pub scale_factor_x100: u16,
}

const _: () = {
    assert!(size_of::<ThemeSnapshot>() == THEME_SNAPSHOT_REPR_C_LEN);
    assert!(offset_of!(ThemeSnapshot, color_scheme) == 0);
    assert!(offset_of!(ThemeSnapshot, contrast) == 1);
    assert!(offset_of!(ThemeSnapshot, reduced_motion) == 2);
    assert!(offset_of!(ThemeSnapshot, reduced_transparency) == 3);
    assert!(offset_of!(ThemeSnapshot, accent_rgba) == 4);
    assert!(offset_of!(ThemeSnapshot, font_scale_x1000) == 8);
    assert!(offset_of!(ThemeSnapshot, scale_factor_x100) == 10);
};

impl Default for ThemeSnapshot {
    fn default() -> Self {
        Self {
            color_scheme: Some(ColorScheme::Light),
            contrast: ContrastLevel::Standard,
            reduced_motion: false,
            reduced_transparency: false,
            accent_rgba: 0x007A_FFFF,
            font_scale_x1000: 1000,
            scale_factor_x100: 100,
        }
    }
}

impl ThemeSnapshot {
    pub fn font_scale(&self) -> f32 {
        f32::from(self.font_scale_x1000) / 1000.0
    }

    pub fn scale_factor(&self) -> f32 {
        f32::from(self.scale_factor_x100) / 100.0
    }

    /// Packs this `#[repr(C)]` POD snapshot directly into its 12-byte C-ABI representation
    /// for cross-`cdylib` `NVE_IFACE_HOST = 5` boundaries.
    pub const fn to_repr_c_bytes(&self) -> [u8; THEME_SNAPSHOT_REPR_C_LEN] {
        let accent = self.accent_rgba.to_ne_bytes();
        let font = self.font_scale_x1000.to_ne_bytes();
        let scale = self.scale_factor_x100.to_ne_bytes();
        [
            ColorScheme::opt_to_u8(self.color_scheme),
            self.contrast as u8,
            self.reduced_motion as u8,
            self.reduced_transparency as u8,
            accent[0],
            accent[1],
            accent[2],
            accent[3],
            font[0],
            font[1],
            scale[0],
            scale[1],
        ]
    }

    /// Reads a validated [`ThemeSnapshot`] from a 12-byte `#[repr(C)]` C-ABI slice.
    pub fn from_repr_c_bytes(bytes: &[u8]) -> Result<Self, ServiceError> {
        if bytes.len() != THEME_SNAPSHOT_REPR_C_LEN {
            return Err(ServiceError::Codec(format!(
                "ThemeSnapshot C-ABI payload must be {THEME_SNAPSHOT_REPR_C_LEN} bytes, got {}",
                bytes.len()
            )));
        }
        let color_scheme = ColorScheme::opt_from_u8(bytes[0])
            .map_err(|raw| ServiceError::Codec(format!("invalid color_scheme {raw}")))?;
        let contrast = ContrastLevel::from_u8(bytes[1])
            .ok_or_else(|| ServiceError::Codec(format!("invalid contrast {}", bytes[1])))?;
        let reduced_motion = match bytes[2] {
            0 => false,
            1 => true,
            v => {
                return Err(ServiceError::Codec(format!(
                    "invalid reduced_motion bool {v}"
                )))
            }
        };
        let reduced_transparency = match bytes[3] {
            0 => false,
            1 => true,
            v => {
                return Err(ServiceError::Codec(format!(
                    "invalid reduced_transparency bool {v}"
                )))
            }
        };
        let accent_rgba = u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let font_scale_x1000 = u16::from_ne_bytes([bytes[8], bytes[9]]);
        let scale_factor_x100 = u16::from_ne_bytes([bytes[10], bytes[11]]);
        Ok(Self {
            color_scheme,
            contrast,
            reduced_motion,
            reduced_transparency,
            accent_rgba,
            font_scale_x1000,
            scale_factor_x100,
        })
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextDirection {
    #[default]
    Ltr = 0,
    Rtl = 1,
}

impl TextDirection {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Ltr),
            1 => Some(Self::Rtl),
            _ => None,
        }
    }
}

/// System locale & regional formatting snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocaleSnapshot {
    pub tag: String,
    pub language: String,
    pub region: String,
    pub text_direction: TextDirection,
    pub uses_24h_clock: bool,
    pub timezone: String,
}

/// Ergonomic alias for [`LocaleSnapshot`].
pub type LocaleInfo = LocaleSnapshot;

impl Default for LocaleSnapshot {
    fn default() -> Self {
        Self {
            tag: "en-US".to_owned(),
            language: "en".to_owned(),
            region: "US".to_owned(),
            text_direction: TextDirection::Ltr,
            uses_24h_clock: false,
            timezone: "UTC".to_owned(),
        }
    }
}

/// Direct typed system environment update event delivered with zero serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemEnvEvent {
    ThemeChanged(ThemeSnapshot),
    LocaleChanged(LocaleSnapshot),
}

pub fn pack_locale_ffi(locale: &LocaleSnapshot) -> Result<Vec<u8>, ServiceError> {
    let mut w = ByteWriter::with_capacity(
        18 + locale.tag.len()
            + locale.language.len()
            + locale.region.len()
            + locale.timezone.len(),
    );
    w.write_u8(locale.text_direction as u8);
    w.write_bool(locale.uses_24h_clock);
    w.write_str(&locale.tag)?;
    w.write_str(&locale.language)?;
    w.write_str(&locale.region)?;
    w.write_str(&locale.timezone)?;
    Ok(w.into_bytes())
}

pub fn unpack_locale_ffi(bytes: &[u8]) -> Result<LocaleSnapshot, ServiceError> {
    let mut r = ByteReader::new(bytes);
    let dir_raw = r.read_u8()?;
    let text_direction = TextDirection::from_u8(dir_raw)
        .ok_or_else(|| ServiceError::Codec(format!("invalid text_direction {dir_raw}")))?;
    let uses_24h_clock = r.read_bool()?;
    let tag = r.read_string(MAX_PAYLOAD_BYTES)?;
    let language = r.read_string(MAX_PAYLOAD_BYTES)?;
    let region = r.read_string(MAX_PAYLOAD_BYTES)?;
    let timezone = r.read_string(MAX_PAYLOAD_BYTES)?;
    r.finish()?;
    Ok(LocaleSnapshot {
        tag,
        language,
        region,
        text_direction,
        uses_24h_clock,
        timezone,
    })
}

/// Direct typed in-process backend for [`SERVICE_SYSTEM_ENV`].
///
/// In-process Rust callers invoke this trait directly via [`nve_host_services::resolve_typed`]
/// with zero serialization and zero byte allocation.
pub trait SystemEnvBackend: Send + Sync {
    fn get_theme(&self) -> Result<ThemeSnapshot, ServiceError>;
    fn get_locale(&self) -> Result<LocaleSnapshot, ServiceError>;
}

/// Bridges a [`SystemEnvBackend`] across a cross-`cdylib` `NVE_IFACE_HOST = 5` C-ABI boundary.
pub fn dispatch_system_env_ffi<B: SystemEnvBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    _request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_GET_THEME => Ok(backend.get_theme()?.to_repr_c_bytes().to_vec()),
        METHOD_GET_LOCALE => pack_locale_ffi(&backend.get_locale()?),
        _ => Err(ServiceError::Unsupported),
    }
}

/// Typed ergonomic client for querying and subscribing to live system theme and locale updates.
#[derive(Debug, Clone, Copy)]
pub struct SystemEnvClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> SystemEnvClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn get_theme(&self) -> Result<ThemeSnapshot, ServiceError> {
        if let Some(backend) = resolve_typed::<dyn SystemEnvBackend>(
            self.provider,
            SERVICE_SYSTEM_ENV,
            METHOD_GET_THEME,
        ) {
            return backend.get_theme();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_SYSTEM_ENV, METHOD_GET_THEME, &[])?;
        ThemeSnapshot::from_repr_c_bytes(&resp)
    }

    pub fn get_locale(&self) -> Result<LocaleSnapshot, ServiceError> {
        if let Some(backend) = resolve_typed::<dyn SystemEnvBackend>(
            self.provider,
            SERVICE_SYSTEM_ENV,
            METHOD_GET_LOCALE,
        ) {
            return backend.get_locale();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_SYSTEM_ENV, METHOD_GET_LOCALE, &[])?;
        unpack_locale_ffi(&resp)
    }

    /// Subscribes directly to typed [`ThemeSnapshot`] updates on the provider's [`TypedEventBus`].
    pub fn subscribe_theme<F>(
        &self,
        replay_retained: bool,
        handler: F,
    ) -> Result<TypedEventSubscription, ServiceError>
    where
        F: Fn(&ThemeSnapshot) + Send + Sync + 'static,
    {
        let bus = self
            .provider
            .typed_event_bus()
            .ok_or(ServiceError::Unsupported)?;
        Ok(SystemEnvBridge::subscribe_theme(
            bus,
            replay_retained,
            handler,
        ))
    }

    /// Subscribes directly to typed [`LocaleSnapshot`] updates on the provider's [`TypedEventBus`].
    pub fn subscribe_locale<F>(
        &self,
        replay_retained: bool,
        handler: F,
    ) -> Result<TypedEventSubscription, ServiceError>
    where
        F: Fn(&LocaleSnapshot) + Send + Sync + 'static,
    {
        let bus = self
            .provider
            .typed_event_bus()
            .ok_or(ServiceError::Unsupported)?;
        Ok(SystemEnvBridge::subscribe_locale(
            bus,
            replay_retained,
            handler,
        ))
    }

    /// Drains queued [`SystemEnvEvent`]s directly from the provider's [`TypedEventBus`].
    pub fn drain_events(&self) -> Vec<Arc<SystemEnvEvent>> {
        self.provider
            .typed_event_bus()
            .map(|bus| bus.drain_typed::<SystemEnvEvent>())
            .unwrap_or_default()
    }
}

/// Zero-serialization bridge delivering live [`ThemeSnapshot`] and [`LocaleSnapshot`] updates
/// over [`TypedEventBus`] with retained snapshot replay.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnvBridge;

impl SystemEnvBridge {
    pub const fn new() -> Self {
        Self
    }

    /// Publishes a retained [`ThemeSnapshot`] (notify-only, zero pull-queue growth) and queues
    /// [`SystemEnvEvent::ThemeChanged`] onto `bus` with zero byte encoding.
    pub fn publish_theme(bus: &TypedEventBus, theme: ThemeSnapshot) -> usize {
        let notified_snap = bus.publish_retained_notify(theme);
        let notified_ev = bus.publish(SystemEnvEvent::ThemeChanged(theme));
        notified_snap + notified_ev
    }

    /// Publishes a retained [`LocaleSnapshot`] (notify-only, zero pull-queue growth) and queues
    /// [`SystemEnvEvent::LocaleChanged`] onto `bus` with zero byte encoding.
    pub fn publish_locale(bus: &TypedEventBus, locale: LocaleSnapshot) -> usize {
        let notified_snap = bus.publish_retained_notify(locale.clone());
        let notified_ev = bus.publish(SystemEnvEvent::LocaleChanged(locale));
        notified_snap + notified_ev
    }

    /// Updates retained [`ThemeSnapshot`] & [`SystemEnvEvent::ThemeChanged`] and notifies live
    /// subscribers without appending to the pull queue (for host-side caches whose pull queue is
    /// not drained).
    pub fn publish_theme_snapshot(bus: &TypedEventBus, theme: ThemeSnapshot) -> usize {
        let notified_snap = bus.publish_retained_notify(theme);
        let notified_ev = bus.publish_retained_notify(SystemEnvEvent::ThemeChanged(theme));
        notified_snap + notified_ev
    }

    /// Updates retained [`LocaleSnapshot`] & [`SystemEnvEvent::LocaleChanged`] and notifies live
    /// subscribers without appending to the pull queue.
    pub fn publish_locale_snapshot(bus: &TypedEventBus, locale: LocaleSnapshot) -> usize {
        let notified_snap = bus.publish_retained_notify(locale.clone());
        let notified_ev = bus.publish_retained_notify(SystemEnvEvent::LocaleChanged(locale));
        notified_snap + notified_ev
    }

    /// Fetches the current theme and locale from `provider` and publishes both retained snapshots
    /// onto `bus` with zero serialization.
    pub fn sync_from_provider<P: HostServiceProvider + ?Sized>(
        &self,
        provider: &P,
        bus: &TypedEventBus,
    ) -> Result<(ThemeSnapshot, LocaleSnapshot), ServiceError> {
        let client = SystemEnvClient::new(provider);
        let theme = client.get_theme()?;
        let locale = client.get_locale()?;
        Self::publish_theme(bus, theme);
        Self::publish_locale(bus, locale.clone());
        Ok((theme, locale))
    }

    pub fn subscribe_theme<F>(
        bus: &TypedEventBus,
        replay_retained: bool,
        handler: F,
    ) -> TypedEventSubscription
    where
        F: Fn(&ThemeSnapshot) + Send + Sync + 'static,
    {
        bus.subscribe::<ThemeSnapshot, F>(replay_retained, handler)
    }

    pub fn subscribe_locale<F>(
        bus: &TypedEventBus,
        replay_retained: bool,
        handler: F,
    ) -> TypedEventSubscription
    where
        F: Fn(&LocaleSnapshot) + Send + Sync + 'static,
    {
        bus.subscribe::<LocaleSnapshot, F>(replay_retained, handler)
    }

    pub fn subscribe_events<F>(bus: &TypedEventBus, handler: F) -> TypedEventSubscription
    where
        F: Fn(&SystemEnvEvent) + Send + Sync + 'static,
    {
        bus.subscribe::<SystemEnvEvent, F>(false, handler)
    }
}

type EnvListener = Arc<dyn Fn(&SystemEnvEvent) + Send + Sync>;

struct MemorySystemEnvState {
    theme: ThemeSnapshot,
    locale: LocaleSnapshot,
    listeners: Vec<EnvListener>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_SYSTEM_ENV`].
pub struct MemorySystemEnv {
    state: Mutex<MemorySystemEnvState>,
}

impl Default for MemorySystemEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl MemorySystemEnv {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemorySystemEnvState {
                theme: ThemeSnapshot::default(),
                locale: LocaleSnapshot::default(),
                listeners: Vec::new(),
            }),
        }
    }

    pub fn with_initial(theme: ThemeSnapshot, locale: LocaleSnapshot) -> Self {
        Self {
            state: Mutex::new(MemorySystemEnvState {
                theme,
                locale,
                listeners: Vec::new(),
            }),
        }
    }

    /// Registers a direct typed callback invoked whenever [`Self::set_theme`] or
    /// [`Self::set_locale`] updates the in-memory environment snapshot.
    pub fn add_listener<F>(&self, listener: F)
    where
        F: Fn(&SystemEnvEvent) + Send + Sync + 'static,
    {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .listeners
            .push(Arc::new(listener));
    }

    pub fn set_theme(&self, theme: ThemeSnapshot) {
        let listeners = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.theme = theme;
            st.listeners.clone()
        };
        if !listeners.is_empty() {
            let ev = SystemEnvEvent::ThemeChanged(theme);
            for cb in listeners {
                cb(&ev);
            }
        }
    }

    pub fn set_locale(&self, locale: LocaleSnapshot) {
        let listeners = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.locale = locale.clone();
            st.listeners.clone()
        };
        if !listeners.is_empty() {
            let ev = SystemEnvEvent::LocaleChanged(locale);
            for cb in listeners {
                cb(&ev);
            }
        }
    }

    pub fn set_theme_and_publish(&self, bus: &TypedEventBus, theme: ThemeSnapshot) -> usize {
        self.set_theme(theme);
        SystemEnvBridge::publish_theme(bus, theme)
    }

    pub fn set_locale_and_publish(&self, bus: &TypedEventBus, locale: LocaleSnapshot) -> usize {
        self.set_locale(locale.clone());
        SystemEnvBridge::publish_locale(bus, locale)
    }

    pub fn theme(&self) -> ThemeSnapshot {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).theme
    }

    pub fn locale(&self) -> LocaleSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .locale
            .clone()
    }
}

impl SystemEnvBackend for MemorySystemEnv {
    fn get_theme(&self) -> Result<ThemeSnapshot, ServiceError> {
        Ok(self.theme())
    }

    fn get_locale(&self) -> Result<LocaleSnapshot, ServiceError> {
        Ok(self.locale())
    }
}

impl HostServiceHandler for MemorySystemEnv {
    fn service_id(&self) -> u32 {
        SERVICE_SYSTEM_ENV
    }

    fn handle(
        &self,
        method_id: u32,
        _request_id: u64,
        request: &[u8],
        _done: &CompletionSink,
    ) -> Result<HandlerReply, ServiceError> {
        dispatch_system_env_ffi(self, method_id, request).map(HandlerReply::ready)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn SystemEnvBackend>(self))
    }
}
