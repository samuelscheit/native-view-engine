//! Zero-dependency native OS clipboard backends (macOS `NSPasteboard`, Windows Win32 Clipboard,
//! Unix/WASM safe fallback).

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_CLIPBOARD;
use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::{dispatch_clipboard_wire, ClipboardBackend};

/// Native OS [`ClipboardBackend`] & [`HostServiceHandler`] for [`SERVICE_CLIPBOARD`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeClipboard;

impl NativeClipboard {
    pub const fn new() -> Self {
        Self
    }
}

impl ClipboardBackend for NativeClipboard {
    fn read_text(&self) -> Result<Option<String>, ServiceError> {
        platform::read_text()
    }

    fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        platform::write_text(text)
    }

    fn has_text(&self) -> Result<bool, ServiceError> {
        platform::has_text()
    }

    fn clear(&self) -> Result<(), ServiceError> {
        platform::clear()
    }
}

impl HostServiceHandler for NativeClipboard {
    fn service_id(&self) -> u32 {
        SERVICE_CLIPBOARD
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_clipboard_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn ClipboardBackend>(self))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use nve_host_services::apple_objc::{
        cls, msg_bool2, msg_id0, msg_id1, msg_isize0, nsstring_from_str, nsstring_to_string, sel,
        with_autorelease_pool,
    };
    use nve_host_services::ServiceError;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    pub fn read_text() -> Result<Option<String>, ServiceError> {
        with_autorelease_pool(|| unsafe {
            let pb_cls = cls(b"NSPasteboard\0");
            if pb_cls.is_null() {
                return Err(ServiceError::Unsupported);
            }
            let pb = msg_id0(pb_cls, sel(b"generalPasteboard\0"));
            if pb.is_null() {
                return Err(ServiceError::Unsupported);
            }
            let ptype = nsstring_from_str("public.utf8-plain-text");
            let str_obj = msg_id1(pb, sel(b"stringForType:\0"), ptype);
            Ok(nsstring_to_string(str_obj))
        })
    }

    pub fn write_text(text: &str) -> Result<(), ServiceError> {
        with_autorelease_pool(|| unsafe {
            let pb_cls = cls(b"NSPasteboard\0");
            if pb_cls.is_null() {
                return Err(ServiceError::Unsupported);
            }
            let pb = msg_id0(pb_cls, sel(b"generalPasteboard\0"));
            if pb.is_null() {
                return Err(ServiceError::Unsupported);
            }
            let _ = msg_isize0(pb, sel(b"clearContents\0"));
            let ptype = nsstring_from_str("public.utf8-plain-text");
            let val = nsstring_from_str(text);
            if msg_bool2(pb, sel(b"setString:forType:\0"), val, ptype) {
                Ok(())
            } else {
                Err(ServiceError::IoError(
                    "NSPasteboard setString:forType: failed".into(),
                ))
            }
        })
    }

    pub fn has_text() -> Result<bool, ServiceError> {
        Ok(read_text()?.is_some_and(|s| !s.is_empty()))
    }

    pub fn clear() -> Result<(), ServiceError> {
        with_autorelease_pool(|| unsafe {
            let pb_cls = cls(b"NSPasteboard\0");
            if pb_cls.is_null() {
                return Err(ServiceError::Unsupported);
            }
            let pb = msg_id0(pb_cls, sel(b"generalPasteboard\0"));
            if pb.is_null() {
                return Err(ServiceError::Unsupported);
            }
            let _ = msg_isize0(pb, sel(b"clearContents\0"));
            Ok(())
        })
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;

    use nve_host_services::win32_str::{to_wide_nul, wide_ptr_to_string};
    use nve_host_services::ServiceError;

    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hwnd: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn IsClipboardFormatAvailable(format: u32) -> i32;
        fn GetClipboardData(format: u32) -> *mut c_void;
        fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(mem: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(mem: *mut c_void) -> i32;
        fn GlobalFree(mem: *mut c_void) -> *mut c_void;
    }

    pub fn read_text() -> Result<Option<String>, ServiceError> {
        unsafe {
            if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
                return Ok(None);
            }
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return Err(ServiceError::IoError("OpenClipboard failed".into()));
            }
            let handle = GetClipboardData(CF_UNICODETEXT);
            let result = if handle.is_null() {
                Ok(None)
            } else {
                let ptr = GlobalLock(handle) as *const u16;
                if ptr.is_null() {
                    Err(ServiceError::IoError("GlobalLock failed".into()))
                } else {
                    let s = wide_ptr_to_string(ptr);
                    GlobalUnlock(handle);
                    Ok(Some(s))
                }
            };
            CloseClipboard();
            result
        }
    }

    pub fn write_text(text: &str) -> Result<(), ServiceError> {
        let wide = to_wide_nul(text);
        let byte_len = wide.len() * 2;
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return Err(ServiceError::IoError("OpenClipboard failed".into()));
            }
            EmptyClipboard();
            let mem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
            if mem.is_null() {
                CloseClipboard();
                return Err(ServiceError::IoError("GlobalAlloc failed".into()));
            }
            let dst = GlobalLock(mem) as *mut u16;
            if dst.is_null() {
                GlobalFree(mem);
                CloseClipboard();
                return Err(ServiceError::IoError("GlobalLock failed".into()));
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
            GlobalUnlock(mem);
            if SetClipboardData(CF_UNICODETEXT, mem).is_null() {
                GlobalFree(mem);
                CloseClipboard();
                return Err(ServiceError::IoError("SetClipboardData failed".into()));
            }
            CloseClipboard();
            Ok(())
        }
    }

    pub fn has_text() -> Result<bool, ServiceError> {
        Ok(unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 })
    }

    pub fn clear() -> Result<(), ServiceError> {
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return Err(ServiceError::IoError("OpenClipboard failed".into()));
            }
            EmptyClipboard();
            CloseClipboard();
            Ok(())
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use nve_host_services::ServiceError;

    pub fn read_text() -> Result<Option<String>, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn write_text(_text: &str) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn has_text() -> Result<bool, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn clear() -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
}
