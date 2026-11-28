//! Direct-FFI callback-sink C-ABI types ([`SurfaceToken`], [`NveBytesSink`], [`NveAsyncBytesDone`])
//! and panic-safe Rust trampoline helpers ([`with_bytes_sink`], [`invoke_bytes_sink`],
//! [`invoke_async_bytes_done`], [`sink_bytes`]).
//!
//! Unlike [`crate::hybrid::NveHybridOwnedBytes`] (which requires heap allocation + paired
//! `free_bytes` across FFI), callback sinks let callees write directly from a stack or borrowed
//! slice into the caller's callback in a single FFI crossing with zero extra heap allocations and
//! zero cross-allocator ownership transfer.

use core::any::Any;
use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

use nve_abi_macros::NveAbi;

use crate::id::SurfaceId;

/// Reload-safe 8-byte C-ABI POD token binding a [`SurfaceId`] to its monotonic `reload_gen`.
///
/// Encoding the reload generation directly in the type prevents stale asynchronous host callbacks
/// from delivery into a reloaded or recreated surface generation.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, NveAbi)]
#[nve_abi(size = 8)]
pub struct SurfaceToken {
    /// Generational surface identifier (`SurfaceId::NULL` when unbound).
    #[nve_abi(offset = 0, c_type = "NveSurfaceId")]
    pub surface_id: SurfaceId,
    /// Monotonic hot-reload / mount generation for `surface_id`.
    #[nve_abi(offset = 4)]
    pub reload_gen: u32,
}

const _: () = {
    assert!(size_of::<SurfaceToken>() == 8);
    assert!(align_of::<SurfaceToken>() == 4);
    assert!(offset_of!(SurfaceToken, surface_id) == 0);
    assert!(offset_of!(SurfaceToken, reload_gen) == 4);
};

impl SurfaceToken {
    /// Unbound / null surface token (`SurfaceId::NULL`, generation `0`).
    pub const NULL: Self = Self {
        surface_id: SurfaceId::NULL,
        reload_gen: 0,
    };

    #[inline(always)]
    pub const fn new(surface_id: SurfaceId, reload_gen: u32) -> Self {
        Self {
            surface_id,
            reload_gen,
        }
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.surface_id.is_null()
    }

    /// Canonicalizes an FFI-received token so any index-`0` `SurfaceId` bit pattern becomes
    /// [`SurfaceId::NULL`].
    #[inline(always)]
    pub const fn canonicalize(self) -> Self {
        Self {
            surface_id: SurfaceId::from_raw_or_null(self.surface_id.raw()),
            reload_gen: self.reload_gen,
        }
    }

    /// Advances `reload_gen` by one (wrapping) while preserving `surface_id`.
    #[inline(always)]
    pub const fn next_reload(self) -> Self {
        Self {
            surface_id: self.surface_id,
            reload_gen: self.reload_gen.wrapping_add(1),
        }
    }

    /// Returns `true` iff `self` is non-null and matches `active` in both `surface_id` and
    /// `reload_gen` (rejecting stale callbacks from prior reloads or destroyed surfaces).
    #[inline(always)]
    pub const fn is_live_for(self, active: Self) -> bool {
        !self.is_null()
            && self.surface_id.raw() == active.surface_id.raw()
            && self.reload_gen == active.reload_gen
    }

    /// Packs `(surface_id, reload_gen)` losslessly into a 64-bit owner tag.
    #[inline(always)]
    pub const fn to_u64(self) -> u64 {
        (self.surface_id.raw() as u64) | ((self.reload_gen as u64) << 32)
    }

    /// Unpacks a 64-bit owner tag into a canonicalized [`SurfaceToken`].
    #[inline(always)]
    pub const fn from_u64(raw: u64) -> Self {
        Self {
            surface_id: SurfaceId::from_raw_or_null(raw as u32),
            reload_gen: (raw >> 32) as u32,
        }
    }
}

/// Synchronous C-ABI borrowed byte-sink callback (`(user_ctx, ptr, len)`).
///
/// The callee invokes this callback synchronously before returning, passing a borrowed slice valid
/// for the duration of the callback (`(NULL, 0)` for empty payloads).
pub type NveBytesSink =
    unsafe extern "C" fn(user_ctx: *mut c_void, ptr: *const u8, len: usize);

/// Asynchronous C-ABI completion callback delivering `(request_id, status, ptr, len)` once.
///
/// `ptr` / `len` borrow the completion payload for the duration of the callback (`(NULL, 0)` for
/// empty payloads or error statuses).
pub type NveAsyncBytesDone = unsafe extern "C" fn(
    user_ctx: *mut c_void,
    request_id: u64,
    status: i32,
    ptr: *const u8,
    len: usize,
);

/// Validates a raw C-ABI `(ptr, len)` pair into a borrowed Rust byte slice, returning `None` on
/// a null pointer with `len > 0` or `len > isize::MAX`.
///
/// # Safety
/// When `len > 0`, `ptr` must be valid for reads of `len` bytes for `'a`.
#[inline]
pub unsafe fn sink_bytes<'a>(ptr: *const u8, len: usize) -> Option<&'a [u8]> {
    // SAFETY: Caller guarantees `ptr` is valid for `len` bytes for `'a` when `len > 0`.
    unsafe { crate::value::guarded_slice(ptr, len) }
}

struct BytesSinkState<F> {
    on_bytes: F,
    panic: Option<Box<dyn Any + Send>>,
}

unsafe extern "C" fn bytes_sink_trampoline<F: FnMut(&[u8])>(
    user_ctx: *mut c_void,
    ptr: *const u8,
    len: usize,
) {
    if user_ctx.is_null() {
        return;
    }
    // SAFETY: `with_bytes_sink` pins `&mut BytesSinkState<F>` on its stack frame for the entire
    // duration of the synchronous foreign call; `(ptr, len)` is validated before reading.
    let state = unsafe { &mut *(user_ctx as *mut BytesSinkState<F>) };
    if state.panic.is_some() {
        return;
    }
    // SAFETY: Foreign callee guarantees `(ptr, len)` is valid for reads during this callback.
    let Some(bytes) = (unsafe { sink_bytes(ptr, len) }) else {
        return;
    };
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| (state.on_bytes)(bytes))) {
        state.panic = Some(payload);
    }
}

/// Exposes a Rust `FnMut(&[u8])` closure as a synchronous C-ABI `(NveBytesSink, *mut c_void)` pair
/// for the duration of `call`.
///
/// - **Zero heap allocation** on the non-panicking fast path.
/// - **Panic-safe across `extern "C"`**: any panic inside `on_bytes` is caught inside the C
///   trampoline (preventing UB across FFI) and resumed cleanly once `call` returns to Rust.
/// - **Null/corrupt slice guarded**: `(NULL, 0)` delivers `&[]`; `(NULL, len > 0)` or
///   `len > isize::MAX` is ignored safely without creating an invalid slice.
#[inline]
pub fn with_bytes_sink<F, R>(
    on_bytes: F,
    call: impl FnOnce(NveBytesSink, *mut c_void) -> R,
) -> R
where
    F: FnMut(&[u8]),
{
    let mut state = BytesSinkState {
        on_bytes,
        panic: None,
    };
    let ctx_ptr = (&mut state as *mut BytesSinkState<F>).cast::<c_void>();
    let result = call(bytes_sink_trampoline::<F>, ctx_ptr);
    if let Some(payload) = state.panic {
        resume_unwind(payload);
    }
    result
}

/// Delivers `bytes` to a foreign C-ABI [`NveBytesSink`] callback, normalizing empty slices to
/// `(NULL, 0)` so no dangling non-null pointer ever crosses FFI. Returns `false` if `sink` is `None`.
///
/// # Safety
/// When `sink` is `Some(f)`, `f` and `user_ctx` must satisfy the [`NveBytesSink`] C ABI contract
/// for this synchronous call.
#[inline]
pub unsafe fn invoke_bytes_sink(
    sink: Option<NveBytesSink>,
    user_ctx: *mut c_void,
    bytes: &[u8],
) -> bool {
    let Some(f) = sink else {
        return false;
    };
    let ptr = crate::slab_ptr(bytes);
    // SAFETY: Forwarded caller contract; `(ptr, bytes.len())` is valid (`NULL` when empty).
    unsafe { f(user_ctx, ptr, bytes.len()) };
    true
}

/// Delivers an asynchronous completion `(request_id, status, bytes)` to a foreign
/// [`NveAsyncBytesDone`] callback, normalizing empty slices to `(NULL, 0)`. Returns `false` if
/// `done` is `None`.
///
/// # Safety
/// When `done` is `Some(f)`, `f` and `user_ctx` must satisfy the [`NveAsyncBytesDone`] C ABI
/// contract.
#[inline]
pub unsafe fn invoke_async_bytes_done(
    done: Option<NveAsyncBytesDone>,
    user_ctx: *mut c_void,
    request_id: u64,
    status: i32,
    bytes: &[u8],
) -> bool {
    let Some(f) = done else {
        return false;
    };
    let ptr = crate::slab_ptr(bytes);
    // SAFETY: Forwarded caller contract; `(ptr, bytes.len())` is valid (`NULL` when empty).
    unsafe { f(user_ctx, request_id, status, ptr, bytes.len()) };
    true
}
