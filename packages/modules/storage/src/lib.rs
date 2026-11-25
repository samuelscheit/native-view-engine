//! Embedded crash-safe binary KV / record store capability (`SERVICE_STORAGE = 0x0020`)
//! with direct zero-serialization in-process dispatch ([`StorageBackend`]), `NVK1` on-disk
//! WAL persistence, and cross-language `NVE_IFACE_HOST` C ABI codecs.

pub mod engine;
pub mod handler;

use std::borrow::Cow;

use nve_host_services::{
    resolve_typed, BinReader, BinWriter, HostServiceProvider, ServiceError, MAX_SEQ_ITEMS,
};

pub use engine::{KvStore, KvStoreManager, NVK1_COMMIT_MARKER, NVK1_MAGIC, NVK1_VERSION};
pub use handler::{dispatch_storage_wire, MemoryStorage, NativeStorage, StorageBackend};

/// Built-in service ID for the embedded crash-safe binary KV store.
pub const SERVICE_STORAGE: u32 = 0x0020;

pub const METHOD_OPEN: u32 = 1;
pub const METHOD_GET: u32 = 2;
pub const METHOD_PUT: u32 = 3;
pub const METHOD_DELETE: u32 = 4;
pub const METHOD_SCAN_PREFIX: u32 = 5;
pub const METHOD_COMMIT_BATCH: u32 = 6;
pub const METHOD_COMPACT: u32 = 7;

/// Maximum allowed namespace identifier length in bytes.
pub const MAX_NAMESPACE_LEN: usize = 128;
/// Maximum allowed key length in bytes (64 KiB).
pub const MAX_KEY_LEN: usize = 64 * 1024;
/// Maximum allowed value length in bytes (16 MiB).
pub const MAX_VALUE_LEN: usize = 16 * 1024 * 1024;

/// Single mutation operation inside an atomic [`StorageClient::commit_batch`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageWriteOp {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

/// Key/value pair returned by [`StorageClient::scan_prefix`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEntry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

/// Metadata returned when opening or recovering a storage namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageOpenInfo {
    pub handle: u64,
    pub key_count: u64,
    pub log_bytes: u64,
    pub recovered_torn_tail: bool,
}

/// Statistics returned by [`StorageClient::compact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactStats {
    pub live_keys: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

/// Validates a storage namespace name (safe single filename component, no traversal).
pub fn validate_namespace(namespace: &str) -> Result<(), ServiceError> {
    if namespace.is_empty() || namespace.len() > MAX_NAMESPACE_LEN {
        return Err(ServiceError::InvalidArg(format!(
            "storage namespace length must be 1..={MAX_NAMESPACE_LEN}"
        )));
    }
    if namespace == "." || namespace == ".." || namespace.contains("..") {
        return Err(ServiceError::InvalidArg(
            "storage namespace must not contain path traversal segments".into(),
        ));
    }
    if !namespace
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(ServiceError::InvalidArg(
            "storage namespace must contain only ASCII [A-Za-z0-9._-]".into(),
        ));
    }
    Ok(())
}

/// Validates a non-empty binary KV key.
pub fn validate_key(key: &[u8]) -> Result<(), ServiceError> {
    if key.is_empty() {
        return Err(ServiceError::InvalidArg(
            "storage key must not be empty".into(),
        ));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(ServiceError::InvalidArg(format!(
            "storage key exceeds maximum length of {MAX_KEY_LEN} bytes"
        )));
    }
    Ok(())
}

/// Validates a binary KV value slice.
pub fn validate_value(value: &[u8]) -> Result<(), ServiceError> {
    if value.len() > MAX_VALUE_LEN {
        return Err(ServiceError::InvalidArg(format!(
            "storage value exceeds maximum length of {MAX_VALUE_LEN} bytes"
        )));
    }
    Ok(())
}

/// Validates every write operation in a batch prior to encoding or execution.
pub fn validate_batch(ops: &[StorageWriteOp]) -> Result<(), ServiceError> {
    if ops.len() > MAX_SEQ_ITEMS {
        return Err(ServiceError::InvalidArg(format!(
            "storage batch length {} exceeds limit {MAX_SEQ_ITEMS}",
            ops.len()
        )));
    }
    for op in ops {
        match op {
            StorageWriteOp::Put { key, value } => {
                validate_key(key)?;
                validate_value(value)?;
            }
            StorageWriteOp::Delete { key } => {
                validate_key(key)?;
            }
        }
    }
    Ok(())
}

pub fn encode_open_request(namespace: &str) -> Result<Vec<u8>, ServiceError> {
    validate_namespace(namespace)?;
    let mut w = BinWriter::with_capacity(4 + namespace.len());
    w.write_str(namespace)?;
    Ok(w.into_bytes())
}

pub fn decode_open_request(bytes: &[u8]) -> Result<String, ServiceError> {
    let mut r = BinReader::new(bytes);
    let namespace = r.read_string()?;
    r.finish()?;
    validate_namespace(&namespace)?;
    Ok(namespace)
}

pub fn encode_open_response(info: StorageOpenInfo) -> Vec<u8> {
    let mut w = BinWriter::with_capacity(25);
    w.write_u64(info.handle);
    w.write_u64(info.key_count);
    w.write_u64(info.log_bytes);
    w.write_bool(info.recovered_torn_tail);
    w.into_bytes()
}

pub fn decode_open_response(bytes: &[u8]) -> Result<StorageOpenInfo, ServiceError> {
    let mut r = BinReader::new(bytes);
    let handle = r.read_u64()?;
    let key_count = r.read_u64()?;
    let log_bytes = r.read_u64()?;
    let recovered_torn_tail = r.read_bool()?;
    r.finish()?;
    Ok(StorageOpenInfo {
        handle,
        key_count,
        log_bytes,
        recovered_torn_tail,
    })
}

pub fn encode_key_request(namespace: &str, key: &[u8]) -> Result<Vec<u8>, ServiceError> {
    validate_namespace(namespace)?;
    validate_key(key)?;
    let mut w = BinWriter::with_capacity(8 + namespace.len() + key.len());
    w.write_str(namespace)?;
    w.write_bytes(key)?;
    Ok(w.into_bytes())
}

pub fn decode_key_request(bytes: &[u8]) -> Result<(String, Vec<u8>), ServiceError> {
    let mut r = BinReader::new(bytes);
    let namespace = r.read_string()?;
    let key = r.read_bytes()?.to_vec();
    r.finish()?;
    validate_namespace(&namespace)?;
    validate_key(&key)?;
    Ok((namespace, key))
}

pub fn encode_put_request(
    namespace: &str,
    key: &[u8],
    value: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    validate_namespace(namespace)?;
    validate_key(key)?;
    validate_value(value)?;
    let mut w = BinWriter::with_capacity(12 + namespace.len() + key.len() + value.len());
    w.write_str(namespace)?;
    w.write_bytes(key)?;
    w.write_bytes(value)?;
    Ok(w.into_bytes())
}

pub fn decode_put_request(bytes: &[u8]) -> Result<(String, Vec<u8>, Vec<u8>), ServiceError> {
    let mut r = BinReader::new(bytes);
    let namespace = r.read_string()?;
    let key = r.read_bytes()?.to_vec();
    let value = r.read_bytes()?.to_vec();
    r.finish()?;
    validate_namespace(&namespace)?;
    validate_key(&key)?;
    validate_value(&value)?;
    Ok((namespace, key, value))
}

pub fn encode_get_response(value: Option<&[u8]>) -> Result<Vec<u8>, ServiceError> {
    let mut w = BinWriter::with_capacity(value.map_or(1, |v| 5 + v.len()));
    w.write_opt_bytes(value)?;
    Ok(w.into_bytes())
}

pub fn decode_get_response(bytes: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> {
    let mut r = BinReader::new(bytes);
    let val = r.read_opt_bytes()?;
    r.finish()?;
    Ok(val)
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

pub fn encode_scan_prefix_request(
    namespace: &str,
    prefix: &[u8],
    limit: u32,
) -> Result<Vec<u8>, ServiceError> {
    validate_namespace(namespace)?;
    if prefix.len() > MAX_KEY_LEN {
        return Err(ServiceError::InvalidArg(
            "storage prefix exceeds MAX_KEY_LEN".into(),
        ));
    }
    let mut w = BinWriter::with_capacity(12 + namespace.len() + prefix.len());
    w.write_str(namespace)?;
    w.write_bytes(prefix)?;
    w.write_u32(limit);
    Ok(w.into_bytes())
}

pub fn decode_scan_prefix_request(bytes: &[u8]) -> Result<(String, Vec<u8>, u32), ServiceError> {
    let mut r = BinReader::new(bytes);
    let namespace = r.read_string()?;
    let prefix = r.read_bytes()?.to_vec();
    let limit = r.read_u32()?;
    r.finish()?;
    validate_namespace(&namespace)?;
    if prefix.len() > MAX_KEY_LEN {
        return Err(ServiceError::InvalidArg(
            "storage prefix exceeds MAX_KEY_LEN".into(),
        ));
    }
    Ok((namespace, prefix, limit))
}

pub fn encode_scan_prefix_response(entries: &[StorageEntry]) -> Result<Vec<u8>, ServiceError> {
    let count = u32::try_from(entries.len())
        .map_err(|_| ServiceError::InvalidArg("scan response exceeds u32::MAX".into()))?;
    let mut w = BinWriter::with_capacity(4 + entries.len() * 16);
    w.write_u32(count);
    for entry in entries {
        w.write_bytes(&entry.key)?;
        w.write_bytes(&entry.value)?;
    }
    Ok(w.into_bytes())
}

pub fn decode_scan_prefix_response(bytes: &[u8]) -> Result<Vec<StorageEntry>, ServiceError> {
    let mut r = BinReader::new(bytes);
    let count = r.read_seq_len(8)?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let key = r.read_bytes()?.to_vec();
        let value = r.read_bytes()?.to_vec();
        entries.push(StorageEntry { key, value });
    }
    r.finish()?;
    Ok(entries)
}

pub fn encode_commit_batch_request(
    namespace: &str,
    ops: &[StorageWriteOp],
) -> Result<Vec<u8>, ServiceError> {
    validate_namespace(namespace)?;
    validate_batch(ops)?;
    let mut w = BinWriter::with_capacity(8 + namespace.len() + ops.len() * 16);
    w.write_str(namespace)?;
    w.write_u32(ops.len() as u32);
    for op in ops {
        match op {
            StorageWriteOp::Put { key, value } => {
                w.write_u8(1);
                w.write_bytes(key)?;
                w.write_bytes(value)?;
            }
            StorageWriteOp::Delete { key } => {
                w.write_u8(2);
                w.write_bytes(key)?;
            }
        }
    }
    Ok(w.into_bytes())
}

pub fn decode_commit_batch_request(
    bytes: &[u8],
) -> Result<(String, Vec<StorageWriteOp>), ServiceError> {
    let mut r = BinReader::new(bytes);
    let namespace = r.read_string()?;
    let count = r.read_seq_len(5)?;
    let mut ops = Vec::with_capacity(count);
    for _ in 0..count {
        match r.read_u8()? {
            1 => {
                let key = r.read_bytes()?.to_vec();
                let value = r.read_bytes()?.to_vec();
                ops.push(StorageWriteOp::Put { key, value });
            }
            2 => {
                let key = r.read_bytes()?.to_vec();
                ops.push(StorageWriteOp::Delete { key });
            }
            other => {
                return Err(ServiceError::Codec(format!(
                    "invalid StorageWriteOp tag {other}"
                )));
            }
        }
    }
    r.finish()?;
    validate_namespace(&namespace)?;
    validate_batch(&ops)?;
    Ok((namespace, ops))
}

pub fn encode_compact_response(stats: CompactStats) -> Vec<u8> {
    let mut w = BinWriter::with_capacity(24);
    w.write_u64(stats.live_keys);
    w.write_u64(stats.bytes_before);
    w.write_u64(stats.bytes_after);
    w.into_bytes()
}

pub fn decode_compact_response(bytes: &[u8]) -> Result<CompactStats, ServiceError> {
    let mut r = BinReader::new(bytes);
    let live_keys = r.read_u64()?;
    let bytes_before = r.read_u64()?;
    let bytes_after = r.read_u64()?;
    r.finish()?;
    Ok(CompactStats {
        live_keys,
        bytes_before,
        bytes_after,
    })
}

/// Typed ergonomic client for the embedded crash-safe binary KV store.
#[derive(Debug, Clone)]
pub struct StorageClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
    namespace: Cow<'a, str>,
}

impl<'a, P: HostServiceProvider + ?Sized> StorageClient<'a, P> {
    /// Creates a client bound to the `"default"` storage namespace.
    pub const fn new(provider: &'a P) -> Self {
        Self {
            provider,
            namespace: Cow::Borrowed("default"),
        }
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn open(&self, namespace: &str) -> Result<Self, ServiceError> {
        let (client, _) = self.open_with_info(namespace)?;
        Ok(client)
    }

    pub fn open_with_info(&self, namespace: &str) -> Result<(Self, StorageOpenInfo), ServiceError> {
        validate_namespace(namespace)?;
        let info = if let Some(backend) =
            resolve_typed::<dyn StorageBackend>(self.provider, SERVICE_STORAGE, METHOD_OPEN)
        {
            backend.open(namespace)?
        } else {
            let req = encode_open_request(namespace)?;
            let resp = self
                .provider
                .invoke_sync(SERVICE_STORAGE, METHOD_OPEN, &req)?;
            decode_open_response(&resp)?
        };
        Ok((
            Self {
                provider: self.provider,
                namespace: Cow::Owned(namespace.to_owned()),
            },
            info,
        ))
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> {
        validate_namespace(&self.namespace)?;
        validate_key(key)?;
        if let Some(backend) =
            resolve_typed::<dyn StorageBackend>(self.provider, SERVICE_STORAGE, METHOD_GET)
        {
            return match backend.get(&self.namespace, key) {
                Ok(val) => Ok(val),
                Err(ServiceError::NotFound) => Ok(None),
                Err(err) => Err(err),
            };
        }
        let req = encode_key_request(&self.namespace, key)?;
        match self.provider.invoke_sync(SERVICE_STORAGE, METHOD_GET, &req) {
            Ok(resp) => decode_get_response(&resp),
            Err(ServiceError::NotFound) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<(), ServiceError> {
        validate_namespace(&self.namespace)?;
        validate_key(key)?;
        validate_value(value)?;
        if let Some(backend) =
            resolve_typed::<dyn StorageBackend>(self.provider, SERVICE_STORAGE, METHOD_PUT)
        {
            return backend.put(&self.namespace, key, value);
        }
        let req = encode_put_request(&self.namespace, key, value)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_STORAGE, METHOD_PUT, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty storage::put response".into(),
            ));
        }
        Ok(())
    }

    pub fn delete(&self, key: &[u8]) -> Result<bool, ServiceError> {
        validate_namespace(&self.namespace)?;
        validate_key(key)?;
        if let Some(backend) =
            resolve_typed::<dyn StorageBackend>(self.provider, SERVICE_STORAGE, METHOD_DELETE)
        {
            return match backend.delete(&self.namespace, key) {
                Ok(deleted) => Ok(deleted),
                Err(ServiceError::NotFound) => Ok(false),
                Err(err) => Err(err),
            };
        }
        let req = encode_key_request(&self.namespace, key)?;
        match self
            .provider
            .invoke_sync(SERVICE_STORAGE, METHOD_DELETE, &req)
        {
            Ok(resp) => decode_delete_response(&resp),
            Err(ServiceError::NotFound) => Ok(false),
            Err(err) => Err(err),
        }
    }

    pub fn scan_prefix(&self, prefix: &[u8], limit: u32) -> Result<Vec<StorageEntry>, ServiceError> {
        validate_namespace(&self.namespace)?;
        if prefix.len() > MAX_KEY_LEN {
            return Err(ServiceError::InvalidArg(
                "storage prefix exceeds MAX_KEY_LEN".into(),
            ));
        }
        if let Some(backend) =
            resolve_typed::<dyn StorageBackend>(self.provider, SERVICE_STORAGE, METHOD_SCAN_PREFIX)
        {
            return backend.scan_prefix(&self.namespace, prefix, limit);
        }
        let req = encode_scan_prefix_request(&self.namespace, prefix, limit)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_STORAGE, METHOD_SCAN_PREFIX, &req)?;
        decode_scan_prefix_response(&resp)
    }

    pub fn commit_batch(&self, ops: &[StorageWriteOp]) -> Result<(), ServiceError> {
        validate_namespace(&self.namespace)?;
        validate_batch(ops)?;
        if let Some(backend) =
            resolve_typed::<dyn StorageBackend>(self.provider, SERVICE_STORAGE, METHOD_COMMIT_BATCH)
        {
            return backend.commit_batch(&self.namespace, ops);
        }
        let req = encode_commit_batch_request(&self.namespace, ops)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_STORAGE, METHOD_COMMIT_BATCH, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty storage::commit_batch response".into(),
            ));
        }
        Ok(())
    }

    pub fn compact(&self) -> Result<CompactStats, ServiceError> {
        validate_namespace(&self.namespace)?;
        if let Some(backend) =
            resolve_typed::<dyn StorageBackend>(self.provider, SERVICE_STORAGE, METHOD_COMPACT)
        {
            return backend.compact(&self.namespace);
        }
        let req = encode_open_request(&self.namespace)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_STORAGE, METHOD_COMPACT, &req)?;
        decode_compact_response(&resp)
    }
}
