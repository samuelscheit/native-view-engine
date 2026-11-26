//! Minimal zero-dependency macOS Objective-C runtime & `NSString` FFI helpers shared by
//! opt-in native capability modules (`nve-clipboard`, `nve-linking`, `nve-system-env`,
//! `nve-haptics`).

use std::ffi::{c_char, c_void, CStr};

pub type Id = *mut c_void;
pub type Sel = *mut c_void;

const NS_UTF8_STRING_ENCODING: usize = 4;

#[link(name = "objc")]
unsafe extern "C" {
    pub fn objc_getClass(name: *const c_char) -> Id;
    pub fn sel_registerName(name: *const c_char) -> Sel;
    pub fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

struct AutoreleasePoolGuard(*mut c_void);

impl Drop for AutoreleasePoolGuard {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: `self.0` was returned by `objc_autoreleasePoolPush` on this thread.
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

pub fn with_autorelease_pool<T>(f: impl FnOnce() -> T) -> T {
    // SAFETY: `objc_autoreleasePoolPush` pairs with `objc_autoreleasePoolPop` in `AutoreleasePoolGuard::drop`.
    let _guard = AutoreleasePoolGuard(unsafe { objc_autoreleasePoolPush() });
    f()
}

/// # Safety
/// `name` must be NUL-terminated ASCII.
pub unsafe fn cls(name: &[u8]) -> Id {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: Caller guarantees `name` is NUL-terminated C string bytes.
    unsafe { objc_getClass(name.as_ptr().cast()) }
}

/// # Safety
/// `name` must be NUL-terminated ASCII.
pub unsafe fn sel(name: &[u8]) -> Sel {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: Caller guarantees `name` is NUL-terminated C string bytes.
    unsafe { sel_registerName(name.as_ptr().cast()) }
}

/// # Safety
/// Caller must ensure `obj` and `s` match an Objective-C selector returning an object pointer.
pub unsafe fn msg_id0(obj: Id, s: Sel) -> Id {
    // SAFETY: `objc_msgSend` uses the platform C ABI for the caller-guaranteed selector signature.
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
        f(obj, s)
    }
}

/// # Safety
/// Caller must ensure `obj` and `s` match an Objective-C selector taking 1 object and returning an object.
pub unsafe fn msg_id1(obj: Id, s: Sel, a1: Id) -> Id {
    // SAFETY: `objc_msgSend` uses the platform C ABI for the caller-guaranteed selector signature.
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, Id) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a1)
    }
}

/// # Safety
/// Caller must ensure `obj` and `s` match an Objective-C selector taking 1 object and returning `BOOL`.
pub unsafe fn msg_bool1(obj: Id, s: Sel, a1: Id) -> bool {
    // SAFETY: `objc_msgSend` uses the platform C ABI for the caller-guaranteed selector signature.
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a1) != 0
    }
}

/// # Safety
/// Caller must ensure `obj` and `s` match an Objective-C selector taking 2 objects and returning `BOOL`.
pub unsafe fn msg_bool2(obj: Id, s: Sel, a1: Id, a2: Id) -> bool {
    // SAFETY: `objc_msgSend` uses the platform C ABI for the caller-guaranteed selector signature.
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, Id, Id) -> i8 =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a1, a2) != 0
    }
}

/// # Safety
/// Caller must ensure `obj` and `s` match an Objective-C selector returning `NSInteger`.
pub unsafe fn msg_isize0(obj: Id, s: Sel) -> isize {
    // SAFETY: `objc_msgSend` uses the platform C ABI for the caller-guaranteed selector signature.
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel) -> isize =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s)
    }
}

/// # Safety
/// Caller must ensure `obj` and `s` match an Objective-C selector taking `(NSInteger, NSUInteger)`.
pub unsafe fn msg_void_isize2(obj: Id, s: Sel, a1: isize, a2: usize) {
    // SAFETY: `objc_msgSend` uses the platform C ABI for the caller-guaranteed selector signature.
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, isize, usize) =
            std::mem::transmute(objc_msgSend as *const ());
        f(obj, s, a1, a2)
    }
}

/// # Safety
/// Must be called inside [`with_autorelease_pool`].
pub unsafe fn nsstring_from_str(s: &str) -> Id {
    // SAFETY: Static NUL-terminated class/selector names and valid UTF-8 slice; caller provides active autorelease pool.
    unsafe {
        let ns_string = cls(b"NSString\0");
        if ns_string.is_null() {
            return std::ptr::null_mut();
        }
        let alloc = msg_id0(ns_string, sel(b"alloc\0"));
        if alloc.is_null() {
            return std::ptr::null_mut();
        }
        let init_fn: unsafe extern "C" fn(Id, Sel, *const u8, usize, usize) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        let obj = init_fn(
            alloc,
            sel(b"initWithBytes:length:encoding:\0"),
            s.as_ptr(),
            s.len(),
            NS_UTF8_STRING_ENCODING,
        );
        if obj.is_null() {
            return std::ptr::null_mut();
        }
        msg_id0(obj, sel(b"autorelease\0"))
    }
}

/// # Safety
/// `obj` must be null or a valid `NSString*`.
pub unsafe fn nsstring_to_string(obj: Id) -> Option<String> {
    if obj.is_null() {
        return None;
    }
    // SAFETY: Caller guarantees `obj` is a valid `NSString*`; `UTF8String` returns a NUL-terminated C string or null.
    unsafe {
        let utf8_fn: unsafe extern "C" fn(Id, Sel) -> *const c_char =
            std::mem::transmute(objc_msgSend as *const ());
        let ptr = utf8_fn(obj, sel(b"UTF8String\0"));
        if ptr.is_null() {
            return None;
        }
        let cstr = CStr::from_ptr(ptr);
        Some(cstr.to_string_lossy().into_owned())
    }
}
