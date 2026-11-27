//! Core handles, shared ownership aliases (`Arc` native / `Rc` wasm32), `Clock`, `QueryOptions`,
//! `QuerySnapshot`, and zero-dependency identity hasher for pre-hashed `XXH64` keys.

use core::any::Any;
use core::hash::{BuildHasher, Hasher};
use core::sync::atomic::{AtomicU64, Ordering};

use nve_rpc::{MaybeSend, MaybeSync, RpcError};

use crate::slab::QuerySlotId;

/// Reference-counted shared pointer (`Arc<T>` on native, `Rc<T>` on `wasm32`).
#[cfg(not(target_arch = "wasm32"))]
pub type Shared<T> = std::sync::Arc<T>;

/// Reference-counted shared pointer (`Arc<T>` on native, `Rc<T>` on `wasm32`).
#[cfg(target_arch = "wasm32")]
pub type Shared<T> = std::rc::Rc<T>;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type SharedAny = std::sync::Arc<dyn Any + Send + Sync>;

#[cfg(target_arch = "wasm32")]
pub(crate) type SharedAny = std::rc::Rc<dyn Any>;

#[inline]
pub(crate) fn into_shared_any<T: MaybeSend + MaybeSync + 'static>(val: T) -> SharedAny {
    Shared::new(val)
}

#[inline]
pub(crate) fn downcast_shared<T: MaybeSend + MaybeSync + 'static>(
    any: &SharedAny,
) -> Option<Shared<T>> {
    Shared::clone(any).downcast::<T>().ok()
}

/// Generational handle identifying a live slot in [`crate::QueryClient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryHandle {
    pub slot: QuerySlotId,
    pub generation: u32,
}

/// Monotonic identifier for a single in-flight mutation / optimistic patch layer.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MutationId(pub u64);

/// Token returned by `begin_fetch` and passed back to `complete_fetch` to guard against stale
/// out-of-order responses, recycled slots, or wildcard invalidations that occurred mid-flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FetchTicket {
    pub handle: QueryHandle,
    pub fetch_gen: u32,
    pub wildcard_epoch: u32,
}

/// Operating mode of a [`crate::QueryClient`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QueryMode {
    /// Normal interactive client: subscribes, deduplicates, fetches, and refetches on invalidation.
    #[default]
    Client = 0,
    /// Server-side render read-only mode: reads prefetched/seeded cache slots synchronously and
    /// never triggers background network fetches during render.
    SsrReadOnly = 1,
}

/// Per-query freshness, GC, retry, and refetch options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryOptions {
    /// How long cached data remains fresh in milliseconds before background revalidation (`0` =
    /// always stale-while-revalidate on remount).
    pub stale_time_ms: u32,
    /// How long an inactive slot (`subscriber_count == 0`) stays cached before GC eviction
    /// (default: 5 minutes = `300_000` ms).
    pub gc_time_ms: u32,
    /// Maximum automatic retry attempts for transient query errors (`Network`, `Timeout`, `429`,
    /// `503`, `408`). Actions NEVER auto-retry. Default: `3`.
    pub max_retries: u8,
    /// Base exponential backoff delay in milliseconds (default: `250` ms, capped at 30 s).
    pub retry_base_delay_ms: u32,
    /// Refetch when stale/errored and the window regains focus (default: `true`).
    pub refetch_on_focus: bool,
    /// Refetch when stale/errored and network connectivity returns (default: `true`).
    pub refetch_on_reconnect: bool,
    /// When `true`, UI hooks keep showing the previous key's data while a new key loads.
    pub keep_previous_data: bool,
}

impl Default for QueryOptions {
    #[inline]
    fn default() -> Self {
        Self {
            stale_time_ms: 0,
            gc_time_ms: 300_000,
            max_retries: 3,
            retry_base_delay_ms: 250,
            refetch_on_focus: true,
            refetch_on_reconnect: true,
            keep_previous_data: false,
        }
    }
}

impl QueryOptions {
    #[inline]
    pub const fn stale_time_ms(mut self, ms: u32) -> Self {
        self.stale_time_ms = ms;
        self
    }

    #[inline]
    pub const fn gc_time_ms(mut self, ms: u32) -> Self {
        self.gc_time_ms = ms;
        self
    }

    #[inline]
    pub const fn max_retries(mut self, retries: u8) -> Self {
        self.max_retries = retries;
        self
    }

    #[inline]
    pub const fn retry_base_delay_ms(mut self, ms: u32) -> Self {
        self.retry_base_delay_ms = ms;
        self
    }

    #[inline]
    pub const fn refetch_on_focus(mut self, enabled: bool) -> Self {
        self.refetch_on_focus = enabled;
        self
    }

    #[inline]
    pub const fn refetch_on_reconnect(mut self, enabled: bool) -> Self {
        self.refetch_on_reconnect = enabled;
        self
    }

    #[inline]
    pub const fn keep_previous_data(mut self, enabled: bool) -> Self {
        self.keep_previous_data = enabled;
        self
    }

    /// Computes exponential retry backoff delay for `attempt` (`0`-based), capped at 30 seconds.
    #[inline]
    pub const fn retry_delay_ms(self, attempt: u8) -> u32 {
        let shift = if attempt > 10 { 10 } else { attempt };
        let base = self.retry_base_delay_ms as u64;
        let scaled = base.saturating_mul(1u64 << shift);
        if scaled > 30_000 {
            30_000
        } else {
            scaled as u32
        }
    }
}

/// Top-level configuration for [`crate::QueryClient`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryClientConfig {
    pub default_options: QueryOptions,
    /// Hard maximum cached query slots before evicting oldest inactive entries (default: `1024`).
    pub max_entries: usize,
    /// Grace window in milliseconds during which hydrated SSR entries remain fresh even when
    /// `QueryOptions::stale_time_ms == 0`, preventing an immediate duplicate fetch on mount while
    /// still expiring cleanly afterward (default: `5_000` ms).
    pub hydration_grace_ms: u32,
    /// Operating mode (`Client` vs `SsrReadOnly`).
    pub mode: QueryMode,
}

impl Default for QueryClientConfig {
    #[inline]
    fn default() -> Self {
        Self {
            default_options: QueryOptions::default(),
            max_entries: 1024,
            hydration_grace_ms: 5_000,
            mode: QueryMode::Client,
        }
    }
}

/// Point-in-time typed view of a query slot (zero data clones via [`Shared<T>`]).
#[derive(Debug, PartialEq, Eq)]
pub struct QuerySnapshot<T, E> {
    pub data: Option<Shared<T>>,
    pub error: Option<Shared<RpcError<E>>>,
    pub is_fetching: bool,
    pub is_stale: bool,
    pub is_invalidated: bool,
    pub data_version: u32,
    /// Monotonic invalidation identity (`(wildcard_epoch << 32) | fetch_gen`). Advances ONLY on
    /// tag/wildcard invalidation, `set_query_data`, or hydration — never on fetch start, error, or
    /// cancellation — allowing reactive hooks to trigger at most one refetch per invalidation.
    pub invalidation_version: u64,
    /// Monotonic committed-state sequence counter. Advances on every data or error commit (fetch,
    /// `set_query_data`, hydration) and never on cancellation or invalidation, allowing reactive
    /// hooks waiting on an external fetch to distinguish settled completion from cancellation even
    /// when `stale_time_ms == 0`.
    pub completed_seq: u32,
    /// Invalidation version (`(wildcard_epoch << 32) | fetch_gen`) answered by the most recent
    /// data or error commit (`0` before the first commit, or on evicted/mismatched handles).
    /// Allows reactive hooks to avoid re-fetching an invalidation already answered by an error
    /// commit while still refetching any invalidation that arrives afterward.
    pub completed_invalidation_version: u64,
}

impl<T, E> Clone for QuerySnapshot<T, E> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.as_ref().map(Shared::clone),
            error: self.error.as_ref().map(Shared::clone),
            is_fetching: self.is_fetching,
            is_stale: self.is_stale,
            is_invalidated: self.is_invalidated,
            data_version: self.data_version,
            invalidation_version: self.invalidation_version,
            completed_seq: self.completed_seq,
            completed_invalidation_version: self.completed_invalidation_version,
        }
    }
}

impl<T, E> QuerySnapshot<T, E> {
    /// True before the first successful value or error has arrived.
    #[inline]
    pub fn is_pending(&self) -> bool {
        self.data.is_none() && self.error.is_none()
    }

    #[inline]
    pub fn is_success(&self) -> bool {
        self.data.is_some()
    }

    #[inline]
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

/// Monotonic millisecond clock seam for freshness, GC, and deterministic tests.
pub trait Clock: MaybeSend + MaybeSync + 'static {
    fn now_ms(&self) -> u64;
}

/// Monotonic system clock backed by `std::time::Instant` on native targets.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct SystemClock {
    origin: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl SystemClock {
    #[inline]
    pub fn new() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for SystemClock {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Clock for SystemClock {
    #[inline]
    fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}


/// Deterministic controllable clock for zero-sleep unit & property tests.
#[derive(Debug, Default)]
pub struct ManualClock {
    now_ms: AtomicU64,
}

impl ManualClock {
    #[inline]
    pub const fn new(initial_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(initial_ms),
        }
    }

    #[inline]
    pub fn set(&self, ms: u64) {
        self.now_ms.store(ms, Ordering::Relaxed);
    }

    #[inline]
    pub fn advance(&self, delta_ms: u64) -> u64 {
        self.now_ms
            .fetch_add(delta_ms, Ordering::Relaxed)
            .saturating_add(delta_ms)
    }
}

impl Clock for ManualClock {
    #[inline]
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::Relaxed)
    }
}

/// Pass-through identity hasher for pre-hashed `XXH64` keys (`u64`, `Tag64`, `ProcId`).
#[derive(Default)]
pub(crate) struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.0 = nve_hash::xxh64(bytes, 0);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }
}

#[derive(Default, Clone, Copy)]
pub(crate) struct IdentityBuildHasher;

impl BuildHasher for IdentityBuildHasher {
    type Hasher = IdentityHasher;

    #[inline]
    fn build_hasher(&self) -> Self::Hasher {
        IdentityHasher(0)
    }
}
