//! Deep linking & external URL opening (`SERVICE_LINKING = 0x0013`) with direct zero-serialization
//! in-process dispatch ([`LinkingBackend`]), cross-language `NVE_IFACE_HOST` C ABI codecs,
//! deterministic [`MemoryLinking`], and OS [`NativeLinking`] handlers.

pub mod native;

use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex};

pub use nve_abi::SERVICE_LINKING;
pub use nve_host_services::validate_url;
use nve_host_services::{
    pack_typed, resolve_typed, BinReader, BinWriter, HostPushEvent, HostServiceHandler,
    HostServiceProvider, ServiceError,
};

pub use native::NativeLinking;

pub const METHOD_OPEN_URL: u32 = 1;
pub const METHOD_CAN_OPEN_URL: u32 = 2;
pub const METHOD_INITIAL_URL: u32 = 3;

/// Host push event ID emitted when the app receives a deep link URL at runtime.
pub const EVENT_URL_RECEIVED: u32 = 1;

/// Push event emitted by the host when an incoming deep link opens the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkingEvent {
    UrlReceived { url: String },
}

pub fn encode_url_request(url: &str) -> Result<Vec<u8>, ServiceError> {
    validate_url(url)?;
    let mut w = BinWriter::with_capacity(4 + url.len());
    w.write_str(url)?;
    Ok(w.into_bytes())
}

pub fn decode_url_request(bytes: &[u8]) -> Result<String, ServiceError> {
    let mut r = BinReader::new(bytes);
    let url = r.read_string()?;
    r.finish()?;
    validate_url(&url)?;
    Ok(url)
}

pub fn encode_can_open_response(can_open: bool) -> Vec<u8> {
    vec![u8::from(can_open)]
}

pub fn decode_can_open_response(bytes: &[u8]) -> Result<bool, ServiceError> {
    let mut r = BinReader::new(bytes);
    let can_open = r.read_bool()?;
    r.finish()?;
    Ok(can_open)
}

pub fn encode_initial_url_response(url: Option<&str>) -> Result<Vec<u8>, ServiceError> {
    let mut w = BinWriter::with_capacity(url.map_or(1, |u| 5 + u.len()));
    w.write_opt_str(url)?;
    Ok(w.into_bytes())
}

pub fn decode_initial_url_response(bytes: &[u8]) -> Result<Option<String>, ServiceError> {
    let mut r = BinReader::new(bytes);
    let url = r.read_opt_string()?;
    r.finish()?;
    Ok(url)
}

pub fn encode_url_received_event(url: &str) -> Result<Vec<u8>, ServiceError> {
    validate_url(url)?;
    let mut w = BinWriter::with_capacity(4 + url.len());
    w.write_str(url)?;
    Ok(w.into_bytes())
}

pub fn decode_url_received_event(bytes: &[u8]) -> Result<String, ServiceError> {
    decode_url_request(bytes)
}

/// Direct typed in-process backend for [`SERVICE_LINKING`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait LinkingBackend: Send + Sync {
    fn open_url(&self, url: &str) -> Result<(), ServiceError>;
    fn can_open_url(&self, url: &str) -> Result<bool, ServiceError>;
    fn initial_url(&self) -> Result<Option<String>, ServiceError>;
}

/// Bridges a [`LinkingBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_linking_wire<B: LinkingBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_OPEN_URL => {
            let url = decode_url_request(request)?;
            backend.open_url(&url)?;
            Ok(Vec::new())
        }
        METHOD_CAN_OPEN_URL => {
            let url = decode_url_request(request)?;
            Ok(encode_can_open_response(backend.can_open_url(&url)?))
        }
        METHOD_INITIAL_URL => encode_initial_url_response(backend.initial_url()?.as_deref()),
        _ => Err(ServiceError::Unsupported),
    }
}

/// Typed ergonomic client for external URL launching and incoming deep links.
#[derive(Debug, Clone, Copy)]
pub struct LinkingClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> LinkingClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn open_url(&self, url: &str) -> Result<(), ServiceError> {
        validate_url(url)?;
        if let Some(backend) =
            resolve_typed::<dyn LinkingBackend>(self.provider, SERVICE_LINKING, METHOD_OPEN_URL)
        {
            return backend.open_url(url);
        }
        let req = encode_url_request(url)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_LINKING, METHOD_OPEN_URL, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty open_url response".into(),
            ));
        }
        Ok(())
    }

    pub fn can_open_url(&self, url: &str) -> Result<bool, ServiceError> {
        validate_url(url)?;
        if let Some(backend) =
            resolve_typed::<dyn LinkingBackend>(self.provider, SERVICE_LINKING, METHOD_CAN_OPEN_URL)
        {
            return backend.can_open_url(url);
        }
        let req = encode_url_request(url)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_LINKING, METHOD_CAN_OPEN_URL, &req)?;
        decode_can_open_response(&resp)
    }

    pub fn initial_url(&self) -> Result<Option<String>, ServiceError> {
        if let Some(backend) =
            resolve_typed::<dyn LinkingBackend>(self.provider, SERVICE_LINKING, METHOD_INITIAL_URL)
        {
            return backend.initial_url();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_LINKING, METHOD_INITIAL_URL, &[])?;
        decode_initial_url_response(&resp)
    }

    /// Decodes a [`HostPushEvent`] if it belongs to [`SERVICE_LINKING`].
    pub fn decode_push_event(event: &HostPushEvent) -> Result<Option<LinkingEvent>, ServiceError> {
        if event.service_id != SERVICE_LINKING {
            return Ok(None);
        }
        match event.event_id {
            EVENT_URL_RECEIVED => Ok(Some(LinkingEvent::UrlReceived {
                url: decode_url_received_event(&event.payload)?,
            })),
            other => Err(ServiceError::Codec(format!(
                "unknown linking event_id {other}"
            ))),
        }
    }
}

struct MemoryLinkingState {
    supported_url_schemes: Vec<String>,
    initial_url: Option<String>,
    opened_urls: Vec<String>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_LINKING`].
pub struct MemoryLinking {
    state: Mutex<MemoryLinkingState>,
}

impl Default for MemoryLinking {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryLinking {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryLinkingState {
                supported_url_schemes: vec![
                    "https".into(),
                    "http".into(),
                    "mailto".into(),
                    "nve".into(),
                ],
                initial_url: None,
                opened_urls: Vec::new(),
            }),
        }
    }

    pub fn set_initial_url(&self, url: Option<String>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .initial_url = url;
    }

    pub fn set_supported_url_schemes(&self, schemes: Vec<String>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .supported_url_schemes = schemes;
    }

    pub fn opened_urls(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .opened_urls
            .clone()
    }
}

impl LinkingBackend for MemoryLinking {
    fn open_url(&self, url: &str) -> Result<(), ServiceError> {
        validate_url(url)?;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let scheme = url
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !st.supported_url_schemes.iter().any(|s| s == &scheme) {
            return Err(ServiceError::Unsupported);
        }
        st.opened_urls.push(url.to_owned());
        Ok(())
    }

    fn can_open_url(&self, url: &str) -> Result<bool, ServiceError> {
        validate_url(url)?;
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let scheme = url
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        Ok(st.supported_url_schemes.iter().any(|s| s == &scheme))
    }

    fn initial_url(&self) -> Result<Option<String>, ServiceError> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(st.initial_url.clone())
    }
}

impl HostServiceHandler for MemoryLinking {
    fn service_id(&self) -> u32 {
        SERVICE_LINKING
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_linking_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn LinkingBackend>(self))
    }
}
