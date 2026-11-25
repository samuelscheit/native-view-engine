//! Clipboard capability (`SERVICE_CLIPBOARD = 0x0010`) with zero-JSON binary wire codecs.

use nve_abi::SERVICE_CLIPBOARD;

use crate::codec::{BinReader, BinWriter};
use crate::error::SdkError;
use crate::provider::HostServiceProvider;

pub const METHOD_READ_TEXT: u32 = 1;
pub const METHOD_WRITE_TEXT: u32 = 2;
pub const METHOD_HAS_TEXT: u32 = 3;
pub const METHOD_CLEAR: u32 = 4;

/// Encodes a `write_text` request payload (`u32 len + UTF-8`).
pub fn encode_write_text_request(text: &str) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::with_capacity(4 + text.len());
    w.write_str(text)?;
    Ok(w.into_bytes())
}

/// Decodes a `write_text` request payload.
pub fn decode_write_text_request(bytes: &[u8]) -> Result<String, SdkError> {
    let mut r = BinReader::new(bytes);
    let text = r.read_string()?;
    r.finish()?;
    Ok(text)
}

/// Encodes a `read_text` response (`Option<String>`).
pub fn encode_read_text_response(text: Option<&str>) -> Result<Vec<u8>, SdkError> {
    let cap = text.map_or(1, |s| 5 + s.len());
    let mut w = BinWriter::with_capacity(cap);
    w.write_opt_str(text)?;
    Ok(w.into_bytes())
}

/// Decodes a `read_text` response.
pub fn decode_read_text_response(bytes: &[u8]) -> Result<Option<String>, SdkError> {
    let mut r = BinReader::new(bytes);
    let text = r.read_opt_string()?;
    r.finish()?;
    Ok(text)
}

/// Encodes a `has_text` boolean response.
pub fn encode_has_text_response(has_text: bool) -> Vec<u8> {
    vec![u8::from(has_text)]
}

/// Decodes a `has_text` boolean response.
pub fn decode_has_text_response(bytes: &[u8]) -> Result<bool, SdkError> {
    let mut r = BinReader::new(bytes);
    let val = r.read_bool()?;
    r.finish()?;
    Ok(val)
}

/// Typed ergonomic client for system clipboard operations.
#[derive(Debug, Clone, Copy)]
pub struct ClipboardClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> ClipboardClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn read_text(&self) -> Result<Option<String>, SdkError> {
        let resp = self.provider.invoke_sync(SERVICE_CLIPBOARD, METHOD_READ_TEXT, &[])?;
        decode_read_text_response(&resp)
    }

    pub fn write_text(&self, text: &str) -> Result<(), SdkError> {
        let req = encode_write_text_request(text)?;
        let resp = self.provider.invoke_sync(SERVICE_CLIPBOARD, METHOD_WRITE_TEXT, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec("unexpected non-empty write_text response".into()));
        }
        Ok(())
    }

    pub fn has_text(&self) -> Result<bool, SdkError> {
        let resp = self.provider.invoke_sync(SERVICE_CLIPBOARD, METHOD_HAS_TEXT, &[])?;
        decode_has_text_response(&resp)
    }

    pub fn clear(&self) -> Result<(), SdkError> {
        let resp = self.provider.invoke_sync(SERVICE_CLIPBOARD, METHOD_CLEAR, &[])?;
        if !resp.is_empty() {
            return Err(SdkError::Codec("unexpected non-empty clear response".into()));
        }
        Ok(())
    }
}
