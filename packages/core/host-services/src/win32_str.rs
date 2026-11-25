//! Zero-dependency UTF-16 Win32 string conversion helpers shared by opt-in Windows capability
//! modules (`nve-clipboard`, `nve-filesystem`, `nve-secure-store`, `nve-linking`, `nve-system-env`).

/// Encodes `s` as a NUL-terminated UTF-16 buffer suitable for `*W` Win32 APIs.
pub fn to_wide_nul(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

/// Decodes a NUL-terminated UTF-16 Win32 pointer into a Rust `String`.
///
/// # Safety
/// `ptr` must point to a valid NUL-terminated UTF-16 string.
pub unsafe fn wide_ptr_to_string(ptr: *const u16) -> String {
    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf16_lossy(slice)
}
