//! Native Share Sheet (`SERVICE_SHARE = 0x0026`) and In-App WebBrowser / OAuth Authentication
//! (`SERVICE_WEB_AUTH = 0x0027`) with zero-JSON binary codecs.

use crate::codec::{validate_url, BinReader, BinWriter};
use crate::error::SdkError;
use crate::provider::HostServiceProvider;

/// Built-in service ID for the native OS share sheet.
#[cfg(feature = "share")]
pub const SERVICE_SHARE: u32 = 0x0026;
/// Built-in service ID for in-app web browser & OAuth (`ASWebAuthenticationSession` / Custom Tabs).
#[cfg(feature = "web-auth")]
pub const SERVICE_WEB_AUTH: u32 = 0x0027;

#[cfg(feature = "share")]
pub const METHOD_SHARE_TEXT: u32 = 1;
#[cfg(feature = "share")]
pub const METHOD_SHARE_URL: u32 = 2;
#[cfg(feature = "share")]
pub const METHOD_SHARE_FILES: u32 = 3;

#[cfg(feature = "web-auth")]
pub const METHOD_OPEN_IN_APP_BROWSER: u32 = 1;
#[cfg(feature = "web-auth")]
pub const METHOD_AUTHENTICATE: u32 = 2;

/// Outcome of presenting the native OS share sheet.
#[cfg(feature = "share")]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShareResult {
    Completed = 0,
    Dismissed = 1,
}

#[cfg(feature = "share")]
impl ShareResult {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Completed),
            1 => Some(Self::Dismissed),
            _ => None,
        }
    }
}

/// Presentation style for [`WebAuthClient::open_in_app_browser`].
#[cfg(feature = "web-auth")]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserPresentation {
    Sheet = 0,
    FullScreen = 1,
    Popover = 2,
}

#[cfg(feature = "web-auth")]
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
#[cfg(feature = "web-auth")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InAppBrowserOptions {
    pub presentation: BrowserPresentation,
    pub toolbar_color_rgba: Option<u32>,
    pub ephemeral_session: bool,
    pub show_title: bool,
}

#[cfg(feature = "web-auth")]
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

/// Outcome of an OAuth / web authentication flow (`ASWebAuthenticationSession` / Custom Tabs / loopback).
#[cfg(feature = "web-auth")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebAuthResult {
    CallbackUri(String),
    Cancelled,
}

#[cfg(feature = "web-auth")]
fn validate_http_url(url: &str) -> Result<(), SdkError> {
    validate_url(url)?;
    let scheme = url
        .trim()
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();
    if scheme != "http" && scheme != "https" {
        return Err(SdkError::InvalidArg(format!(
            "web browser URL scheme must be http or https, got '{scheme}'"
        )));
    }
    Ok(())
}

/// Validates an RFC 3986 callback URI scheme (e.g. `myapp`, `com.example.auth`).
#[cfg(feature = "web-auth")]
pub fn validate_callback_scheme(scheme: &str) -> Result<(), SdkError> {
    if scheme.is_empty()
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return Err(SdkError::InvalidArg(format!(
            "invalid callback scheme '{scheme}'"
        )));
    }
    Ok(())
}

#[cfg(feature = "share")]
pub fn encode_share_text_request(text: &str, subject: Option<&str>) -> Result<Vec<u8>, SdkError> {
    if text.is_empty() {
        return Err(SdkError::InvalidArg("share text must not be empty".into()));
    }
    let mut w = BinWriter::with_capacity(8 + text.len());
    w.write_str(text)?;
    w.write_opt_str(subject)?;
    Ok(w.into_bytes())
}

#[cfg(feature = "share")]
pub fn decode_share_text_request(bytes: &[u8]) -> Result<(String, Option<String>), SdkError> {
    let mut r = BinReader::new(bytes);
    let text = r.read_string()?;
    let subject = r.read_opt_string()?;
    r.finish()?;
    if text.is_empty() {
        return Err(SdkError::InvalidArg("share text must not be empty".into()));
    }
    Ok((text, subject))
}

#[cfg(feature = "share")]
pub fn encode_share_url_request(url: &str, title: Option<&str>) -> Result<Vec<u8>, SdkError> {
    validate_url(url)?;
    let mut w = BinWriter::with_capacity(8 + url.len());
    w.write_str(url)?;
    w.write_opt_str(title)?;
    Ok(w.into_bytes())
}

#[cfg(feature = "share")]
pub fn decode_share_url_request(bytes: &[u8]) -> Result<(String, Option<String>), SdkError> {
    let mut r = BinReader::new(bytes);
    let url = r.read_string()?;
    let title = r.read_opt_string()?;
    r.finish()?;
    validate_url(&url)?;
    Ok((url, title))
}

#[cfg(feature = "share")]
pub fn encode_share_files_request<S: AsRef<str>>(paths: &[S]) -> Result<Vec<u8>, SdkError> {
    if paths.is_empty() {
        return Err(SdkError::InvalidArg(
            "share_files requires at least one file path".into(),
        ));
    }
    let count = u32::try_from(paths.len())
        .map_err(|_| SdkError::InvalidArg("share_files list exceeds u32::MAX".into()))?;
    let mut w = BinWriter::with_capacity(4 + paths.len() * 16);
    w.write_u32(count);
    for p in paths {
        let s = p.as_ref();
        if s.trim().is_empty() {
            return Err(SdkError::InvalidArg(
                "share_files path must not be empty".into(),
            ));
        }
        w.write_str(s)?;
    }
    Ok(w.into_bytes())
}

#[cfg(feature = "share")]
pub fn decode_share_files_request(bytes: &[u8]) -> Result<Vec<String>, SdkError> {
    let mut r = BinReader::new(bytes);
    let paths = r.read_string_vec()?;
    r.finish()?;
    if paths.is_empty() || paths.iter().any(|p| p.trim().is_empty()) {
        return Err(SdkError::InvalidArg(
            "share_files requires non-empty file paths".into(),
        ));
    }
    Ok(paths)
}

#[cfg(feature = "share")]
pub fn encode_share_result(result: ShareResult) -> Vec<u8> {
    vec![result as u8]
}

#[cfg(feature = "share")]
pub fn decode_share_result(bytes: &[u8]) -> Result<ShareResult, SdkError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    ShareResult::from_u8(raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid ShareResult {raw}")))
}

#[cfg(feature = "web-auth")]
pub fn encode_open_browser_request(
    url: &str,
    options: &InAppBrowserOptions,
) -> Result<Vec<u8>, SdkError> {
    validate_http_url(url)?;
    let mut w = BinWriter::with_capacity(16 + url.len());
    w.write_str(url)?;
    w.write_u8(options.presentation as u8);
    w.write_opt_u32(options.toolbar_color_rgba);
    w.write_bool(options.ephemeral_session);
    w.write_bool(options.show_title);
    Ok(w.into_bytes())
}

#[cfg(feature = "web-auth")]
pub fn decode_open_browser_request(
    bytes: &[u8],
) -> Result<(String, InAppBrowserOptions), SdkError> {
    let mut r = BinReader::new(bytes);
    let url = r.read_string()?;
    let pres_raw = r.read_u8()?;
    let presentation = BrowserPresentation::from_u8(pres_raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid BrowserPresentation {pres_raw}")))?;
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

#[cfg(feature = "web-auth")]
pub fn encode_authenticate_request(
    auth_url: &str,
    callback_scheme: &str,
) -> Result<Vec<u8>, SdkError> {
    validate_http_url(auth_url)?;
    validate_callback_scheme(callback_scheme)?;
    let mut w = BinWriter::with_capacity(8 + auth_url.len() + callback_scheme.len());
    w.write_str(auth_url)?;
    w.write_str(callback_scheme)?;
    Ok(w.into_bytes())
}

#[cfg(feature = "web-auth")]
pub fn decode_authenticate_request(bytes: &[u8]) -> Result<(String, String), SdkError> {
    let mut r = BinReader::new(bytes);
    let auth_url = r.read_string()?;
    let callback_scheme = r.read_string()?;
    r.finish()?;
    validate_http_url(&auth_url)?;
    validate_callback_scheme(&callback_scheme)?;
    Ok((auth_url, callback_scheme))
}

#[cfg(feature = "web-auth")]
pub fn encode_web_auth_result(result: &WebAuthResult) -> Result<Vec<u8>, SdkError> {
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

#[cfg(feature = "web-auth")]
pub fn decode_web_auth_result(bytes: &[u8]) -> Result<WebAuthResult, SdkError> {
    let mut r = BinReader::new(bytes);
    let res = match r.read_u8()? {
        0 => WebAuthResult::Cancelled,
        1 => {
            let uri = r.read_string()?;
            validate_url(&uri)?;
            WebAuthResult::CallbackUri(uri)
        }
        other => {
            return Err(SdkError::Codec(format!(
                "invalid WebAuthResult tag {other}"
            )));
        }
    };
    r.finish()?;
    Ok(res)
}

/// Typed ergonomic client for the native OS share sheet.
#[cfg(feature = "share")]
#[derive(Debug, Clone, Copy)]
pub struct ShareClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

#[cfg(feature = "share")]
impl<'a, P: HostServiceProvider + ?Sized> ShareClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn share_text(&self, text: &str, subject: Option<&str>) -> Result<ShareResult, SdkError> {
        let req = encode_share_text_request(text, subject)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SHARE, METHOD_SHARE_TEXT, &req)?;
        decode_share_result(&resp)
    }

    pub fn share_url(&self, url: &str, title: Option<&str>) -> Result<ShareResult, SdkError> {
        let req = encode_share_url_request(url, title)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SHARE, METHOD_SHARE_URL, &req)?;
        decode_share_result(&resp)
    }

    pub fn share_files<S: AsRef<str>>(&self, paths: &[S]) -> Result<ShareResult, SdkError> {
        let req = encode_share_files_request(paths)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SHARE, METHOD_SHARE_FILES, &req)?;
        decode_share_result(&resp)
    }
}

/// Typed ergonomic client for in-app web browsers and OAuth authentication sessions.
#[cfg(feature = "web-auth")]
#[derive(Debug, Clone, Copy)]
pub struct WebAuthClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

#[cfg(feature = "web-auth")]
impl<'a, P: HostServiceProvider + ?Sized> WebAuthClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn open_in_app_browser(
        &self,
        url: &str,
        options: &InAppBrowserOptions,
    ) -> Result<(), SdkError> {
        let req = encode_open_browser_request(url, options)?;
        let resp =
            self.provider
                .invoke_sync(SERVICE_WEB_AUTH, METHOD_OPEN_IN_APP_BROWSER, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec(
                "unexpected non-empty web_auth::open_in_app_browser response".into(),
            ));
        }
        Ok(())
    }

    pub fn authenticate(
        &self,
        auth_url: &str,
        callback_scheme: &str,
    ) -> Result<WebAuthResult, SdkError> {
        let req = encode_authenticate_request(auth_url, callback_scheme)?;
        match self
            .provider
            .invoke_sync(SERVICE_WEB_AUTH, METHOD_AUTHENTICATE, &req)
        {
            Ok(resp) => decode_web_auth_result(&resp),
            Err(SdkError::Cancelled) => Ok(WebAuthResult::Cancelled),
            Err(err) => Err(err),
        }
    }
}
