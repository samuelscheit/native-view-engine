//! Zero-dependency native OS host-capability backends (macOS, Windows, Unix/Linux).

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use nve_abi::{
    NveBufferSlice, NveHostCallbacks, NveHostStatus, NVE_CHANNEL_FLAG_RETAIN, SERVICE_CLIPBOARD,
    SERVICE_FILESYSTEM, SERVICE_HAPTICS, SERVICE_LINKING, SERVICE_NOTIFICATIONS,
    SERVICE_SECURE_STORE, SERVICE_SYSTEM_ENV,
};
use nve_interop::{ChannelPayload, NveChannelBus, PayloadEncoding};

use crate::clipboard;
use crate::error::SdkError;
use crate::filesystem;
use crate::haptics;
use crate::linking;
use crate::notifications;
use crate::provider::{HostEvent, HostServiceProvider, InvokeResult};
use crate::secure_store;
use crate::system_env::{self, LocaleSnapshot, TextDirection};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unix;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use unix as platform;

/// Parses a POSIX (`en_US.UTF-8`, `ar_EG@euro`) or BCP-47 (`en-US`, `he-IL`) locale string
/// into a normalized [`LocaleSnapshot`].
pub fn parse_locale_tag(
    raw: &str,
    timezone: Option<String>,
    uses_24h_clock: bool,
) -> LocaleSnapshot {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("C") || trimmed.eq_ignore_ascii_case("POSIX")
    {
        return LocaleSnapshot {
            timezone: timezone.unwrap_or_else(|| "UTC".to_owned()),
            uses_24h_clock,
            ..LocaleSnapshot::default()
        };
    }

    // Strip encoding (`.UTF-8`) and modifier (`@euro`) suffixes.
    let core = trimmed
        .split(['.', '@'])
        .next()
        .unwrap_or(trimmed)
        .replace('_', "-");
    let parts: Vec<&str> = core.split('-').filter(|p| !p.is_empty()).collect();
    let language = parts
        .first()
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "en".to_owned());

    let region = parts
        .iter()
        .skip(1)
        .find(|p| (p.len() == 2 && p.bytes().all(|b| b.is_ascii_alphabetic())) || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit())))
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_default();

    let tag = if region.is_empty() {
        language.clone()
    } else {
        format!("{language}-{region}")
    };

    let text_direction = if matches!(
        language.as_str(),
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "yi" | "dv"
    ) {
        TextDirection::Rtl
    } else {
        TextDirection::Ltr
    };

    LocaleSnapshot {
        tag,
        language,
        region,
        text_direction,
        uses_24h_clock,
        timezone: timezone.unwrap_or_else(|| "UTC".to_owned()),
    }
}

/// Native OS implementation of [`HostServiceProvider`] using zero third-party crates.
pub struct NativeHostServices {
    channel_bus: Arc<Mutex<NveChannelBus>>,
    events: Mutex<VecDeque<HostEvent>>,
}

impl Default for NativeHostServices {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeHostServices {
    pub fn new() -> Self {
        Self {
            channel_bus: Arc::new(Mutex::new(NveChannelBus::new())),
            events: Mutex::new(VecDeque::new()),
        }
    }

    pub fn with_channel_bus(channel_bus: Arc<Mutex<NveChannelBus>>) -> Self {
        Self {
            channel_bus,
            events: Mutex::new(VecDeque::new()),
        }
    }

    pub fn channel_bus(&self) -> Arc<Mutex<NveChannelBus>> {
        self.channel_bus.clone()
    }

    pub fn push_event(&self, event: HostEvent) {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).push_back(event);
    }
}

impl HostServiceProvider for NativeHostServices {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError> {
        let bytes = match (service_id, method_id) {
            (SERVICE_CLIPBOARD, clipboard::METHOD_READ_TEXT) => {
                let text = platform::clipboard_read_text()?;
                clipboard::encode_read_text_response(text.as_deref())?
            }
            (SERVICE_CLIPBOARD, clipboard::METHOD_WRITE_TEXT) => {
                let text = clipboard::decode_write_text_request(request)?;
                platform::clipboard_write_text(&text)?;
                Vec::new()
            }
            (SERVICE_CLIPBOARD, clipboard::METHOD_HAS_TEXT) => {
                clipboard::encode_has_text_response(platform::clipboard_has_text()?)
            }
            (SERVICE_CLIPBOARD, clipboard::METHOD_CLEAR) => {
                platform::clipboard_clear()?;
                Vec::new()
            }

            (SERVICE_FILESYSTEM, filesystem::METHOD_STANDARD_PATH) => {
                let kind = filesystem::decode_standard_path_request(request)?;
                let path = platform::standard_path(kind)?;
                filesystem::encode_standard_path_response(&path)?
            }
            (SERVICE_FILESYSTEM, filesystem::METHOD_PICK_FILES) => {
                let opts = filesystem::decode_file_picker_options(request)?;
                let paths = platform::pick_files(&opts)?;
                filesystem::encode_pick_files_response(&paths)?
            }
            (SERVICE_FILESYSTEM, filesystem::METHOD_SAVE_FILE) => {
                let opts = filesystem::decode_save_file_options(request)?;
                let saved = platform::save_file(&opts)?;
                filesystem::encode_save_file_response(saved.as_deref())?
            }

            (SERVICE_SECURE_STORE, secure_store::METHOD_GET) => {
                let (svc, acc) = secure_store::decode_key_request(request)?;
                let secret = platform::secure_store_get(&svc, &acc)?;
                secure_store::encode_get_response(secret.as_deref())?
            }
            (SERVICE_SECURE_STORE, secure_store::METHOD_SET) => {
                let (svc, acc, secret) = secure_store::decode_set_request(request)?;
                platform::secure_store_set(&svc, &acc, &secret)?;
                Vec::new()
            }
            (SERVICE_SECURE_STORE, secure_store::METHOD_DELETE) => {
                let (svc, acc) = secure_store::decode_key_request(request)?;
                secure_store::encode_delete_response(platform::secure_store_delete(&svc, &acc)?)
            }

            (SERVICE_LINKING, linking::METHOD_OPEN_URL) => {
                let url = linking::decode_url_request(request)?;
                platform::linking_open_url(&url)?;
                Vec::new()
            }
            (SERVICE_LINKING, linking::METHOD_CAN_OPEN_URL) => {
                let url = linking::decode_url_request(request)?;
                linking::encode_can_open_response(platform::linking_can_open_url(&url)?)
            }
            (SERVICE_LINKING, linking::METHOD_INITIAL_URL) => {
                linking::encode_initial_url_response(None)?
            }

            (SERVICE_SYSTEM_ENV, system_env::METHOD_GET_THEME) => {
                platform::system_env_get_theme()?.encode_nvt2().to_vec()
            }
            (SERVICE_SYSTEM_ENV, system_env::METHOD_GET_LOCALE) => {
                platform::system_env_get_locale()?.encode_nvl1()?
            }

            (SERVICE_NOTIFICATIONS, notifications::METHOD_REQUEST_PERMISSION) => {
                notifications::encode_permission_response(platform::notifications_request_permission()?)
            }
            (SERVICE_NOTIFICATIONS, notifications::METHOD_SHOW) => {
                let req = notifications::decode_show_request(request)?;
                platform::notifications_show(&req)?;
                Vec::new()
            }
            (SERVICE_NOTIFICATIONS, notifications::METHOD_CANCEL) => {
                let id = notifications::decode_cancel_request(request)?;
                platform::notifications_cancel(&id)?;
                Vec::new()
            }

            (SERVICE_HAPTICS, haptics::METHOD_IMPACT) => {
                let style = haptics::decode_impact_request(request)?;
                platform::haptics_impact(style)?;
                Vec::new()
            }
            (SERVICE_HAPTICS, haptics::METHOD_NOTIFICATION) => {
                let kind = haptics::decode_notification_haptic_request(request)?;
                platform::haptics_notification(kind)?;
                Vec::new()
            }
            (SERVICE_HAPTICS, haptics::METHOD_SELECTION) => {
                platform::haptics_selection()?;
                Vec::new()
            }

            _ => return Err(SdkError::Unsupported),
        };
        Ok(InvokeResult::Ready(bytes))
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), SdkError> {
        let mut bus = self.channel_bus.lock().unwrap_or_else(|e| e.into_inner());
        bus.publish_by_id(
            channel_id,
            None,
            PayloadEncoding::Binary,
            ChannelPayload::from_slice(payload),
            retain,
        )
        .map(|_| ())
        .map_err(|e| SdkError::InternalError(e.to_string()))
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }
}

/// Heap-pinned C-ABI bridge exposing any [`HostServiceProvider`] as [`NveHostCallbacks`]
/// suitable for `NveHostApi::bind_host`.
pub struct HostCallbackBridge<P: HostServiceProvider> {
    provider: Box<P>,
}

impl<P: HostServiceProvider> HostCallbackBridge<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider: Box::new(provider),
        }
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Returns a C-ABI [`NveHostCallbacks`] valid for as long as `self` remains alive and unmoved.
    pub fn callbacks(&self) -> NveHostCallbacks {
        let ptr: *const P = &*self.provider;
        NveHostCallbacks {
            host_ctx: ptr as *mut c_void,
            invoke: Some(bridge_invoke::<P>),
            channel_publish: Some(bridge_channel_publish::<P>),
        }
    }
}

unsafe extern "C" fn bridge_invoke<P: HostServiceProvider>(
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
    let provider = unsafe { &*(host_ctx as *const P) };
    let req_bytes = unsafe { request.as_slice() };
    match provider.invoke_raw(service_id, method_id, req_bytes) {
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

unsafe extern "C" fn bridge_channel_publish<P: HostServiceProvider>(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    let provider = unsafe { &*(host_ctx as *const P) };
    let bytes = unsafe { payload.as_slice() };
    let retain = (flags & NVE_CHANNEL_FLAG_RETAIN) != 0;
    match provider.channel_publish(channel_id, retain, bytes) {
        Ok(()) => NveHostStatus::Ok.as_i32(),
        Err(err) => err.to_status().as_i32(),
    }
}
