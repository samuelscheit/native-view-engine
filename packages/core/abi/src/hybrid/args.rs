// Validation of raw trampoline arguments.
//
// Everything a foreign caller passes by value crosses as a plain C-ABI carrier (`u8` for `bool`
// and `is_some`, `u32` for enums, `NveHybridRaw<Struct>` for structs holding either), so no bit
// pattern a C caller can produce is undefined behavior on entry. Trampolines turn carriers into
// Rust values only through these checks.

use std::mem::{ManuallyDrop, MaybeUninit};

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

/// Decodes a fixed-size array of carriers element-wise, without allocating and without a
/// `Default` bound; it stops at the first failing element and drops the ones decoded before it.
pub fn decode_array<R, T, const N: usize>(
    raw: [R; N],
    mut decode: impl FnMut(R) -> Result<T, DecodeError>,
) -> Result<[T; N], DecodeError> {
    let mut out = InitPrefix::<T, N> {
        items: [const { MaybeUninit::uninit() }; N],
        len: 0,
    };
    for (slot, item) in out.items.iter_mut().zip(raw) {
        slot.write(decode(item)?);
        out.len += 1;
    }
    // `raw` has exactly `N` items, so every slot is written; the guard must not drop them now.
    let out = ManuallyDrop::new(out);
    // SAFETY: All `N` slots are initialized (the loop ran `N` times without an early return), and
    // `ManuallyDrop` hands their ownership to the returned array exactly once.
    Ok(unsafe { std::ptr::read(&out.items) }.map(|slot| {
        // SAFETY: Every slot is initialized, see above.
        unsafe { slot.assume_init() }
    }))
}

/// Slots of which the first `len` are initialized; dropping it drops exactly those.
struct InitPrefix<T, const N: usize> {
    items: [MaybeUninit<T>; N],
    len: usize,
}

impl<T, const N: usize> Drop for InitPrefix<T, N> {
    fn drop(&mut self) {
        for slot in self.items.iter_mut().take(self.len) {
            // SAFETY: The first `len` slots were written by `decode_array` and not moved out.
            unsafe { slot.assume_init_drop() };
        }
    }
}
