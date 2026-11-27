//! Crash-atomic binary state persistence (`StateFile` / `state.nve`).

#[cfg(not(target_arch = "wasm32"))]
use std::io;
use std::path::{Path, PathBuf};

use nve_abi::{Xxh64Hasher, xxh64};

use super::{PersistenceError, le_u32, le_u64, write_file_atomic};
#[cfg(not(target_arch = "wasm32"))]
use super::{ReadBoundedError, backup_corrupt_file, backup_rotated_file, read_file_bounded};

pub const DEFAULT_STATE_FILE_NAME: &str = "state.nve";
pub const STATE_FILE_MAGIC: [u8; 8] = *b"NVESTATE";
pub const STATE_FILE_HEADER_SIZE: usize = 24;
/// Maximum accepted `state.nve` payload size (64 MiB) to bound memory against hostile/corrupt files.
pub const MAX_STATE_FILE_BYTES: usize = 64 * 1024 * 1024;

const STATE_XXH64_SEED: u64 = 0x4E56_4553_5441_5445;

#[inline]
fn state_envelope_xxh64(header_prefix_16: &[u8], payload: &[u8]) -> u64 {
    let mut hasher = Xxh64Hasher::new(STATE_XXH64_SEED);
    hasher.update(header_prefix_16);
    hasher.update(payload);
    hasher.digest()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateRecoveryStats {
    pub loaded_from_disk: bool,
    pub recovered_from_corrupt: bool,
    /// When an older on-disk schema (`actual < expected`) was rotated to `<file>.v{actual}`,
    /// records `Some(actual)` so callers know persistence stayed active with a preserved backup.
    pub rotated_from_older_schema: Option<u32>,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateSaveOutcome {
    pub generation: u64,
    pub byte_len: usize,
    pub payload_checksum: u64,
}

/// Plain atomic-rename state file: 24-byte header `[magic:8, schema:u32, len:u32, xxh64:u64]` +
/// payload, where `xxh64` covers both the 16-byte header prefix and the payload bytes.
pub struct StateFile {
    path: Option<PathBuf>,
    schema_version: u32,
    sync_on_save: bool,
    generation: u64,
    recovery: StateRecoveryStats,
    cached: Option<Vec<u8>>,
}

impl StateFile {
    pub fn open(path: impl AsRef<Path>, schema_version: u32) -> Result<Self, PersistenceError> {
        Self::open_with_sync(path, schema_version, true)
    }

    pub fn open_with_sync(
        path: impl AsRef<Path>,
        schema_version: u32,
        sync_on_save: bool,
    ) -> Result<Self, PersistenceError> {
        let path = path.as_ref().to_path_buf();
        #[cfg(not(target_arch = "wasm32"))]
        let (recovery, cached) = {
            if path.file_name().is_none() {
                return Err(PersistenceError::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "state file path must include a file name",
                )));
            }
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
            let mut r = StateRecoveryStats::default();
            let mut cached = None;
            match std::fs::File::open(&path) {
                Ok(mut file) => {
                    let mut raw = Vec::new();
                    match read_file_bounded(
                        &mut file,
                        MAX_STATE_FILE_BYTES + STATE_FILE_HEADER_SIZE,
                        &mut raw,
                    ) {
                        Ok(()) => match Self::decode_envelope(&raw, schema_version) {
                            Ok(payload) => {
                                r.loaded_from_disk = true;
                                r.generation = 1;
                                cached = Some(payload.to_vec());
                            }
                            Err(PersistenceError::SchemaMismatch { expected, actual }) => {
                                if actual < expected {
                                    drop(file);
                                    backup_rotated_file(&path, &format!("v{actual}"))?;
                                    r.rotated_from_older_schema = Some(actual);
                                } else {
                                    return Err(PersistenceError::SchemaMismatch {
                                        expected,
                                        actual,
                                    });
                                }
                            }
                            Err(
                                PersistenceError::Corrupt(_) | PersistenceError::PayloadTooLarge(_),
                            ) => {
                                drop(file);
                                backup_corrupt_file(&path);
                                r.recovered_from_corrupt = true;
                            }
                            Err(e) => return Err(e),
                        },
                        Err(ReadBoundedError::Io(e)) => return Err(PersistenceError::Io(e)),
                        Err(ReadBoundedError::TooLarge) => {
                            drop(file);
                            backup_corrupt_file(&path);
                            r.recovered_from_corrupt = true;
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(PersistenceError::Io(e)),
            }
            (r, cached)
        };
        #[cfg(target_arch = "wasm32")]
        let (recovery, cached) = (StateRecoveryStats::default(), None);

        Ok(Self {
            path: Some(path),
            schema_version,
            sync_on_save,
            generation: recovery.generation,
            recovery,
            cached,
        })
    }

    pub fn new_in_memory(schema_version: u32) -> Self {
        Self {
            path: None,
            schema_version,
            sync_on_save: false,
            generation: 0,
            recovery: StateRecoveryStats::default(),
            cached: None,
        }
    }

    #[inline]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[inline]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[inline]
    pub fn recovery_stats(&self) -> StateRecoveryStats {
        self.recovery
    }

    pub fn encode_envelope(
        schema_version: u32,
        payload: &[u8],
    ) -> Result<Vec<u8>, PersistenceError> {
        if payload.len() > MAX_STATE_FILE_BYTES {
            return Err(PersistenceError::PayloadTooLarge(payload.len()));
        }
        let len = u32::try_from(payload.len())
            .map_err(|_| PersistenceError::PayloadTooLarge(payload.len()))?;
        let mut out = Vec::with_capacity(STATE_FILE_HEADER_SIZE + payload.len());
        out.extend_from_slice(&STATE_FILE_MAGIC);
        out.extend_from_slice(&schema_version.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        let checksum = state_envelope_xxh64(&out[0..16], payload);
        out.extend_from_slice(&checksum.to_le_bytes());
        out.extend_from_slice(payload);
        Ok(out)
    }

    pub fn decode_envelope(bytes: &[u8], expected_schema: u32) -> Result<&[u8], PersistenceError> {
        if bytes.len() > MAX_STATE_FILE_BYTES + STATE_FILE_HEADER_SIZE {
            return Err(PersistenceError::PayloadTooLarge(bytes.len()));
        }
        if bytes.len() < STATE_FILE_HEADER_SIZE {
            return Err(PersistenceError::Corrupt("state file truncated"));
        }
        let expected_hash = le_u64(&bytes[16..24]);
        let payload = &bytes[STATE_FILE_HEADER_SIZE..];
        if state_envelope_xxh64(&bytes[0..16], payload) != expected_hash {
            return Err(PersistenceError::Corrupt("state envelope XXH64 mismatch"));
        }
        if bytes[0..8] != STATE_FILE_MAGIC {
            return Err(PersistenceError::Corrupt("invalid state file magic"));
        }
        let schema = le_u32(&bytes[8..12]);
        let payload_len = le_u32(&bytes[12..16]) as usize;
        if payload.len() != payload_len {
            return Err(PersistenceError::Corrupt("state payload length mismatch"));
        }
        if schema != expected_schema {
            return Err(PersistenceError::SchemaMismatch {
                expected: expected_schema,
                actual: schema,
            });
        }
        Ok(payload)
    }

    /// Returns the validated payload read on open or written by the last [`Self::save`] without
    /// re-reading or re-hashing the file from disk.
    pub fn load(&self) -> Result<Option<Vec<u8>>, PersistenceError> {
        Ok(self.cached.clone())
    }

    pub fn save(&mut self, payload: &[u8]) -> Result<StateSaveOutcome, PersistenceError> {
        let env = Self::encode_envelope(self.schema_version, payload)?;
        let payload_checksum = xxh64(payload, STATE_XXH64_SEED);
        let byte_len = env.len();
        if let Some(path) = self.path.as_deref() {
            write_file_atomic(path, &env, self.sync_on_save)?;
        }
        self.cached = Some(payload.to_vec());
        self.generation = self.generation.saturating_add(1);
        Ok(StateSaveOutcome {
            generation: self.generation,
            byte_len,
            payload_checksum,
        })
    }

    /// Moves a payload-corrupt state file aside to `<file>.corrupt` and clears cached state.
    pub fn quarantine_corrupt(&mut self) -> Result<(), PersistenceError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = self.path.as_deref() {
            backup_corrupt_file(path);
        }
        self.cached = None;
        self.generation = 0;
        self.recovery.loaded_from_disk = false;
        self.recovery.recovered_from_corrupt = true;
        self.recovery.generation = 0;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), PersistenceError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = self.path.as_deref() {
            Self::remove(path)?;
        }
        self.cached = None;
        Ok(())
    }

    /// Deletes the state file at `path` (a cold start); succeeds when it is already absent.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn remove(path: &Path) -> Result<(), PersistenceError> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PersistenceError::Io(e)),
        }
    }
}
