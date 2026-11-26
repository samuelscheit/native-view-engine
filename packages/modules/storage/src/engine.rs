//! Zero-dependency crash-safe `NVK2` append-only binary KV persistence engine.

use std::collections::BTreeMap;

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{self, File, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use nve_hash::xxh64;
use nve_host_services::{BinReader, BinWriter, ServiceError, MAX_SEQ_ITEMS};

use crate::{
    validate_batch, validate_key, validate_namespace, validate_value, CompactStats, StorageEntry,
    StorageOpenInfo, StorageWriteOp,
};

/// File magic for the crash-safe append-only KV log.
pub const NVK2_MAGIC: [u8; 4] = *b"NVK2";
/// Current binary log wire version.
pub const NVK2_VERSION: u16 = 1;
/// Atomic record commit marker placed after the per-record XXH64 checksum.
pub const NVK2_COMMIT_MARKER: [u8; 4] = *b"NKCM";

const HEADER_LEN: usize = 16;
const RECORD_HEADER_LEN: usize = 16;
const RECORD_TRAILER_LEN: usize = 12;

const KIND_PUT: u8 = 1;
const KIND_DELETE: u8 = 2;
const KIND_BATCH: u8 = 3;

enum LogBackend {
    Memory(Vec<u8>),
    #[cfg(not(target_arch = "wasm32"))]
    Disk {
        path: PathBuf,
        file: File,
        byte_len: u64,
    },
}

/// Crash-safe append-only `NVK2` binary KV store supporting both disk and in-memory modes.
pub struct KvStore {
    handle: u64,
    next_seq: u64,
    recovered_torn_tail: bool,
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
    backend: LogBackend,
}

impl KvStore {
    pub fn open_in_memory(handle: u64) -> Self {
        let mut buf = Vec::with_capacity(HEADER_LEN);
        write_file_header(&mut buf);
        Self {
            handle,
            next_seq: 1,
            recovered_torn_tail: false,
            entries: BTreeMap::new(),
            backend: LogBackend::Memory(buf),
        }
    }

    pub fn open_from_memory_log(handle: u64, mut bytes: Vec<u8>) -> Result<Self, ServiceError> {
        if bytes.is_empty() {
            return Ok(Self::open_in_memory(handle));
        }
        let replay = replay_nvk1_buffer(&bytes)?;
        if replay.valid_len < bytes.len() {
            bytes.truncate(replay.valid_len);
        }
        Ok(Self {
            handle,
            next_seq: replay.last_seq.saturating_add(1),
            recovered_torn_tail: replay.recovered_torn_tail,
            entries: replay.entries,
            backend: LogBackend::Memory(bytes),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_file(handle: u64, path: impl AsRef<Path>) -> Result<Self, ServiceError> {
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
            let mut hdr = Vec::with_capacity(HEADER_LEN);
            write_file_header(&mut hdr);
            file.write_all(&hdr)?;
            file.sync_all()?;
            return Ok(Self {
                handle,
                next_seq: 1,
                recovered_torn_tail: false,
                entries: BTreeMap::new(),
                backend: LogBackend::Disk {
                    path,
                    file,
                    byte_len: HEADER_LEN as u64,
                },
            });
        }

        let replay = replay_nvk1_buffer(&bytes)?;
        let valid_len = replay.valid_len as u64;
        if replay.recovered_torn_tail {
            file.set_len(valid_len)?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            handle,
            next_seq: replay.last_seq.saturating_add(1),
            recovered_torn_tail: replay.recovered_torn_tail,
            entries: replay.entries,
            backend: LogBackend::Disk {
                path,
                file,
                byte_len: valid_len,
            },
        })
    }

    pub fn open_info(&self) -> StorageOpenInfo {
        StorageOpenInfo {
            handle: self.handle,
            key_count: self.entries.len() as u64,
            log_bytes: self.log_bytes(),
            recovered_torn_tail: self.recovered_torn_tail,
        }
    }

    pub fn log_bytes(&self) -> u64 {
        match &self.backend {
            LogBackend::Memory(buf) => buf.len() as u64,
            #[cfg(not(target_arch = "wasm32"))]
            LogBackend::Disk { byte_len, .. } => *byte_len,
        }
    }

    pub fn memory_log_bytes(&self) -> Option<&[u8]> {
        match &self.backend {
            LogBackend::Memory(buf) => Some(buf),
            #[cfg(not(target_arch = "wasm32"))]
            LogBackend::Disk { .. } => None,
        }
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> {
        validate_key(key)?;
        Ok(self.entries.get(key).cloned())
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), ServiceError> {
        validate_key(key)?;
        validate_value(value)?;
        let mut payload = BinWriter::with_capacity(8 + key.len() + value.len());
        payload.write_bytes(key)?;
        payload.write_bytes(value)?;
        let seq = self.next_seq;
        let frame = encode_record_frame(KIND_PUT, seq, payload.as_slice())?;
        self.append_frame(&frame)?;
        self.next_seq = seq.saturating_add(1);
        self.entries.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<bool, ServiceError> {
        validate_key(key)?;
        let existed = self.entries.contains_key(key);
        if !existed {
            return Ok(false);
        }
        let mut payload = BinWriter::with_capacity(4 + key.len());
        payload.write_bytes(key)?;
        let seq = self.next_seq;
        let frame = encode_record_frame(KIND_DELETE, seq, payload.as_slice())?;
        self.append_frame(&frame)?;
        self.next_seq = seq.saturating_add(1);
        self.entries.remove(key);
        Ok(true)
    }

    pub fn scan_prefix(&self, prefix: &[u8], limit: u32) -> Vec<StorageEntry> {
        let cap = (limit as usize).min(MAX_SEQ_ITEMS);
        if cap == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (k, v) in self.entries.range(prefix.to_vec()..) {
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

    pub fn commit_batch(&mut self, ops: &[StorageWriteOp]) -> Result<(), ServiceError> {
        validate_batch(ops)?;
        if ops.is_empty() {
            return Ok(());
        }
        let mut payload = BinWriter::with_capacity(4 + ops.len() * 16);
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
        let frame = encode_record_frame(KIND_BATCH, seq, payload.as_slice())?;
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

    pub fn compact(&mut self) -> Result<CompactStats, ServiceError> {
        let bytes_before = self.log_bytes();
        let mut compacted = Vec::with_capacity(HEADER_LEN + self.entries.len() * 32);
        write_file_header(&mut compacted);
        let mut seq = 1u64;
        for (k, v) in &self.entries {
            let mut payload = BinWriter::with_capacity(8 + k.len() + v.len());
            payload.write_bytes(k)?;
            payload.write_bytes(v)?;
            let frame = encode_record_frame(KIND_PUT, seq, payload.as_slice())?;
            compacted.extend_from_slice(&frame);
            seq = seq.saturating_add(1);
        }
        let bytes_after = compacted.len() as u64;

        match &mut self.backend {
            LogBackend::Memory(buf) => {
                *buf = compacted;
            }
            #[cfg(not(target_arch = "wasm32"))]
            LogBackend::Disk {
                path,
                file,
                byte_len,
            } => {
                let tmp_path = temp_compact_path(path);
                {
                    let mut tmp = OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(&tmp_path)?;
                    tmp.write_all(&compacted)?;
                    tmp.sync_all()?;
                }
                fs::rename(&tmp_path, &*path)?;
                let mut reopened = OpenOptions::new().read(true).write(true).open(&*path)?;
                reopened.seek(SeekFrom::End(0))?;
                *file = reopened;
                *byte_len = bytes_after;
            }
        }

        self.next_seq = seq;
        self.recovered_torn_tail = false;

        Ok(CompactStats {
            live_keys: self.entries.len() as u64,
            bytes_before,
            bytes_after,
        })
    }

    fn append_frame(&mut self, frame: &[u8]) -> Result<(), ServiceError> {
        match &mut self.backend {
            LogBackend::Memory(buf) => {
                buf.extend_from_slice(frame);
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            LogBackend::Disk { file, byte_len, .. } => {
                file.write_all(frame)?;
                file.sync_data()?;
                *byte_len = byte_len.saturating_add(frame.len() as u64);
                Ok(())
            }
        }
    }
}

/// Manages named `NVK2` namespaces either in-memory or under a root directory on disk.
pub struct KvStoreManager {
    next_handle: u64,
    #[cfg(not(target_arch = "wasm32"))]
    root_dir: Option<PathBuf>,
    stores: BTreeMap<String, KvStore>,
}

impl KvStoreManager {
    pub fn in_memory() -> Self {
        Self {
            next_handle: 1,
            #[cfg(not(target_arch = "wasm32"))]
            root_dir: None,
            stores: BTreeMap::new(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_root_dir(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            next_handle: 1,
            root_dir: Some(root_dir.into()),
            stores: BTreeMap::new(),
        }
    }

    pub fn open(&mut self, namespace: &str) -> Result<StorageOpenInfo, ServiceError> {
        validate_namespace(namespace)?;
        if let Some(store) = self.stores.get(namespace) {
            return Ok(store.open_info());
        }
        let handle = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        let store = self.create_store(handle, namespace)?;
        let info = store.open_info();
        self.stores.insert(namespace.to_owned(), store);
        Ok(info)
    }

    pub fn get_or_open(&mut self, namespace: &str) -> Result<&mut KvStore, ServiceError> {
        validate_namespace(namespace)?;
        if !self.stores.contains_key(namespace) {
            self.open(namespace)?;
        }
        self.stores
            .get_mut(namespace)
            .ok_or_else(|| ServiceError::InternalError("missing storage namespace".into()))
    }

    fn create_store(&self, handle: u64, namespace: &str) -> Result<KvStore, ServiceError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(root) = &self.root_dir {
            let path = root.join(format!("{namespace}.nvk1"));
            return KvStore::open_file(handle, path);
        }
        let _ = namespace;
        Ok(KvStore::open_in_memory(handle))
    }
}

fn write_file_header(out: &mut Vec<u8>) {
    let mut prefix = [0u8; 8];
    prefix[0..4].copy_from_slice(&NVK2_MAGIC);
    prefix[4..6].copy_from_slice(&NVK2_VERSION.to_le_bytes());
    prefix[6..8].copy_from_slice(&0u16.to_le_bytes());
    let checksum = xxh64(&prefix, 0);
    out.extend_from_slice(&prefix);
    out.extend_from_slice(&checksum.to_le_bytes());
}

fn encode_record_frame(kind: u8, seq_no: u64, payload: &[u8]) -> Result<Vec<u8>, ServiceError> {
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| ServiceError::InvalidArg("NVK2 record payload exceeds u32::MAX".into()))?;
    let mut out = Vec::with_capacity(RECORD_HEADER_LEN + payload.len() + RECORD_TRAILER_LEN);
    out.push(kind);
    out.extend_from_slice(&[0u8; 3]);
    out.extend_from_slice(&seq_no.to_le_bytes());
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(payload);
    let checksum = xxh64(&out, 0);
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(&NVK2_COMMIT_MARKER);
    Ok(out)
}

struct ReplayResult {
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
    last_seq: u64,
    valid_len: usize,
    recovered_torn_tail: bool,
}

fn replay_nvk1_buffer(bytes: &[u8]) -> Result<ReplayResult, ServiceError> {
    if bytes.len() < HEADER_LEN {
        return Err(ServiceError::Codec("NVK2 header truncated".into()));
    }
    if bytes[0..4] != NVK2_MAGIC {
        return Err(ServiceError::Codec("invalid NVK2 file magic".into()));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != NVK2_VERSION {
        return Err(ServiceError::Codec(format!(
            "unsupported NVK2 version {version}"
        )));
    }
    let le64 = |o: usize| {
        u64::from_le_bytes([
            bytes[o],
            bytes[o + 1],
            bytes[o + 2],
            bytes[o + 3],
            bytes[o + 4],
            bytes[o + 5],
            bytes[o + 6],
            bytes[o + 7],
        ])
    };
    let stored_hdr_crc = le64(8);
    if xxh64(&bytes[0..8], 0) != stored_hdr_crc {
        return Err(ServiceError::Codec(
            "corrupt NVK2 file header checksum".into(),
        ));
    }

    let mut entries = BTreeMap::new();
    let mut cursor = HEADER_LEN;
    let mut last_seq = 0u64;
    let mut recovered_torn_tail = false;

    while cursor < bytes.len() {
        let remaining = bytes.len() - cursor;
        if remaining < RECORD_HEADER_LEN + RECORD_TRAILER_LEN {
            recovered_torn_tail = true;
            break;
        }
        let kind = bytes[cursor];
        let seq_no = le64(cursor + 4);
        let payload_len = u32::from_le_bytes([
            bytes[cursor + 12],
            bytes[cursor + 13],
            bytes[cursor + 14],
            bytes[cursor + 15],
        ]) as usize;
        let Some(frame_len) = RECORD_HEADER_LEN
            .checked_add(payload_len)
            .and_then(|v| v.checked_add(RECORD_TRAILER_LEN))
        else {
            recovered_torn_tail = true;
            break;
        };
        if frame_len > remaining {
            recovered_torn_tail = true;
            break;
        }

        let payload_end = cursor + RECORD_HEADER_LEN + payload_len;
        let checksum_end = payload_end + 8;
        let frame_end = cursor + frame_len;

        if bytes[checksum_end..frame_end] != NVK2_COMMIT_MARKER {
            recovered_torn_tail = true;
            break;
        }
        let stored_crc = le64(payload_end);
        let computed_crc = xxh64(&bytes[cursor..payload_end], 0);
        if stored_crc != computed_crc {
            recovered_torn_tail = true;
            break;
        }

        let payload = &bytes[cursor + RECORD_HEADER_LEN..payload_end];
        if apply_record_payload(kind, payload, &mut entries).is_err() {
            recovered_torn_tail = true;
            break;
        }

        last_seq = last_seq.max(seq_no);
        cursor = frame_end;
    }

    Ok(ReplayResult {
        entries,
        last_seq,
        valid_len: cursor,
        recovered_torn_tail,
    })
}

fn apply_record_payload(
    kind: u8,
    payload: &[u8],
    entries: &mut BTreeMap<Vec<u8>, Vec<u8>>,
) -> Result<(), ServiceError> {
    let mut r = BinReader::new(payload);
    match kind {
        KIND_PUT => {
            let key = r.read_bytes()?.to_vec();
            let value = r.read_bytes()?.to_vec();
            r.finish()?;
            validate_key(&key)?;
            validate_value(&value)?;
            entries.insert(key, value);
        }
        KIND_DELETE => {
            let key = r.read_bytes()?.to_vec();
            r.finish()?;
            validate_key(&key)?;
            entries.remove(&key);
        }
        KIND_BATCH => {
            let count = r.read_seq_len(5)?;
            let mut decoded = Vec::with_capacity(count);
            for _ in 0..count {
                match r.read_u8()? {
                    KIND_PUT => {
                        let key = r.read_bytes()?.to_vec();
                        let value = r.read_bytes()?.to_vec();
                        validate_key(&key)?;
                        validate_value(&value)?;
                        decoded.push(StorageWriteOp::Put { key, value });
                    }
                    KIND_DELETE => {
                        let key = r.read_bytes()?.to_vec();
                        validate_key(&key)?;
                        decoded.push(StorageWriteOp::Delete { key });
                    }
                    other => {
                        return Err(ServiceError::Codec(format!(
                            "invalid batch op kind {other}"
                        )));
                    }
                }
            }
            r.finish()?;
            for op in decoded {
                match op {
                    StorageWriteOp::Put { key, value } => {
                        entries.insert(key, value);
                    }
                    StorageWriteOp::Delete { key } => {
                        entries.remove(&key);
                    }
                }
            }
        }
        other => {
            return Err(ServiceError::Codec(format!(
                "unknown NVK2 record kind {other}"
            )));
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn temp_compact_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".compact.tmp");
    PathBuf::from(os)
}
