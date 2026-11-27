//! Crash-atomic persistence (`Frame0File` + `StateFile`) with zero `mmap` and zero `unsafe`.
use std::{fmt, io, path::{Path, PathBuf}};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};
use nve_abi::xxh64;
use nve_store::{NodeStore, wire::{WireDecoder, WireEncoder, WireError}};
use crate::{BackendApplyStats, StoreBackend, StoreWireReplica};

pub const DEFAULT_STATE_FILE_NAME: &str = "state.nve";
pub const DEFAULT_FRAME0_FILE_NAME: &str = "frame0.nve2";
pub const STATE_FILE_MAGIC: [u8; 8] = *b"NVESTATE";
pub const STATE_FILE_HEADER_SIZE: usize = 24;
pub const FRAME0_FOOTER_SIZE: usize = 40;
const STATE_XXH64_SEED: u64 = 0x4E56_4553_5441_5445;
const FRAME0_XXH64_SEED: u64 = 0x4E56_4546_524D_3032;
#[cfg(not(target_arch = "wasm32"))]
static ATOMIC_TMP_SEQ: AtomicU64 = AtomicU64::new(0);
#[inline] fn le_u32(b: &[u8]) -> u32 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) }
#[inline] fn le_u64(b: &[u8]) -> u64 { u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) }

#[derive(Debug)]
pub enum PersistenceError {
    Io(io::Error), PayloadTooLarge(usize), InvalidNve2(WireError), Corrupt(&'static str),
    SchemaMismatch { expected: u32, actual: u32 },
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "persistence I/O error: {e}"),
            Self::PayloadTooLarge(n) => write!(f, "payload length {n} exceeds u32::MAX"),
            Self::InvalidNve2(e) => write!(f, "invalid NVE2 keyframe: {e}"),
            Self::Corrupt(m) => write!(f, "corrupt persistence file: {m}"),
            Self::SchemaMismatch { expected, actual } => write!(f, "schema mismatch: {expected} != {actual}"),
        }
    }
}
impl std::error::Error for PersistenceError {}
impl From<io::Error> for PersistenceError { fn from(e: io::Error) -> Self { Self::Io(e) } }
impl From<WireError> for PersistenceError { fn from(e: WireError) -> Self { Self::InvalidNve2(e) } }

/// Atomically writes `bytes` to `path` via `<file>.tmp.<pid>.<seq>` -> optional `sync_all` -> `rename`.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_file_atomic(path: &Path, bytes: &[u8], sync: bool) -> Result<(), PersistenceError> {
    use std::io::Write as _;
    if let Some(p) = path.parent() && !p.as_os_str().is_empty() { std::fs::create_dir_all(p)?; }
    let seq = ATOMIC_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("nve_file");
    let tmp = path.with_file_name(format!("{name}.tmp.{}.{seq}", std::process::id()));
    let res = (|| -> io::Result<()> { let mut f = std::fs::File::create(&tmp)?; f.write_all(bytes)?; if sync { f.sync_all()?; } Ok(()) })()
        .and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = res { let _ = std::fs::remove_file(&tmp); return Err(PersistenceError::Io(e)); }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub fn write_file_atomic(_path: &Path, _bytes: &[u8], _sync: bool) -> Result<(), PersistenceError> {
    Err(PersistenceError::Io(io::Error::new(io::ErrorKind::Unsupported, "unsupported on wasm32")))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateRecoveryStats { pub loaded_from_disk: bool, pub schema_mismatch_reset: bool, pub recovered_from_corrupt: bool, pub generation: u64 }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateSaveOutcome { pub generation: u64, pub byte_len: usize, pub payload_checksum: u64 }
/// Plain atomic-rename state file: 24-byte header `[magic:8, schema:u32, len:u32, xxh64:u64]` + payload.
pub struct StateFile {
    path: Option<PathBuf>, schema_version: u32, sync_on_save: bool,
    generation: u64, recovery: StateRecoveryStats, in_memory: Option<Vec<u8>>,
}

impl StateFile {
    pub fn open(path: impl AsRef<Path>, schema_version: u32) -> Result<Self, PersistenceError> {
        Self::open_with_sync(path, schema_version, true)
    }
    pub fn open_with_sync(path: impl AsRef<Path>, schema_version: u32, sync_on_save: bool) -> Result<Self, PersistenceError> {
        let path = path.as_ref().to_path_buf();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = path.parent() && !p.as_os_str().is_empty() { std::fs::create_dir_all(p)?; }
        #[cfg(not(target_arch = "wasm32"))]
        let recovery = {
            let mut r = StateRecoveryStats::default();
            if path.exists() {
                match std::fs::read(&path).map_err(PersistenceError::Io).and_then(|raw| Self::decode_envelope(&raw, schema_version).map(|_| ())) {
                    Ok(()) => { r.loaded_from_disk = true; r.generation = 1; }
                    Err(PersistenceError::SchemaMismatch { .. }) => { r.schema_mismatch_reset = true; let _ = std::fs::remove_file(&path); }
                    Err(_) => { r.recovered_from_corrupt = true; let _ = std::fs::remove_file(&path); }
                }
            }
            r
        };
        #[cfg(target_arch = "wasm32")]
        let recovery = StateRecoveryStats::default();
        Ok(Self { path: Some(path), schema_version, sync_on_save, generation: recovery.generation, recovery, in_memory: None })
    }
    pub fn new_in_memory(schema_version: u32) -> Self {
        Self { path: None, schema_version, sync_on_save: false, generation: 0, recovery: StateRecoveryStats::default(), in_memory: None }
    }

    #[inline] pub fn path(&self) -> Option<&Path> { self.path.as_deref() }
    #[inline] pub fn schema_version(&self) -> u32 { self.schema_version }
    #[inline] pub fn generation(&self) -> u64 { self.generation }
    #[inline] pub fn recovery_stats(&self) -> StateRecoveryStats { self.recovery }

    pub fn encode_envelope(schema_version: u32, payload: &[u8]) -> Result<Vec<u8>, PersistenceError> {
        let len = u32::try_from(payload.len()).map_err(|_| PersistenceError::PayloadTooLarge(payload.len()))?;
        let mut out = Vec::with_capacity(STATE_FILE_HEADER_SIZE + payload.len());
        out.extend_from_slice(&STATE_FILE_MAGIC);
        out.extend_from_slice(&schema_version.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&xxh64(payload, STATE_XXH64_SEED).to_le_bytes());
        out.extend_from_slice(payload);
        Ok(out)
    }
    pub fn decode_envelope(bytes: &[u8], expected_schema: u32) -> Result<&[u8], PersistenceError> {
        if bytes.len() < STATE_FILE_HEADER_SIZE { return Err(PersistenceError::Corrupt("state file truncated")); }
        if bytes[0..8] != STATE_FILE_MAGIC { return Err(PersistenceError::Corrupt("invalid state file magic")); }
        let (schema, payload_len, expected_hash) = (le_u32(&bytes[8..12]), le_u32(&bytes[12..16]) as usize, le_u64(&bytes[16..24]));
        let payload = &bytes[STATE_FILE_HEADER_SIZE..];
        if payload.len() != payload_len { return Err(PersistenceError::Corrupt("state payload length mismatch")); }
        if xxh64(payload, STATE_XXH64_SEED) != expected_hash { return Err(PersistenceError::Corrupt("state payload XXH64 mismatch")); }
        if schema != expected_schema { return Err(PersistenceError::SchemaMismatch { expected: expected_schema, actual: schema }); }
        Ok(payload)
    }
    pub fn load(&self) -> Result<Option<Vec<u8>>, PersistenceError> {
        let Some(path) = self.path.as_deref() else { return Ok(self.in_memory.clone()); };
        #[cfg(target_arch = "wasm32")] { let _ = path; Ok(self.in_memory.clone()) }
        #[cfg(not(target_arch = "wasm32"))]
        match std::fs::read(path) {
            Ok(raw) => Ok(Some(Self::decode_envelope(&raw, self.schema_version)?.to_vec())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(PersistenceError::Io(e)),
        }
    }
    pub fn save(&mut self, payload: &[u8]) -> Result<StateSaveOutcome, PersistenceError> {
        let payload_checksum = xxh64(payload, STATE_XXH64_SEED);
        let byte_len = if let Some(path) = self.path.as_deref() {
            let env = Self::encode_envelope(self.schema_version, payload)?;
            write_file_atomic(path, &env, self.sync_on_save)?;
            env.len()
        } else {
            self.in_memory = Some(payload.to_vec());
            STATE_FILE_HEADER_SIZE + payload.len()
        };
        self.generation = self.generation.saturating_add(1);
        Ok(StateSaveOutcome { generation: self.generation, byte_len, payload_checksum })
    }
    pub fn reset(&mut self) -> Result<(), PersistenceError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = self.path.as_deref() && path.exists() { std::fs::remove_file(path)?; }
        self.in_memory = None;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Frame0Meta { pub table_schema_hash: u64, pub app_build_id: u64, pub env_hash: u64 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame0Footer {
    pub node_count: u32, pub flags: u32,
    pub table_schema_hash: u64, pub app_build_id: u64, pub env_hash: u64, pub payload_xxh64: u64,
}

impl Frame0Footer {
    pub fn to_bytes(self) -> [u8; FRAME0_FOOTER_SIZE] {
        let mut out = [0u8; FRAME0_FOOTER_SIZE];
        out[0..4].copy_from_slice(&self.node_count.to_le_bytes());
        out[4..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..16].copy_from_slice(&self.table_schema_hash.to_le_bytes());
        out[16..24].copy_from_slice(&self.app_build_id.to_le_bytes());
        out[24..32].copy_from_slice(&self.env_hash.to_le_bytes());
        out[32..40].copy_from_slice(&self.payload_xxh64.to_le_bytes());
        out
    }
    pub fn from_bytes(b: &[u8; FRAME0_FOOTER_SIZE]) -> Self {
        Self {
            node_count: le_u32(&b[0..4]), flags: le_u32(&b[4..8]),
            table_schema_hash: le_u64(&b[8..16]), app_build_id: le_u64(&b[16..24]),
            env_hash: le_u64(&b[24..32]), payload_xxh64: le_u64(&b[32..40]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame0Miss {
    NotFound, Io(String), FlagsMismatch(u32), Corrupt(&'static str),
    SchemaMismatch { expected: u64, actual: u64 },
    BuildIdMismatch { expected: u64, actual: u64 },
    EnvHashMismatch { expected: u64, actual: u64 },
}

impl fmt::Display for Frame0Miss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "frame0.nve2 not found"),
            Self::Io(e) => write!(f, "frame0.nve2 I/O error: {e}"),
            Self::FlagsMismatch(fl) => write!(f, "frame0.nve2 unsupported flags: {fl:#x}"),
            Self::Corrupt(m) => write!(f, "frame0.nve2 corrupt: {m}"),
            Self::SchemaMismatch { expected, actual } => write!(f, "frame0.nve2 schema mismatch: {expected:#x} != {actual:#x}"),
            Self::BuildIdMismatch { expected, actual } => write!(f, "frame0.nve2 build_id mismatch: {expected:#x} != {actual:#x}"),
            Self::EnvHashMismatch { expected, actual } => write!(f, "frame0.nve2 env_hash mismatch: {expected:#x} != {actual:#x}"),
        }
    }
}
impl std::error::Error for Frame0Miss {}

/// Frame-0 `NVE2` keyframe file (`frame0.nve2`) + 40-byte little-endian footer.
pub struct Frame0File;

impl Frame0File {
    pub fn encode(store: &NodeStore, meta: Frame0Meta) -> Vec<u8> {
        let node_count = store.stats().live_nodes as u32;
        let mut out = Vec::with_capacity((node_count as usize).saturating_mul(64) + FRAME0_FOOTER_SIZE + 64);
        WireEncoder::default().encode_full(store, &mut out);
        append_footer(&mut out, node_count, meta);
        out
    }
    pub fn encode_nve2(nve2_payload: &[u8], node_count: u32, meta: Frame0Meta) -> Result<Vec<u8>, PersistenceError> {
        let _ = WireDecoder::validate_header(nve2_payload)?;
        let mut out = Vec::with_capacity(nve2_payload.len() + FRAME0_FOOTER_SIZE);
        out.extend_from_slice(nve2_payload);
        append_footer(&mut out, node_count, meta);
        Ok(out)
    }
    pub fn save(path: impl AsRef<Path>, store: &NodeStore, meta: Frame0Meta) -> Result<Frame0Footer, PersistenceError> {
        let bytes = Self::encode(store, meta);
        write_file_atomic(path.as_ref(), &bytes, true)?;
        let arr: [u8; FRAME0_FOOTER_SIZE] = bytes[bytes.len() - FRAME0_FOOTER_SIZE..].try_into().map_err(|_| PersistenceError::Corrupt("footer slice"))?;
        Ok(Frame0Footer::from_bytes(&arr))
    }
    pub fn save_bytes(path: impl AsRef<Path>, envelope_bytes: &[u8]) -> Result<(), PersistenceError> {
        write_file_atomic(path.as_ref(), envelope_bytes, true)
    }
    pub fn validate_bytes(bytes: &[u8], expected: Frame0Meta) -> Result<(Frame0Footer, &[u8]), Frame0Miss> {
        if bytes.len() < FRAME0_FOOTER_SIZE { return Err(Frame0Miss::Corrupt("truncated below 40-byte footer")); }
        let split = bytes.len() - FRAME0_FOOTER_SIZE;
        let payload = &bytes[..split];
        let arr: [u8; FRAME0_FOOTER_SIZE] = bytes[split..].try_into().map_err(|_| Frame0Miss::Corrupt("invalid footer slice"))?;
        let footer = Frame0Footer::from_bytes(&arr);
        if xxh64(&bytes[..bytes.len() - 8], FRAME0_XXH64_SEED) != footer.payload_xxh64 { return Err(Frame0Miss::Corrupt("XXH64 checksum mismatch")); }
        if WireDecoder::validate_header(payload).is_err() { return Err(Frame0Miss::Corrupt("invalid NVE2 keyframe header")); }
        if footer.flags != 0 { return Err(Frame0Miss::FlagsMismatch(footer.flags)); }
        if footer.table_schema_hash != expected.table_schema_hash { return Err(Frame0Miss::SchemaMismatch { expected: expected.table_schema_hash, actual: footer.table_schema_hash }); }
        if footer.app_build_id != expected.app_build_id { return Err(Frame0Miss::BuildIdMismatch { expected: expected.app_build_id, actual: footer.app_build_id }); }
        if footer.env_hash != expected.env_hash { return Err(Frame0Miss::EnvHashMismatch { expected: expected.env_hash, actual: footer.env_hash }); }
        Ok((footer, payload))
    }
    pub fn read_and_validate(path: impl AsRef<Path>, expected: Frame0Meta) -> Result<(Frame0Footer, Vec<u8>), Frame0Miss> {
        let mut buf = Vec::new();
        let footer = Self::read_into_buf(path.as_ref(), expected, &mut buf)?;
        buf.truncate(buf.len() - FRAME0_FOOTER_SIZE);
        Ok((footer, buf))
    }
    pub fn boot_bytes_into_replica<B: StoreBackend>(bytes: &[u8], expected: Frame0Meta, replica: &mut StoreWireReplica<B>) -> Result<(Frame0Footer, BackendApplyStats), Frame0Miss> {
        let (footer, payload) = Self::validate_bytes(bytes, expected)?;
        replica.submit_nve2_frame_with_hint(payload, footer.node_count as usize).map(|s| (footer, s)).map_err(|_| Frame0Miss::Corrupt("NVE2 decode or commit failed"))
    }
    pub fn boot_into_replica<B: StoreBackend>(path: impl AsRef<Path>, expected: Frame0Meta, replica: &mut StoreWireReplica<B>) -> Result<(Frame0Footer, BackendApplyStats), Frame0Miss> {
        Self::boot_into_replica_with_buf(path, expected, &mut Vec::new(), replica)
    }
    pub fn boot_into_replica_with_buf<B: StoreBackend>(path: impl AsRef<Path>, expected: Frame0Meta, scratch: &mut Vec<u8>, replica: &mut StoreWireReplica<B>) -> Result<(Frame0Footer, BackendApplyStats), Frame0Miss> {
        let path = path.as_ref();
        let footer = Self::read_into_buf(path, expected, scratch)?;
        let payload = &scratch[..scratch.len() - FRAME0_FOOTER_SIZE];
        replica.submit_nve2_frame_with_hint(payload, footer.node_count as usize).map(|s| (footer, s)).map_err(|_| {
            #[cfg(not(target_arch = "wasm32"))] let _ = std::fs::remove_file(path);
            Frame0Miss::Corrupt("NVE2 decode or commit failed")
        })
    }
    fn read_into_buf(path: &Path, expected: Frame0Meta, buf: &mut Vec<u8>) -> Result<Frame0Footer, Frame0Miss> {
        #[cfg(target_arch = "wasm32")] { let _ = (path, expected, buf); Err(Frame0Miss::NotFound) }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::io::Read as _;
            buf.clear();
            let mut file = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Frame0Miss::NotFound),
                Err(e) => return Err(Frame0Miss::Io(e.to_string())),
            };
            if let Ok(m) = file.metadata() && buf.capacity() < m.len() as usize { buf.reserve(m.len() as usize); }
            file.read_to_end(buf).map_err(|e| Frame0Miss::Io(e.to_string()))?;
            Self::validate_bytes(buf, expected).map(|(f, _)| f).map_err(|miss| {
                if matches!(miss, Frame0Miss::Corrupt(_)) { let _ = std::fs::remove_file(path); }
                miss
            })
        }
    }
}

fn append_footer(out: &mut Vec<u8>, node_count: u32, meta: Frame0Meta) {
    let f = Frame0Footer { node_count, flags: 0, table_schema_hash: meta.table_schema_hash, app_build_id: meta.app_build_id, env_hash: meta.env_hash, payload_xxh64: 0 };
    let start = out.len();
    out.extend_from_slice(&f.to_bytes());
    let hash = xxh64(&out[..start + 32], FRAME0_XXH64_SEED);
    out[start + 32..start + 40].copy_from_slice(&hash.to_le_bytes());
}
