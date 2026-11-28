//! Direct [`StorageBackend`] trait plus thread-safe [`NativeStorage`] (file-backed or in-memory
//! `NVK2`) and deterministic in-memory [`MemoryStorage`] implementations.

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use crate::engine::KvStoreManager;
use crate::{CompactStats, StorageEntry, StorageError, StorageOpenInfo, StorageWriteOp, MAX_KEY_LEN};

/// Direct thread-safe backend trait for namespaced crash-safe binary KV storage.
pub trait StorageBackend: Send + Sync {
    fn open(&self, namespace: &str) -> Result<StorageOpenInfo, StorageError>;
    fn get(&self, namespace: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError>;
    fn put(&self, namespace: &str, key: &[u8], value: &[u8]) -> Result<(), StorageError>;
    fn delete(&self, namespace: &str, key: &[u8]) -> Result<bool, StorageError>;
    fn scan_prefix(
        &self,
        namespace: &str,
        prefix: &[u8],
        limit: u32,
    ) -> Result<Vec<StorageEntry>, StorageError>;
    fn commit_batch(&self, namespace: &str, ops: &[StorageWriteOp]) -> Result<(), StorageError>;
    fn compact(&self, namespace: &str) -> Result<CompactStats, StorageError>;
}

struct LockedKvStorage(Mutex<KvStoreManager>);

impl LockedKvStorage {
    fn new(manager: KvStoreManager) -> Self {
        Self(Mutex::new(manager))
    }

    fn lock(&self) -> MutexGuard<'_, KvStoreManager> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl StorageBackend for LockedKvStorage {
    fn open(&self, namespace: &str) -> Result<StorageOpenInfo, StorageError> {
        self.lock().open(namespace)
    }

    fn get(&self, namespace: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        self.lock().get_or_open(namespace)?.get(key)
    }

    fn put(&self, namespace: &str, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        self.lock().get_or_open(namespace)?.put(key, value)
    }

    fn delete(&self, namespace: &str, key: &[u8]) -> Result<bool, StorageError> {
        self.lock().get_or_open(namespace)?.delete(key)
    }

    fn scan_prefix(
        &self,
        namespace: &str,
        prefix: &[u8],
        limit: u32,
    ) -> Result<Vec<StorageEntry>, StorageError> {
        if prefix.len() > MAX_KEY_LEN {
            return Err(StorageError::InvalidArg(
                "storage prefix exceeds MAX_KEY_LEN".into(),
            ));
        }
        Ok(self.lock().get_or_open(namespace)?.scan_prefix(prefix, limit))
    }

    fn commit_batch(&self, namespace: &str, ops: &[StorageWriteOp]) -> Result<(), StorageError> {
        self.lock().get_or_open(namespace)?.commit_batch(ops)
    }

    fn compact(&self, namespace: &str) -> Result<CompactStats, StorageError> {
        self.lock().get_or_open(namespace)?.compact()
    }
}

/// Thread-safe file-backed (or in-memory) `NVK2` [`StorageBackend`].
pub struct NativeStorage {
    inner: LockedKvStorage,
}

impl Default for NativeStorage {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl NativeStorage {
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            inner: LockedKvStorage::new(KvStoreManager::in_memory()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn with_root_dir(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            inner: LockedKvStorage::new(KvStoreManager::with_root_dir(root_dir)),
        }
    }
}

/// Deterministic in-memory `NVK2` [`StorageBackend`] ideal for parallel unit tests and `wasm32`.
pub struct MemoryStorage {
    inner: LockedKvStorage,
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorage {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: LockedKvStorage::new(KvStoreManager::in_memory()),
        }
    }
}

macro_rules! delegate_storage_backend {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl StorageBackend for $ty {
                fn open(&self, namespace: &str) -> Result<StorageOpenInfo, StorageError> {
                    self.inner.open(namespace)
                }

                fn get(&self, namespace: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
                    self.inner.get(namespace, key)
                }

                fn put(&self, namespace: &str, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
                    self.inner.put(namespace, key, value)
                }

                fn delete(&self, namespace: &str, key: &[u8]) -> Result<bool, StorageError> {
                    self.inner.delete(namespace, key)
                }

                fn scan_prefix(
                    &self,
                    namespace: &str,
                    prefix: &[u8],
                    limit: u32,
                ) -> Result<Vec<StorageEntry>, StorageError> {
                    self.inner.scan_prefix(namespace, prefix, limit)
                }

                fn commit_batch(
                    &self,
                    namespace: &str,
                    ops: &[StorageWriteOp],
                ) -> Result<(), StorageError> {
                    self.inner.commit_batch(namespace, ops)
                }

                fn compact(&self, namespace: &str) -> Result<CompactStats, StorageError> {
                    self.inner.compact(namespace)
                }
            }
        )+
    };
}

delegate_storage_backend!(NativeStorage, MemoryStorage);
