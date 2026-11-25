//! Clipboard capability (`SERVICE_CLIPBOARD = 0x0010`) with direct zero-serialization
//! [`ClipboardBackend`] dispatch for in-process Rust hosts and minimal binary wire framing for
//! cross-language `NVE_IFACE_HOST = 5` FFI calls.

pub mod native;

use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex};

pub use nve_abi::SERVICE_CLIPBOARD;
use nve_host_services::{
    pack_typed, resolve_typed, BinReader, BinWriter, HostServiceHandler, HostServiceProvider,
    ServiceError,
};

pub use native::NativeClipboard;

pub const METHOD_READ_TEXT: u32 = 1;
pub const METHOD_WRITE_TEXT: u32 = 2;
pub const METHOD_HAS_TEXT: u32 = 3;
pub const METHOD_CLEAR: u32 = 4;

/// Direct zero-serialization in-process backend trait for [`SERVICE_CLIPBOARD`].
pub trait ClipboardBackend: Send + Sync {
    fn read_text(&self) -> Result<Option<String>, ServiceError>;
    fn write_text(&self, text: &str) -> Result<(), ServiceError>;
    fn has_text(&self) -> Result<bool, ServiceError>;
    fn clear(&self) -> Result<(), ServiceError>;
}

/// Encodes a `write_text` request payload (`u32 len + UTF-8`) for cross-language FFI.
pub fn encode_write_text_request(text: &str) -> Result<Vec<u8>, ServiceError> {
    let mut w = BinWriter::with_capacity(4 + text.len());
    w.write_str(text)?;
    Ok(w.into_bytes())
}

/// Decodes a `write_text` request payload.
pub fn decode_write_text_request(bytes: &[u8]) -> Result<String, ServiceError> {
    let mut r = BinReader::new(bytes);
    let text = r.read_string()?;
    r.finish()?;
    Ok(text)
}

/// Encodes a `read_text` response (`Option<String>`) for cross-language FFI.
pub fn encode_read_text_response(text: Option<&str>) -> Result<Vec<u8>, ServiceError> {
    let cap = text.map_or(1, |s| 5 + s.len());
    let mut w = BinWriter::with_capacity(cap);
    w.write_opt_str(text)?;
    Ok(w.into_bytes())
}

/// Decodes a `read_text` response.
pub fn decode_read_text_response(bytes: &[u8]) -> Result<Option<String>, ServiceError> {
    let mut r = BinReader::new(bytes);
    let text = r.read_opt_string()?;
    r.finish()?;
    Ok(text)
}

/// Encodes a `has_text` boolean response for cross-language FFI.
pub fn encode_has_text_response(has_text: bool) -> Vec<u8> {
    vec![u8::from(has_text)]
}

/// Decodes a `has_text` boolean response.
pub fn decode_has_text_response(bytes: &[u8]) -> Result<bool, ServiceError> {
    let mut r = BinReader::new(bytes);
    let val = r.read_bool()?;
    r.finish()?;
    Ok(val)
}

/// Shared C-ABI wire dispatcher for any [`ClipboardBackend`].
pub fn dispatch_clipboard_wire(
    backend: &dyn ClipboardBackend,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_READ_TEXT => encode_read_text_response(backend.read_text()?.as_deref()),
        METHOD_WRITE_TEXT => {
            let text = decode_write_text_request(request)?;
            backend.write_text(&text)?;
            Ok(Vec::new())
        }
        METHOD_HAS_TEXT => Ok(encode_has_text_response(backend.has_text()?)),
        METHOD_CLEAR => {
            backend.clear()?;
            Ok(Vec::new())
        }
        _ => Err(ServiceError::Unsupported),
    }
}

/// Typed ergonomic client for system clipboard operations.
///
/// Uses zero-serialization direct [`ClipboardBackend`] calls for in-process Rust providers
/// (`MemoryHostServices`, `NativeHostServices`, `SkiaHostServices`) and falls back seamlessly
/// to `NVE_IFACE_HOST = 5` C-ABI framing on `FfiHostServices`.
#[derive(Debug, Clone, Copy)]
pub struct ClipboardClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> ClipboardClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn read_text(&self) -> Result<Option<String>, ServiceError> {
        if let Some(b) =
            resolve_typed::<dyn ClipboardBackend>(self.provider, SERVICE_CLIPBOARD, METHOD_READ_TEXT)
        {
            return b.read_text();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_CLIPBOARD, METHOD_READ_TEXT, &[])?;
        decode_read_text_response(&resp)
    }

    pub fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        if let Some(b) = resolve_typed::<dyn ClipboardBackend>(
            self.provider,
            SERVICE_CLIPBOARD,
            METHOD_WRITE_TEXT,
        ) {
            return b.write_text(text);
        }
        let req = encode_write_text_request(text)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_CLIPBOARD, METHOD_WRITE_TEXT, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty write_text response".into(),
            ));
        }
        Ok(())
    }

    pub fn has_text(&self) -> Result<bool, ServiceError> {
        if let Some(b) =
            resolve_typed::<dyn ClipboardBackend>(self.provider, SERVICE_CLIPBOARD, METHOD_HAS_TEXT)
        {
            return b.has_text();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_CLIPBOARD, METHOD_HAS_TEXT, &[])?;
        decode_has_text_response(&resp)
    }

    pub fn clear(&self) -> Result<(), ServiceError> {
        if let Some(b) =
            resolve_typed::<dyn ClipboardBackend>(self.provider, SERVICE_CLIPBOARD, METHOD_CLEAR)
        {
            return b.clear();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_CLIPBOARD, METHOD_CLEAR, &[])?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty clear response".into(),
            ));
        }
        Ok(())
    }
}

/// Deterministic in-memory [`ClipboardBackend`] & [`HostServiceHandler`] for [`SERVICE_CLIPBOARD`].
#[derive(Debug, Default)]
pub struct MemoryClipboard {
    text: Mutex<Option<String>>,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> Option<String> {
        self.text.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_text(&self, text: Option<String>) {
        *self.text.lock().unwrap_or_else(|e| e.into_inner()) = text;
    }
}

impl ClipboardBackend for MemoryClipboard {
    fn read_text(&self) -> Result<Option<String>, ServiceError> {
        Ok(self.text.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        *self.text.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
        Ok(())
    }

    fn has_text(&self) -> Result<bool, ServiceError> {
        Ok(self
            .text
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|s| !s.is_empty()))
    }

    fn clear(&self) -> Result<(), ServiceError> {
        *self.text.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}

impl HostServiceHandler for MemoryClipboard {
    fn service_id(&self) -> u32 {
        SERVICE_CLIPBOARD
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_clipboard_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn ClipboardBackend>(self))
    }
}
