//! Secure credential store capability (`SERVICE_SECURE_STORE = 0x0012`) with binary codecs.

use nve_abi::SERVICE_SECURE_STORE;

use crate::codec::{BinReader, BinWriter};
use crate::error::SdkError;
use crate::provider::HostServiceProvider;

pub const METHOD_GET: u32 = 1;
pub const METHOD_SET: u32 = 2;
pub const METHOD_DELETE: u32 = 3;

fn validate_key(service: &str, account: &str) -> Result<(), SdkError> {
    if service.is_empty() {
        return Err(SdkError::InvalidArg("secure store service must not be empty".into()));
    }
    if account.is_empty() {
        return Err(SdkError::InvalidArg("secure store account must not be empty".into()));
    }
    Ok(())
}

pub fn encode_key_request(service: &str, account: &str) -> Result<Vec<u8>, SdkError> {
    validate_key(service, account)?;
    let mut w = BinWriter::with_capacity(8 + service.len() + account.len());
    w.write_str(service)?;
    w.write_str(account)?;
    Ok(w.into_bytes())
}

pub fn decode_key_request(bytes: &[u8]) -> Result<(String, String), SdkError> {
    let mut r = BinReader::new(bytes);
    let service = r.read_string()?;
    let account = r.read_string()?;
    r.finish()?;
    validate_key(&service, &account)?;
    Ok((service, account))
}

pub fn encode_set_request(service: &str, account: &str, secret: &[u8]) -> Result<Vec<u8>, SdkError> {
    validate_key(service, account)?;
    let mut w = BinWriter::with_capacity(12 + service.len() + account.len() + secret.len());
    w.write_str(service)?;
    w.write_str(account)?;
    w.write_bytes(secret)?;
    Ok(w.into_bytes())
}

pub fn decode_set_request(bytes: &[u8]) -> Result<(String, String, Vec<u8>), SdkError> {
    let mut r = BinReader::new(bytes);
    let service = r.read_string()?;
    let account = r.read_string()?;
    let secret = r.read_bytes()?.to_vec();
    r.finish()?;
    validate_key(&service, &account)?;
    Ok((service, account, secret))
}

pub fn encode_get_response(secret: Option<&[u8]>) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::with_capacity(secret.map_or(1, |s| 5 + s.len()));
    w.write_opt_bytes(secret)?;
    Ok(w.into_bytes())
}

pub fn decode_get_response(bytes: &[u8]) -> Result<Option<Vec<u8>>, SdkError> {
    let mut r = BinReader::new(bytes);
    let secret = r.read_opt_bytes()?;
    r.finish()?;
    Ok(secret)
}

pub fn encode_delete_response(deleted: bool) -> Vec<u8> {
    vec![u8::from(deleted)]
}

pub fn decode_delete_response(bytes: &[u8]) -> Result<bool, SdkError> {
    let mut r = BinReader::new(bytes);
    let deleted = r.read_bool()?;
    r.finish()?;
    Ok(deleted)
}

/// Typed ergonomic client for OS Keychain / Credential Manager storage.
#[derive(Debug, Clone, Copy)]
pub struct SecureStoreClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> SecureStoreClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, SdkError> {
        let req = encode_key_request(service, account)?;
        match self.provider.invoke_sync(SERVICE_SECURE_STORE, METHOD_GET, &req) {
            Ok(resp) => decode_get_response(&resp),
            Err(SdkError::NotFound) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub fn get_string(&self, service: &str, account: &str) -> Result<Option<String>, SdkError> {
        match self.get(service, account)? {
            Some(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|e| SdkError::Codec(format!("stored secret is not valid UTF-8: {e}"))),
            None => Ok(None),
        }
    }

    pub fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), SdkError> {
        let req = encode_set_request(service, account, secret)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SECURE_STORE, METHOD_SET, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec("unexpected non-empty secure_store::set response".into()));
        }
        Ok(())
    }

    pub fn set_string(&self, service: &str, account: &str, secret: &str) -> Result<(), SdkError> {
        self.set(service, account, secret.as_bytes())
    }

    pub fn delete(&self, service: &str, account: &str) -> Result<bool, SdkError> {
        let req = encode_key_request(service, account)?;
        match self.provider.invoke_sync(SERVICE_SECURE_STORE, METHOD_DELETE, &req) {
            Ok(resp) => decode_delete_response(&resp),
            Err(SdkError::NotFound) => Ok(false),
            Err(err) => Err(err),
        }
    }
}
