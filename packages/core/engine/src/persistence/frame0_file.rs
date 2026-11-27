//! Frame-0 `NVE2` keyframe persistence (`Frame0File` / `frame0.nve2`).

use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::io;
use std::path::Path;

use nve_abi::xxh64;
use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{NodeId, NodeStore};

use crate::store_backend::Frame0SubmitError;
use crate::{BackendApplyStats, StoreBackend, StoreWireReplica};

use super::{PersistenceError, le_u32, le_u64, remove_corrupt_frame0, write_file_atomic};
#[cfg(not(target_arch = "wasm32"))]
use super::{ReadBoundedError, read_file_bounded};

pub const DEFAULT_FRAME0_FILE_NAME: &str = "frame0.nve2";
pub const FRAME0_FOOTER_SIZE: usize = 40;
/// Maximum accepted `frame0.nve2` size (64 MiB) to bound memory against hostile/corrupt files.
pub const MAX_FRAME0_FILE_BYTES: usize = 64 * 1024 * 1024;

const FRAME0_XXH64_SEED: u64 = 0x4E56_4546_524D_3032;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Frame0Meta {
    pub table_schema_hash: u64,
    pub app_build_id: u64,
    pub env_hash: u64,
}

impl Frame0Meta {
    /// Canonical Frame-0 metadata for all 17 core tables (`app_build_id = 0`, `env_hash = 0`),
    /// shared across `nve prerender`, `nve-skia-shell`, and native hosts.
    pub fn default_core() -> Self {
        Self {
            table_schema_hash: nve_components::tables::core_tables_schema_hash(),
            app_build_id: 0,
            env_hash: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame0Footer {
    pub node_count: u32,
    pub flags: u32,
    pub table_schema_hash: u64,
    pub app_build_id: u64,
    pub env_hash: u64,
    pub payload_xxh64: u64,
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
            node_count: le_u32(&b[0..4]),
            flags: le_u32(&b[4..8]),
            table_schema_hash: le_u64(&b[8..16]),
            app_build_id: le_u64(&b[16..24]),
            env_hash: le_u64(&b[24..32]),
            payload_xxh64: le_u64(&b[32..40]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame0Miss {
    NotFound,
    Io(String),
    FlagsMismatch(u32),
    Corrupt(&'static str),
    Backend(String),
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
            Self::Backend(e) => write!(f, "frame0.nve2 backend commit error: {e}"),
            Self::SchemaMismatch { expected, actual } => {
                write!(
                    f,
                    "frame0.nve2 schema mismatch: {expected:#x} != {actual:#x}"
                )
            }
            Self::BuildIdMismatch { expected, actual } => {
                write!(
                    f,
                    "frame0.nve2 build_id mismatch: {expected:#x} != {actual:#x}"
                )
            }
            Self::EnvHashMismatch { expected, actual } => {
                write!(
                    f,
                    "frame0.nve2 env_hash mismatch: {expected:#x} != {actual:#x}"
                )
            }
        }
    }
}

impl std::error::Error for Frame0Miss {}

/// Frame-0 `NVE2` keyframe file (`frame0.nve2`) + 40-byte little-endian footer.
pub struct Frame0File;

impl Frame0File {
    pub fn encode(store: &NodeStore, meta: Frame0Meta) -> Result<Vec<u8>, PersistenceError> {
        let live_nodes = store.stats().live_nodes;
        let node_count =
            u32::try_from(live_nodes).map_err(|_| PersistenceError::PayloadTooLarge(live_nodes))?;
        let mut out = Vec::with_capacity(live_nodes.saturating_mul(64) + FRAME0_FOOTER_SIZE + 64);
        WireEncoder::default().encode_full(store, &mut out);
        if out.len().saturating_add(FRAME0_FOOTER_SIZE) > MAX_FRAME0_FILE_BYTES {
            return Err(PersistenceError::PayloadTooLarge(out.len()));
        }
        append_footer(&mut out, node_count, meta);
        Ok(out)
    }

    pub fn encode_nve2(
        nve2_payload: &[u8],
        node_count: u32,
        meta: Frame0Meta,
    ) -> Result<Vec<u8>, PersistenceError> {
        let (frame, _ops) = WireDecoder::validate_header(nve2_payload)?;
        if frame != 1 {
            return Err(PersistenceError::Corrupt(
                "NVE2 frame0 must be keyframe (frame == 1)",
            ));
        }
        let max_nodes = (nve2_payload.len() / 4).min(NodeId::MAX_INDEX as usize);
        if (node_count as usize) > max_nodes {
            return Err(PersistenceError::Corrupt(
                "node_count exceeds NVE2 payload capacity",
            ));
        }
        if nve2_payload.len().saturating_add(FRAME0_FOOTER_SIZE) > MAX_FRAME0_FILE_BYTES {
            return Err(PersistenceError::PayloadTooLarge(nve2_payload.len()));
        }
        let mut out = Vec::with_capacity(nve2_payload.len() + FRAME0_FOOTER_SIZE);
        out.extend_from_slice(nve2_payload);
        append_footer(&mut out, node_count, meta);
        Ok(out)
    }

    pub fn save(
        path: impl AsRef<Path>,
        store: &NodeStore,
        meta: Frame0Meta,
    ) -> Result<Frame0Footer, PersistenceError> {
        let bytes = Self::encode(store, meta)?;
        write_file_atomic(path.as_ref(), &bytes, true)?;
        let arr: [u8; FRAME0_FOOTER_SIZE] = bytes[bytes.len() - FRAME0_FOOTER_SIZE..]
            .try_into()
            .map_err(|_| PersistenceError::Corrupt("footer slice"))?;
        Ok(Frame0Footer::from_bytes(&arr))
    }

    pub fn save_bytes(
        path: impl AsRef<Path>,
        envelope_bytes: &[u8],
    ) -> Result<(), PersistenceError> {
        write_file_atomic(path.as_ref(), envelope_bytes, true)
    }

    pub fn validate_bytes(
        bytes: &[u8],
        expected: Frame0Meta,
    ) -> Result<(Frame0Footer, &[u8]), Frame0Miss> {
        if bytes.len() > MAX_FRAME0_FILE_BYTES {
            return Err(Frame0Miss::Corrupt("frame0.nve2 exceeds max size"));
        }
        if bytes.len() < FRAME0_FOOTER_SIZE {
            return Err(Frame0Miss::Corrupt("truncated below 40-byte footer"));
        }
        let split = bytes.len() - FRAME0_FOOTER_SIZE;
        let payload = &bytes[..split];
        let arr: [u8; FRAME0_FOOTER_SIZE] = bytes[split..]
            .try_into()
            .map_err(|_| Frame0Miss::Corrupt("invalid footer slice"))?;
        let footer = Frame0Footer::from_bytes(&arr);
        if xxh64(&bytes[..bytes.len() - 8], FRAME0_XXH64_SEED) != footer.payload_xxh64 {
            return Err(Frame0Miss::Corrupt("XXH64 checksum mismatch"));
        }
        let Ok((frame, _ops)) = WireDecoder::validate_header(payload) else {
            return Err(Frame0Miss::Corrupt("invalid NVE2 keyframe header"));
        };
        if frame != 1 {
            return Err(Frame0Miss::Corrupt(
                "NVE2 frame0 must be keyframe (frame == 1)",
            ));
        }
        let max_nodes = (payload.len() / 4).min(NodeId::MAX_INDEX as usize);
        if (footer.node_count as usize) > max_nodes {
            return Err(Frame0Miss::Corrupt(
                "footer node_count exceeds NVE2 payload capacity",
            ));
        }
        if footer.flags != 0 {
            return Err(Frame0Miss::FlagsMismatch(footer.flags));
        }
        if footer.table_schema_hash != expected.table_schema_hash {
            return Err(Frame0Miss::SchemaMismatch {
                expected: expected.table_schema_hash,
                actual: footer.table_schema_hash,
            });
        }
        if footer.app_build_id != expected.app_build_id {
            return Err(Frame0Miss::BuildIdMismatch {
                expected: expected.app_build_id,
                actual: footer.app_build_id,
            });
        }
        if footer.env_hash != expected.env_hash {
            return Err(Frame0Miss::EnvHashMismatch {
                expected: expected.env_hash,
                actual: footer.env_hash,
            });
        }
        Ok((footer, payload))
    }

    pub fn read_and_validate(
        path: impl AsRef<Path>,
        expected: Frame0Meta,
    ) -> Result<(Frame0Footer, Vec<u8>), Frame0Miss> {
        let mut buf = Vec::new();
        let footer = Self::read_into_buf(path.as_ref(), expected, &mut buf)?;
        buf.truncate(buf.len() - FRAME0_FOOTER_SIZE);
        Ok((footer, buf))
    }

    pub fn boot_bytes_into_replica<B: StoreBackend>(
        bytes: &[u8],
        expected: Frame0Meta,
        replica: &mut StoreWireReplica<B>,
    ) -> Result<(Frame0Footer, BackendApplyStats), Frame0Miss> {
        let (footer, payload) = Self::validate_bytes(bytes, expected)?;
        Self::boot_validated_into_replica(footer, payload, replica, None)
    }

    pub fn boot_into_replica<B: StoreBackend>(
        path: impl AsRef<Path>,
        expected: Frame0Meta,
        replica: &mut StoreWireReplica<B>,
    ) -> Result<(Frame0Footer, BackendApplyStats), Frame0Miss> {
        Self::boot_into_replica_with_buf(path, expected, &mut Vec::new(), replica)
    }

    pub fn boot_into_replica_with_buf<B: StoreBackend>(
        path: impl AsRef<Path>,
        expected: Frame0Meta,
        scratch: &mut Vec<u8>,
        replica: &mut StoreWireReplica<B>,
    ) -> Result<(Frame0Footer, BackendApplyStats), Frame0Miss> {
        let path = path.as_ref();
        let footer = Self::read_into_buf(path, expected, scratch)?;
        let payload = &scratch[..scratch.len() - FRAME0_FOOTER_SIZE];
        Self::boot_validated_into_replica(footer, payload, replica, Some(path))
    }

    fn boot_validated_into_replica<B: StoreBackend>(
        footer: Frame0Footer,
        payload: &[u8],
        replica: &mut StoreWireReplica<B>,
        disk_path: Option<&Path>,
    ) -> Result<(Frame0Footer, BackendApplyStats), Frame0Miss> {
        let res = replica.submit_frame0_nve2(payload, footer.node_count as usize);
        map_frame0_submit_result(res, footer, disk_path)
    }

    fn read_into_buf(
        path: &Path,
        expected: Frame0Meta,
        buf: &mut Vec<u8>,
    ) -> Result<Frame0Footer, Frame0Miss> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, expected, buf);
            Err(Frame0Miss::NotFound)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut file = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Frame0Miss::NotFound),
                Err(e) => return Err(Frame0Miss::Io(e.to_string())),
            };
            match read_file_bounded(&mut file, MAX_FRAME0_FILE_BYTES, buf) {
                Ok(()) => {}
                Err(ReadBoundedError::Io(e)) => return Err(Frame0Miss::Io(e.to_string())),
                Err(ReadBoundedError::TooLarge) => {
                    drop(file);
                    let _ = std::fs::remove_file(path);
                    return Err(Frame0Miss::Corrupt("frame0.nve2 exceeds max size"));
                }
            }
            drop(file);
            Self::validate_bytes(buf, expected)
                .map(|(f, _)| f)
                .map_err(|miss| {
                    if matches!(miss, Frame0Miss::Corrupt(_)) {
                        let _ = std::fs::remove_file(path);
                    }
                    miss
                })
        }
    }
}

fn map_frame0_submit_result(
    res: Result<BackendApplyStats, Frame0SubmitError>,
    footer: Frame0Footer,
    disk_path: Option<&Path>,
) -> Result<(Frame0Footer, BackendApplyStats), Frame0Miss> {
    match res {
        Ok(stats) => Ok((footer, stats)),
        Err(Frame0SubmitError::NodeCountMismatch) => {
            remove_corrupt_frame0(disk_path);
            Err(Frame0Miss::Corrupt("footer node_count mismatch"))
        }
        Err(Frame0SubmitError::Wire) => {
            remove_corrupt_frame0(disk_path);
            Err(Frame0Miss::Corrupt("NVE2 decode failed"))
        }
        Err(Frame0SubmitError::Alloc) => {
            Err(Frame0Miss::Backend("NVE2 allocation failed".to_string()))
        }
        Err(Frame0SubmitError::Backend(err)) => Err(Frame0Miss::Backend(err.to_string())),
    }
}

fn append_footer(out: &mut Vec<u8>, node_count: u32, meta: Frame0Meta) {
    let f = Frame0Footer {
        node_count,
        flags: 0,
        table_schema_hash: meta.table_schema_hash,
        app_build_id: meta.app_build_id,
        env_hash: meta.env_hash,
        payload_xxh64: 0,
    };
    let start = out.len();
    out.extend_from_slice(&f.to_bytes());
    let hash = xxh64(&out[..start + 32], FRAME0_XXH64_SEED);
    out[start + 32..start + 40].copy_from_slice(&hash.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame0_submit_alloc_error_maps_to_backend_miss_and_preserves_disk_file() {
        let path = std::env::temp_dir().join(format!(
            "nve_frame0_alloc_preserve_{}.nve2",
            std::process::id()
        ));
        std::fs::write(&path, b"valid-cache-sentinel").expect("write sentinel");
        let footer = Frame0Footer {
            node_count: 1,
            flags: 0,
            table_schema_hash: 0,
            app_build_id: 0,
            env_hash: 0,
            payload_xxh64: 0,
        };
        assert!(matches!(
            map_frame0_submit_result(Err(Frame0SubmitError::Alloc), footer, Some(&path)),
            Err(Frame0Miss::Backend(ref msg)) if msg.contains("allocation")
        ));
        assert!(
            path.exists(),
            "transient Frame0SubmitError::Alloc must preserve disk cache"
        );
        assert!(matches!(
            map_frame0_submit_result(Err(Frame0SubmitError::Wire), footer, Some(&path)),
            Err(Frame0Miss::Corrupt(_))
        ));
        assert!(
            !path.exists(),
            "deterministic Frame0SubmitError::Wire / Budget must remove corrupt disk cache"
        );
    }
}
