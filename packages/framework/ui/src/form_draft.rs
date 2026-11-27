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
pub fn encode_form_draft(fields: &BTreeMap<String, String>) -> Vec<u8> {
    let payload_hint: usize = fields.iter().map(|(k, v)| 8 + k.len() + v.len()).sum();
    let mut out = Vec::with_capacity(FORM_DRAFT_MAGIC.len() + 8 + payload_hint + 8);
    out.extend_from_slice(FORM_DRAFT_MAGIC);
    out.extend_from_slice(&FORM_DRAFT_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for (k, v) in fields {
        let kb = k.as_bytes();
        let vb = v.as_bytes();
        out.extend_from_slice(&(kb.len() as u32).to_le_bytes());
        out.extend_from_slice(kb);
        out.extend_from_slice(&(vb.len() as u32).to_le_bytes());
        out.extend_from_slice(vb);
    }
    let checksum = xxh64(&out, 0);
    out.extend_from_slice(&checksum.to_le_bytes());
    out
}

/// Strictly decodes and verifies an `NVE_FORM_DRAFT_V1` binary draft blob.
pub fn decode_form_draft(bytes: &[u8]) -> Option<BTreeMap<String, String>> {
    let min_len = FORM_DRAFT_MAGIC.len() + 8 + 8;
    if bytes.len() < min_len {
        return None;
    }
    let (body, crc_bytes) = bytes.split_at(bytes.len() - 8);
    let expected_crc = u64::from_le_bytes(crc_bytes.try_into().ok()?);
    if xxh64(body, 0) != expected_crc {
        return None;
    }
    if !body.starts_with(FORM_DRAFT_MAGIC) {
        return None;
    }
    let mut pos = FORM_DRAFT_MAGIC.len();
    let version = read_u32(body, &mut pos)?;
    if version != FORM_DRAFT_SCHEMA_VERSION {
        return None;
    }
    let count = read_u32(body, &mut pos)? as usize;
    let mut fields = BTreeMap::new();
    for _ in 0..count {
        let key = read_utf8_str(body, &mut pos)?;
        let val = read_utf8_str(body, &mut pos)?;
        fields.insert(key, val);
    }
    if pos != body.len() {
        return None;
    }
    Some(fields)
}

#[inline]
fn read_u32(buf: &[u8], pos: &mut usize) -> Option<u32> {
    let end = pos.checked_add(4)?;
    let slice = buf.get(*pos..end)?;
    *pos = end;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

fn read_utf8_str(buf: &[u8], pos: &mut usize) -> Option<String> {
    let len = read_u32(buf, pos)? as usize;
    let end = pos.checked_add(len)?;
    let slice = buf.get(*pos..end)?;
    *pos = end;
    std::str::from_utf8(slice).ok().map(str::to_owned)
}

fn read_byte_vec(buf: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    let len = read_u32(buf, pos)? as usize;
    let end = pos.checked_add(len)?;
    let slice = buf.get(*pos..end)?;
    *pos = end;
    Some(slice.to_vec())
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
        let encoded = encode_draft_table(table);
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

fn encode_draft_table(table: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + table.len() * 64);
    out.extend_from_slice(FORM_DRAFT_TABLE_MAGIC);
    out.extend_from_slice(&(table.len() as u32).to_le_bytes());
    for (k, blob) in table {
        let kb = k.as_bytes();
        out.extend_from_slice(&(kb.len() as u32).to_le_bytes());
        out.extend_from_slice(kb);
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(blob);
    }
    out
}

fn decode_draft_table(bytes: &[u8]) -> Option<BTreeMap<String, Vec<u8>>> {
    if !bytes.starts_with(FORM_DRAFT_TABLE_MAGIC) {
        return None;
    }
    let mut pos = FORM_DRAFT_TABLE_MAGIC.len();
    let count = read_u32(bytes, &mut pos)? as usize;
    let mut table = BTreeMap::new();
    for _ in 0..count {
        let key = read_utf8_str(bytes, &mut pos)?;
        let blob = read_byte_vec(bytes, &mut pos)?;
        table.insert(key, blob);
    }
    if pos != bytes.len() {
        return None;
    }
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
        let encoded = encode_form_draft(fields);
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
