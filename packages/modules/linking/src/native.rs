//! Zero-dependency native OS URL launcher backends (macOS `NSWorkspace`, Windows `ShellExecuteW`,
//! Unix `xdg-open`, WASM safe fallback).

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_LINKING;
use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::{dispatch_linking_wire, LinkingBackend};

/// Native OS [`HostServiceHandler`] for [`SERVICE_LINKING`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeLinking;

impl NativeLinking {
    pub const fn new() -> Self {
        Self
    }
}

impl LinkingBackend for NativeLinking {
    fn open_url(&self, url: &str) -> Result<(), ServiceError> {
        platform::open_url(url)
    }

    fn can_open_url(&self, url: &str) -> Result<bool, ServiceError> {
        platform::can_open_url(url)
    }

    fn initial_url(&self) -> Result<Option<String>, ServiceError> {
        Ok(None)
    }
}

impl HostServiceHandler for NativeLinking {
    fn service_id(&self) -> u32 {
        SERVICE_LINKING
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_linking_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn LinkingBackend>(self))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use nve_host_services::apple_objc::{
        cls, msg_bool1, msg_id0, msg_id1, nsstring_from_str, sel, with_autorelease_pool,
    };
    use nve_host_services::ServiceError;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    pub fn open_url(url: &str) -> Result<(), ServiceError> {
        with_autorelease_pool(|| unsafe {
            let ns_url_cls = cls(b"NSURL\0");
            let ws_cls = cls(b"NSWorkspace\0");
            if ns_url_cls.is_null() || ws_cls.is_null() {
                return Err(ServiceError::Unsupported);
            }
            let url_str = nsstring_from_str(url);
            let ns_url = msg_id1(ns_url_cls, sel(b"URLWithString:\0"), url_str);
            if ns_url.is_null() {
                return Err(ServiceError::InvalidArg("NSURL failed to parse URL".into()));
            }
            let ws = msg_id0(ws_cls, sel(b"sharedWorkspace\0"));
            if ws.is_null() {
                return Err(ServiceError::Unsupported);
            }
            if msg_bool1(ws, sel(b"openURL:\0"), ns_url) {
                Ok(())
            } else {
                Err(ServiceError::IoError(
                    "NSWorkspace openURL: returned false".into(),
                ))
            }
        })
    }

    pub fn can_open_url(url: &str) -> Result<bool, ServiceError> {
        with_autorelease_pool(|| unsafe {
            let ns_url_cls = cls(b"NSURL\0");
            let ws_cls = cls(b"NSWorkspace\0");
            if ns_url_cls.is_null() || ws_cls.is_null() {
                return Ok(false);
            }
            let url_str = nsstring_from_str(url);
            let ns_url = msg_id1(ns_url_cls, sel(b"URLWithString:\0"), url_str);
            if ns_url.is_null() {
                return Ok(false);
            }
            let ws = msg_id0(ws_cls, sel(b"sharedWorkspace\0"));
            if ws.is_null() {
                return Ok(false);
            }
            let app_url = msg_id1(ws, sel(b"URLForApplicationToOpenURL:\0"), ns_url);
            Ok(!app_url.is_null())
        })
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;

    use nve_host_services::win32_str::to_wide_nul;
    use nve_host_services::{validate_url, ServiceError};

    const SW_SHOWNORMAL: i32 = 1;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show_cmd: i32,
        ) -> isize;
    }

    pub fn open_url(url: &str) -> Result<(), ServiceError> {
        let op = to_wide_nul("open");
        let target = to_wide_nul(url);
        let rc = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                op.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if rc > 32 {
            Ok(())
        } else {
            Err(ServiceError::IoError(format!(
                "ShellExecuteW failed ({rc})"
            )))
        }
    }

    pub fn can_open_url(url: &str) -> Result<bool, ServiceError> {
        Ok(validate_url(url).is_ok())
    }
}

#[cfg(all(
    not(any(target_os = "macos", target_os = "windows")),
    not(target_arch = "wasm32")
))]
mod platform {
    use std::process::Command;

    use nve_host_services::{validate_url, ServiceError};

    fn has_xdg_open() -> bool {
        let Some(paths) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&paths).any(|dir| dir.join("xdg-open").is_file())
    }

    pub fn open_url(url: &str) -> Result<(), ServiceError> {
        if !has_xdg_open() {
            return Err(ServiceError::Unsupported);
        }
        let status = Command::new("xdg-open")
            .arg(url)
            .status()
            .map_err(|e| ServiceError::IoError(e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(ServiceError::IoError(format!(
                "xdg-open exited with {status}"
            )))
        }
    }

    pub fn can_open_url(url: &str) -> Result<bool, ServiceError> {
        Ok(has_xdg_open() && validate_url(url).is_ok())
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    use nve_host_services::ServiceError;

    pub fn open_url(_url: &str) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn can_open_url(_url: &str) -> Result<bool, ServiceError> {
        Ok(false)
    }
}
