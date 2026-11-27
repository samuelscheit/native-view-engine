//! Static runtime of generated Rust hybrid bindings (`<stem>_hybrid.generated.rs`).
//!
//! Generated files hold only what depends on a spec (types, traits, vtables, trampolines); the
//! C-ABI buffer and status types, the `NveBinValue` codec of every hybrid type ([`HybridCodec`]),
//! and the trampoline error plumbing live here once.

mod codec;
#[cfg(test)]
mod tests;
mod value;

use core::mem::{offset_of, size_of};
use std::mem::ManuallyDrop;

use crate::DecodeError;

pub use codec::{
    HybridCodec, HybridElem, HybridFields, HybridStruct, decode_bin_arg, decode_enum,
    decode_struct_bin, decode_struct_value_ref, encode_struct, encode_to_vec,
};
pub use value::NveHybridValue;

/// Version of the hybrid vtable layout (`abi_version`, the first field of every vtable).
pub const NVE_HYBRID_ABI_VERSION: u32 = 1;

/// Status code returned by fallible hybrid trampolines (`NveHybridStatus` in the C header).
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NveHybridStatus {
    Ok = 0,
    Err = 1,
    InvalidArg = 2,
    SchemaMismatch = 3,
    NullHandle = 4,
}

/// Rust-allocated byte buffer returned across the C ABI.
///
/// Foreign callers must release a non-empty buffer exactly once through the vtable's
/// `free_bytes`; it is deliberately neither `Clone` nor `Copy`.
#[repr(C)]
#[derive(Debug)]
pub struct NveHybridOwnedBytes {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

const _: () = {
    let word = size_of::<usize>();
    assert!(size_of::<NveHybridOwnedBytes>() == 3 * word);
    assert!(offset_of!(NveHybridOwnedBytes, len) == word);
    assert!(offset_of!(NveHybridOwnedBytes, cap) == 2 * word);
    assert!(size_of::<NveHybridStatus>() == 4);
};

impl NveHybridOwnedBytes {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null_mut(),
        len: 0,
        cap: 0,
    };

    #[inline]
    pub fn from_vec(v: Vec<u8>) -> Self {
        if v.capacity() == 0 {
            return Self::EMPTY;
        }
        let mut v = ManuallyDrop::new(v);
        Self {
            ptr: v.as_mut_ptr(),
            len: v.len(),
            cap: v.capacity(),
        }
    }

    #[inline]
    pub fn from_string(s: String) -> Self {
        Self::from_vec(s.into_bytes())
    }

    /// `value` encoded as one `NveBinValue`.
    #[inline]
    pub fn encode<T: HybridCodec>(value: &T) -> Result<Self, DecodeError> {
        encode_to_vec(value).map(Self::from_vec)
    }

    /// # Safety
    /// `self` must be `EMPTY` or come unmodified from `from_vec` / `nve_hybrid_alloc_bytes`.
    #[inline]
    pub unsafe fn as_slice(&self) -> &[u8] {
        if self.ptr.is_null() || self.len == 0 {
            return &[];
        }
        // SAFETY: The caller guarantees `ptr` is a live allocation of at least `len` bytes.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Reclaims the allocation.
    ///
    /// # Safety
    /// Same as [`Self::as_slice`], and the buffer must not be used or released again.
    #[inline]
    pub unsafe fn into_vec(self) -> Vec<u8> {
        if self.ptr.is_null() || self.cap == 0 {
            return Vec::new();
        }
        // SAFETY: The caller guarantees `(ptr, len, cap)` came unmodified from a `Vec<u8>`.
        unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) }
    }
}

/// `alloc_bytes` of every generated vtable: a zeroed buffer of `len` bytes.
pub extern "C" fn nve_hybrid_alloc_bytes(len: usize) -> NveHybridOwnedBytes {
    NveHybridOwnedBytes::from_vec(vec![0; len])
}

/// `free_bytes` of every generated vtable.
///
/// # Safety
/// See [`NveHybridOwnedBytes::into_vec`].
pub unsafe extern "C" fn nve_hybrid_free_bytes(buf: NveHybridOwnedBytes) {
    // SAFETY: Forwarded caller contract.
    drop(unsafe { buf.into_vec() });
}

/// Stores `value` through a trampoline out-pointer; a null pointer discards it.
///
/// # Safety
/// `out` must be null or valid for writes.
#[inline]
pub unsafe fn store<T>(out: *mut T, value: T) {
    if !out.is_null() {
        // SAFETY: Non-null checked; the caller guarantees `out` is valid for writes.
        unsafe { out.write(value) };
    }
}

/// Clears `*out_err` and returns `NveHybridStatus::Ok` as a trampoline result code.
///
/// # Safety
/// `out_err` must be null or valid for writes.
#[inline]
pub unsafe fn succeed(out_err: *mut NveHybridOwnedBytes) -> i32 {
    // SAFETY: Forwarded caller contract.
    unsafe { store(out_err, NveHybridOwnedBytes::EMPTY) };
    NveHybridStatus::Ok as i32
}

/// Stores `message` in `*out_err` and returns `status` as a trampoline result code.
///
/// # Safety
/// `out_err` must be null or valid for writes.
#[inline]
pub unsafe fn fail(
    out_err: *mut NveHybridOwnedBytes,
    status: NveHybridStatus,
    message: String,
) -> i32 {
    // SAFETY: Forwarded caller contract.
    unsafe { store(out_err, NveHybridOwnedBytes::from_string(message)) };
    status as i32
}
