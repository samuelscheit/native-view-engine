//! Direct-FFI callback-sink C-ABI types ([`SurfaceToken`], [`NveBytesSink`], [`NveAsyncBytesDone`])
//! and panic-safe, re-entrancy-safe Rust trampoline helpers ([`with_bytes_sink`],
//! [`try_with_bytes_sink`], [`with_bytes_sink_once`], [`try_with_bytes_sink_once`],
//! [`AsyncBytesDoneGuard`], [`into_async_bytes_done`], [`invoke_bytes_sink`],
//! [`invoke_async_bytes_done`], [`sink_bytes`]).
//!
//! Lives outside `feature = "wire"` so `--no-default-features` builds (no `serde`) can use direct
//! callback-sink FFI with zero heap allocation on the synchronous fast path.

mod async_guard;

use core::any::Any;
use core::cell::{Cell, UnsafeCell};
use core::ffi::c_void;
use core::fmt;
use core::mem::{align_of, offset_of, size_of};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

use nve_abi_macros::NveAbi;

use crate::id::SurfaceId;

pub use async_guard::{
    into_async_bytes_done, invoke_async_bytes_done, AsyncBytesDoneGuard, NveAsyncBytesDone,
};

/// Reload-safe 8-byte C-ABI POD token binding a [`SurfaceId`] to its monotonic `reload_gen`.
///
/// Encoding the reload generation directly in the type prevents stale asynchronous host callbacks
/// from delivery into a reloaded or recreated surface generation.
///
/// # Canonical Null Invariant
/// Every Rust constructor and decoder enforces `surface_id.is_null() => reload_gen == 0`, so
/// `token.is_null() <=> token == SurfaceToken::NULL`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, NveAbi)]
#[nve_abi(name = "NveSurfaceToken", size = 8)]
pub struct SurfaceToken {
    /// Generational surface identifier (`SurfaceId::NULL` when unbound).
    #[nve_abi(offset = 0, c_type = "NveSurfaceId")]
    pub surface_id: SurfaceId,
    /// Monotonic hot-reload / mount generation for `surface_id` (always `0` when null).
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

    /// Constructs a canonical [`SurfaceToken`]; any null `surface_id` canonicalizes to
    /// [`Self::NULL`] (`reload_gen == 0`).
    #[must_use]
    #[inline(always)]
    pub const fn new(surface_id: SurfaceId, reload_gen: u32) -> Self {
        let surface_id = SurfaceId::from_raw_or_null(surface_id.raw());
        if surface_id.is_null() {
            Self::NULL
        } else {
            Self {
                surface_id,
                reload_gen,
            }
        }
    }

    #[must_use]
    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.surface_id.is_null()
    }

    /// Canonicalizes an FFI-received token so any index-`0` `SurfaceId` bit pattern becomes
    /// [`Self::NULL`] (forcing `reload_gen == 0`).
    #[must_use]
    #[inline(always)]
    pub const fn canonicalize(self) -> Self {
        let surface_id = SurfaceId::from_raw_or_null(self.surface_id.raw());
        if surface_id.is_null() {
            Self::NULL
        } else {
            Self {
                surface_id,
                reload_gen: self.reload_gen,
            }
        }
    }

    /// Advances `reload_gen` by one (wrapping) for a live token, or returns [`Self::NULL`] if
    /// `self` is null (preserving `SurfaceToken::NULL.next_reload() == SurfaceToken::NULL`).
    #[must_use]
    #[inline(always)]
    pub const fn next_reload(self) -> Self {
        let canon = self.canonicalize();
        if canon.is_null() {
            Self::NULL
        } else {
            Self {
                surface_id: canon.surface_id,
                reload_gen: canon.reload_gen.wrapping_add(1),
            }
        }
    }

    /// Returns `true` iff `self` is non-null and matches `active` in both `surface_id` and
    /// `reload_gen` (rejecting stale callbacks from prior reloads or destroyed surfaces).
    #[must_use]
    #[inline(always)]
    pub const fn is_live_for(self, active: Self) -> bool {
        !self.is_null()
            && self.surface_id.raw() == active.surface_id.raw()
            && self.reload_gen == active.reload_gen
    }

    /// Packs a canonicalized `(surface_id, reload_gen)` losslessly into a 64-bit owner tag.
    #[must_use]
    #[inline(always)]
    pub const fn to_u64(self) -> u64 {
        let canon = self.canonicalize();
        (canon.surface_id.raw() as u64) | ((canon.reload_gen as u64) << 32)
    }

    /// Unpacks a 64-bit owner tag into a canonicalized [`SurfaceToken`] (`surface_id.is_null() =>
    /// Self::NULL`).
    #[must_use]
    #[inline(always)]
    pub const fn from_u64(raw: u64) -> Self {
        let surface_id = SurfaceId::from_raw_or_null(raw as u32);
        if surface_id.is_null() {
            Self::NULL
        } else {
            Self {
                surface_id,
                reload_gen: (raw >> 32) as u32,
            }
        }
    }
}

/// Protocol violation observed while receiving bytes through [`NveBytesSink`] or
/// [`NveAsyncBytesDone`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BytesSinkError {
    /// Foreign callee delivered a corrupt `(ptr, len)` pair (`(NULL, len > 0)` or
    /// `len > isize::MAX`).
    CorruptSlice { len: usize },
    /// Foreign callee invoked a synchronous [`NveBytesSink`] re-entrantly while an earlier
    /// invocation of the same sink was still active on the stack.
    Reentrant,
    /// Foreign callee invoked a single-shot synchronous sink ([`with_bytes_sink_once`]) more than
    /// once.
    DuplicateCall,
}

impl fmt::Display for BytesSinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CorruptSlice { len } => {
                write!(f, "corrupt C-ABI byte slice delivered to sink (len={len})")
            }
            Self::Reentrant => {
                f.write_str("re-entrant invocation of synchronous NveBytesSink rejected")
            }
            Self::DuplicateCall => {
                f.write_str("duplicate invocation of single-shot NveBytesSink rejected")
            }
        }
    }
}

impl std::error::Error for BytesSinkError {}

/// Synchronous C-ABI borrowed byte-sink callback (`(user_ctx, ptr, len)`).
///
/// # FFI Contract
/// - **Single-threaded & call-scoped**: the callee may invoke this callback only on the calling
///   thread before the enclosing FFI call returns; `user_ctx` MUST NOT be stored or used after the
///   call returns.
/// - **Non-reentrant**: the callee MUST NOT invoke the sink recursively while a prior sink call is
///   still on the stack (re-entrancy is rejected safely and surfaced as
///   [`BytesSinkError::Reentrant`]).
/// - **Valid slice**: pass `(NULL, 0)` for empty payloads; when `len > 0`, `ptr` must point to a
///   single allocated object of at least `len` initialized, immutable bytes for the duration of
///   the callback (`len <= isize::MAX`).
pub type NveBytesSink =
    unsafe extern "C" fn(user_ctx: *mut c_void, ptr: *const u8, len: usize);

/// Validates a raw C-ABI `(ptr, len)` pair into a borrowed Rust byte slice, returning `None` on
/// a null pointer with `len > 0` or `len > isize::MAX`.
///
/// # Safety
/// When `len > 0` and `ptr` is non-null with `len <= isize::MAX as usize`, `ptr` must point to a
/// single allocated object containing at least `len` initialized `u8` bytes that remain valid for
/// reads and unmutated for the entire lifetime `'a`.
#[must_use]
#[inline]
pub unsafe fn sink_bytes<'a>(ptr: *const u8, len: usize) -> Option<&'a [u8]> {
    // SAFETY: Caller guarantees `ptr` points to a single allocated object of `len` initialized,
    // immutable bytes for `'a` when `len > 0`.
    unsafe { crate::value::guarded_slice(ptr, len) }
}

struct BytesSinkState<F> {
    entered: Cell<bool>,
    allow_multiple: bool,
    calls: Cell<u32>,
    panicked: Cell<bool>,
    violation: Cell<Option<BytesSinkError>>,
    panic: Cell<Option<Box<dyn Any + Send>>>,
    on_bytes: UnsafeCell<F>,
}

struct ReentrancyGuard<'a>(&'a Cell<bool>);

impl Drop for ReentrancyGuard<'_> {
    #[inline]
    fn drop(&mut self) {
        self.0.set(false);
    }
}

unsafe extern "C" fn bytes_sink_trampoline<F: FnMut(&[u8])>(
    user_ctx: *mut c_void,
    ptr: *const u8,
    len: usize,
) {
    if user_ctx.is_null() {
        return;
    }
    // SAFETY: `run_bytes_sink` keeps `BytesSinkState<F>` alive on the calling thread's stack for
    // the entire synchronous foreign call and only ever exposes shared `&BytesSinkState<F>`
    // references (interior mutability via `Cell` / `UnsafeCell`), avoiding `&mut` aliasing UB on
    // re-entrant foreign calls.
    let state = unsafe { &*(user_ctx as *const BytesSinkState<F>) };
    if state.entered.replace(true) {
        if state.violation.get().is_none() {
            state.violation.set(Some(BytesSinkError::Reentrant));
        }
        return;
    }
    let _entered_guard = ReentrancyGuard(&state.entered);

    if state.panicked.get() || state.violation.get().is_some() {
        return;
    }
    // SAFETY: Foreign callee guarantees `(ptr, len)` satisfies `sink_bytes` preconditions for the
    // duration of this callback when structurally valid.
    let Some(bytes) = (unsafe { sink_bytes(ptr, len) }) else {
        if state.violation.get().is_none() {
            state
                .violation
                .set(Some(BytesSinkError::CorruptSlice { len }));
        }
        return;
    };
    let prev_calls = state.calls.get();
    if !state.allow_multiple && prev_calls > 0 {
        if state.violation.get().is_none() {
            state.violation.set(Some(BytesSinkError::DuplicateCall));
        }
        return;
    }
    state.calls.set(prev_calls.saturating_add(1));

    // SAFETY: `entered` transitioned `false -> true` above (reset by `_entered_guard`) and the
    // `NveBytesSink` contract is strictly single-threaded, so `&mut *state.on_bytes.get()` is
    // uniquely borrowed for this invocation.
    let on_bytes = unsafe { &mut *state.on_bytes.get() };
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| on_bytes(bytes))) {
        state.panicked.set(true);
        state.panic.set(Some(payload));
    }
}

#[inline]
fn run_bytes_sink<F, R>(
    allow_multiple: bool,
    on_bytes: F,
    call: impl FnOnce(NveBytesSink, *mut c_void) -> R,
) -> Result<R, BytesSinkError>
where
    F: FnMut(&[u8]),
{
    let state = BytesSinkState {
        entered: Cell::new(false),
        allow_multiple,
        calls: Cell::new(0),
        panicked: Cell::new(false),
        violation: Cell::new(None),
        panic: Cell::new(None),
        on_bytes: UnsafeCell::new(on_bytes),
    };
    let ctx_ptr = (&state as *const BytesSinkState<F>)
        .cast_mut()
        .cast::<c_void>();
    let result = call(bytes_sink_trampoline::<F>, ctx_ptr);
    if let Some(payload) = state.panic.into_inner() {
        resume_unwind(payload);
    }
    if let Some(violation) = state.violation.get() {
        return Err(violation);
    }
    Ok(result)
}

/// Exposes a Rust `FnMut(&[u8])` closure as a synchronous, single-threaded, non-reentrant,
/// call-scoped C-ABI `(NveBytesSink, *mut c_void)` pair for the duration of `call`.
///
/// - **Zero heap allocation** on the non-panicking fast path.
/// - **Panic-safe across `extern "C"`**: any panic inside `on_bytes` is caught inside the C
///   trampoline (preventing UB across FFI) and resumed cleanly once `call` returns to Rust.
/// - **Explicit protocol errors**: `(NULL, 0)` delivers `&[]`; `(NULL, len > 0)`,
///   `len > isize::MAX`, or re-entrant invocation returns `Err(BytesSinkError::...)` instead of
///   silently delivering an empty reply.
#[inline]
pub fn with_bytes_sink<F, R>(
    on_bytes: F,
    call: impl FnOnce(NveBytesSink, *mut c_void) -> R,
) -> Result<R, BytesSinkError>
where
    F: FnMut(&[u8]),
{
    run_bytes_sink(true, on_bytes, call)
}

/// Fallible companion to [`with_bytes_sink`] flattening [`BytesSinkError`] into caller error `E`.
#[inline]
pub fn try_with_bytes_sink<F, R, E>(
    on_bytes: F,
    call: impl FnOnce(NveBytesSink, *mut c_void) -> Result<R, E>,
) -> Result<R, E>
where
    F: FnMut(&[u8]),
    E: From<BytesSinkError>,
{
    match with_bytes_sink(on_bytes, call) {
        Ok(inner) => inner,
        Err(violation) => Err(E::from(violation)),
    }
}

/// Single-shot companion to [`with_bytes_sink`] taking `FnOnce(&[u8])` and rejecting duplicate
/// foreign invocations with [`BytesSinkError::DuplicateCall`].
#[inline]
pub fn with_bytes_sink_once<F, R>(
    on_bytes: F,
    call: impl FnOnce(NveBytesSink, *mut c_void) -> R,
) -> Result<R, BytesSinkError>
where
    F: FnOnce(&[u8]),
{
    let mut slot = Some(on_bytes);
    run_bytes_sink(
        false,
        move |bytes| {
            if let Some(f) = slot.take() {
                f(bytes);
            }
        },
        call,
    )
}

/// Fallible single-shot companion to [`with_bytes_sink_once`] flattening [`BytesSinkError`] into
/// caller error `E`.
#[inline]
pub fn try_with_bytes_sink_once<F, R, E>(
    on_bytes: F,
    call: impl FnOnce(NveBytesSink, *mut c_void) -> Result<R, E>,
) -> Result<R, E>
where
    F: FnOnce(&[u8]),
    E: From<BytesSinkError>,
{
    match with_bytes_sink_once(on_bytes, call) {
        Ok(inner) => inner,
        Err(violation) => Err(E::from(violation)),
    }
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
