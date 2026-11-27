// Validation of raw trampoline arguments.
//
// Everything a foreign caller passes by value crosses as a plain C-ABI carrier (`u8` for `bool`
// and `is_some`, `u32` for enums, `NveHybridRaw<Struct>` for structs holding either), so no bit
// pattern a C caller can produce is undefined behavior on entry. Trampolines turn carriers into
// Rust values only through these checks.

use crate::{DecodeError, NveBufferSlice};

/// Borrows a `Bytes` or `NveBinValue` trampoline argument, rejecting `(null, len > 0)` and
/// `len > isize::MAX`.
///
/// # Safety
/// When `arg.len > 0` and `arg.ptr` is non-null, it must point to `arg.len` readable bytes for `'a`.
#[inline]
pub unsafe fn buffer_arg<'a>(arg: NveBufferSlice) -> Result<&'a [u8], DecodeError> {
    // SAFETY: Forwarded caller contract.
    unsafe { arg.try_as_slice() }.ok_or(DecodeError::Truncated)
}

/// A C `bool` byte: exactly 0 or 1.
#[inline]
pub fn decode_bool(raw: u8) -> Result<bool, DecodeError> {
    match raw {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DecodeError::OutOfRange("bool")),
    }
}

/// An `NveHybridOpt*` carrier. `value` is decoded only when `is_some` is 1: a `None` may carry
/// any bits (C++ passes `std::nullopt` zeroed, even for an enum without a 0 variant).
#[inline]
pub fn decode_opt<R, T>(
    is_some: u8,
    value: R,
    decode: impl FnOnce(R) -> Result<T, DecodeError>,
) -> Result<Option<T>, DecodeError> {
    if decode_bool(is_some)? {
        decode(value).map(Some)
    } else {
        Ok(None)
    }
}

/// Decodes a fixed-size array of carriers element-wise, without allocating; the first failing
/// element's error wins.
#[inline]
pub fn decode_array<R, T: Default, const N: usize>(
    raw: [R; N],
    mut decode: impl FnMut(R) -> Result<T, DecodeError>,
) -> Result<[T; N], DecodeError> {
    let mut error = None;
    let out = raw.map(|item| match decode(item) {
        Ok(v) => v,
        Err(e) => {
            error.get_or_insert(e);
            T::default()
        }
    });
    error.map_or(Ok(out), Err)
}
