//! Deep linking & external URL opening (`SERVICE_LINKING = 0x0013`) with binary codecs.

use nve_abi::SERVICE_LINKING;

use crate::codec::{BinReader, BinWriter};
use crate::error::SdkError;
use crate::provider::{HostPushEvent, HostServiceProvider};

pub const METHOD_OPEN_URL: u32 = 1;
pub const METHOD_CAN_OPEN_URL: u32 = 2;
pub const METHOD_INITIAL_URL: u32 = 3;

/// Host push event ID emitted when the app receives a deep link URL at runtime.
pub const EVENT_URL_RECEIVED: u32 = 1;

/// Validates a non-empty URL containing a scheme (`scheme:...`) without control characters.
pub fn validate_url(url: &str) -> Result<(), SdkError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(SdkError::InvalidArg("URL must not be empty".into()));
    }
    if trimmed.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(SdkError::InvalidArg(
            "URL must not contain ASCII control characters".into(),
        ));
    }
    let Some((scheme, rest)) = trimmed.split_once(':') else {
        return Err(SdkError::InvalidArg("URL must include a scheme".into()));
    };
    if scheme.is_empty()
        || rest.is_empty()
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return Err(SdkError::InvalidArg(format!("invalid URL scheme '{scheme}'")));
    }
    Ok(())
}

/// Push event emitted by the host when an incoming deep link opens the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkingEvent {
    UrlReceived { url: String },
}

pub fn encode_url_request(url: &str) -> Result<Vec<u8>, SdkError> {
    validate_url(url)?;
    let mut w = BinWriter::with_capacity(4 + url.len());
    w.write_str(url)?;
    Ok(w.into_bytes())
}

pub fn decode_url_request(bytes: &[u8]) -> Result<String, SdkError> {
    let mut r = BinReader::new(bytes);
    let url = r.read_string()?;
    r.finish()?;
    validate_url(&url)?;
    Ok(url)
}

pub fn encode_can_open_response(can_open: bool) -> Vec<u8> {
    vec![u8::from(can_open)]
}

pub fn decode_can_open_response(bytes: &[u8]) -> Result<bool, SdkError> {
    let mut r = BinReader::new(bytes);
    let can_open = r.read_bool()?;
    r.finish()?;
    Ok(can_open)
}

pub fn encode_initial_url_response(url: Option<&str>) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::with_capacity(url.map_or(1, |u| 5 + u.len()));
    w.write_opt_str(url)?;
    Ok(w.into_bytes())
}

pub fn decode_initial_url_response(bytes: &[u8]) -> Result<Option<String>, SdkError> {
    let mut r = BinReader::new(bytes);
    let url = r.read_opt_string()?;
    r.finish()?;
    Ok(url)
}

pub fn encode_url_received_event(url: &str) -> Result<Vec<u8>, SdkError> {
    validate_url(url)?;
    let mut w = BinWriter::with_capacity(4 + url.len());
    w.write_str(url)?;
    Ok(w.into_bytes())
}

pub fn decode_url_received_event(bytes: &[u8]) -> Result<String, SdkError> {
    decode_url_request(bytes)
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

    pub fn open_url(&self, url: &str) -> Result<(), SdkError> {
        let req = encode_url_request(url)?;
        let resp = self.provider.invoke_sync(SERVICE_LINKING, METHOD_OPEN_URL, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec("unexpected non-empty open_url response".into()));
        }
        Ok(())
    }

    pub fn can_open_url(&self, url: &str) -> Result<bool, SdkError> {
        let req = encode_url_request(url)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_LINKING, METHOD_CAN_OPEN_URL, &req)?;
        decode_can_open_response(&resp)
    }

    pub fn initial_url(&self) -> Result<Option<String>, SdkError> {
        let resp = self
            .provider
            .invoke_sync(SERVICE_LINKING, METHOD_INITIAL_URL, &[])?;
        decode_initial_url_response(&resp)
    }

    /// Decodes a [`HostPushEvent`] if it belongs to [`SERVICE_LINKING`].
    pub fn decode_push_event(event: &HostPushEvent) -> Result<Option<LinkingEvent>, SdkError> {
        if event.service_id != SERVICE_LINKING {
            return Ok(None);
        }
        match event.event_id {
            EVENT_URL_RECEIVED => Ok(Some(LinkingEvent::UrlReceived {
                url: decode_url_received_event(&event.payload)?,
            })),
            other => Err(SdkError::Codec(format!("unknown linking event_id {other}"))),
        }
    }
}
