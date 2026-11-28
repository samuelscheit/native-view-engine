//! Standalone embedded crash-safe binary KV / record store (`NVK2` append-only WAL) with
//! direct [`KvStore`], [`KvStoreManager`], [`StorageBackend`], [`NativeStorage`], and
//! [`MemoryStorage`] APIs.
//!
//! Pure Rust library with zero dependency on `nve-engine`, `nve-store`, or `nve-host-services`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod backend;
pub mod engine;

use core::fmt;

use nve_codec::{CodecError, DEFAULT_MAX_ITEMS};

pub use backend::{MemoryStorage, NativeStorage, StorageBackend};
pub use engine::{KvStore, KvStoreManager, NVK2_COMMIT_MARKER, NVK2_MAGIC, NVK2_VERSION};

/// Maximum allowed namespace identifier length in bytes.
pub const MAX_NAMESPACE_LEN: usize = 128;
/// Maximum allowed key length in bytes (64 KiB).
pub const MAX_KEY_LEN: usize = 64 * 1024;
/// Maximum allowed value length in bytes (16 MiB).
pub const MAX_VALUE_LEN: usize = 16 * 1024 * 1024;
/// Maximum number of operations permitted in a single atomic batch or prefix scan.
pub const MAX_BATCH_OPS: usize = DEFAULT_MAX_ITEMS;

/// Error returned by [`KvStore`], [`KvStoreManager`], and [`StorageBackend`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// Caller supplied an invalid namespace, key, value, prefix, or batch.
    InvalidArg(String),
    /// Underlying filesystem I/O error.
    Io(String),
    /// Corrupt or malformed `NVK2` WAL header or record payload.
    Corrupt(String),
    /// Internal storage manager failure.
    Internal(String),
}

impl From<std::io::Error> for StorageError {
    #[inline]
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<CodecError> for StorageError {
    #[inline]
    fn from(err: CodecError) -> Self {
        Self::Corrupt(err.to_string())
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArg(msg) => write!(f, "invalid storage argument: {msg}"),
            Self::Io(msg) => write!(f, "storage I/O error: {msg}"),
            Self::Corrupt(msg) => write!(f, "corrupt NVK2 storage log: {msg}"),
            Self::Internal(msg) => write!(f, "internal storage error: {msg}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// Single mutation operation inside an atomic [`KvStore::commit_batch`] /
/// [`StorageBackend::commit_batch`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageWriteOp {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

/// Key/value pair returned by [`KvStore::scan_prefix`] / [`StorageBackend::scan_prefix`].
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

/// Statistics returned by [`KvStore::compact`] / [`StorageBackend::compact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactStats {
    pub live_keys: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

/// Validates a storage namespace name (safe single filename component, no traversal).
pub fn validate_namespace(namespace: &str) -> Result<(), StorageError> {
    if namespace.is_empty() || namespace.len() > MAX_NAMESPACE_LEN {
        return Err(StorageError::InvalidArg(format!(
            "storage namespace length must be 1..={MAX_NAMESPACE_LEN}"
        )));
    }
    if namespace == "." || namespace == ".." || namespace.contains("..") {
        return Err(StorageError::InvalidArg(
            "storage namespace must not contain path traversal segments".into(),
        ));
    }
    if !namespace
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(StorageError::InvalidArg(
            "storage namespace must contain only ASCII [A-Za-z0-9._-]".into(),
        ));
    }
    Ok(())
}

/// Validates a non-empty binary KV key.
pub fn validate_key(key: &[u8]) -> Result<(), StorageError> {
    if key.is_empty() {
        return Err(StorageError::InvalidArg(
            "storage key must not be empty".into(),
        ));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(StorageError::InvalidArg(format!(
            "storage key exceeds maximum length of {MAX_KEY_LEN} bytes"
        )));
    }
    Ok(())
}

/// Validates a binary KV value slice.
pub fn validate_value(value: &[u8]) -> Result<(), StorageError> {
    if value.len() > MAX_VALUE_LEN {
        return Err(StorageError::InvalidArg(format!(
            "storage value exceeds maximum length of {MAX_VALUE_LEN} bytes"
        )));
    }
    Ok(())
}

/// Validates every write operation in a batch prior to execution.
pub fn validate_batch(ops: &[StorageWriteOp]) -> Result<(), StorageError> {
    if ops.len() > MAX_BATCH_OPS {
        return Err(StorageError::InvalidArg(format!(
            "storage batch length {} exceeds limit {MAX_BATCH_OPS}",
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
