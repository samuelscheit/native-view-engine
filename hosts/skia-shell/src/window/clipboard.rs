//! Zero-dependency cross-platform clipboard abstraction for `nve-skia-shell`.
//!
//! - macOS: native `NSPasteboard` via Objective-C runtime C FFI (`AppKit` + `libobjc`)
//! - Windows: native Win32 `OpenClipboard` / `GetClipboardData` / `SetClipboardData` (`user32` + `kernel32`)
//! - Linux / headless / unit tests: [`InMemoryClipboard`]

/// Platform-agnostic plain-text clipboard interface.
pub trait ClipboardProvider: Send {
    fn get_text(&mut self) -> Option<String>;
    fn set_text(&mut self, text: &str) -> bool;
}

/// Deterministic in-memory clipboard used on headless/Linux hosts and unit tests.
#[derive(Debug, Default, Clone)]
pub struct InMemoryClipboard {
    content: Option<String>,
}

impl InMemoryClipboard {
    pub fn new() -> Self {
        Self { content: None }
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        Self {
            content: Some(text.into()),
        }
    }
}

impl ClipboardProvider for InMemoryClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.content.clone()
    }

    fn set_text(&mut self, text: &str) -> bool {
        self.content = Some(text.to_owned());
        true
    }
}

/// Returns the default clipboard provider for the current host platform.
pub fn default_clipboard() -> Box<dyn ClipboardProvider> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOsClipboard)
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(windows_impl::WindowsClipboard)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Box::new(InMemoryClipboard::new())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::mem::transmute;

    use super::ClipboardProvider;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    type MsgSend0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type MsgSendVoid = unsafe extern "C" fn(*mut c_void, *mut c_void);
    type MsgSendIsize = unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize;
    type MsgSend1Ptr = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
    type MsgSend1CStr =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char) -> *mut c_void;
    type MsgSend2Bool =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> bool;
    type MsgSendCStr = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *const c_char;

    const PASTEBOARD_TYPE_UTF8: &[u8] = b"public.utf8-plain-text\0";

    struct AutoreleasePool {
        pool: *mut c_void,
    }

    impl AutoreleasePool {
        unsafe fn new() -> Self {
            unsafe {
                let cls = objc_getClass(c"NSAutoreleasePool".as_ptr());
                if cls.is_null() {
                    return Self {
                        pool: std::ptr::null_mut(),
                    };
                }
                let msg0: MsgSend0 = transmute(objc_msgSend as *const ());
                let alloc_sel = sel_registerName(c"alloc".as_ptr());
                let init_sel = sel_registerName(c"init".as_ptr());
                let obj = msg0(cls, alloc_sel);
                let pool = if obj.is_null() {
                    std::ptr::null_mut()
                } else {
                    msg0(obj, init_sel)
                };
                Self { pool }
            }
        }
    }

    impl Drop for AutoreleasePool {
        fn drop(&mut self) {
            if !self.pool.is_null() {
                unsafe {
                    let msg_void: MsgSendVoid = transmute(objc_msgSend as *const ());
                    let drain_sel = sel_registerName(c"drain".as_ptr());
                    msg_void(self.pool, drain_sel);
                }
            }
        }
    }

    unsafe fn ns_string(bytes_nul: *const c_char) -> *mut c_void {
        unsafe {
            let cls = objc_getClass(c"NSString".as_ptr());
            if cls.is_null() {
                return std::ptr::null_mut();
            }
            let sel = sel_registerName(c"stringWithUTF8String:".as_ptr());
            let msg: MsgSend1CStr = transmute(objc_msgSend as *const ());
            msg(cls, sel, bytes_nul)
        }
    }

    unsafe fn general_pasteboard() -> *mut c_void {
        unsafe {
            let cls = objc_getClass(c"NSPasteboard".as_ptr());
            if cls.is_null() {
                return std::ptr::null_mut();
            }
            let sel = sel_registerName(c"generalPasteboard".as_ptr());
            let msg: MsgSend0 = transmute(objc_msgSend as *const ());
            msg(cls, sel)
        }
    }

    pub(super) struct MacOsClipboard;

    impl ClipboardProvider for MacOsClipboard {
        fn get_text(&mut self) -> Option<String> {
            unsafe {
                let _pool = AutoreleasePool::new();
                let pb = general_pasteboard();
                if pb.is_null() {
                    return None;
                }
                let pb_type = ns_string(PASTEBOARD_TYPE_UTF8.as_ptr() as *const c_char);
                if pb_type.is_null() {
                    return None;
                }
                let sel_get = sel_registerName(c"stringForType:".as_ptr());
                let msg_get: MsgSend1Ptr = transmute(objc_msgSend as *const ());
                let ns_str = msg_get(pb, sel_get, pb_type);
                if ns_str.is_null() {
                    return None;
                }
                let sel_utf8 = sel_registerName(c"UTF8String".as_ptr());
                let msg_utf8: MsgSendCStr = transmute(objc_msgSend as *const ());
                let c_ptr = msg_utf8(ns_str, sel_utf8);
                if c_ptr.is_null() {
                    return None;
                }
                Some(CStr::from_ptr(c_ptr).to_string_lossy().into_owned())
            }
        }

        fn set_text(&mut self, text: &str) -> bool {
            let Ok(c_text) = CString::new(text) else {
                return false;
            };
            unsafe {
                let _pool = AutoreleasePool::new();
                let pb = general_pasteboard();
                if pb.is_null() {
                    return false;
                }
                let sel_clear = sel_registerName(c"clearContents".as_ptr());
                let msg_clear: MsgSendIsize = transmute(objc_msgSend as *const ());
                let _ = msg_clear(pb, sel_clear);

                let ns_val = ns_string(c_text.as_ptr());
                let pb_type = ns_string(PASTEBOARD_TYPE_UTF8.as_ptr() as *const c_char);
                if ns_val.is_null() || pb_type.is_null() {
                    return false;
                }
                let sel_set = sel_registerName(c"setString:forType:".as_ptr());
                let msg_set: MsgSend2Bool = transmute(objc_msgSend as *const ());
                msg_set(pb, sel_set, ns_val, pb_type)
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::ffi::c_void;

    use super::ClipboardProvider;

    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hwnd: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(format: u32) -> *mut c_void;
        fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalFree(mem: *mut c_void) -> *mut c_void;
        fn GlobalLock(mem: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(mem: *mut c_void) -> i32;
    }

    pub(super) struct WindowsClipboard;

    impl ClipboardProvider for WindowsClipboard {
        fn get_text(&mut self) -> Option<String> {
            unsafe {
                if OpenClipboard(std::ptr::null_mut()) == 0 {
                    return None;
                }
                let handle = GetClipboardData(CF_UNICODETEXT);
                if handle.is_null() {
                    CloseClipboard();
                    return None;
                }
                let ptr = GlobalLock(handle) as *const u16;
                if ptr.is_null() {
                    CloseClipboard();
                    return None;
                }
                let mut len = 0usize;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                let out = String::from_utf16_lossy(slice);
                GlobalUnlock(handle);
                CloseClipboard();
                Some(out)
            }
        }

        fn set_text(&mut self, text: &str) -> bool {
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let byte_len = wide.len() * std::mem::size_of::<u16>();
            unsafe {
                if OpenClipboard(std::ptr::null_mut()) == 0 {
                    return false;
                }
                EmptyClipboard();
                let mem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
                if mem.is_null() {
                    CloseClipboard();
                    return false;
                }
                let ptr = GlobalLock(mem) as *mut u16;
                if ptr.is_null() {
                    GlobalFree(mem);
                    CloseClipboard();
                    return false;
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                GlobalUnlock(mem);
                let ok = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
                if !ok {
                    GlobalFree(mem);
                }
                CloseClipboard();
                ok
            }
        }
    }
}
