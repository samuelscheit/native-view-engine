//! Binary automatic form draft persistence (`NVE_FORM_DRAFT_V1`), pluggable
//! [`FormDraftStorage`] backends (crash-atomic [`NativeMmapFormDraftStorage`] over
//! [`nve_engine::NativeStateStore`]/`MmapOffsetArena`, Web [`StringKvFormDraftAdapter`] /
//! `localStorage`, and deterministic [`MemoryFormDraftStorage`]), and [`FormController`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use nve_abi::xxh64;
use nve_codec::{ByteReader, ByteWriter, CodecError, DEFAULT_MAX_BYTES, DEFAULT_MAX_ITEMS};
use nve_engine::{NativeStateStore, StateStoreConfig};

pub use crate::form_controller::FormController;

/// Self-documenting binary envelope header for persisted unsaved form drafts.
pub const FORM_DRAFT_MAGIC: &[u8; 17] = b"NVE_FORM_DRAFT_V1";

/// Binary envelope schema version.
pub const FORM_DRAFT_SCHEMA_VERSION: u32 = 1;

/// Binary table header used by [`NativeMmapFormDraftStorage`] to store multiple keyed drafts
/// crash-atomically inside one `MmapOffsetArena`.
const FORM_DRAFT_TABLE_MAGIC: &[u8; 23] = b"NVE_FORM_DRAFT_TABLE_V1";

/// Storage key prefix used by string key-value backends (`localStorage`, `WebSnapshotStore`).
pub const FORM_DRAFT_KEY_PREFIX: &str = "nve:form-draft:";

/// Text framing prefix used when storing binary `NVE_FORM_DRAFT_V1` inside string KV stores.
const FORM_DRAFT_TEXT_PREFIX: &str = "NVE_FORM_DRAFT_V1|";

/// Default maximum size (128 KiB) for one serialized form draft payload.
pub const DEFAULT_MAX_FORM_DRAFT_BYTES: usize = 128 * 1024;

/// Errors returned by [`FormDraftStorage`] and [`FormDraftStore`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormDraftError {
    SizeLimitExceeded { bytes: usize, limit: usize },
    StorageQuotaExceeded,
    StorageUnavailable,
    Persistence(String),
}

impl fmt::Display for FormDraftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeLimitExceeded { bytes, limit } => {
                write!(f, "form draft size {bytes} exceeds limit {limit}")
            }
            Self::StorageQuotaExceeded => write!(f, "form draft storage quota exceeded"),
            Self::StorageUnavailable => write!(f, "form draft storage unavailable"),
            Self::Persistence(msg) => write!(f, "form draft persistence error: {msg}"),
        }
    }
}

impl std::error::Error for FormDraftError {}

/// Encodes `fields` into a compact, binary FFI, XXH64-checksummed `NVE_FORM_DRAFT_V1` binary blob.
pub fn encode_form_draft(fields: &BTreeMap<String, String>) -> Result<Vec<u8>, CodecError> {
    let payload_hint: usize = fields.iter().map(|(k, v)| 8 + k.len() + v.len()).sum();
    let mut w = ByteWriter::with_capacity(FORM_DRAFT_MAGIC.len() + 8 + payload_hint + 8);
    w.write_raw(FORM_DRAFT_MAGIC);
    w.write_u32(FORM_DRAFT_SCHEMA_VERSION);
    w.write_len_max(fields.len(), DEFAULT_MAX_ITEMS)?;
    for (k, v) in fields {
        w.write_str_max(k, DEFAULT_MAX_BYTES)?;
        w.write_str_max(v, DEFAULT_MAX_BYTES)?;
    }
    let checksum = xxh64(w.as_slice(), 0);
    w.write_u64(checksum);
    Ok(w.into_bytes())
}

/// Strictly decodes and verifies an `NVE_FORM_DRAFT_V1` binary draft blob.
pub fn decode_form_draft(bytes: &[u8]) -> Option<BTreeMap<String, String>> {
    let min_len = FORM_DRAFT_MAGIC.len() + 8 + 8;
    if bytes.len() < min_len {
        return None;
    }
    let (body, crc_bytes) = bytes.split_at(bytes.len() - 8);
    let expected_crc = ByteReader::new(crc_bytes).read_u64().ok()?;
    if xxh64(body, 0) != expected_crc {
        return None;
    }
    let mut r = ByteReader::new(body);
    r.expect_magic(FORM_DRAFT_MAGIC).ok()?;
    let version = r.read_u32().ok()?;
    if version != FORM_DRAFT_SCHEMA_VERSION {
        return None;
    }
    let count = r.read_len(DEFAULT_MAX_ITEMS, 8).ok()?;
    let mut fields = BTreeMap::new();
    for _ in 0..count {
        let key = r.read_string(DEFAULT_MAX_BYTES).ok()?;
        let val = r.read_string(DEFAULT_MAX_BYTES).ok()?;
        fields.insert(key, val);
    }
    r.finish().ok()?;
    Some(fields)
}

/// Pluggable persistence backend for [`FormDraftStore`] and [`FormController`].
pub trait FormDraftStorage {
    fn load_draft(&self, key: &str) -> Option<Vec<u8>>;
    fn save_draft(&mut self, key: &str, bytes: &[u8]) -> Result<(), FormDraftError>;
    fn clear_draft(&mut self, key: &str);
}

/// Deterministic in-memory [`FormDraftStorage`] for tests and ephemeral forms.
#[derive(Debug, Clone, Default)]
pub struct MemoryFormDraftStorage {
    entries: BTreeMap<String, Vec<u8>>,
}

impl MemoryFormDraftStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl FormDraftStorage for MemoryFormDraftStorage {
    fn load_draft(&self, key: &str) -> Option<Vec<u8>> {
        self.entries.get(key).cloned()
    }

    fn save_draft(&mut self, key: &str, bytes: &[u8]) -> Result<(), FormDraftError> {
        self.entries.insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn clear_draft(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

/// Minimal string key-value store trait bridging `WebSnapshotStore` / `SnapshotStorage` /
/// browser `localStorage` into [`FormDraftStorage`] without circular crate dependencies.
pub trait StringKeyValueDraftBackend {
    fn get_string(&self, key: &str) -> Option<String>;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), FormDraftError>;
    fn remove_string(&mut self, key: &str);
}

/// Adapts any [`StringKeyValueDraftBackend`] (e.g. `MemorySnapshotStorage`, `BrowserLocalStorage`,
/// `WebSnapshotStore`) into a binary [`FormDraftStorage`].
#[derive(Debug, Clone)]
pub struct StringKeyValueFormDraftStorage<S> {
    backend: S,
}

impl<S: StringKeyValueDraftBackend> StringKeyValueFormDraftStorage<S> {
    pub fn new(backend: S) -> Self {
        Self { backend }
    }

    pub fn storage_key(persist_key: &str) -> String {
        format!("{FORM_DRAFT_KEY_PREFIX}{persist_key}")
    }

    pub fn inner(&self) -> &S {
        &self.backend
    }

    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.backend
    }
}

impl<S: StringKeyValueDraftBackend> FormDraftStorage for StringKeyValueFormDraftStorage<S> {
    fn load_draft(&self, key: &str) -> Option<Vec<u8>> {
        let raw = self.backend.get_string(&Self::storage_key(key))?;
        let b64 = raw.strip_prefix(FORM_DRAFT_TEXT_PREFIX)?;
        BASE64_STANDARD.decode(b64).ok()
    }

    fn save_draft(&mut self, key: &str, bytes: &[u8]) -> Result<(), FormDraftError> {
        let mut encoded = String::with_capacity(FORM_DRAFT_TEXT_PREFIX.len() + bytes.len().div_ceil(3) * 4);
        encoded.push_str(FORM_DRAFT_TEXT_PREFIX);
        BASE64_STANDARD.encode_string(bytes, &mut encoded);
        self.backend.set_string(&Self::storage_key(key), &encoded)
    }

    fn clear_draft(&mut self, key: &str) {
        self.backend.remove_string(&Self::storage_key(key));
    }
}

/// Browser `window.localStorage` [`FormDraftStorage`] implementation for `wasm32`.
#[cfg(target_arch = "wasm32")]
pub struct BrowserLocalStorageFormDraftStorage {
    storage: web_sys::Storage,
}

#[cfg(target_arch = "wasm32")]
impl BrowserLocalStorageFormDraftStorage {
    pub fn open() -> Option<Self> {
        let storage = web_sys::window()?.local_storage().ok()??;
        Some(Self { storage })
    }
}

#[cfg(target_arch = "wasm32")]
impl StringKeyValueDraftBackend for BrowserLocalStorageFormDraftStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.storage.get_item(key).ok().flatten()
    }

    fn set_string(&mut self, key: &str, value: &str) -> Result<(), FormDraftError> {
        self.storage
            .set_item(key, value)
            .map_err(|_| FormDraftError::StorageQuotaExceeded)
    }

    fn remove_string(&mut self, key: &str) {
        let _ = self.storage.remove_item(key);
    }
}

/// Crash-atomic native [`FormDraftStorage`] backed directly by double-buffered
/// [`NativeStateStore`] / `MmapOffsetArena`.
pub struct NativeMmapFormDraftStorage {
    store: NativeStateStore,
}

impl NativeMmapFormDraftStorage {
    /// Opens or initializes a crash-atomic `MmapOffsetArena` draft file at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FormDraftError> {
        let store = NativeStateStore::open(
            path,
            FORM_DRAFT_SCHEMA_VERSION,
            StateStoreConfig::default(),
        )
        .map_err(|e| FormDraftError::Persistence(e.to_string()))?;
        Ok(Self { store })
    }

    /// Creates an in-memory `MmapOffsetArena`-backed draft store (useful for headless tests).
    pub fn new_in_memory() -> Result<Self, FormDraftError> {
        let store = NativeStateStore::new_in_memory(
            FORM_DRAFT_SCHEMA_VERSION,
            StateStoreConfig::default(),
        )
        .map_err(|e| FormDraftError::Persistence(e.to_string()))?;
        Ok(Self { store })
    }

    pub fn generation(&self) -> u64 {
        self.store.generation()
    }

    fn load_table(&self) -> BTreeMap<String, Vec<u8>> {
        let Ok(Some(bytes)) = self.store.load_bytes() else {
            return BTreeMap::new();
        };
        decode_draft_table(bytes).unwrap_or_default()
    }

    fn save_table(&mut self, table: &BTreeMap<String, Vec<u8>>) -> Result<(), FormDraftError> {
        if table.is_empty() {
            self.store
                .reset()
                .map_err(|e| FormDraftError::Persistence(e.to_string()))?;
            return Ok(());
        }
        let encoded = encode_draft_table(table)
            .map_err(|e| FormDraftError::Persistence(e.to_string()))?;
        self.store
            .save(&encoded)
            .map_err(|e| FormDraftError::Persistence(e.to_string()))?;
        Ok(())
    }
}

impl FormDraftStorage for NativeMmapFormDraftStorage {
    fn load_draft(&self, key: &str) -> Option<Vec<u8>> {
        self.load_table().remove(key)
    }

    fn save_draft(&mut self, key: &str, bytes: &[u8]) -> Result<(), FormDraftError> {
        let mut table = self.load_table();
        table.insert(key.to_owned(), bytes.to_vec());
        self.save_table(&table)
    }

    fn clear_draft(&mut self, key: &str) {
        let mut table = self.load_table();
        if table.remove(key).is_some() {
            let _ = self.save_table(&table);
        }
    }
}

fn encode_draft_table(table: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, CodecError> {
    let mut w = ByteWriter::with_capacity(64 + table.len() * 64);
    w.write_raw(FORM_DRAFT_TABLE_MAGIC);
    w.write_len_max(table.len(), DEFAULT_MAX_ITEMS)?;
    for (k, blob) in table {
        w.write_str_max(k, DEFAULT_MAX_BYTES)?;
        w.write_bytes_max(blob, DEFAULT_MAX_BYTES)?;
    }
    Ok(w.into_bytes())
}

fn decode_draft_table(bytes: &[u8]) -> Option<BTreeMap<String, Vec<u8>>> {
    let mut r = ByteReader::new(bytes);
    r.expect_magic(FORM_DRAFT_TABLE_MAGIC).ok()?;
    let count = r.read_len(DEFAULT_MAX_ITEMS, 8).ok()?;
    let mut table = BTreeMap::new();
    for _ in 0..count {
        let key = r.read_string(DEFAULT_MAX_BYTES).ok()?;
        let blob = r.read_bytes(DEFAULT_MAX_BYTES).ok()?.to_vec();
        table.insert(key, blob);
    }
    r.finish().ok()?;
    Some(table)
}

/// High-level typed draft store enforcing size guards and `NVE_FORM_DRAFT_V1` encoding.
pub struct FormDraftStore<S: FormDraftStorage> {
    storage: S,
    max_bytes: usize,
}

impl<S: FormDraftStorage> FormDraftStore<S> {
    pub fn new(storage: S) -> Self {
        Self {
            storage,
            max_bytes: DEFAULT_MAX_FORM_DRAFT_BYTES,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    pub fn save_fields(
        &mut self,
        persist_key: &str,
        fields: &BTreeMap<String, String>,
    ) -> Result<usize, FormDraftError> {
        let encoded = encode_form_draft(fields).map_err(|err| match err {
            CodecError::LenOverflow { len, .. } => FormDraftError::SizeLimitExceeded {
                bytes: len,
                limit: self.max_bytes,
            },
            other => FormDraftError::Persistence(other.to_string()),
        })?;
        if encoded.len() > self.max_bytes {
            return Err(FormDraftError::SizeLimitExceeded {
                bytes: encoded.len(),
                limit: self.max_bytes,
            });
        }
        self.storage.save_draft(persist_key, &encoded)?;
        Ok(encoded.len())
    }

    pub fn load_fields(&self, persist_key: &str) -> Option<BTreeMap<String, String>> {
        let bytes = self.storage.load_draft(persist_key)?;
        decode_form_draft(&bytes)
    }

    pub fn clear(&mut self, persist_key: &str) {
        self.storage.clear_draft(persist_key);
    }
}

pub(crate) fn default_draft_storage() -> Box<dyn FormDraftStorage> {
    #[cfg(target_arch = "wasm32")]
    if let Some(ls) = BrowserLocalStorageFormDraftStorage::open() {
        return Box::new(StringKeyValueFormDraftStorage::new(ls));
    }
    Box::new(MemoryFormDraftStorage::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_form_draft_enforces_decoder_byte_and_item_caps() {
        let mut oversized_val = BTreeMap::new();
        oversized_val.insert("body".to_owned(), "x".repeat(DEFAULT_MAX_BYTES + 1));
        assert_eq!(
            encode_form_draft(&oversized_val),
            Err(CodecError::LenOverflow {
                len: DEFAULT_MAX_BYTES + 1,
                max: DEFAULT_MAX_BYTES,
            })
        );

        let mut store = FormDraftStore::new(MemoryFormDraftStorage::new()).with_max_bytes(1024);
        assert_eq!(
            store.save_fields("k", &oversized_val),
            Err(FormDraftError::SizeLimitExceeded {
                bytes: DEFAULT_MAX_BYTES + 1,
                limit: 1024,
            })
        );
    }
}
