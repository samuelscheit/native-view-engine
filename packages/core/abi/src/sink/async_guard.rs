//! Once-only, panic-catching Rust receiver guard for [`NveAsyncBytesDone`] completions.

use core::any::Any;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, Ordering};
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

/// Low 3 bits encode the lifecycle phase; bit 3 encodes whether [`AsyncBytesDoneGuard`] has
/// already been dropped (handing off deallocation responsibility to an in-flight trampoline).
const PHASE_MASK: u8 = 0b0111;
const PHASE_PENDING: u8 = 0;
const PHASE_RUNNING: u8 = 1;
const PHASE_DONE_OK: u8 = 2;
const PHASE_DONE_PANICKED: u8 = 3;
const PHASE_CANCELLED: u8 = 4;
const FLAG_GUARD_DROPPED: u8 = 0b1000;

struct AsyncBytesDoneState<F> {
    state: AtomicU8,
    callback: UnsafeCell<Option<F>>,
    panic: UnsafeCell<Option<Box<dyn Any + Send>>>,
}

// SAFETY: `callback` is moved out by at most one thread winning the atomic `PENDING -> RUNNING`
// or `PENDING -> CANCELLED` CAS; `panic` is written only under `PHASE_RUNNING` before the
// `AcqRel` transition to `PHASE_DONE_PANICKED` and read only after `Acquire` load of
// `PHASE_DONE_PANICKED` (or exclusively during final deallocation).
unsafe impl<F: Send> Send for AsyncBytesDoneState<F> {}

// SAFETY: Concurrent shared references only access atomic `state` or claim exclusive access to
// `callback`/`panic` via the single-winner `PENDING -> RUNNING`/`CANCELLED` CAS, so `F: Send`
// suffices for `Sync`.
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
    let raw_ptr = user_ctx as *mut AsyncBytesDoneState<F>;

    let must_free = {
        // SAFETY: Caller keeps `AsyncBytesDoneGuard` alive until `done` starts; once we win
        // `PHASE_PENDING -> PHASE_RUNNING`, `AsyncBytesDoneGuard::drop` will not free `raw_ptr`
        // (it sets `FLAG_GUARD_DROPPED` and defers deallocation to us).
        let inner = unsafe { &*raw_ptr };
        if inner
            .state
            .compare_exchange(
                PHASE_PENDING,
                PHASE_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }

        // SAFETY: Winning `PHASE_PENDING -> PHASE_RUNNING` grants exclusive access to `callback`.
        let callback = unsafe { (*inner.callback.get()).take() };
        // SAFETY: Foreign callee guarantees `(ptr, len)` satisfies `sink_bytes` preconditions for
        // the duration of this callback when structurally valid.
        let payload = unsafe { sink_bytes(ptr, len) }.ok_or(BytesSinkError::CorruptSlice { len });

        let terminal_phase = if let Some(on_done) = callback {
            match catch_unwind(AssertUnwindSafe(|| on_done(request_id, status, payload))) {
                Ok(()) => PHASE_DONE_OK,
                Err(panic_payload) => {
                    // SAFETY: Exclusive writer under `PHASE_RUNNING`; published via the `AcqRel`
                    // transition to `PHASE_DONE_PANICKED` below.
                    unsafe {
                        *inner.panic.get() = Some(panic_payload);
                    }
                    PHASE_DONE_PANICKED
                }
            }
        } else {
            PHASE_DONE_OK
        };

        // Publish terminal phase while atomically observing whether `AsyncBytesDoneGuard::drop`
        // ran during `PHASE_RUNNING` (synchronizing with `Drop`'s `Release` store).
        let mut cur = inner.state.load(Ordering::Acquire);
        loop {
            debug_assert_eq!(cur & PHASE_MASK, PHASE_RUNNING);
            let next = terminal_phase | (cur & FLAG_GUARD_DROPPED);
            match inner.state.compare_exchange_weak(
                cur,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break (cur & FLAG_GUARD_DROPPED) != 0,
                Err(actual) => cur = actual,
            }
        }
    };

    if must_free {
        // Guard already dropped during `PHASE_RUNNING` and handed off deallocation to us.
        // Wrap in `catch_unwind` so even a custom panic payload with a panicking `Drop` cannot
        // unwind across `extern "C"`.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: `raw_ptr` came from `Box::into_raw`; `FLAG_GUARD_DROPPED` observed with
            // `Acquire` guarantees no `AsyncBytesDoneGuard` or `&AsyncBytesDoneState` remains.
            drop(unsafe { Box::from_raw(raw_ptr) });
        }));
    }
}

/// Reusable, once-only, panic-catching Rust receiver guard for [`NveAsyncBytesDone`].
///
/// - **Exactly-once delivery**: concurrent or duplicate foreign invocations are ignored cleanly
///   with zero use-after-free while the guard stays alive.
/// - **Drop-during-callback safe**: dropping the guard synchronously inside `on_done` or from a
///   thread woken by `on_done` atomically defers deallocation until the trampoline exits, avoiding
///   use-after-free.
/// - **Explicit corrupt-slice surfacing**: delivers `Err(BytesSinkError::CorruptSlice { len })`
///   whenever `sink_bytes(ptr, len)` rejects a corrupt `(ptr, len)` pair (including `status == 0`
///   with `(NULL, len > 0)` or `len > isize::MAX`).
/// - **Panic-safe across `extern "C"`**: catches any panic inside the callback inside the C
///   trampoline instead of unwinding across FFI; inspectable via [`Self::panicked`] /
///   [`Self::take_panic`] over a single atomic terminal state.
/// - **Move-safe raw pointer**: heap-pinned via `NonNull`, so moving `AsyncBytesDoneGuard` does
///   not invalidate the raw `user_ctx` pointer.
pub struct AsyncBytesDoneGuard<F>
where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send,
{
    raw: NonNull<AsyncBytesDoneState<F>>,
}

// SAFETY: `AsyncBytesDoneGuard` uniquely owns the guard side of `AsyncBytesDoneState<F>`, which
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
            state: AtomicU8::new(PHASE_PENDING),
            callback: UnsafeCell::new(Some(on_done)),
            panic: UnsafeCell::new(None),
        });
        // SAFETY: `Box::into_raw` always returns a non-null pointer.
        let raw = unsafe { NonNull::new_unchecked(Box::into_raw(boxed)) };
        Self { raw }
    }

    #[inline]
    fn inner(&self) -> &AsyncBytesDoneState<F> {
        // SAFETY: `self.raw` remains live until `Drop` (and while `FLAG_GUARD_DROPPED == 0`,
        // the trampoline never frees it).
        unsafe { self.raw.as_ref() }
    }

    /// Returns the C-ABI `(NveAsyncBytesDone, *mut c_void)` pair for a foreign async call.
    ///
    /// The caller must ensure no foreign code starts invoking the returned pair *after* this
    /// guard has been dropped (dropping the guard while an invocation is already running is safe
    /// and hands deallocation off to the in-flight trampoline).
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

    /// Returns `true` once the foreign callback has finished executing (normally or with panic).
    #[must_use]
    #[inline]
    pub fn is_completed(&self) -> bool {
        matches!(
            self.inner().state.load(Ordering::Acquire) & PHASE_MASK,
            PHASE_DONE_OK | PHASE_DONE_PANICKED
        )
    }

    /// Returns `true` iff the trampoline reached the terminal panicked state (guaranteeing
    /// [`Self::take_panic`] observes the stored panic payload until taken).
    #[must_use]
    #[inline]
    pub fn panicked(&self) -> bool {
        (self.inner().state.load(Ordering::Acquire) & PHASE_MASK) == PHASE_DONE_PANICKED
    }

    /// Claims the slot and drops the pending callback if it has not yet started. Returns `true`
    /// iff this call cancelled a pending callback.
    pub fn cancel(&self) -> bool {
        let inner = self.inner();
        if inner
            .state
            .compare_exchange(
                PHASE_PENDING,
                PHASE_CANCELLED,
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

    /// Takes any panic payload caught inside the C trampoline after reaching the terminal
    /// panicked state.
    pub fn take_panic(&mut self) -> Option<Box<dyn Any + Send>> {
        let inner = self.inner();
        if (inner.state.load(Ordering::Acquire) & PHASE_MASK) != PHASE_DONE_PANICKED {
            return None;
        }
        // SAFETY: `&mut self` guarantees exclusive guard access, and `Acquire` load of
        // `PHASE_DONE_PANICKED` synchronizes-with the trampoline's `AcqRel` store after writing
        // `panic` (and the trampoline never touches `panic` again).
        unsafe { (*inner.panic.get()).take() }
    }
}

impl<F> Drop for AsyncBytesDoneGuard<F>
where
    F: FnOnce(u64, i32, Result<&[u8], BytesSinkError>) + Send,
{
    fn drop(&mut self) {
        let raw_ptr = self.raw.as_ptr();
        let should_free = {
            // SAFETY: `raw_ptr` remains live until we either free it below or hand it off to an
            // in-flight trampoline by setting `FLAG_GUARD_DROPPED`.
            let inner = unsafe { &*raw_ptr };
            let mut cur = inner.state.load(Ordering::Acquire);
            loop {
                match cur & PHASE_MASK {
                    PHASE_PENDING => match inner.state.compare_exchange_weak(
                        cur,
                        PHASE_CANCELLED | FLAG_GUARD_DROPPED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            // SAFETY: Winning `PENDING -> CANCELLED` guarantees exclusive access
                            // to `callback` and prevents any trampoline from entering `RUNNING`.
                            drop(unsafe { (*inner.callback.get()).take() });
                            break true;
                        }
                        Err(actual) => cur = actual,
                    },
                    PHASE_RUNNING => match inner.state.compare_exchange_weak(
                        cur,
                        PHASE_RUNNING | FLAG_GUARD_DROPPED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => break false,
                        Err(actual) => cur = actual,
                    },
                    PHASE_DONE_OK | PHASE_DONE_PANICKED | PHASE_CANCELLED => break true,
                    _ => break true,
                }
            }
        };
        if should_free {
            // SAFETY: Either we won `PENDING -> CANCELLED` or observed a terminal phase with
            // `Acquire` while `FLAG_GUARD_DROPPED == 0`, so we are the sole remaining owner.
            drop(unsafe { Box::from_raw(raw_ptr) });
        }
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
