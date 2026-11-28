//! Generational `OwnerBinding` handle table, re-entrancy-safe `CallbacksSlot` cross-thread drain,
//! and bounded `CallbackRetryIds` single-execution retry mapping for [`super::RuntimeHostRouter`].

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, RwLock};

use nve_abi::NveHostCallbacks;

use super::HostRouterShared;

#[cfg(target_pointer_width = "64")]
const SLOT_BITS: u32 = 20;
#[cfg(not(target_pointer_width = "64"))]
const SLOT_BITS: u32 = 14;

const SLOT_MASK: usize = (1usize << SLOT_BITS) - 1;
const GEN_MASK: usize = usize::MAX >> SLOT_BITS;

struct BindingSlot {
    generation: usize,
    binding: Option<Arc<OwnerBinding>>,
}

static OWNER_BINDINGS: LazyLock<RwLock<Vec<BindingSlot>>> = LazyLock::new(|| {
    let lock = RwLock::new(Vec::new());
    // Prime OS rwlock storage eagerly so hot-path lookups allocate 0 bytes.
    drop(lock.read());
    lock
});

/// Per-attach owner binding stored in the generational [`OWNER_BINDINGS`] table.
///
/// `NveHostCallbacks::host_ctx` carries only the opaque non-zero generational `token` (never a raw
/// pointer to heap memory), so even a stale worker thread invoking a copied `NveHostCallbacks`
/// after `detach_app` or `SurfaceRuntime::drop` can never cause use-after-free.
pub(crate) struct OwnerBinding {
    pub(super) owner_tag: u64,
    token: usize,
    pub(super) active: AtomicBool,
    pub(super) frame_requested: AtomicBool,
    pub(super) shared: Arc<HostRouterShared>,
}

/// Owned handle to one registered [`OwnerBinding`]; dropping it deactivates and unregisters the
/// token even during stack unwinding.
pub(crate) struct OwnerBindingHandle {
    inner: Arc<OwnerBinding>,
}

impl OwnerBindingHandle {
    pub(super) fn register(owner_tag: u64, shared: Arc<HostRouterShared>) -> Self {
        let mut table = OWNER_BINDINGS.write().unwrap_or_else(|e| e.into_inner());
        let idx = if let Some(free_idx) = table.iter().position(|s| s.binding.is_none()) {
            free_idx
        } else {
            let next_idx = table.len();
            assert!(next_idx <= SLOT_MASK, "RuntimeHostRouter: live owner binding table exhausted");
            table.push(BindingSlot {
                generation: 0,
                binding: None,
            });
            next_idx
        };
        let slot = &mut table[idx];
        let mut next_gen = slot.generation.wrapping_add(1) & GEN_MASK;
        if next_gen == 0 {
            next_gen = 1;
        }
        slot.generation = next_gen;
        let token = (next_gen << SLOT_BITS) | idx;
        let binding = Arc::new(OwnerBinding {
            owner_tag,
            token,
            active: AtomicBool::new(true),
            frame_requested: AtomicBool::new(false),
            shared,
        });
        slot.binding = Some(Arc::clone(&binding));
        Self { inner: binding }
    }

    #[inline]
    pub(crate) fn owner_tag(&self) -> u64 {
        self.inner.owner_tag
    }

    #[inline]
    pub(crate) fn take_frame_requested(&self) -> bool {
        self.inner.frame_requested.swap(false, Ordering::AcqRel)
    }

    /// Marks the binding inactive and removes its entry from [`OWNER_BINDINGS`] so no future
    /// FFI calls can resolve it (idempotent).
    pub(crate) fn deactivate(&self) {
        self.inner.active.store(false, Ordering::Release);
        unregister_owner_binding(self.inner.token);
    }

    /// Returns the C-ABI [`NveHostCallbacks`] carrying the opaque non-zero generational token.
    pub(crate) fn callbacks(&self) -> NveHostCallbacks {
        NveHostCallbacks {
            host_ctx: std::ptr::without_provenance_mut(self.inner.token),
            invoke: Some(super::router_invoke),
            channel_publish: Some(super::router_channel_publish),
        }
    }
}

impl Drop for OwnerBindingHandle {
    fn drop(&mut self) {
        self.deactivate();
    }
}

#[inline]
pub(super) fn lookup_owner_binding(host_ctx: *mut c_void) -> Option<Arc<OwnerBinding>> {
    let token = host_ctx as usize;
    if token == 0 {
        return None;
    }
    let idx = token & SLOT_MASK;
    let generation = token >> SLOT_BITS;
    if generation == 0 {
        return None;
    }
    let guard = OWNER_BINDINGS.read().unwrap_or_else(|e| e.into_inner());
    let slot = guard.get(idx)?;
    if slot.generation != generation {
        return None;
    }
    slot.binding.as_ref().map(Arc::clone)
}

fn unregister_owner_binding(token: usize) {
    let idx = token & SLOT_MASK;
    let generation = token >> SLOT_BITS;
    let removed = {
        let mut guard = OWNER_BINDINGS.write().unwrap_or_else(|e| e.into_inner());
        match guard.get_mut(idx) {
            Some(slot) if slot.generation == generation => slot.binding.take(),
            _ => None,
        }
    };
    drop(removed);
}

const TLS_INLINE_SLOTS: usize = 8;

struct ActiveThreadSlots {
    inline: [(usize, usize); TLS_INLINE_SLOTS],
    overflow: Vec<(usize, usize)>,
}

impl ActiveThreadSlots {
    const fn new() -> Self {
        Self {
            inline: [(0, 0); TLS_INLINE_SLOTS],
            overflow: Vec::new(),
        }
    }

    fn inc(&mut self, addr: usize) {
        let mut free: Option<usize> = None;
        for (i, entry) in self.inline.iter_mut().enumerate() {
            if entry.0 == addr {
                entry.1 += 1;
                return;
            }
            if entry.1 == 0 && free.is_none() {
                free = Some(i);
            }
        }
        if let Some(entry) = self.overflow.iter_mut().find(|e| e.0 == addr) {
            entry.1 += 1;
            return;
        }
        if let Some(i) = free {
            self.inline[i] = (addr, 1);
        } else {
            self.overflow.push((addr, 1));
        }
    }

    fn dec(&mut self, addr: usize) {
        for entry in &mut self.inline {
            if entry.0 == addr && entry.1 > 0 {
                entry.1 -= 1;
                if entry.1 == 0 {
                    entry.0 = 0;
                }
                return;
            }
        }
        if let Some(pos) = self.overflow.iter().position(|e| e.0 == addr) {
            if self.overflow[pos].1 > 1 {
                self.overflow[pos].1 -= 1;
            } else {
                self.overflow.swap_remove(pos);
            }
        }
    }

    fn count(&self, addr: usize) -> usize {
        for entry in &self.inline {
            if entry.0 == addr {
                return entry.1;
            }
        }
        self.overflow
            .iter()
            .find(|e| e.0 == addr)
            .map_or(0, |e| e.1)
    }
}

thread_local! {
    static ACTIVE_CB_SLOTS: RefCell<ActiveThreadSlots> = const { RefCell::new(ActiveThreadSlots::new()) };
}

fn current_thread_slot_count(addr: usize) -> usize {
    ACTIVE_CB_SLOTS
        .try_with(|cell| cell.borrow().count(addr))
        .unwrap_or(0)
}

/// Ref-counted host C-callback slot supporting lock-free hot entry, same-thread re-entrant
/// `set_callbacks` replacement without self-deadlock, and synchronous cross-thread drain on
/// `set_callbacks(None)` / `destroy` so hosts can safely free `callbacks.host_ctx`.
pub(super) struct CallbacksSlot {
    callbacks: NveHostCallbacks,
    in_flight: AtomicUsize,
    waiters: AtomicUsize,
    drain_mu: Mutex<()>,
    drain_cv: Condvar,
}

impl CallbacksSlot {
    pub(super) fn new(callbacks: NveHostCallbacks) -> Arc<Self> {
        Arc::new(Self {
            callbacks,
            in_flight: AtomicUsize::new(0),
            waiters: AtomicUsize::new(0),
            drain_mu: Mutex::new(()),
            drain_cv: Condvar::new(),
        })
    }

    /// Registers an active call on this slot BEFORE releasing `backend.read()`.
    #[inline]
    pub(super) fn enter(self: &Arc<Self>) -> SlotCallGuard {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        let addr = Arc::as_ptr(self) as usize;
        let tls_tracked = ACTIVE_CB_SLOTS
            .try_with(|cell| cell.borrow_mut().inc(addr))
            .is_ok();
        SlotCallGuard {
            slot: Arc::clone(self),
            addr,
            tls_tracked,
        }
    }

    /// Waits for all OTHER threads currently executing callbacks on this slot to return, while
    /// returning immediately for any calls active on the current thread (avoiding self-deadlock).
    pub(super) fn drain_other_threads(&self) {
        let addr = std::ptr::from_ref(self) as usize;
        let mine = current_thread_slot_count(addr);
        if self.in_flight.load(Ordering::SeqCst) <= mine {
            return;
        }
        let mut guard = self.drain_mu.lock().unwrap_or_else(|e| e.into_inner());
        self.waiters.fetch_add(1, Ordering::SeqCst);
        while self.in_flight.load(Ordering::SeqCst) > mine {
            guard = self.drain_cv.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        self.waiters.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) struct SlotCallGuard {
    slot: Arc<CallbacksSlot>,
    addr: usize,
    tls_tracked: bool,
}

impl SlotCallGuard {
    #[inline]
    pub(super) fn callbacks(&self) -> NveHostCallbacks {
        self.slot.callbacks
    }
}

impl Drop for SlotCallGuard {
    fn drop(&mut self) {
        if self.tls_tracked {
            let _ = ACTIVE_CB_SLOTS.try_with(|cell| cell.borrow_mut().dec(self.addr));
        }
        self.slot.in_flight.fetch_sub(1, Ordering::SeqCst);
        if self.slot.waiters.load(Ordering::SeqCst) > 0 {
            let _g = self.slot.drain_mu.lock().unwrap_or_else(|e| e.into_inner());
            self.slot.drain_cv.notify_all();
        }
    }
}

const MAX_CALLBACK_RETRY_IDS: usize = 16;

struct CallbackRetryEntry {
    owner_tag: u64,
    caller_request_id: u64,
    service_id: u32,
    method_id: u32,
    host_request_id: u64,
}

/// Bounded lock-free-checked mapping preserving `host_request_id` across C-callback
/// `BufferTooSmall` retries so caching C hosts (e.g. `HostCallbackBridge`) never execute twice.
pub(super) struct CallbackRetryIds {
    has_entries: AtomicBool,
    entries: Mutex<VecDeque<CallbackRetryEntry>>,
}

impl CallbackRetryIds {
    pub(super) fn new() -> Self {
        let entries = Mutex::new(VecDeque::new());
        drop(entries.lock());
        Self {
            has_entries: AtomicBool::new(false),
            entries,
        }
    }

    #[inline]
    pub(super) fn take(
        &self,
        owner_tag: u64,
        caller_request_id: u64,
        service_id: u32,
        method_id: u32,
    ) -> Option<u64> {
        if !self.has_entries.load(Ordering::Acquire) {
            return None;
        }
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let pos = guard.iter().position(|e| {
            e.owner_tag == owner_tag
                && e.caller_request_id == caller_request_id
                && e.service_id == service_id
                && e.method_id == method_id
        })?;
        let entry = guard.remove(pos)?;
        if guard.is_empty() {
            self.has_entries.store(false, Ordering::Release);
        }
        Some(entry.host_request_id)
    }

    pub(super) fn store(
        &self,
        owner_tag: u64,
        caller_request_id: u64,
        service_id: u32,
        method_id: u32,
        host_request_id: u64,
    ) {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|e| {
            !(e.owner_tag == owner_tag
                && e.caller_request_id == caller_request_id
                && e.service_id == service_id
                && e.method_id == method_id)
        });
        while guard.len() >= MAX_CALLBACK_RETRY_IDS {
            let _ = guard.pop_front();
        }
        guard.push_back(CallbackRetryEntry {
            owner_tag,
            caller_request_id,
            service_id,
            method_id,
            host_request_id,
        });
        self.has_entries.store(true, Ordering::Release);
    }

    pub(super) fn clear_owner(&self, owner_tag: u64) {
        if !self.has_entries.load(Ordering::Acquire) {
            return;
        }
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|e| e.owner_tag != owner_tag);
        if guard.is_empty() {
            self.has_entries.store(false, Ordering::Release);
        }
    }

    pub(super) fn clear(&self) {
        if !self.has_entries.load(Ordering::Acquire) {
            return;
        }
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
        self.has_entries.store(false, Ordering::Release);
    }
}
