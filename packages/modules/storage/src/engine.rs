//! Crash-safe `NVK2` append-only binary KV persistence engine backed by `nve-codec` and `nve-hash`.

use std::collections::BTreeMap;
use std::ops::Bound;

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{self, File, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use nve_codec::ByteWriter;

pub use crate::wal::{NVK2_COMMIT_MARKER, NVK2_MAGIC, NVK2_VERSION};
use crate::wal::{
    encode_file_header, encode_record_frame_into, replay_nvk2_buffer, stream_compacted_entries,
    HEADER_LEN, KIND_BATCH, KIND_DELETE, KIND_PUT,
};
use crate::{
    validate_batch, validate_key, validate_namespace, validate_value, CompactStats, StorageEntry,
    StorageError, StorageOpenInfo, StorageWriteOp, MAX_BATCH_OPS,
};

enum LogBackend {
    Memory(Vec<u8>),
    #[cfg(not(target_arch = "wasm32"))]
    Disk {
        path: PathBuf,
        file: Option<File>,
        byte_len: u64,
    },
}

/// Crash-safe append-only `NVK2` binary KV store supporting both disk and in-memory modes.
pub struct KvStore {
    next_seq: u64,
    recovered_torn_tail: bool,
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
    backend: LogBackend,
}

impl KvStore {
    /// Opens a fresh in-memory `NVK2` KV store.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            next_seq: 1,
            recovered_torn_tail: false,
            entries: BTreeMap::new(),
            backend: LogBackend::Memory(encode_file_header().to_vec()),
        }
    }

    /// Replays and opens an in-memory `NVK2` KV store from an existing WAL byte buffer.
    pub fn open_from_memory_log(mut bytes: Vec<u8>) -> Result<Self, StorageError> {
        if bytes.is_empty() {
            return Ok(Self::in_memory());
        }
        let replay = replay_nvk2_buffer(&bytes)?;
        if replay.valid_len < bytes.len() {
            bytes.truncate(replay.valid_len);
        }
        Ok(Self {
            next_seq: replay.last_seq.saturating_add(1),
            recovered_torn_tail: replay.recovered_torn_tail,
            entries: replay.entries,
            backend: LogBackend::Memory(bytes),
        })
    }

    /// Opens or creates a crash-safe `NVK2` KV store directly at `path`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;

        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        if bytes.is_empty() {
            let hdr = encode_file_header();
            file.write_all(&hdr)?;
            file.sync_all()?;
            #[cfg(unix)]
            sync_parent_dir(&path)?;
            return Ok(Self {
                next_seq: 1,
                recovered_torn_tail: false,
                entries: BTreeMap::new(),
                backend: LogBackend::Disk {
                    path,
                    file: Some(file),
                    byte_len: HEADER_LEN as u64,
                },
            });
        }

        let replay = replay_nvk2_buffer(&bytes)?;
        let valid_len = replay.valid_len as u64;
        if replay.recovered_torn_tail {
            file.set_len(valid_len)?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            next_seq: replay.last_seq.saturating_add(1),
            recovered_torn_tail: replay.recovered_torn_tail,
            entries: replay.entries,
            backend: LogBackend::Disk {
                path,
                file: Some(file),
                byte_len: valid_len,
            },
        })
    }

    #[must_use]
    pub fn open_info(&self) -> StorageOpenInfo {
        StorageOpenInfo {
            key_count: self.entries.len() as u64,
            log_bytes: self.log_bytes(),
            recovered_torn_tail: self.recovered_torn_tail,
        }
    }

    #[must_use]
    pub fn log_bytes(&self) -> u64 {
        match &self.backend {
            LogBackend::Memory(buf) => buf.len() as u64,
            #[cfg(not(target_arch = "wasm32"))]
            LogBackend::Disk { byte_len, .. } => *byte_len,
        }
    }

    #[must_use]
    pub fn memory_log_bytes(&self) -> Option<&[u8]> {
        match &self.backend {
            LogBackend::Memory(buf) => Some(buf),
            #[cfg(not(target_arch = "wasm32"))]
            LogBackend::Disk { .. } => None,
        }
    }

    /// Zero-copy borrowed lookup for `key`.
    pub fn get_ref(&self, key: &[u8]) -> Result<Option<&[u8]>, StorageError> {
        validate_key(key)?;
        Ok(self.entries.get(key).map(Vec::as_slice))
    }

    /// Owned lookup cloning the stored value for `key`.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self.get_ref(key)?.map(ToOwned::to_owned))
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        validate_key(key)?;
        validate_value(value)?;
        let mut payload = ByteWriter::with_capacity(8 + key.len() + value.len());
        payload.write_bytes(key)?;
        payload.write_bytes(value)?;
        let seq = self.next_seq;
        let mut frame = Vec::new();
        encode_record_frame_into(&mut frame, KIND_PUT, seq, payload.as_slice())?;
        self.append_frame(&frame)?;
        self.next_seq = seq.saturating_add(1);
        self.entries.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<bool, StorageError> {
        validate_key(key)?;
        if !self.entries.contains_key(key) {
            return Ok(false);
        }
        let mut payload = ByteWriter::with_capacity(4 + key.len());
        payload.write_bytes(key)?;
        let seq = self.next_seq;
        let mut frame = Vec::new();
        encode_record_frame_into(&mut frame, KIND_DELETE, seq, payload.as_slice())?;
        self.append_frame(&frame)?;
        self.next_seq = seq.saturating_add(1);
        self.entries.remove(key);
        Ok(true)
    }

    #[must_use]
    pub fn scan_prefix(&self, prefix: &[u8], limit: u32) -> Vec<StorageEntry> {
        let cap = (limit as usize).min(MAX_BATCH_OPS);
        if cap == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (k, v) in self
            .entries
            .range::<[u8], _>((Bound::Included(prefix), Bound::Unbounded))
        {
            if !k.starts_with(prefix) {
                break;
            }
            out.push(StorageEntry {
                key: k.clone(),
                value: v.clone(),
            });
            if out.len() >= cap {
                break;
            }
        }
        out
    }

    pub fn commit_batch(&mut self, ops: &[StorageWriteOp]) -> Result<(), StorageError> {
        validate_batch(ops)?;
        if ops.is_empty() {
            return Ok(());
        }
        let mut payload = ByteWriter::with_capacity(4 + ops.len() * 16);
        payload.write_u32(ops.len() as u32);
        for op in ops {
            match op {
                StorageWriteOp::Put { key, value } => {
                    payload.write_u8(KIND_PUT);
                    payload.write_bytes(key)?;
                    payload.write_bytes(value)?;
                }
                StorageWriteOp::Delete { key } => {
                    payload.write_u8(KIND_DELETE);
                    payload.write_bytes(key)?;
                }
            }
        }
        let seq = self.next_seq;
        let mut frame = Vec::new();
        encode_record_frame_into(&mut frame, KIND_BATCH, seq, payload.as_slice())?;
        self.append_frame(&frame)?;
        self.next_seq = seq.saturating_add(1);
        for op in ops {
            match op {
                StorageWriteOp::Put { key, value } => {
                    self.entries.insert(key.clone(), value.clone());
                }
                StorageWriteOp::Delete { key } => {
                    self.entries.remove(key);
                }
            }
        }
        Ok(())
    }

    pub fn compact(&mut self) -> Result<CompactStats, StorageError> {
        let bytes_before = self.log_bytes();
        let (next_seq, bytes_after) = match &mut self.backend {
            LogBackend::Memory(buf) => {
                let mut compacted = Vec::with_capacity(HEADER_LEN + self.entries.len() * 32);
                let (seq, written) = stream_compacted_entries(&self.entries, &mut compacted)?;
                *buf = compacted;
                (seq, written)
            }
            #[cfg(not(target_arch = "wasm32"))]
            LogBackend::Disk {
                path,
                file,
                byte_len,
            } => {
                let tmp_path = temp_compact_path(path);
                let (seq, written) = match write_compacted_temp(&tmp_path, &self.entries) {
                    Ok(pair) => pair,
                    Err(err) => {
                        let _ = fs::remove_file(&tmp_path);
                        return Err(err);
                    }
                };

                // Close the live handle BEFORE renaming so Windows does not reject the atomic
                // replace with a sharing violation and POSIX never keeps writing to an unlinked
                // old inode if reopen subsequently fails.
                *file = None;

                if let Err(err) = fs::rename(&tmp_path, &*path) {
                    let _ = fs::remove_file(&tmp_path);
                    if let Ok(mut restored) =
                        OpenOptions::new().read(true).write(true).open(&*path)
                        && restored.seek(SeekFrom::Start(*byte_len)).is_ok()
                    {
                        *file = Some(restored);
                    }
                    return Err(err.into());
                }

                #[cfg(unix)]
                sync_parent_dir(path)?;

                let mut reopened = OpenOptions::new().read(true).write(true).open(&*path)?;
                reopened.seek(SeekFrom::End(0))?;
                *file = Some(reopened);
                *byte_len = written;
                (seq, written)
            }
        };

        self.next_seq = next_seq;
        self.recovered_torn_tail = false;

        Ok(CompactStats {
            live_keys: self.entries.len() as u64,
            bytes_before,
            bytes_after,
        })
    }

    fn append_frame(&mut self, frame: &[u8]) -> Result<(), StorageError> {
        match &mut self.backend {
            LogBackend::Memory(buf) => {
                buf.extend_from_slice(frame);
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            LogBackend::Disk {
                path,
                file,
                byte_len,
            } => {
                let Some(f) = file.as_mut() else {
                    return Err(StorageError::Io(format!(
                        "storage file {} is closed after failed compaction",
                        path.display()
                    )));
                };
                if let Err(err) = f.write_all(frame).and_then(|()| f.sync_data()) {
                    // Roll back any partial/unsynced tail so later writes never append past torn
                    // bytes and lose committed records on reopen.
                    let _ = f.set_len(*byte_len);
                    let _ = f.seek(SeekFrom::Start(*byte_len));
                    return Err(err.into());
                }
                *byte_len = byte_len.saturating_add(frame.len() as u64);
                Ok(())
            }
        }
    }
}

/// Manages named `NVK2` namespaces either in-memory or under a root directory on disk.
pub struct KvStoreManager {
    #[cfg(not(target_arch = "wasm32"))]
    root_dir: Option<PathBuf>,
    stores: BTreeMap<String, KvStore>,
}

impl KvStoreManager {
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            root_dir: None,
            stores: BTreeMap::new(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn with_root_dir(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: Some(root_dir.into()),
            stores: BTreeMap::new(),
        }
    }

    pub fn open(&mut self, namespace: &str) -> Result<StorageOpenInfo, StorageError> {
        validate_namespace(namespace)?;
        if let Some(store) = self.stores.get(namespace) {
            return Ok(store.open_info());
        }
        let store = self.create_store(namespace)?;
        let info = store.open_info();
        self.stores.insert(namespace.to_owned(), store);
        Ok(info)
    }

    pub fn get_or_open(&mut self, namespace: &str) -> Result<&mut KvStore, StorageError> {
        validate_namespace(namespace)?;
        if !self.stores.contains_key(namespace) {
            self.open(namespace)?;
        }
        self.stores
            .get_mut(namespace)
            .ok_or_else(|| StorageError::Internal("missing storage namespace".into()))
    }

    fn create_store(&self, namespace: &str) -> Result<KvStore, StorageError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(root) = &self.root_dir {
            let path = root.join(format!("{namespace}.nvk2"));
            return KvStore::open(path);
        }
        let _ = namespace;
        Ok(KvStore::in_memory())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_compacted_temp(
    tmp_path: &Path,
    entries: &BTreeMap<Vec<u8>, Vec<u8>>,
) -> Result<(u64, u64), StorageError> {
    let tmp = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(tmp_path)?;
    let mut buffered = BufWriter::new(tmp);
    let pair = stream_compacted_entries(entries, &mut buffered)?;
    buffered.flush()?;
    let tmp = buffered.into_inner().map_err(|e| e.into_error())?;
    tmp.sync_all()?;
    Ok(pair)
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    File::open(parent)?.sync_all()
}

#[cfg(not(target_arch = "wasm32"))]
fn temp_compact_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".compact.tmp");
    PathBuf::from(os)
}
