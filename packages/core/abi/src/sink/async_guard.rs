//! Once-only, panic-catching Rust receiver guard for [`NveAsyncBytesDone`] completions.

use core::any::Any;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::panic::{catch_unwind, AssertUnwindSafe};

use super::{sink_bytes, BytesSinkError};

/// Asynchronous C-ABI completion callback delivering `(request_id, status, ptr, len)` at most once.
///
/// # FFI Contract
/// - May be invoked synchronously or from another thread while the receiver's `user_ctx` remains
///   live.
/// - Pass `(NULL, 0)` for empty payloads or error statuses without detail bytes; when `len > 0`,
///   `ptr` must point to a single allocated object of at least `len` initialized, immutable bytes
///   for the duration of the callback (`len <= isize::MAX`).
pub type NveAsyncBytesDone = unsafe extern "C" fn(
    user_ctx: *mut c_void,
    request_id: u64,
    status: i32,
    ptr: *const u8,
    len: usize,
);

const ASYNC_STATE_PENDING: u8 = 0;
const ASYNC_STATE_RUNNING: u8 = 1;
const ASYNC_STATE_DONE: u8 = 2;
const ASYNC_STATE_CANCELLED: u8 = 3;

struct AsyncBytesDoneState<F> {
    state: AtomicU8,
    panicked: AtomicBool,
    callback: UnsafeCell<Option<F>>,
    panic: UnsafeCell<Option<Box<dyn Any + Send>>>,
}

// SAFETY: `callback` is moved out by at most one thread winning the atomic `PENDING -> RUNNING`
// or `PENDING -> CANCELLED` CAS, and `panic` is written only under `STATE_RUNNING` before the
// `Release` store of `STATE_DONE` and read only after `Acquire` load of `STATE_DONE` (or in Drop).
unsafe impl<F: Send> Send for AsyncBytesDoneState<F> {}

// SAFETY: Concurrent shared references only access atomic state (`state`, `panicked`) or claim
// exclusive access to `callback`/`panic` via the single-winner `PENDING -> RUNNING`/`CANCELLED`
// CAS, so `F: Send` suffices for `Sync`.
unsafe impl<F: Send> Sync for AsyncBytesDoneState<F> {}

unsafe extern "C" fn async_bytes_done_trampoline<F>(
    user_ctx: *mut c_void,
    request_id: u64,
    status: i32,
    ptr: *const u8,
    len: usize,
) where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send,
{
    if user_ctx.is_null() {
        return;
    }
    // SAFETY: Caller keeps `AsyncBytesDoneGuard` alive while the foreign callee may invoke this
    // callback; `raw` is heap-pinned via `Box::into_raw` and shared immutably via atomics.
    let inner = unsafe { &*(user_ctx as *const AsyncBytesDoneState<F>) };
    if inner
        .state
        .compare_exchange(
            ASYNC_STATE_PENDING,
            ASYNC_STATE_RUNNING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }

    // SAFETY: This thread exclusively owns `STATE_RUNNING`, so no other thread can access
    // `callback` or `panic`.
    let callback = unsafe { (*inner.callback.get()).take() };
    // SAFETY: Foreign callee guarantees `(ptr, len)` satisfies `sink_bytes` preconditions for the
    // duration of this callback when structurally valid.
    let payload = unsafe { sink_bytes(ptr, len) }.ok_or(BytesSinkError::CorruptSlice { len });

    if let Some(on_done) = callback
        && let Err(payload) = catch_unwind(AssertUnwindSafe(|| on_done(request_id, status, payload)))
    {
        // SAFETY: Exclusive access under `STATE_RUNNING`, published by `STATE_DONE` `Release`.
        unsafe {
            *inner.panic.get() = Some(payload);
        }
        inner.panicked.store(true, Ordering::Release);
    }
    inner.state.store(ASYNC_STATE_DONE, Ordering::Release);
}

/// Reusable, once-only, panic-catching Rust receiver guard for [`NveAsyncBytesDone`].
///
/// - **Exactly-once delivery**: concurrent or duplicate foreign invocations are ignored cleanly
///   with zero use-after-free while the guard stays alive.
/// - **Explicit corrupt-slice surfacing**: delivers `Err(BytesSinkError::CorruptSlice { len })`
///   whenever `sink_bytes(ptr, len)` rejects a corrupt `(ptr, len)` pair (including `status == 0`
///   with `(NULL, len > 0)` or `len > isize::MAX`).
/// - **Panic-safe across `extern "C"`**: catches any panic inside the callback inside the C
///   trampoline instead of unwinding across FFI; inspectable via [`Self::panicked`] /
///   [`Self::take_panic`].
/// - **Move-safe raw pointer**: heap-pinned via `NonNull`, so moving `AsyncBytesDoneGuard` does
///   not invalidate the raw `user_ctx` pointer.
pub struct AsyncBytesDoneGuard<F>
where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send,
{
    raw: NonNull<AsyncBytesDoneState<F>>,
}

// SAFETY: `AsyncBytesDoneGuard` uniquely owns the heap-allocated `AsyncBytesDoneState<F>`, which
// is `Send` whenever `F: Send`.
unsafe impl<F> Send for AsyncBytesDoneGuard<F> where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send
{
}

// SAFETY: Shared references to `AsyncBytesDoneGuard` only perform atomic reads or single-winner
// CAS cancellation on `AsyncBytesDoneState<F>`, which is `Sync` whenever `F: Send`.
unsafe impl<F> Sync for AsyncBytesDoneGuard<F> where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send
{
}

impl<F> AsyncBytesDoneGuard<F>
where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send,
{
    /// Allocates a pinned once-only guard wrapping `on_done`.
    #[must_use]
    pub fn new(on_done: F) -> Self {
        let boxed = Box::new(AsyncBytesDoneState {
            state: AtomicU8::new(ASYNC_STATE_PENDING),
            panicked: AtomicBool::new(false),
            callback: UnsafeCell::new(Some(on_done)),
            panic: UnsafeCell::new(None),
        });
        // SAFETY: `Box::into_raw` always returns a non-null pointer.
        let raw = unsafe { NonNull::new_unchecked(Box::into_raw(boxed)) };
        Self { raw }
    }

    #[inline]
    fn inner(&self) -> &AsyncBytesDoneState<F> {
        // SAFETY: `self.raw` remains live until `Drop`.
        unsafe { self.raw.as_ref() }
    }

    /// Returns the C-ABI `(NveAsyncBytesDone, *mut c_void)` pair for a foreign async call.
    ///
    /// The caller must keep this guard alive until no foreign code can invoke the returned pair.
    #[must_use]
    #[inline]
    pub fn as_raw(&self) -> (NveAsyncBytesDone, *mut c_void) {
        (self.callback(), self.user_ctx())
    }

    /// Returns the panic-catching C trampoline function pointer.
    #[must_use]
    #[inline]
    pub fn callback(&self) -> NveAsyncBytesDone {
        async_bytes_done_trampoline::<F>
    }

    /// Returns the opaque `user_ctx` pointer paired with [`Self::callback`].
    #[must_use]
    #[inline]
    pub fn user_ctx(&self) -> *mut c_void {
        self.raw.as_ptr().cast::<c_void>()
    }

    /// Returns `true` once the foreign callback has finished executing.
    #[must_use]
    #[inline]
    pub fn is_completed(&self) -> bool {
        self.inner().state.load(Ordering::Acquire) == ASYNC_STATE_DONE
    }

    /// Returns `true` if the Rust completion closure panicked inside the C trampoline.
    #[must_use]
    #[inline]
    pub fn panicked(&self) -> bool {
        self.inner().panicked.load(Ordering::Acquire)
    }

    /// Claims the slot and drops the pending callback if it has not yet started. Returns `true`
    /// iff this call cancelled a pending callback.
    pub fn cancel(&self) -> bool {
        let inner = self.inner();
        if inner
            .state
            .compare_exchange(
                ASYNC_STATE_PENDING,
                ASYNC_STATE_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            // SAFETY: Winning `PENDING -> CANCELLED` guarantees exclusive access to `callback`.
            drop(unsafe { (*inner.callback.get()).take() });
            true
        } else {
            false
        }
    }

    /// Takes any panic payload caught inside the C trampoline after completion.
    pub fn take_panic(&mut self) -> Option<Box<dyn Any + Send>> {
        let inner = self.inner();
        if inner.state.load(Ordering::Acquire) != ASYNC_STATE_DONE {
            return None;
        }
        // SAFETY: `&mut self` + `Acquire` load of `STATE_DONE` guarantees the trampoline has
        // finished writing `panic` and will never touch it again.
        unsafe { (*inner.panic.get()).take() }
    }
}

impl<F> Drop for AsyncBytesDoneGuard<F>
where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send,
{
    fn drop(&mut self) {
        let _ = self.cancel();
        // SAFETY: `self.raw` was allocated via `Box::into_raw` in `Self::new`, and by contract no
        // foreign invocation accesses `user_ctx` once `self` is dropped.
        drop(unsafe { Box::from_raw(self.raw.as_ptr()) });
    }
}

/// Convenience constructor returning a once-only, panic-catching [`AsyncBytesDoneGuard`] for
/// [`NveAsyncBytesDone`].
#[must_use]
#[inline]
pub fn into_async_bytes_done<F>(on_done: F) -> AsyncBytesDoneGuard<F>
where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send,
{
    AsyncBytesDoneGuard::new(on_done)
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
