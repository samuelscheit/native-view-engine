//! Coalesced wake bridge implementing `std::task::Wake` for async app tasks and background
//! network/pairing workers, with synchronized callback teardown so hosts can safely free wake
//! callback contexts immediately after `clear_sink()` / `set_wake_callback(NULL)` / `destroy()`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Wake, Waker};

/// Shared callback invoked when a surface needs the host event loop to schedule a tick.
///
/// # Re-entrancy Contract
/// Installed callbacks MUST be lightweight and strictly non-re-entrant (e.g. post a message or
/// signal a semaphore to the host event loop). They MUST NOT synchronously invoke `set_sink`,
/// `clear_sink`, `SurfaceRuntime::destroy`, or mutating runtime methods on the same thread.
pub type WakeCallback = Arc<dyn Fn() + Send + Sync>;

/// Coalesced cross-thread wake notifier shared by in-process async frontends (`DioxusRuntimeApp`),
/// background workers, and the C ABI `set_wake_callback` hook.
///
/// Multiple `trigger()` / `Waker::wake()` calls before [`Self::take_pending`] coalesce into a
/// single host callback invocation so worker bursts never flood the UI event loop.
///
/// [`Self::clear_sink`] and [`Self::set_sink`] acquire the internal write lock before mutating
/// the callback slot, blocking until any in-flight callback invocation on another thread finishes
/// so host `ctx` pointers are never accessed after clearing or surface destruction.
pub struct WakeBridge {
    pending: AtomicBool,
    defer_depth: AtomicUsize,
    deferred_wake: AtomicBool,
    sink: RwLock<Option<WakeCallback>>,
}

/// RAII deferral guard that postpones synchronous [`WakeCallback`] invocations while a surface
/// write lock is held, flushing one coalesced wake strictly after the outermost guard drops.
pub struct WakeDeferGuard<'a> {
    bridge: &'a WakeBridge,
}

impl Drop for WakeDeferGuard<'_> {
    fn drop(&mut self) {
        if self.bridge.defer_depth.fetch_sub(1, Ordering::SeqCst) == 1
            && !std::thread::panicking()
            && self.bridge.deferred_wake.swap(false, Ordering::SeqCst)
            && self.bridge.pending.load(Ordering::Acquire)
        {
            self.bridge.fire_sink();
        }
    }
}

/// RAII lifetime token paired with a [`Waker`] created by [`WakeBridge::detachable_waker`].
///
/// Dropping this token immediately disables the paired waker so zombie background futures belonging
/// to a replaced or dropped [`crate::RuntimeApp`] can never wake the surface.
pub struct WakeToken {
    alive: Arc<AtomicBool>,
}

impl Drop for WakeToken {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

struct DetachableWake {
    bridge: Arc<WakeBridge>,
    alive: Arc<AtomicBool>,
}

impl Wake for DetachableWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if self.alive.load(Ordering::Acquire) {
            self.bridge.trigger();
        }
    }
}

impl Default for WakeBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl WakeBridge {
    pub fn new() -> Self {
        let sink = RwLock::new(None);
        // Prime OS rwlock storage eagerly so steady-state wakes allocate 0 bytes.
        drop(sink.read());
        Self {
            pending: AtomicBool::new(false),
            defer_depth: AtomicUsize::new(0),
            deferred_wake: AtomicBool::new(false),
            sink,
        }
    }

    #[inline]
    fn fire_sink(&self) {
        let guard = self.sink.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref cb) = *guard {
            cb();
        }
    }

    /// Defers synchronous sink invocations until the returned [`WakeDeferGuard`] is dropped
    /// (typically declared *outside* a surface write lock so wakes flush after unlock).
    #[inline]
    pub fn defer_guard(&self) -> WakeDeferGuard<'_> {
        self.defer_depth.fetch_add(1, Ordering::SeqCst);
        WakeDeferGuard { bridge: self }
    }

    /// Creates a `std::task::Waker` backed directly by this `WakeBridge`.
    pub fn waker(self: &Arc<Self>) -> Waker {
        Waker::from(Arc::clone(self))
    }

    /// Creates a `std::task::Waker` gated by an RAII [`WakeToken`]; dropping the token renders
    /// all clones of that waker silent no-ops while still coalescing directly through this single
    /// `WakeBridge` while alive.
    pub fn detachable_waker(self: &Arc<Self>) -> (Waker, WakeToken) {
        let alive = Arc::new(AtomicBool::new(true));
        let waker = Waker::from(Arc::new(DetachableWake {
            bridge: Arc::clone(self),
            alive: Arc::clone(&alive),
        }));
        (waker, WakeToken { alive })
    }

    /// Installs or replaces the host wake sink, waiting for any in-flight invocation of a prior
    /// sink to return before replacing it (dropping any replaced sink *after* releasing the lock).
    /// If a wake was already pending, invokes the installed sink once under the shared read lock
    /// (or defers until any active [`Self::defer_guard`] exits).
    pub fn set_sink(&self, cb: WakeCallback) {
        let (replaced, fire_now) = {
            let mut guard = self.sink.write().unwrap_or_else(|e| e.into_inner());
            let old = guard.replace(cb);
            (old, self.pending.load(Ordering::Acquire))
        };
        drop(replaced);
        if fire_now {
            if self.defer_depth.load(Ordering::SeqCst) > 0 {
                self.deferred_wake.store(true, Ordering::SeqCst);
                if self.defer_depth.load(Ordering::SeqCst) == 0
                    && self.deferred_wake.swap(false, Ordering::SeqCst)
                    && self.pending.load(Ordering::Acquire)
                {
                    self.fire_sink();
                }
            } else {
                self.fire_sink();
            }
        }
    }

    /// Clears the installed host wake sink, waiting for any in-flight callback invocation on
    /// another thread to finish before returning (dropping the removed sink *after* unlocking).
    pub fn clear_sink(&self) {
        let removed = {
            let mut guard = self.sink.write().unwrap_or_else(|e| e.into_inner());
            guard.take()
        };
        drop(removed);
    }

    /// Marks the bridge dirty and fires the installed sink on the `false -> true` transition while
    /// holding the shared read lock so `clear_sink()`/`destroy()` cannot free host callback state
    /// mid-call. Defers sink invocation while any [`WakeDeferGuard`] is active.
    pub fn trigger(&self) {
        if !self.pending.swap(true, Ordering::SeqCst) {
            if self.defer_depth.load(Ordering::SeqCst) > 0 {
                self.deferred_wake.store(true, Ordering::SeqCst);
                if self.defer_depth.load(Ordering::SeqCst) == 0
                    && self.deferred_wake.swap(false, Ordering::SeqCst)
                    && self.pending.load(Ordering::Acquire)
                {
                    self.fire_sink();
                }
            } else {
                self.fire_sink();
            }
        }
    }

    /// Returns whether a wake is currently pending without clearing the latch.
    #[inline]
    pub fn is_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    /// Clears and returns the coalesced pending flag after acquiring the surface write lock.
    #[inline]
    pub fn take_pending(&self) -> bool {
        self.deferred_wake.store(false, Ordering::SeqCst);
        self.pending.swap(false, Ordering::SeqCst)
    }
}

impl Wake for WakeBridge {
    fn wake(self: Arc<Self>) {
        self.trigger();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.trigger();
    }
}
