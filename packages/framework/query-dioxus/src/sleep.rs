//! Platform retry-backoff sleep abstraction (`SleepCallback`) with a zero-dependency shared
//! background timer thread on native and injectable test/browser sleep hooks.

use core::future::poll_fn;
use core::task::Poll;

use nve_query::Shared;
use nve_rpc::BoxFuture;

/// Async sleep callback used by [`crate::QueryClientProvider`] for exponential retry backoff.
#[cfg(not(target_arch = "wasm32"))]
pub type SleepCallback = Shared<dyn Fn(u32) -> BoxFuture<'static, ()> + Send + Sync>;

/// Async sleep callback used by [`crate::QueryClientProvider`] for exponential retry backoff.
#[cfg(target_arch = "wasm32")]
pub type SleepCallback = Shared<dyn Fn(u32) -> BoxFuture<'static, ()>>;

/// Cloneable, pointer-comparable handle wrapping a [`SleepCallback`] for use in Dioxus
/// `#[component]` props (e.g. [`crate::QueryClientProvider`]).
#[derive(Clone)]
pub struct SleepHandle(pub SleepCallback);

impl SleepHandle {
    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub fn new(f: impl Fn(u32) -> BoxFuture<'static, ()> + Send + Sync + 'static) -> Self {
        Self(Shared::new(f))
    }

    #[cfg(target_arch = "wasm32")]
    #[inline]
    pub fn new(f: impl Fn(u32) -> BoxFuture<'static, ()> + 'static) -> Self {
        Self(Shared::new(f))
    }

    #[inline]
    pub fn sleep_ms(&self, ms: u32) -> BoxFuture<'static, ()> {
        (self.0)(ms)
    }
}

impl PartialEq for SleepHandle {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Shared::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SleepHandle {}

/// Returns the default platform [`SleepCallback`]:
/// - On native targets: backed by a single lazily spawned shared `Condvar` timer thread (zero
///   `tokio` dependency, handles arbitrary concurrent retries with 1 thread total).
/// - On `wasm32`: falls back to yielding once unless a browser `setTimeout` callback (such as
///   `nve_http_web::browser_sleep_ms`) is passed to `QueryClientProvider::sleep`.
pub fn default_sleep_callback() -> SleepCallback {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Shared::new(native_sleep_ms)
    }
    #[cfg(target_arch = "wasm32")]
    {
        Shared::new(|_ms: u32| -> BoxFuture<'static, ()> { Box::pin(yield_once()) })
    }
}

/// Yields once to the active executor without depending on Tokio or `web-sys`.
pub(crate) async fn yield_once() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

#[cfg(not(target_arch = "wasm32"))]
fn native_sleep_ms(ms: u32) -> BoxFuture<'static, ()> {
    if ms == 0 {
        return Box::pin(yield_once());
    }
    use core::task::Waker;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    struct TimerEntry {
        done: Arc<AtomicBool>,
        waker: Arc<Mutex<Option<Waker>>>,
    }

    struct TimerWheel {
        state: Mutex<BTreeMap<(Instant, u64), TimerEntry>>,
        next_seq: std::sync::atomic::AtomicU64,
        cv: Condvar,
    }

    static WHEEL: OnceLock<Option<Arc<TimerWheel>>> = OnceLock::new();
    let Some(wheel) = WHEEL
        .get_or_init(|| {
            let wheel = Arc::new(TimerWheel {
                state: Mutex::new(BTreeMap::new()),
                next_seq: std::sync::atomic::AtomicU64::new(1),
                cv: Condvar::new(),
            });
            let bg = Arc::clone(&wheel);
            std::thread::Builder::new()
                .name("nve-query-timer".into())
                .spawn(move || {
                    let mut to_wake = Vec::new();
                    let mut guard = bg.state.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        let now = Instant::now();
                        while let Some((&(deadline, _), _)) = guard.first_key_value() {
                            if deadline > now {
                                break;
                            }
                            if let Some((_, entry)) = guard.pop_first() {
                                entry.done.store(true, Ordering::Release);
                                if let Some(w) = entry
                                    .waker
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .take()
                                {
                                    to_wake.push(w);
                                }
                            }
                        }
                        if !to_wake.is_empty() {
                            drop(guard);
                            for w in to_wake.drain(..) {
                                w.wake();
                            }
                            guard = bg.state.lock().unwrap_or_else(|e| e.into_inner());
                            continue;
                        }
                        if let Some((&(next_deadline, _), _)) = guard.first_key_value() {
                            let wait = next_deadline.saturating_duration_since(Instant::now());
                            let (g, _) = bg
                                .cv
                                .wait_timeout(guard, wait)
                                .unwrap_or_else(|e| e.into_inner());
                            guard = g;
                        } else {
                            guard = bg.cv.wait(guard).unwrap_or_else(|e| e.into_inner());
                        }
                    }
                })
                .ok()
                .map(|_| wheel)
        })
        .as_ref()
        .map(Arc::clone)
    else {
        return Box::pin(yield_once());
    };

    let deadline = Instant::now() + Duration::from_millis(u64::from(ms));
    let seq = wheel.next_seq.fetch_add(1, Ordering::Relaxed);
    let done = Arc::new(AtomicBool::new(false));
    let waker_slot: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
    {
        let mut guard = wheel.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(
            (deadline, seq),
            TimerEntry {
                done: Arc::clone(&done),
                waker: Arc::clone(&waker_slot),
            },
        );
        wheel.cv.notify_one();
    }

    struct NativeSleepFuture {
        wheel: Arc<TimerWheel>,
        key: (Instant, u64),
        done: Arc<AtomicBool>,
        waker: Arc<Mutex<Option<Waker>>>,
    }

    impl Drop for NativeSleepFuture {
        fn drop(&mut self) {
            if !self.done.load(Ordering::Acquire) {
                let mut guard = self.wheel.state.lock().unwrap_or_else(|e| e.into_inner());
                guard.remove(&self.key);
            }
        }
    }

    impl core::future::Future for NativeSleepFuture {
        type Output = ();

        fn poll(
            self: core::pin::Pin<&mut Self>,
            cx: &mut core::task::Context<'_>,
        ) -> Poll<()> {
            if self.done.load(Ordering::Acquire) {
                return Poll::Ready(());
            }
            *self.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
            if self.done.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }

    Box::pin(NativeSleepFuture {
        wheel,
        key: (deadline, seq),
        done,
        waker: waker_slot,
    })
}
