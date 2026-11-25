//! Direct typed [`StorageBackend`] plus [`NativeStorage`] (disk or in-memory `NVK1`) and
//! [`MemoryStorage`] [`HostServiceHandler`] implementations.

use std::any::{Any, TypeId};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::engine::KvStoreManager;
use crate::{
    decode_commit_batch_request, decode_key_request, decode_open_request, decode_put_request,
    decode_scan_prefix_request, encode_compact_response, encode_delete_response,
    encode_get_response, encode_open_response, encode_scan_prefix_response, CompactStats,
    StorageEntry, StorageOpenInfo, StorageWriteOp, METHOD_COMMIT_BATCH, METHOD_COMPACT,
    METHOD_DELETE, METHOD_GET, METHOD_OPEN, METHOD_PUT, METHOD_SCAN_PREFIX, SERVICE_STORAGE,
};

/// Direct typed in-process backend for [`SERVICE_STORAGE`].
///
/// In-process Rust callers invoke `KvStoreManager` directly with zero request/response binary
/// wire encoding via [`nve_host_services::resolve_typed`].
pub trait StorageBackend: Send + Sync {
    fn open(&self, namespace: &str) -> Result<StorageOpenInfo, ServiceError>;
    fn get(&self, namespace: &str, key: &[u8]) -> Result<Option<Vec<u8>>, ServiceError>;
    fn put(&self, namespace: &str, key: &[u8], value: &[u8]) -> Result<(), ServiceError>;
    fn delete(&self, namespace: &str, key: &[u8]) -> Result<bool, ServiceError>;
    fn scan_prefix(
        &self,
        namespace: &str,
        prefix: &[u8],
        limit: u32,
    ) -> Result<Vec<StorageEntry>, ServiceError>;
    fn commit_batch(&self, namespace: &str, ops: &[StorageWriteOp]) -> Result<(), ServiceError>;
    fn compact(&self, namespace: &str) -> Result<CompactStats, ServiceError>;
}

/// Bridges a [`StorageBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_storage_wire<B: StorageBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_OPEN => {
            let ns = decode_open_request(request)?;
            Ok(encode_open_response(backend.open(&ns)?))
        }
        METHOD_GET => {
            let (ns, key) = decode_key_request(request)?;
            let val = backend.get(&ns, &key)?;
            encode_get_response(val.as_deref())
        }
        METHOD_PUT => {
            let (ns, key, value) = decode_put_request(request)?;
            backend.put(&ns, &key, &value)?;
            Ok(Vec::new())
        }
        METHOD_DELETE => {
            let (ns, key) = decode_key_request(request)?;
            Ok(encode_delete_response(backend.delete(&ns, &key)?))
        }
        METHOD_SCAN_PREFIX => {
            let (ns, prefix, limit) = decode_scan_prefix_request(request)?;
            encode_scan_prefix_response(&backend.scan_prefix(&ns, &prefix, limit)?)
        }
        METHOD_COMMIT_BATCH => {
            let (ns, ops) = decode_commit_batch_request(request)?;
            backend.commit_batch(&ns, &ops)?;
            Ok(Vec::new())
        }
        METHOD_COMPACT => {
            let ns = decode_open_request(request)?;
            Ok(encode_compact_response(backend.compact(&ns)?))
        }
        _ => Err(ServiceError::Unsupported),
    }
}

fn kv_backend_open(
    kv: &Mutex<KvStoreManager>,
    namespace: &str,
) -> Result<StorageOpenInfo, ServiceError> {
    kv.lock().unwrap_or_else(|e| e.into_inner()).open(namespace)
}

fn kv_backend_get(
    kv: &Mutex<KvStoreManager>,
    namespace: &str,
    key: &[u8],
) -> Result<Option<Vec<u8>>, ServiceError> {
    kv.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_open(namespace)?
        .get(key)
}

fn kv_backend_put(
    kv: &Mutex<KvStoreManager>,
    namespace: &str,
    key: &[u8],
    value: &[u8],
) -> Result<(), ServiceError> {
    kv.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_open(namespace)?
        .put(key, value)
}

fn kv_backend_delete(
    kv: &Mutex<KvStoreManager>,
    namespace: &str,
    key: &[u8],
) -> Result<bool, ServiceError> {
    kv.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_open(namespace)?
        .delete(key)
}

fn kv_backend_scan_prefix(
    kv: &Mutex<KvStoreManager>,
    namespace: &str,
    prefix: &[u8],
    limit: u32,
) -> Result<Vec<StorageEntry>, ServiceError> {
    Ok(kv
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_open(namespace)?
        .scan_prefix(prefix, limit))
}

fn kv_backend_commit_batch(
    kv: &Mutex<KvStoreManager>,
    namespace: &str,
    ops: &[StorageWriteOp],
) -> Result<(), ServiceError> {
    kv.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_open(namespace)?
        .commit_batch(ops)
}

fn kv_backend_compact(
    kv: &Mutex<KvStoreManager>,
    namespace: &str,
) -> Result<CompactStats, ServiceError> {
    kv.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_open(namespace)?
        .compact()
}

/// Native OS (file-backed or in-memory) [`HostServiceHandler`] for [`SERVICE_STORAGE`].
pub struct NativeStorage {
    kv: Mutex<KvStoreManager>,
}

impl Default for NativeStorage {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl NativeStorage {
    pub fn in_memory() -> Self {
        Self {
            kv: Mutex::new(KvStoreManager::in_memory()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_root_dir(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            kv: Mutex::new(KvStoreManager::with_root_dir(root_dir)),
        }
    }
}

impl StorageBackend for NativeStorage {
    fn open(&self, namespace: &str) -> Result<StorageOpenInfo, ServiceError> {
        kv_backend_open(&self.kv, namespace)
    }

    fn get(&self, namespace: &str, key: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> {
        kv_backend_get(&self.kv, namespace, key)
    }

    fn put(&self, namespace: &str, key: &[u8], value: &[u8]) -> Result<(), ServiceError> {
        kv_backend_put(&self.kv, namespace, key, value)
    }

    fn delete(&self, namespace: &str, key: &[u8]) -> Result<bool, ServiceError> {
        kv_backend_delete(&self.kv, namespace, key)
    }

    fn scan_prefix(
        &self,
        namespace: &str,
        prefix: &[u8],
        limit: u32,
    ) -> Result<Vec<StorageEntry>, ServiceError> {
        kv_backend_scan_prefix(&self.kv, namespace, prefix, limit)
    }

    fn commit_batch(&self, namespace: &str, ops: &[StorageWriteOp]) -> Result<(), ServiceError> {
        kv_backend_commit_batch(&self.kv, namespace, ops)
    }

    fn compact(&self, namespace: &str) -> Result<CompactStats, ServiceError> {
        kv_backend_compact(&self.kv, namespace)
    }
}

impl HostServiceHandler for NativeStorage {
    fn service_id(&self) -> u32 {
        SERVICE_STORAGE
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_storage_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn StorageBackend>(self))
    }
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_STORAGE`].
pub struct MemoryStorage {
    kv: Mutex<KvStoreManager>,
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            kv: Mutex::new(KvStoreManager::in_memory()),
        }
    }
}

impl StorageBackend for MemoryStorage {
    fn open(&self, namespace: &str) -> Result<StorageOpenInfo, ServiceError> {
        kv_backend_open(&self.kv, namespace)
    }

    fn get(&self, namespace: &str, key: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> {
        kv_backend_get(&self.kv, namespace, key)
    }

    fn put(&self, namespace: &str, key: &[u8], value: &[u8]) -> Result<(), ServiceError> {
        kv_backend_put(&self.kv, namespace, key, value)
    }

    fn delete(&self, namespace: &str, key: &[u8]) -> Result<bool, ServiceError> {
        kv_backend_delete(&self.kv, namespace, key)
    }

    fn scan_prefix(
        &self,
        namespace: &str,
        prefix: &[u8],
        limit: u32,
    ) -> Result<Vec<StorageEntry>, ServiceError> {
        kv_backend_scan_prefix(&self.kv, namespace, prefix, limit)
    }

    fn commit_batch(&self, namespace: &str, ops: &[StorageWriteOp]) -> Result<(), ServiceError> {
        kv_backend_commit_batch(&self.kv, namespace, ops)
    }

    fn compact(&self, namespace: &str) -> Result<CompactStats, ServiceError> {
        kv_backend_compact(&self.kv, namespace)
    }
}

impl HostServiceHandler for MemoryStorage {
    fn service_id(&self) -> u32 {
        SERVICE_STORAGE
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_storage_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn StorageBackend>(self))
    }
}
