//! Coalesced lock-minimal wake bridge implementing `std::task::Wake` for async app tasks and
//! background network/pairing workers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};

/// Shared callback invoked when a surface needs the host event loop to schedule a tick.
pub type WakeCallback = Arc<dyn Fn() + Send + Sync>;

/// Coalesced cross-thread wake notifier shared by in-process async frontends (`DioxusRuntimeApp`),
/// background workers, and the C ABI `set_wake_callback` hook.
///
/// Multiple `trigger()` / `Waker::wake()` calls before [`Self::take_pending`] coalesce into a
/// single host callback invocation so worker bursts never flood the UI event loop.
pub struct WakeBridge {
    pending: AtomicBool,
    sink: Mutex<Option<WakeCallback>>,
}

impl Default for WakeBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl WakeBridge {
    pub fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            sink: Mutex::new(None),
        }
    }

    /// Creates a `std::task::Waker` backed by this `WakeBridge`.
    pub fn waker(self: &Arc<Self>) -> Waker {
        Waker::from(Arc::clone(self))
    }

    /// Installs or replaces the host wake sink. If a wake arrived before the sink was installed,
    /// invokes `cb` immediately once.
    pub fn set_sink(&self, cb: WakeCallback) {
        let fire_now = {
            let mut guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(Arc::clone(&cb));
            self.pending.load(Ordering::Acquire)
        };
        if fire_now {
            cb();
        }
    }

    /// Clears the installed host wake sink.
    pub fn clear_sink(&self) {
        let mut guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    /// Marks the bridge dirty and fires the installed sink on the `false -> true` transition.
    pub fn trigger(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            let sink = self.sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(cb) = sink {
                cb();
            }
        }
    }

    /// Returns whether a wake is currently pending without clearing the latch.
    #[inline]
    pub fn is_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    /// Clears and returns the coalesced pending flag at the start of a host tick.
    #[inline]
    pub fn take_pending(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel)
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
