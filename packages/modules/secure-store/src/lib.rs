//! Secure credential store capability (`SERVICE_SECURE_STORE = 0x0012`) with direct zero-serialization
//! in-process dispatch ([`SecureStoreBackend`]), cross-language `NVE_IFACE_HOST` C ABI codecs,
//! deterministic [`MemorySecureStore`], and OS Keychain / Credential Manager [`NativeSecureStore`].

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod native;

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub use nve_abi::SERVICE_SECURE_STORE;
use nve_host_services::{
    pack_typed, resolve_typed, BinReader, BinWriter, HostServiceHandler, HostServiceProvider,
    ServiceError,
};

pub use native::NativeSecureStore;

pub const METHOD_GET: u32 = 1;
pub const METHOD_SET: u32 = 2;
pub const METHOD_DELETE: u32 = 3;

fn validate_key(service: &str, account: &str) -> Result<(), ServiceError> {
    if service.is_empty() {
        return Err(ServiceError::InvalidArg(
            "secure store service must not be empty".into(),
        ));
    }
    if account.is_empty() {
        return Err(ServiceError::InvalidArg(
            "secure store account must not be empty".into(),
        ));
    }
    Ok(())
}

pub fn encode_key_request(service: &str, account: &str) -> Result<Vec<u8>, ServiceError> {
    validate_key(service, account)?;
    let mut w = BinWriter::with_capacity(8 + service.len() + account.len());
    w.write_str(service)?;
    w.write_str(account)?;
    Ok(w.into_bytes())
}

pub fn decode_key_request(bytes: &[u8]) -> Result<(String, String), ServiceError> {
    let mut r = BinReader::new(bytes);
    let service = r.read_string()?;
    let account = r.read_string()?;
    r.finish()?;
    validate_key(&service, &account)?;
    Ok((service, account))
}

pub fn encode_set_request(
    service: &str,
    account: &str,
    secret: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    validate_key(service, account)?;
    let mut w = BinWriter::with_capacity(12 + service.len() + account.len() + secret.len());
    w.write_str(service)?;
    w.write_str(account)?;
    w.write_bytes(secret)?;
    Ok(w.into_bytes())
}

pub fn decode_set_request(bytes: &[u8]) -> Result<(String, String, Vec<u8>), ServiceError> {
    let mut r = BinReader::new(bytes);
    let service = r.read_string()?;
    let account = r.read_string()?;
    let secret = r.read_bytes()?.to_vec();
    r.finish()?;
    validate_key(&service, &account)?;
    Ok((service, account, secret))
}

pub fn encode_get_response(secret: Option<&[u8]>) -> Result<Vec<u8>, ServiceError> {
    let mut w = BinWriter::with_capacity(secret.map_or(1, |s| 5 + s.len()));
    w.write_opt_bytes(secret)?;
    Ok(w.into_bytes())
}

pub fn decode_get_response(bytes: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> {
    let mut r = BinReader::new(bytes);
    let secret = r.read_opt_bytes()?;
    r.finish()?;
    Ok(secret)
}

pub fn encode_delete_response(deleted: bool) -> Vec<u8> {
    vec![u8::from(deleted)]
}

pub fn decode_delete_response(bytes: &[u8]) -> Result<bool, ServiceError> {
    let mut r = BinReader::new(bytes);
    let deleted = r.read_bool()?;
    r.finish()?;
    Ok(deleted)
}

/// Direct typed in-process backend for [`SERVICE_SECURE_STORE`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait SecureStoreBackend: Send + Sync {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, ServiceError>;
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), ServiceError>;
    fn delete(&self, service: &str, account: &str) -> Result<bool, ServiceError>;
}

/// Bridges a [`SecureStoreBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_secure_store_wire<B: SecureStoreBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_GET => {
            let (svc, acc) = decode_key_request(request)?;
            let secret = backend.get(&svc, &acc)?;
            encode_get_response(secret.as_deref())
        }
        METHOD_SET => {
            let (svc, acc, secret) = decode_set_request(request)?;
            backend.set(&svc, &acc, &secret)?;
            Ok(Vec::new())
        }
        METHOD_DELETE => {
            let (svc, acc) = decode_key_request(request)?;
            Ok(encode_delete_response(backend.delete(&svc, &acc)?))
        }
        _ => Err(ServiceError::Unsupported),
    }
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

    pub fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, ServiceError> {
        validate_key(service, account)?;
        if let Some(backend) =
            resolve_typed::<dyn SecureStoreBackend>(self.provider, SERVICE_SECURE_STORE, METHOD_GET)
        {
            return match backend.get(service, account) {
                Ok(secret) => Ok(secret),
                Err(ServiceError::NotFound) => Ok(None),
                Err(err) => Err(err),
            };
        }
        let req = encode_key_request(service, account)?;
        match self
            .provider
            .invoke_sync(SERVICE_SECURE_STORE, METHOD_GET, &req)
        {
            Ok(resp) => decode_get_response(&resp),
            Err(ServiceError::NotFound) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub fn get_string(&self, service: &str, account: &str) -> Result<Option<String>, ServiceError> {
        match self.get(service, account)? {
            Some(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|e| ServiceError::Codec(format!("stored secret is not valid UTF-8: {e}"))),
            None => Ok(None),
        }
    }

    pub fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), ServiceError> {
        validate_key(service, account)?;
        if let Some(backend) =
            resolve_typed::<dyn SecureStoreBackend>(self.provider, SERVICE_SECURE_STORE, METHOD_SET)
        {
            return backend.set(service, account, secret);
        }
        let req = encode_set_request(service, account, secret)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_SECURE_STORE, METHOD_SET, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty secure_store::set response".into(),
            ));
        }
        Ok(())
    }

    pub fn set_string(
        &self,
        service: &str,
        account: &str,
        secret: &str,
    ) -> Result<(), ServiceError> {
        self.set(service, account, secret.as_bytes())
    }

    pub fn delete(&self, service: &str, account: &str) -> Result<bool, ServiceError> {
        validate_key(service, account)?;
        if let Some(backend) = resolve_typed::<dyn SecureStoreBackend>(
            self.provider,
            SERVICE_SECURE_STORE,
            METHOD_DELETE,
        ) {
            return match backend.delete(service, account) {
                Ok(deleted) => Ok(deleted),
                Err(ServiceError::NotFound) => Ok(false),
                Err(err) => Err(err),
            };
        }
        let req = encode_key_request(service, account)?;
        match self
            .provider
            .invoke_sync(SERVICE_SECURE_STORE, METHOD_DELETE, &req)
        {
            Ok(resp) => decode_delete_response(&resp),
            Err(ServiceError::NotFound) => Ok(false),
            Err(err) => Err(err),
        }
    }
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_SECURE_STORE`].
#[derive(Debug, Default)]
pub struct MemorySecureStore {
    secrets: Mutex<BTreeMap<(String, String), Vec<u8>>>,
}

impl MemorySecureStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecureStoreBackend for MemorySecureStore {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, ServiceError> {
        validate_key(service, account)?;
        let secrets = self.secrets.lock().unwrap_or_else(|e| e.into_inner());
        Ok(secrets.get(&(service.to_owned(), account.to_owned())).cloned())
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), ServiceError> {
        validate_key(service, account)?;
        let mut secrets = self.secrets.lock().unwrap_or_else(|e| e.into_inner());
        secrets.insert((service.to_owned(), account.to_owned()), secret.to_vec());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool, ServiceError> {
        validate_key(service, account)?;
        let mut secrets = self.secrets.lock().unwrap_or_else(|e| e.into_inner());
        Ok(secrets
            .remove(&(service.to_owned(), account.to_owned()))
            .is_some())
    }
}

impl HostServiceHandler for MemorySecureStore {
    fn service_id(&self) -> u32 {
        SERVICE_SECURE_STORE
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_secure_store_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn SecureStoreBackend>(self))
    }
}
