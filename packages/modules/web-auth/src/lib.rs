//! In-App WebBrowser & OAuth Authentication (`ASWebAuthenticationSession` / Custom Tabs /
//! loopback) capability (`SERVICE_WEB_AUTH = 0x0027`) with direct zero-serialization
//! in-process dispatch ([`WebAuthBackend`]), cross-language `NVE_IFACE_HOST` binary codecs,
//! deterministic [`MemoryWebAuth`], and headless-safe [`NativeWebAuth`] handlers.

use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex};

use nve_host_services::{
    pack_typed, resolve_typed, validate_url, BinReader, BinWriter, HostServiceHandler,
    HostServiceProvider, ServiceError,
};

/// Built-in service ID for in-app web browser & OAuth authentication sessions.
pub const SERVICE_WEB_AUTH: u32 = 0x0027;

pub const METHOD_OPEN_IN_APP_BROWSER: u32 = 1;
pub const METHOD_AUTHENTICATE: u32 = 2;

/// Presentation style for [`WebAuthClient::open_in_app_browser`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserPresentation {
    Sheet = 0,
    FullScreen = 1,
    Popover = 2,
}

impl BrowserPresentation {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Sheet),
            1 => Some(Self::FullScreen),
            2 => Some(Self::Popover),
            _ => None,
        }
    }
}

/// Configuration for [`WebAuthClient::open_in_app_browser`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InAppBrowserOptions {
    pub presentation: BrowserPresentation,
    pub toolbar_color_rgba: Option<u32>,
    pub ephemeral_session: bool,
    pub show_title: bool,
}

impl Default for InAppBrowserOptions {
    fn default() -> Self {
        Self {
            presentation: BrowserPresentation::Sheet,
            toolbar_color_rgba: None,
            ephemeral_session: false,
            show_title: true,
        }
    }
}

/// Outcome of an OAuth / web authentication flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebAuthResult {
    CallbackUri(String),
    Cancelled,
}

/// Recorded in-app browser launch captured by [`MemoryWebAuth`] for test assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedBrowserLaunch {
    pub url: String,
    pub options: InAppBrowserOptions,
}

fn validate_http_url(url: &str) -> Result<(), ServiceError> {
    validate_url(url)?;
    let scheme = url
        .trim()
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();
    if scheme != "http" && scheme != "https" {
        return Err(ServiceError::InvalidArg(format!(
            "web browser URL scheme must be http or https, got '{scheme}'"
        )));
    }
    Ok(())
}

/// Validates an RFC 3986 callback URI scheme (e.g. `myapp`, `com.example.auth`).
pub fn validate_callback_scheme(scheme: &str) -> Result<(), ServiceError> {
    if scheme.is_empty()
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return Err(ServiceError::InvalidArg(format!(
            "invalid callback scheme '{scheme}'"
        )));
    }
    Ok(())
}

pub fn encode_open_browser_request(
    url: &str,
    options: &InAppBrowserOptions,
) -> Result<Vec<u8>, ServiceError> {
    validate_http_url(url)?;
    let mut w = BinWriter::with_capacity(16 + url.len());
    w.write_str(url)?;
    w.write_u8(options.presentation as u8);
    w.write_opt_u32(options.toolbar_color_rgba);
    w.write_bool(options.ephemeral_session);
    w.write_bool(options.show_title);
    Ok(w.into_bytes())
}

pub fn decode_open_browser_request(
    bytes: &[u8],
) -> Result<(String, InAppBrowserOptions), ServiceError> {
    let mut r = BinReader::new(bytes);
    let url = r.read_string()?;
    let pres_raw = r.read_u8()?;
    let presentation = BrowserPresentation::from_u8(pres_raw)
        .ok_or_else(|| ServiceError::Codec(format!("invalid BrowserPresentation {pres_raw}")))?;
    let toolbar_color_rgba = r.read_opt_u32()?;
    let ephemeral_session = r.read_bool()?;
    let show_title = r.read_bool()?;
    r.finish()?;
    validate_http_url(&url)?;
    Ok((
        url,
        InAppBrowserOptions {
            presentation,
            toolbar_color_rgba,
            ephemeral_session,
            show_title,
        },
    ))
}

pub fn encode_authenticate_request(
    auth_url: &str,
    callback_scheme: &str,
) -> Result<Vec<u8>, ServiceError> {
    validate_http_url(auth_url)?;
    validate_callback_scheme(callback_scheme)?;
    let mut w = BinWriter::with_capacity(8 + auth_url.len() + callback_scheme.len());
    w.write_str(auth_url)?;
    w.write_str(callback_scheme)?;
    Ok(w.into_bytes())
}

pub fn decode_authenticate_request(bytes: &[u8]) -> Result<(String, String), ServiceError> {
    let mut r = BinReader::new(bytes);
    let auth_url = r.read_string()?;
    let callback_scheme = r.read_string()?;
    r.finish()?;
    validate_http_url(&auth_url)?;
    validate_callback_scheme(&callback_scheme)?;
    Ok((auth_url, callback_scheme))
}

pub fn encode_web_auth_result(result: &WebAuthResult) -> Result<Vec<u8>, ServiceError> {
    let mut w = BinWriter::new();
    match result {
        WebAuthResult::CallbackUri(uri) => {
            validate_url(uri)?;
            w.write_u8(1);
            w.write_str(uri)?;
        }
        WebAuthResult::Cancelled => {
            w.write_u8(0);
        }
    }
    Ok(w.into_bytes())
}

pub fn decode_web_auth_result(bytes: &[u8]) -> Result<WebAuthResult, ServiceError> {
    let mut r = BinReader::new(bytes);
    let res = match r.read_u8()? {
        0 => WebAuthResult::Cancelled,
        1 => {
            let uri = r.read_string()?;
            validate_url(&uri)?;
            WebAuthResult::CallbackUri(uri)
        }
        other => {
            return Err(ServiceError::Codec(format!(
                "invalid WebAuthResult tag {other}"
            )));
        }
    };
    r.finish()?;
    Ok(res)
}

/// Direct typed in-process backend for [`SERVICE_WEB_AUTH`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait WebAuthBackend: Send + Sync {
    fn open_in_app_browser(
        &self,
        url: &str,
        options: &InAppBrowserOptions,
    ) -> Result<(), ServiceError>;
    fn authenticate(
        &self,
        auth_url: &str,
        callback_scheme: &str,
    ) -> Result<WebAuthResult, ServiceError>;
}

/// Bridges a [`WebAuthBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_web_auth_wire<B: WebAuthBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_OPEN_IN_APP_BROWSER => {
            let (url, options) = decode_open_browser_request(request)?;
            backend.open_in_app_browser(&url, &options)?;
            Ok(Vec::new())
        }
        METHOD_AUTHENTICATE => {
            let (auth_url, callback_scheme) = decode_authenticate_request(request)?;
            encode_web_auth_result(&backend.authenticate(&auth_url, &callback_scheme)?)
        }
        _ => Err(ServiceError::Unsupported),
    }
}

/// Typed ergonomic client for in-app web browsers and OAuth authentication sessions.
#[derive(Debug, Clone, Copy)]
pub struct WebAuthClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> WebAuthClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn open_in_app_browser(
        &self,
        url: &str,
        options: &InAppBrowserOptions,
    ) -> Result<(), ServiceError> {
        validate_http_url(url)?;
        if let Some(backend) = resolve_typed::<dyn WebAuthBackend>(
            self.provider,
            SERVICE_WEB_AUTH,
            METHOD_OPEN_IN_APP_BROWSER,
        ) {
            return backend.open_in_app_browser(url, options);
        }
        let req = encode_open_browser_request(url, options)?;
        let resp =
            self.provider
                .invoke_sync(SERVICE_WEB_AUTH, METHOD_OPEN_IN_APP_BROWSER, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty web_auth::open_in_app_browser response".into(),
            ));
        }
        Ok(())
    }

    pub fn authenticate(
        &self,
        auth_url: &str,
        callback_scheme: &str,
    ) -> Result<WebAuthResult, ServiceError> {
        validate_http_url(auth_url)?;
        validate_callback_scheme(callback_scheme)?;
        if let Some(backend) = resolve_typed::<dyn WebAuthBackend>(
            self.provider,
            SERVICE_WEB_AUTH,
            METHOD_AUTHENTICATE,
        ) {
            return match backend.authenticate(auth_url, callback_scheme) {
                Ok(res) => Ok(res),
                Err(ServiceError::Cancelled) => Ok(WebAuthResult::Cancelled),
                Err(err) => Err(err),
            };
        }
        let req = encode_authenticate_request(auth_url, callback_scheme)?;
        match self
            .provider
            .invoke_sync(SERVICE_WEB_AUTH, METHOD_AUTHENTICATE, &req)
        {
            Ok(resp) => decode_web_auth_result(&resp),
            Err(ServiceError::Cancelled) => Ok(WebAuthResult::Cancelled),
            Err(err) => Err(err),
        }
    }
}

#[derive(Default)]
struct MemoryWebAuthState {
    opened_browsers: Vec<RecordedBrowserLaunch>,
    web_auth_result: Option<WebAuthResult>,
    web_auth_requests: Vec<(String, String)>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_WEB_AUTH`].
#[derive(Default)]
pub struct MemoryWebAuth {
    state: Mutex<MemoryWebAuthState>,
}

impl MemoryWebAuth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn opened_browsers(&self) -> Vec<RecordedBrowserLaunch> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .opened_browsers
            .clone()
    }

    pub fn set_result(&self, result: Option<WebAuthResult>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .web_auth_result = result;
    }

    pub fn requests(&self) -> Vec<(String, String)> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .web_auth_requests
            .clone()
    }
}

impl WebAuthBackend for MemoryWebAuth {
    fn open_in_app_browser(
        &self,
        url: &str,
        options: &InAppBrowserOptions,
    ) -> Result<(), ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.opened_browsers.push(RecordedBrowserLaunch {
            url: url.to_owned(),
            options: *options,
        });
        Ok(())
    }

    fn authenticate(
        &self,
        auth_url: &str,
        callback_scheme: &str,
    ) -> Result<WebAuthResult, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.web_auth_requests
            .push((auth_url.to_owned(), callback_scheme.to_owned()));
        Ok(st.web_auth_result.clone().unwrap_or_else(|| {
            WebAuthResult::CallbackUri(format!("{callback_scheme}://callback?code=mock_auth_code"))
        }))
    }
}

impl HostServiceHandler for MemoryWebAuth {
    fn service_id(&self) -> u32 {
        SERVICE_WEB_AUTH
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_web_auth_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn WebAuthBackend>(self))
    }
}

/// Safe non-modal native [`HostServiceHandler`] for [`SERVICE_WEB_AUTH`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeWebAuth;

impl NativeWebAuth {
    pub const fn new() -> Self {
        Self
    }
}

impl WebAuthBackend for NativeWebAuth {
    fn open_in_app_browser(
        &self,
        _url: &str,
        _options: &InAppBrowserOptions,
    ) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn authenticate(
        &self,
        _auth_url: &str,
        _callback_scheme: &str,
    ) -> Result<WebAuthResult, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

impl HostServiceHandler for NativeWebAuth {
    fn service_id(&self) -> u32 {
        SERVICE_WEB_AUTH
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_web_auth_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn WebAuthBackend>(self))
    }
}
