//! File Picker & Standard Paths capability (`SERVICE_FILESYSTEM = 0x0011`) with binary codecs.

use nve_abi::SERVICE_FILESYSTEM;

use crate::codec::{BinReader, BinWriter};
use crate::error::SdkError;
use crate::provider::HostServiceProvider;

pub const METHOD_STANDARD_PATH: u32 = 1;
pub const METHOD_PICK_FILES: u32 = 2;
pub const METHOD_SAVE_FILE: u32 = 3;

/// Well-known platform directory categories.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardPathKind {
    Data = 1,
    Cache = 2,
    Config = 3,
    Temp = 4,
    Documents = 5,
    Downloads = 6,
}

impl StandardPathKind {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Data),
            2 => Some(Self::Cache),
            3 => Some(Self::Config),
            4 => Some(Self::Temp),
            5 => Some(Self::Documents),
            6 => Some(Self::Downloads),
            _ => None,
        }
    }
}

/// Extension filter displayed by native open/save file dialogs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

impl FileFilter {
    pub fn new(name: impl Into<String>, extensions: impl IntoIterator<Item = impl Into<String>>) -> Self {
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

pub fn decode_standard_path_request(bytes: &[u8]) -> Result<StandardPathKind, SdkError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    StandardPathKind::from_u8(raw)
        .ok_or_else(|| SdkError::InvalidArg(format!("unknown StandardPathKind {raw}")))
}

pub fn encode_standard_path_response(path: &str) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::with_capacity(4 + path.len());
    w.write_str(path)?;
    Ok(w.into_bytes())
}

pub fn decode_standard_path_response(bytes: &[u8]) -> Result<String, SdkError> {
    let mut r = BinReader::new(bytes);
    let path = r.read_string()?;
    r.finish()?;
    Ok(path)
}

fn write_filters(w: &mut BinWriter, filters: &[FileFilter]) -> Result<(), SdkError> {
    let count = u32::try_from(filters.len())
        .map_err(|_| SdkError::InvalidArg("filter count exceeds u32::MAX".into()))?;
    w.write_u32(count);
    for filter in filters {
        w.write_str(&filter.name)?;
        w.write_str_vec(&filter.extensions)?;
    }
    Ok(())
}

fn read_filters(r: &mut BinReader<'_>) -> Result<Vec<FileFilter>, SdkError> {
    let count = r.read_seq_len(8)?;
    let mut filters = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.read_string()?;
        let extensions = r.read_string_vec()?;
        filters.push(FileFilter { name, extensions });
    }
    Ok(filters)
}

pub fn encode_file_picker_options(opts: &FilePickerOptions) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::new();
    w.write_opt_str(opts.title.as_deref())?;
    w.write_opt_str(opts.initial_dir.as_deref())?;
    write_filters(&mut w, &opts.filters)?;
    w.write_bool(opts.allow_multiple);
    w.write_bool(opts.pick_directories);
    Ok(w.into_bytes())
}

pub fn decode_file_picker_options(bytes: &[u8]) -> Result<FilePickerOptions, SdkError> {
    let mut r = BinReader::new(bytes);
    let title = r.read_opt_string()?;
    let initial_dir = r.read_opt_string()?;
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

pub fn encode_pick_files_response(paths: &[String]) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::new();
    w.write_str_vec(paths)?;
    Ok(w.into_bytes())
}

pub fn decode_pick_files_response(bytes: &[u8]) -> Result<Vec<String>, SdkError> {
    let mut r = BinReader::new(bytes);
    let paths = r.read_string_vec()?;
    r.finish()?;
    Ok(paths)
}

pub fn encode_save_file_options(opts: &SaveFileOptions) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::new();
    w.write_opt_str(opts.title.as_deref())?;
    w.write_opt_str(opts.default_name.as_deref())?;
    w.write_opt_str(opts.initial_dir.as_deref())?;
    write_filters(&mut w, &opts.filters)?;
    Ok(w.into_bytes())
}

pub fn decode_save_file_options(bytes: &[u8]) -> Result<SaveFileOptions, SdkError> {
    let mut r = BinReader::new(bytes);
    let title = r.read_opt_string()?;
    let default_name = r.read_opt_string()?;
    let initial_dir = r.read_opt_string()?;
    let filters = read_filters(&mut r)?;
    r.finish()?;
    Ok(SaveFileOptions {
        title,
        default_name,
        initial_dir,
        filters,
    })
}

pub fn encode_save_file_response(path: Option<&str>) -> Result<Vec<u8>, SdkError> {
    let mut w = BinWriter::new();
    w.write_opt_str(path)?;
    Ok(w.into_bytes())
}

pub fn decode_save_file_response(bytes: &[u8]) -> Result<Option<String>, SdkError> {
    let mut r = BinReader::new(bytes);
    let path = r.read_opt_string()?;
    r.finish()?;
    Ok(path)
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

    pub fn standard_path(&self, kind: StandardPathKind) -> Result<String, SdkError> {
        let req = encode_standard_path_request(kind);
        let resp = self
            .provider
            .invoke_sync(SERVICE_FILESYSTEM, METHOD_STANDARD_PATH, &req)?;
        decode_standard_path_response(&resp)
    }

    pub fn pick_files(&self, opts: &FilePickerOptions) -> Result<Vec<String>, SdkError> {
        let req = encode_file_picker_options(opts)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_FILESYSTEM, METHOD_PICK_FILES, &req)?;
        decode_pick_files_response(&resp)
    }

    pub fn save_file(&self, opts: &SaveFileOptions) -> Result<Option<String>, SdkError> {
        let req = encode_save_file_options(opts)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_FILESYSTEM, METHOD_SAVE_FILE, &req)?;
        decode_save_file_response(&resp)
    }
}
