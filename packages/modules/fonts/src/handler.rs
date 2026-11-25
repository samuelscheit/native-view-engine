//! Direct typed [`FontsBackend`] plus [`NativeFonts`] and [`MemoryFonts`] implementations.

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::{
    decode_handle, decode_register_bytes_request, decode_register_file_request, encode_handle,
    encode_list_registered_response, encode_unregister_response, FontHandle, FontRegistry,
    RegisteredFont, METHOD_LIST_REGISTERED, METHOD_REGISTER_BYTES, METHOD_REGISTER_FILE,
    METHOD_UNREGISTER, SERVICE_FONTS,
};

/// Direct typed in-process backend for [`SERVICE_FONTS`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait FontsBackend: Send + Sync {
    fn register_bytes(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        bytes: &[u8],
    ) -> Result<FontHandle, ServiceError>;
    fn register_file(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        path: &str,
    ) -> Result<FontHandle, ServiceError>;
    fn unregister(&self, handle: FontHandle) -> Result<bool, ServiceError>;
    fn list_registered(&self) -> Result<Vec<RegisteredFont>, ServiceError>;
}

/// Bridges a [`FontsBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_fonts_wire<B: FontsBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_REGISTER_BYTES => {
            let (family, weight, italic, bytes) = decode_register_bytes_request(request)?;
            Ok(encode_handle(
                backend.register_bytes(&family, weight, italic, &bytes)?,
            ))
        }
        METHOD_REGISTER_FILE => {
            let (family, weight, italic, path) = decode_register_file_request(request)?;
            Ok(encode_handle(
                backend.register_file(&family, weight, italic, &path)?,
            ))
        }
        METHOD_UNREGISTER => {
            let handle = decode_handle(request)?;
            Ok(encode_unregister_response(backend.unregister(handle)?))
        }
        METHOD_LIST_REGISTERED => encode_list_registered_response(&backend.list_registered()?),
        _ => Err(ServiceError::Unsupported),
    }
}

/// Native OS [`HostServiceHandler`] for [`SERVICE_FONTS`] with magic validation & deduplication.
pub struct NativeFonts {
    registry: Mutex<FontRegistry>,
}

impl Default for NativeFonts {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeFonts {
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(FontRegistry::new()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_file(path: &str) -> Result<Vec<u8>, ServiceError> {
        std::fs::read(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ServiceError::NotFound
            } else {
                ServiceError::IoError(e.to_string())
            }
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn read_file(_path: &str) -> Result<Vec<u8>, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}

impl FontsBackend for NativeFonts {
    fn register_bytes(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        bytes: &[u8],
    ) -> Result<FontHandle, ServiceError> {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register_bytes(family.to_owned(), weight, italic, bytes, None)
    }

    fn register_file(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        path: &str,
    ) -> Result<FontHandle, ServiceError> {
        let bytes = Self::read_file(path)?;
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register_bytes(
                family.to_owned(),
                weight,
                italic,
                &bytes,
                Some(path.to_owned()),
            )
    }

    fn unregister(&self, handle: FontHandle) -> Result<bool, ServiceError> {
        Ok(self
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unregister(handle))
    }

    fn list_registered(&self) -> Result<Vec<RegisteredFont>, ServiceError> {
        Ok(self
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .list())
    }
}

impl HostServiceHandler for NativeFonts {
    fn service_id(&self) -> u32 {
        SERVICE_FONTS
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_fonts_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn FontsBackend>(self))
    }
}

struct MemoryFontsState {
    registry: FontRegistry,
    mock_files: BTreeMap<String, Vec<u8>>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_FONTS`].
pub struct MemoryFonts {
    state: Mutex<MemoryFontsState>,
}

impl Default for MemoryFonts {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryFonts {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryFontsState {
                registry: FontRegistry::new(),
                mock_files: BTreeMap::new(),
            }),
        }
    }

    pub fn add_mock_font_file(&self, path: impl Into<String>, bytes: Vec<u8>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mock_files
            .insert(path.into(), bytes);
    }
}

impl FontsBackend for MemoryFonts {
    fn register_bytes(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        bytes: &[u8],
    ) -> Result<FontHandle, ServiceError> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .registry
            .register_bytes(family.to_owned(), weight, italic, bytes, None)
    }

    fn register_file(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        path: &str,
    ) -> Result<FontHandle, ServiceError> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = st
            .mock_files
            .get(path)
            .ok_or(ServiceError::NotFound)?
            .clone();
        st.registry.register_bytes(
            family.to_owned(),
            weight,
            italic,
            &bytes,
            Some(path.to_owned()),
        )
    }

    fn unregister(&self, handle: FontHandle) -> Result<bool, ServiceError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .registry
            .unregister(handle))
    }

    fn list_registered(&self) -> Result<Vec<RegisteredFont>, ServiceError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .registry
            .list())
    }
}

impl HostServiceHandler for MemoryFonts {
    fn service_id(&self) -> u32 {
        SERVICE_FONTS
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_fonts_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn FontsBackend>(self))
    }
}
