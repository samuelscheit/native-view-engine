//! File Picker & Standard Paths capability (`SERVICE_FILESYSTEM = 0x0011`) with binary codecs,
//! deterministic [`MemoryFilesystem`], and OS [`NativeFilesystem`] handlers.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod native;

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub use nve_abi::SERVICE_FILESYSTEM;
use nve_host_services::{
    pack_typed, resolve_typed, ByteReader, ByteWriter, HostServiceHandler, HostServiceProvider,
    NveCodec, NveDecode, NveEncode, ServiceError, MAX_PAYLOAD_BYTES, MAX_SEQ_ITEMS,
};

pub use native::{native_standard_path, NativeFilesystem};

pub const METHOD_STANDARD_PATH: u32 = 1;
pub const METHOD_PICK_FILES: u32 = 2;
pub const METHOD_SAVE_FILE: u32 = 3;

/// Well-known platform directory categories.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveCodec)]
#[nve_codec(crate_path = "::nve_host_services")]
pub enum StandardPathKind {
    Data = 1,
    Cache = 2,
    Config = 3,
    Temp = 4,
    Documents = 5,
    Downloads = 6,
}


/// Extension filter displayed by native open/save file dialogs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

impl FileFilter {
    pub fn new(
        name: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            extensions: extensions.into_iter().map(Into::into).collect(),
        }
    }
}

/// Options for [`FilesystemClient::pick_files`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilePickerOptions {
    pub title: Option<String>,
    pub initial_dir: Option<String>,
    pub filters: Vec<FileFilter>,
    pub allow_multiple: bool,
    pub pick_directories: bool,
}

/// Options for [`FilesystemClient::save_file`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveFileOptions {
    pub title: Option<String>,
    pub default_name: Option<String>,
    pub initial_dir: Option<String>,
    pub filters: Vec<FileFilter>,
}

pub fn encode_standard_path_request(kind: StandardPathKind) -> Vec<u8> {
    vec![kind as u8]
}

pub fn decode_standard_path_request(bytes: &[u8]) -> Result<StandardPathKind, ServiceError> {
    Ok(StandardPathKind::decode_exact(bytes)?)
}

pub fn encode_standard_path_response(path: &str) -> Result<Vec<u8>, ServiceError> {
    let mut w = ByteWriter::with_capacity(4 + path.len());
    w.write_str(path)?;
    Ok(w.into_bytes())
}

pub fn decode_standard_path_response(bytes: &[u8]) -> Result<String, ServiceError> {
    let mut r = ByteReader::new(bytes);
    let path = r.read_string(MAX_PAYLOAD_BYTES)?;
    r.finish()?;
    Ok(path)
}

fn write_filters(w: &mut ByteWriter, filters: &[FileFilter]) -> Result<(), ServiceError> {
    let count = u32::try_from(filters.len())
        .map_err(|_| ServiceError::InvalidArg("filter count exceeds u32::MAX".into()))?;
    w.write_u32(count);
    for filter in filters {
        w.write_str(&filter.name)?;
        filter.extensions.encode(w)?;
    }
    Ok(())
}

fn read_filters(r: &mut ByteReader<'_>) -> Result<Vec<FileFilter>, ServiceError> {
    let count = r.read_len(MAX_SEQ_ITEMS, 8)?;
    let mut filters = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.read_string(MAX_PAYLOAD_BYTES)?;
        let extensions = Vec::<String>::decode(r)?;
        filters.push(FileFilter { name, extensions });
    }
    Ok(filters)
}

pub fn encode_file_picker_options(opts: &FilePickerOptions) -> Result<Vec<u8>, ServiceError> {
    let mut w = ByteWriter::new();
    w.write_opt(opts.title.as_deref(), ByteWriter::write_str)?;
    w.write_opt(opts.initial_dir.as_deref(), ByteWriter::write_str)?;
    write_filters(&mut w, &opts.filters)?;
    w.write_bool(opts.allow_multiple);
    w.write_bool(opts.pick_directories);
    Ok(w.into_bytes())
}

pub fn decode_file_picker_options(bytes: &[u8]) -> Result<FilePickerOptions, ServiceError> {
    let mut r = ByteReader::new(bytes);
    let title = r.read_opt(|r| r.read_string(MAX_PAYLOAD_BYTES))?;
    let initial_dir = r.read_opt(|r| r.read_string(MAX_PAYLOAD_BYTES))?;
    let filters = read_filters(&mut r)?;
    let allow_multiple = r.read_bool()?;
    let pick_directories = r.read_bool()?;
    r.finish()?;
    Ok(FilePickerOptions {
        title,
        initial_dir,
        filters,
        allow_multiple,
        pick_directories,
    })
}

pub fn encode_pick_files_response(paths: &[String]) -> Result<Vec<u8>, ServiceError> {
    let mut w = ByteWriter::new();
    paths.encode(&mut w)?;
    Ok(w.into_bytes())
}

pub fn decode_pick_files_response(bytes: &[u8]) -> Result<Vec<String>, ServiceError> {
    let mut r = ByteReader::new(bytes);
    let paths = Vec::<String>::decode(&mut r)?;
    r.finish()?;
    Ok(paths)
}

pub fn encode_save_file_options(opts: &SaveFileOptions) -> Result<Vec<u8>, ServiceError> {
    let mut w = ByteWriter::new();
    w.write_opt(opts.title.as_deref(), ByteWriter::write_str)?;
    w.write_opt(opts.default_name.as_deref(), ByteWriter::write_str)?;
    w.write_opt(opts.initial_dir.as_deref(), ByteWriter::write_str)?;
    write_filters(&mut w, &opts.filters)?;
    Ok(w.into_bytes())
}

pub fn decode_save_file_options(bytes: &[u8]) -> Result<SaveFileOptions, ServiceError> {
    let mut r = ByteReader::new(bytes);
    let title = r.read_opt(|r| r.read_string(MAX_PAYLOAD_BYTES))?;
    let default_name = r.read_opt(|r| r.read_string(MAX_PAYLOAD_BYTES))?;
    let initial_dir = r.read_opt(|r| r.read_string(MAX_PAYLOAD_BYTES))?;
    let filters = read_filters(&mut r)?;
    r.finish()?;
    Ok(SaveFileOptions {
        title,
        default_name,
        initial_dir,
        filters,
    })
}

pub fn encode_save_file_response(path: Option<&str>) -> Result<Vec<u8>, ServiceError> {
    let mut w = ByteWriter::new();
    w.write_opt(path, ByteWriter::write_str)?;
    Ok(w.into_bytes())
}

pub fn decode_save_file_response(bytes: &[u8]) -> Result<Option<String>, ServiceError> {
    let mut r = ByteReader::new(bytes);
    let path = r.read_opt(|r| r.read_string(MAX_PAYLOAD_BYTES))?;
    r.finish()?;
    Ok(path)
}

/// Direct zero-serialization in-process backend trait for [`SERVICE_FILESYSTEM`].
pub trait FilesystemBackend: Send + Sync {
    fn standard_path(&self, kind: StandardPathKind) -> Result<String, ServiceError>;
    fn pick_files(&self, opts: &FilePickerOptions) -> Result<Vec<String>, ServiceError>;
    fn save_file(&self, opts: &SaveFileOptions) -> Result<Option<String>, ServiceError>;
}

/// Shared C-ABI wire dispatcher for any [`FilesystemBackend`].
pub fn dispatch_filesystem_wire(
    backend: &dyn FilesystemBackend,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_STANDARD_PATH => {
            let kind = decode_standard_path_request(request)?;
            encode_standard_path_response(&backend.standard_path(kind)?)
        }
        METHOD_PICK_FILES => {
            let opts = decode_file_picker_options(request)?;
            encode_pick_files_response(&backend.pick_files(&opts)?)
        }
        METHOD_SAVE_FILE => {
            let opts = decode_save_file_options(request)?;
            encode_save_file_response(backend.save_file(&opts)?.as_deref())
        }
        _ => Err(ServiceError::Unsupported),
    }
}

/// Typed ergonomic client for standard platform directories and native file pickers.
#[derive(Debug, Clone, Copy)]
pub struct FilesystemClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> FilesystemClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn standard_path(&self, kind: StandardPathKind) -> Result<String, ServiceError> {
        if let Some(b) = resolve_typed::<dyn FilesystemBackend>(
            self.provider,
            SERVICE_FILESYSTEM,
            METHOD_STANDARD_PATH,
        ) {
            return b.standard_path(kind);
        }
        let req = encode_standard_path_request(kind);
        let resp = self
            .provider
            .invoke_sync(SERVICE_FILESYSTEM, METHOD_STANDARD_PATH, &req)?;
        decode_standard_path_response(&resp)
    }

    pub fn pick_files(&self, opts: &FilePickerOptions) -> Result<Vec<String>, ServiceError> {
        if let Some(b) = resolve_typed::<dyn FilesystemBackend>(
            self.provider,
            SERVICE_FILESYSTEM,
            METHOD_PICK_FILES,
        ) {
            return b.pick_files(opts);
        }
        let req = encode_file_picker_options(opts)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_FILESYSTEM, METHOD_PICK_FILES, &req)?;
        decode_pick_files_response(&resp)
    }

    pub fn save_file(&self, opts: &SaveFileOptions) -> Result<Option<String>, ServiceError> {
        if let Some(b) = resolve_typed::<dyn FilesystemBackend>(
            self.provider,
            SERVICE_FILESYSTEM,
            METHOD_SAVE_FILE,
        ) {
            return b.save_file(opts);
        }
        let req = encode_save_file_options(opts)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_FILESYSTEM, METHOD_SAVE_FILE, &req)?;
        decode_save_file_response(&resp)
    }
}

struct MemoryFilesystemState {
    standard_paths: BTreeMap<u8, String>,
    pick_files_result: Result<Vec<String>, ServiceError>,
    save_file_result: Result<Option<String>, ServiceError>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_FILESYSTEM`].
pub struct MemoryFilesystem {
    state: Mutex<MemoryFilesystemState>,
}

impl Default for MemoryFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryFilesystem {
    pub fn new() -> Self {
        let mut standard_paths = BTreeMap::new();
        standard_paths.insert(StandardPathKind::Data as u8, "/mem/data".into());
        standard_paths.insert(StandardPathKind::Cache as u8, "/mem/cache".into());
        standard_paths.insert(StandardPathKind::Config as u8, "/mem/config".into());
        standard_paths.insert(StandardPathKind::Temp as u8, "/mem/tmp".into());
        standard_paths.insert(StandardPathKind::Documents as u8, "/mem/documents".into());
        standard_paths.insert(StandardPathKind::Downloads as u8, "/mem/downloads".into());
        Self {
            state: Mutex::new(MemoryFilesystemState {
                standard_paths,
                pick_files_result: Ok(vec!["/mem/documents/sample.txt".into()]),
                save_file_result: Ok(Some("/mem/documents/output.bin".into())),
            }),
        }
    }

    pub fn set_standard_path(&self, kind: StandardPathKind, path: impl Into<String>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .standard_paths
            .insert(kind as u8, path.into());
    }

    pub fn set_pick_files_result(&self, res: Result<Vec<String>, ServiceError>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pick_files_result = res;
    }

    pub fn set_save_file_result(&self, res: Result<Option<String>, ServiceError>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .save_file_result = res;
    }
}

impl FilesystemBackend for MemoryFilesystem {
    fn standard_path(&self, kind: StandardPathKind) -> Result<String, ServiceError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .standard_paths
            .get(&(kind as u8))
            .cloned()
            .ok_or(ServiceError::NotFound)
    }

    fn pick_files(&self, _opts: &FilePickerOptions) -> Result<Vec<String>, ServiceError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pick_files_result
            .clone()
    }

    fn save_file(&self, _opts: &SaveFileOptions) -> Result<Option<String>, ServiceError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .save_file_result
            .clone()
    }
}

impl HostServiceHandler for MemoryFilesystem {
    fn service_id(&self) -> u32 {
        SERVICE_FILESYSTEM
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_filesystem_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn FilesystemBackend>(self))
    }
}
