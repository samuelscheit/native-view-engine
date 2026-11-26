//! Zero-dependency native OS standard paths & file picker backends (macOS, Windows, Unix/XDG).

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_FILESYSTEM;
use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::{
    dispatch_filesystem_wire, FilePickerOptions, FilesystemBackend, SaveFileOptions,
    StandardPathKind,
};

/// Resolves a well-known OS directory directly for callers that need native paths in-process.
pub fn native_standard_path(kind: StandardPathKind) -> Result<String, ServiceError> {
    platform::standard_path(kind)
}

/// Native OS [`FilesystemBackend`] & [`HostServiceHandler`] for [`SERVICE_FILESYSTEM`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeFilesystem;

impl NativeFilesystem {
    pub const fn new() -> Self {
        Self
    }
}

impl FilesystemBackend for NativeFilesystem {
    fn standard_path(&self, kind: StandardPathKind) -> Result<String, ServiceError> {
        platform::standard_path(kind)
    }

    fn pick_files(&self, opts: &FilePickerOptions) -> Result<Vec<String>, ServiceError> {
        platform::pick_files(opts)
    }

    fn save_file(&self, opts: &SaveFileOptions) -> Result<Option<String>, ServiceError> {
        platform::save_file(opts)
    }
}

impl HostServiceHandler for NativeFilesystem {
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

#[cfg(target_os = "macos")]
mod platform {
    use std::path::PathBuf;

    use nve_host_services::ServiceError;

    use crate::{FilePickerOptions, SaveFileOptions, StandardPathKind};

    pub fn standard_path(kind: StandardPathKind) -> Result<String, ServiceError> {
        if kind == StandardPathKind::Temp {
            return Ok(std::env::temp_dir().to_string_lossy().into_owned());
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| ServiceError::IoError("HOME environment variable unset".into()))?;
        let path = match kind {
            StandardPathKind::Data => home.join("Library").join("Application Support"),
            StandardPathKind::Cache => home.join("Library").join("Caches"),
            StandardPathKind::Config => home.join("Library").join("Preferences"),
            StandardPathKind::Documents => home.join("Documents"),
            StandardPathKind::Downloads => home.join("Downloads"),
            StandardPathKind::Temp => std::env::temp_dir(),
        };
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn pick_files(_opts: &FilePickerOptions) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn save_file(_opts: &SaveFileOptions) -> Result<Option<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;
    use std::path::PathBuf;

    use nve_host_services::win32_str::wide_ptr_to_string;
    use nve_host_services::ServiceError;

    use crate::{FilePickerOptions, SaveFileOptions, StandardPathKind};

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const FOLDERID_ROAMING_APP_DATA: Guid = Guid {
        data1: 0x3EB685DB,
        data2: 0x65F9,
        data3: 0x4CF6,
        data4: [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D],
    };
    const FOLDERID_LOCAL_APP_DATA: Guid = Guid {
        data1: 0xF1B32785,
        data2: 0x6FBA,
        data3: 0x4FCF,
        data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
    };
    const FOLDERID_DOCUMENTS: Guid = Guid {
        data1: 0xFDD39AD0,
        data2: 0x238F,
        data3: 0x46AF,
        data4: [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7],
    };
    const FOLDERID_DOWNLOADS: Guid = Guid {
        data1: 0x374DE290,
        data2: 0x123F,
        data3: 0x4565,
        data4: [0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B],
    };

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            rfid: *const Guid,
            flags: u32,
            token: *mut c_void,
            out_path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(pv: *mut c_void);
    }

    fn known_folder(guid: &Guid) -> Option<String> {
        let mut ptr: *mut u16 = std::ptr::null_mut();
        // SAFETY: `guid` is a valid KNOWNFOLDERID reference and `ptr` receives the allocated PWSTR.
        let hr = unsafe { SHGetKnownFolderPath(guid, 0, std::ptr::null_mut(), &mut ptr) };
        if hr == 0 && !ptr.is_null() {
            // SAFETY: On S_OK with non-null `ptr`, `SHGetKnownFolderPath` returns a NUL-terminated UTF-16 string owned by CoTaskMem.
            let s = unsafe { wide_ptr_to_string(ptr) };
            // SAFETY: `ptr` was allocated by `SHGetKnownFolderPath` and must be freed via `CoTaskMemFree`.
            unsafe { CoTaskMemFree(ptr.cast()) };
            Some(s)
        } else {
            None
        }
    }

    pub fn standard_path(kind: StandardPathKind) -> Result<String, ServiceError> {
        match kind {
            StandardPathKind::Temp => Ok(std::env::temp_dir().to_string_lossy().into_owned()),
            StandardPathKind::Data => known_folder(&FOLDERID_ROAMING_APP_DATA)
                .or_else(|| std::env::var("APPDATA").ok())
                .ok_or_else(|| ServiceError::IoError("APPDATA unavailable".into())),
            StandardPathKind::Cache => known_folder(&FOLDERID_LOCAL_APP_DATA)
                .or_else(|| std::env::var("LOCALAPPDATA").ok())
                .ok_or_else(|| ServiceError::IoError("LOCALAPPDATA unavailable".into())),
            StandardPathKind::Config => known_folder(&FOLDERID_ROAMING_APP_DATA)
                .or_else(|| std::env::var("APPDATA").ok())
                .ok_or_else(|| ServiceError::IoError("APPDATA unavailable".into())),
            StandardPathKind::Documents => known_folder(&FOLDERID_DOCUMENTS)
                .or_else(|| {
                    std::env::var("USERPROFILE")
                        .ok()
                        .map(|p| PathBuf::from(p).join("Documents").to_string_lossy().into_owned())
                })
                .ok_or_else(|| ServiceError::IoError("Documents path unavailable".into())),
            StandardPathKind::Downloads => known_folder(&FOLDERID_DOWNLOADS)
                .or_else(|| {
                    std::env::var("USERPROFILE")
                        .ok()
                        .map(|p| PathBuf::from(p).join("Downloads").to_string_lossy().into_owned())
                })
                .ok_or_else(|| ServiceError::IoError("Downloads path unavailable".into())),
        }
    }

    pub fn pick_files(_opts: &FilePickerOptions) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn save_file(_opts: &SaveFileOptions) -> Result<Option<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

#[cfg(all(
    not(any(target_os = "macos", target_os = "windows")),
    not(target_arch = "wasm32")
))]
mod platform {
    use std::path::PathBuf;

    use nve_host_services::ServiceError;

    use crate::{FilePickerOptions, SaveFileOptions, StandardPathKind};

    fn non_empty_env(key: &str) -> Option<PathBuf> {
        std::env::var_os(key)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }

    pub fn standard_path(kind: StandardPathKind) -> Result<String, ServiceError> {
        if kind == StandardPathKind::Temp {
            return Ok(std::env::temp_dir().to_string_lossy().into_owned());
        }
        let home = non_empty_env("HOME")
            .ok_or_else(|| ServiceError::IoError("HOME environment variable unset".into()))?;
        let path = match kind {
            StandardPathKind::Data => {
                non_empty_env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"))
            }
            StandardPathKind::Cache => {
                non_empty_env("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"))
            }
            StandardPathKind::Config => {
                non_empty_env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"))
            }
            StandardPathKind::Documents => {
                non_empty_env("XDG_DOCUMENTS_DIR").unwrap_or_else(|| home.join("Documents"))
            }
            StandardPathKind::Downloads => {
                non_empty_env("XDG_DOWNLOAD_DIR").unwrap_or_else(|| home.join("Downloads"))
            }
            StandardPathKind::Temp => std::env::temp_dir(),
        };
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn pick_files(_opts: &FilePickerOptions) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn save_file(_opts: &SaveFileOptions) -> Result<Option<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    use nve_host_services::ServiceError;

    use crate::{FilePickerOptions, SaveFileOptions, StandardPathKind};

    pub fn standard_path(_kind: StandardPathKind) -> Result<String, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn pick_files(_opts: &FilePickerOptions) -> Result<Vec<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn save_file(_opts: &SaveFileOptions) -> Result<Option<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}
