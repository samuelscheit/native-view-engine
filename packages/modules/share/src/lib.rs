//! Native Share Sheet capability (`SERVICE_SHARE = 0x0026`) with direct zero-serialization
//! in-process dispatch ([`ShareBackend`]), cross-language `NVE_IFACE_HOST` binary codecs,
//! deterministic [`MemoryShare`], and headless-safe [`NativeShare`] handlers.

use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex};

use nve_host_services::{
    pack_typed, resolve_typed, validate_url, BinReader, BinWriter, HostServiceHandler,
    HostServiceProvider, ServiceError,
};

/// Built-in service ID for the native OS share sheet.
pub const SERVICE_SHARE: u32 = 0x0026;

pub const METHOD_SHARE_TEXT: u32 = 1;
pub const METHOD_SHARE_URL: u32 = 2;
pub const METHOD_SHARE_FILES: u32 = 3;

/// Outcome of presenting the native OS share sheet.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShareResult {
    Completed = 0,
    Dismissed = 1,
}

impl ShareResult {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Completed),
            1 => Some(Self::Dismissed),
            _ => None,
        }
    }
}

/// Recorded share-sheet invocation captured by [`MemoryShare`] for test assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedShare {
    Text {
        text: String,
        subject: Option<String>,
    },
    Url {
        url: String,
        title: Option<String>,
    },
    Files(Vec<String>),
}

pub fn encode_share_text_request(
    text: &str,
    subject: Option<&str>,
) -> Result<Vec<u8>, ServiceError> {
    if text.is_empty() {
        return Err(ServiceError::InvalidArg(
            "share text must not be empty".into(),
        ));
    }
    let mut w = BinWriter::with_capacity(8 + text.len());
    w.write_str(text)?;
    w.write_opt_str(subject)?;
    Ok(w.into_bytes())
}

pub fn decode_share_text_request(bytes: &[u8]) -> Result<(String, Option<String>), ServiceError> {
    let mut r = BinReader::new(bytes);
    let text = r.read_string()?;
    let subject = r.read_opt_string()?;
    r.finish()?;
    if text.is_empty() {
        return Err(ServiceError::InvalidArg(
            "share text must not be empty".into(),
        ));
    }
    Ok((text, subject))
}

pub fn encode_share_url_request(
    url: &str,
    title: Option<&str>,
) -> Result<Vec<u8>, ServiceError> {
    validate_url(url)?;
    let mut w = BinWriter::with_capacity(8 + url.len());
    w.write_str(url)?;
    w.write_opt_str(title)?;
    Ok(w.into_bytes())
}

pub fn decode_share_url_request(bytes: &[u8]) -> Result<(String, Option<String>), ServiceError> {
    let mut r = BinReader::new(bytes);
    let url = r.read_string()?;
    let title = r.read_opt_string()?;
    r.finish()?;
    validate_url(&url)?;
    Ok((url, title))
}

pub fn encode_share_files_request<S: AsRef<str>>(paths: &[S]) -> Result<Vec<u8>, ServiceError> {
    if paths.is_empty() {
        return Err(ServiceError::InvalidArg(
            "share_files requires at least one file path".into(),
        ));
    }
    let count = u32::try_from(paths.len())
        .map_err(|_| ServiceError::InvalidArg("share_files list exceeds u32::MAX".into()))?;
    let mut w = BinWriter::with_capacity(4 + paths.len() * 16);
    w.write_u32(count);
    for p in paths {
        let s = p.as_ref();
        if s.trim().is_empty() {
            return Err(ServiceError::InvalidArg(
                "share_files path must not be empty".into(),
            ));
        }
        w.write_str(s)?;
    }
    Ok(w.into_bytes())
}

pub fn decode_share_files_request(bytes: &[u8]) -> Result<Vec<String>, ServiceError> {
    let mut r = BinReader::new(bytes);
    let paths = r.read_string_vec()?;
    r.finish()?;
    if paths.is_empty() || paths.iter().any(|p| p.trim().is_empty()) {
        return Err(ServiceError::InvalidArg(
            "share_files requires non-empty file paths".into(),
        ));
    }
    Ok(paths)
}

pub fn encode_share_result(result: ShareResult) -> Vec<u8> {
    vec![result as u8]
}

pub fn decode_share_result(bytes: &[u8]) -> Result<ShareResult, ServiceError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    ShareResult::from_u8(raw)
        .ok_or_else(|| ServiceError::Codec(format!("invalid ShareResult {raw}")))
}

/// Direct typed in-process backend for [`SERVICE_SHARE`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait ShareBackend: Send + Sync {
    fn share_text(&self, text: &str, subject: Option<&str>) -> Result<ShareResult, ServiceError>;
    fn share_url(&self, url: &str, title: Option<&str>) -> Result<ShareResult, ServiceError>;
    fn share_files(&self, paths: &[String]) -> Result<ShareResult, ServiceError>;
}

/// Bridges a [`ShareBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_share_wire<B: ShareBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_SHARE_TEXT => {
            let (text, subject) = decode_share_text_request(request)?;
            Ok(encode_share_result(
                backend.share_text(&text, subject.as_deref())?,
            ))
        }
        METHOD_SHARE_URL => {
            let (url, title) = decode_share_url_request(request)?;
            Ok(encode_share_result(
                backend.share_url(&url, title.as_deref())?,
            ))
        }
        METHOD_SHARE_FILES => {
            let paths = decode_share_files_request(request)?;
            Ok(encode_share_result(backend.share_files(&paths)?))
        }
        _ => Err(ServiceError::Unsupported),
    }
}

/// Typed ergonomic client for the native OS share sheet.
#[derive(Debug, Clone, Copy)]
pub struct ShareClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> ShareClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn share_text(
        &self,
        text: &str,
        subject: Option<&str>,
    ) -> Result<ShareResult, ServiceError> {
        if text.is_empty() {
            return Err(ServiceError::InvalidArg(
                "share text must not be empty".into(),
            ));
        }
        if let Some(backend) =
            resolve_typed::<dyn ShareBackend>(self.provider, SERVICE_SHARE, METHOD_SHARE_TEXT)
        {
            return backend.share_text(text, subject);
        }
        let req = encode_share_text_request(text, subject)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SHARE, METHOD_SHARE_TEXT, &req)?;
        decode_share_result(&resp)
    }

    pub fn share_url(&self, url: &str, title: Option<&str>) -> Result<ShareResult, ServiceError> {
        validate_url(url)?;
        if let Some(backend) =
            resolve_typed::<dyn ShareBackend>(self.provider, SERVICE_SHARE, METHOD_SHARE_URL)
        {
            return backend.share_url(url, title);
        }
        let req = encode_share_url_request(url, title)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SHARE, METHOD_SHARE_URL, &req)?;
        decode_share_result(&resp)
    }

    pub fn share_files<S: AsRef<str>>(&self, paths: &[S]) -> Result<ShareResult, ServiceError> {
        if paths.is_empty() {
            return Err(ServiceError::InvalidArg(
                "share_files requires at least one file path".into(),
            ));
        }
        for p in paths {
            if p.as_ref().trim().is_empty() {
                return Err(ServiceError::InvalidArg(
                    "share_files path must not be empty".into(),
                ));
            }
        }
        if let Some(backend) =
            resolve_typed::<dyn ShareBackend>(self.provider, SERVICE_SHARE, METHOD_SHARE_FILES)
        {
            let owned: Vec<String> = paths.iter().map(|p| p.as_ref().to_owned()).collect();
            return backend.share_files(&owned);
        }
        let req = encode_share_files_request(paths)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SHARE, METHOD_SHARE_FILES, &req)?;
        decode_share_result(&resp)
    }
}

struct MemoryShareState {
    result: ShareResult,
    recorded: Vec<RecordedShare>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_SHARE`].
pub struct MemoryShare {
    state: Mutex<MemoryShareState>,
}

impl Default for MemoryShare {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryShare {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryShareState {
                result: ShareResult::Completed,
                recorded: Vec::new(),
            }),
        }
    }

    pub fn set_result(&self, result: ShareResult) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).result = result;
    }

    pub fn recorded_shares(&self) -> Vec<RecordedShare> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recorded
            .clone()
    }
}

impl ShareBackend for MemoryShare {
    fn share_text(&self, text: &str, subject: Option<&str>) -> Result<ShareResult, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.recorded.push(RecordedShare::Text {
            text: text.to_owned(),
            subject: subject.map(str::to_owned),
        });
        Ok(st.result)
    }

    fn share_url(&self, url: &str, title: Option<&str>) -> Result<ShareResult, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.recorded.push(RecordedShare::Url {
            url: url.to_owned(),
            title: title.map(str::to_owned),
        });
        Ok(st.result)
    }

    fn share_files(&self, paths: &[String]) -> Result<ShareResult, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.recorded.push(RecordedShare::Files(paths.to_vec()));
        Ok(st.result)
    }
}

impl HostServiceHandler for MemoryShare {
    fn service_id(&self) -> u32 {
        SERVICE_SHARE
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_share_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn ShareBackend>(self))
    }
}

/// Safe non-modal native [`HostServiceHandler`] for [`SERVICE_SHARE`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeShare;

impl NativeShare {
    pub const fn new() -> Self {
        Self
    }
}

impl ShareBackend for NativeShare {
    fn share_text(&self, _text: &str, _subject: Option<&str>) -> Result<ShareResult, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn share_url(&self, _url: &str, _title: Option<&str>) -> Result<ShareResult, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn share_files(&self, _paths: &[String]) -> Result<ShareResult, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

impl HostServiceHandler for NativeShare {
    fn service_id(&self) -> u32 {
        SERVICE_SHARE
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_share_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn ShareBackend>(self))
    }
}
