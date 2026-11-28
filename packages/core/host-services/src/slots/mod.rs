//! Lock-free, immutable snapshot-friendly dense service slot table ([`ServiceSlots`]) and
//! cached typed handles ([`ServiceHandle`]).
//!
//! Replaces per-call `RwLock<BTreeMap<u32, Arc<Arc<dyn Backend>>>>` lookups with direct
//! dense-index array access over split hot/cold storage:
//! - **0 locks** (immutable `Arc<SlotTable>` snapshot with cached hot slice pointer)
//! - **0 `BTreeMap` / hash lookups** (indexed directly by validated [`ServiceSlotId`])
//! - **0 double-`Arc` allocations** (`*const T` stored inline in 32-byte `HotSlotEntry`, exact
//!   half cache line on 64-bit targets with zero cross-cache-line straddling)
//! - **0 atomic refcount increments** on [`ServiceSlots::get`] (`&T` borrowed directly)
//! - **O(1) snapshot cloning** (`ServiceSlots::clone` is a single `Arc::clone`) and single-pass
//!   copy-on-write per-surface/test overrides via [`ServiceSlots::with_override`]
//!
//! # Single-Binary Safety Boundary
//! [`ServiceSlots`] and [`ServiceHandle`] store Rust [`core::any::TypeId`]s, `Arc<T>` raw
//! pointers, and Rust trait object vtables. They are **strictly single-binary / statically linked
//! Rust-only** and MUST NEVER be passed across `HotAppInstance` `cdylib` / `dlopen` boundaries
//! (where `TypeId` hashes, allocator instances, and trait vtables are not ABI-stable).
//! Cross-dylib / foreign host calls must cross the C ABI (`NVE_IFACE_HOST = 5` / `NveBytesSink`).

mod table;

use core::fmt;
use core::ops::Deref;
use core::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use table::{HotSlotEntry, OwnedSlot, SlotTable, StagedTable};

/// Maximum number of dense service slots (`0..MAX_SERVICE_SLOTS`).
pub const MAX_SERVICE_SLOTS: usize = 256;

/// First dense slot index reserved for application / third-party modules.
pub const SLOT_USER_BASE: u16 = 16;

static NEXT_USER_SLOT: AtomicU16 = AtomicU16::new(SLOT_USER_BASE);

/// Dense, ingress-validated slot identifier (`0..MAX_SERVICE_SLOTS`) indexing directly into
/// [`ServiceSlots`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceSlotId(u16);

impl ServiceSlotId {
    /// Maximum supported dense slot index (exclusive).
    pub const MAX_SLOTS: usize = MAX_SERVICE_SLOTS;

    /// Constructs a compile-time validated dense slot ID (`INDEX < MAX_SLOTS`).
    ///
    /// Tests working with local [`ServiceSlots`] snapshots should prefer deterministic constants
    /// such as `ServiceSlotId::from_const::<SLOT_USER_BASE>()` and reserve
    /// [`Self::alloc_user_slot`] for process-wide dynamic modules.
    #[must_use]
    #[inline(always)]
    pub const fn from_const<const INDEX: u16>() -> Self {
        const {
            assert!(
                (INDEX as usize) < Self::MAX_SLOTS,
                "ServiceSlotId index exceeds MAX_SERVICE_SLOTS"
            );
        }
        Self(INDEX)
    }

    /// Fallible runtime constructor sealing `index < MAX_SLOTS` at ingress.
    #[must_use]
    #[inline(always)]
    pub const fn try_new(index: u16) -> Option<Self> {
        if (index as usize) < Self::MAX_SLOTS {
            Some(Self(index))
        } else {
            None
        }
    }

    /// Allocates a process-unique dense slot ID in `SLOT_USER_BASE..MAX_SERVICE_SLOTS` for
    /// process-wide dynamic / third-party modules. Returns `None` once all 240 dynamic slots are
    /// exhausted.
    ///
    /// Unit tests constructing isolated local snapshots should use
    /// `ServiceSlotId::from_const::<SLOT_USER_BASE>()` (or explicit test indices) instead of
    /// consuming process-global dynamic slots.
    #[must_use]
    pub fn alloc_user_slot() -> Option<Self> {
        let mut cur = NEXT_USER_SLOT.load(Ordering::Relaxed);
        loop {
            if (cur as usize) >= Self::MAX_SLOTS {
                return None;
            }
            match NEXT_USER_SLOT.compare_exchange_weak(
                cur,
                cur + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(Self(cur)),
                Err(actual) => cur = actual,
            }
        }
    }

    #[must_use]
    #[inline(always)]
    pub const fn raw(self) -> u16 {
        self.0
    }

    #[must_use]
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for ServiceSlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "slot({})", self.0)
    }
}

/// Built-in dense slot IDs for core host capabilities (`0..SLOT_USER_BASE`).
pub const SLOT_CLIPBOARD: ServiceSlotId = ServiceSlotId::from_const::<0>();
pub const SLOT_FILESYSTEM: ServiceSlotId = ServiceSlotId::from_const::<1>();
pub const SLOT_SECURE_STORE: ServiceSlotId = ServiceSlotId::from_const::<2>();
pub const SLOT_LINKING: ServiceSlotId = ServiceSlotId::from_const::<3>();
pub const SLOT_SYSTEM_ENV: ServiceSlotId = ServiceSlotId::from_const::<4>();
pub const SLOT_NOTIFICATIONS: ServiceSlotId = ServiceSlotId::from_const::<5>();
pub const SLOT_HAPTICS: ServiceSlotId = ServiceSlotId::from_const::<6>();

/// Associates a typed service interface (`dyn SomeBackend` or concrete `T`) with its compile-time
/// dense [`ServiceSlotId`], enabling zero-argument `slots.get::<dyn SomeBackend>()` calls.
pub trait ServiceSlot: Send + Sync + 'static {
    const SLOT: ServiceSlotId;
}

/// Cached typed handle (`Arc<T>`) resolved once from [`ServiceSlots`].
///
/// Ideal for dependency injection, component/view state, multi-surface overrides, and unit tests:
/// subsequent calls dereference directly to `&T` with zero slot lookup, zero `TypeId` comparison,
/// zero locks, and zero atomic operations.
///
/// Strictly single-binary / statically linked Rust-only — never pass across `cdylib`/`dlopen`.
pub struct ServiceHandle<T: ?Sized + Send + Sync + 'static> {
    inner: Arc<T>,
}

impl<T: ?Sized + Send + Sync + 'static> ServiceHandle<T> {
    #[must_use]
    #[inline]
    pub fn new(service: Arc<T>) -> Self {
        Self { inner: service }
    }

    #[must_use]
    #[inline]
    pub fn from_value(service: T) -> Self
    where
        T: Sized,
    {
        Self {
            inner: Arc::new(service),
        }
    }

    #[must_use]
    #[inline]
    pub fn as_arc(&self) -> &Arc<T> {
        &self.inner
    }

    #[must_use]
    #[inline]
    pub fn into_arc(self) -> Arc<T> {
        self.inner
    }

    #[must_use]
    #[inline]
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.inner)
    }

    #[must_use]
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<T: ?Sized + Send + Sync + 'static> Clone for ServiceHandle<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: ?Sized + Send + Sync + 'static> Deref for ServiceHandle<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: ?Sized + Send + Sync + 'static> AsRef<T> for ServiceHandle<T> {
    #[inline(always)]
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

impl<T: ?Sized + Send + Sync + 'static> From<Arc<T>> for ServiceHandle<T> {
    #[inline]
    fn from(inner: Arc<T>) -> Self {
        Self { inner }
    }
}

impl<T: ?Sized + Send + Sync + 'static> fmt::Debug for ServiceHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServiceHandle<{}>", core::any::type_name::<T>())
    }
}

/// Immutable, snapshot-friendly dense slot table for lock-free O(1) typed service lookup.
///
/// Cloning `ServiceSlots` is an O(1) `Arc::clone`. Per-surface overrides or unit-test mocks
/// produce a new immutable snapshot via [`Self::with_override`] or [`Self::to_builder`] without
/// mutating sibling surfaces or taking any locks on the read path.
///
/// Strictly single-binary / statically linked Rust-only — never pass across `cdylib`/`dlopen`.
pub struct ServiceSlots {
    /// Cached slice pointer into `table.hot` so `get()` uses one indirection directly onto 32-byte
    /// cache-line-aligned entries without touching `table.cold`.
    hot: *const [HotSlotEntry],
    table: Option<Arc<SlotTable>>,
}

// SAFETY: `hot` points either to an empty slice (`&[]`) or into immutable `SlotTable::hot` owned
// by `table: Option<Arc<SlotTable>>` (`SlotTable` is `Send`), and is only dereferenced while
// `self` keeps `table` alive.
unsafe impl Send for ServiceSlots {}

// SAFETY: `hot` points either to an empty slice (`&[]`) or into immutable `SlotTable::hot` owned
// by `table: Option<Arc<SlotTable>>` (`SlotTable` is `Sync`), so concurrent reads safely yield
// `&T` / cloned `Arc<T>`.
unsafe impl Sync for ServiceSlots {}

impl Clone for ServiceSlots {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            hot: self.hot,
            table: self.table.clone(),
        }
    }
}

impl Default for ServiceSlots {
    #[inline]
    fn default() -> Self {
        Self::EMPTY
    }
}

impl fmt::Debug for ServiceSlots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceSlots")
            .field("occupied", &self.len())
            .field("capacity", &self.slot_capacity())
            .finish()
    }
}

impl ServiceSlots {
    /// Empty zero-allocation slot table.
    pub const EMPTY: Self = Self {
        hot: &[] as *const [HotSlotEntry],
        table: None,
    };

    fn from_staged(staged: StagedTable) -> Self {
        match staged.into_arc() {
            None => Self::EMPTY,
            Some(table) => {
                let hot: *const [HotSlotEntry] = &*table.hot;
                Self {
                    hot,
                    table: Some(table),
                }
            }
        }
    }

    #[must_use]
    #[inline]
    pub const fn empty() -> Self {
        Self::EMPTY
    }

    #[must_use]
    #[inline]
    pub fn builder() -> ServiceSlotsBuilder {
        ServiceSlotsBuilder::new()
    }

    /// Creates a mutable builder pre-populated with shallow clones of this snapshot's slots
    /// (bumps each occupied service's `Arc` refcount; never clones underlying services).
    #[must_use]
    pub fn to_builder(&self) -> ServiceSlotsBuilder {
        let Some(table) = &self.table else {
            return ServiceSlotsBuilder::new();
        };
        let mut slots = Vec::with_capacity(table.hot.len());
        for (hot, cold) in table.hot.iter().zip(table.cold.iter()) {
            match cold {
                Some(vt) => {
                    // SAFETY: `vt` matches the exact `T` stored in `hot`.
                    let cloned = unsafe { (vt.clone_fn)(hot) };
                    slots.push(Some(OwnedSlot {
                        hot: cloned,
                        vtable: vt,
                    }));
                }
                None => slots.push(None),
            }
        }
        ServiceSlotsBuilder { slots }
    }

    /// Returns a new immutable snapshot with `T::SLOT` replaced by `service` in one pass (leaving
    /// `self` untouched and never bumping/dropping the replaced slot's `Arc`).
    #[must_use]
    pub fn with_override<T: ServiceSlot + ?Sized>(&self, service: Arc<T>) -> Self {
        self.with_override_at::<T>(T::SLOT, service)
    }

    /// Returns a new immutable snapshot with `slot` replaced by `service` in one pass (without
    /// intermediate builder allocation or bumping/dropping the replaced slot's `Arc`).
    #[must_use]
    pub fn with_override_at<T: ?Sized + Send + Sync + 'static>(
        &self,
        slot: ServiceSlotId,
        service: Arc<T>,
    ) -> Self {
        let idx = slot.as_usize();
        let new_len = self.slot_capacity().max(idx + 1);
        let mut staged = StagedTable::with_capacity(new_len);
        let old_table = self.table.as_deref();
        let mut new_slot = Some(HotSlotEntry::new::<T>(service));
        for i in 0..new_len {
            if i == idx {
                if let Some((hot, vt)) = new_slot.take() {
                    staged.push_slot(hot, vt);
                }
            } else if let Some(tbl) = old_table
                && i < tbl.hot.len()
                && let Some(vt) = tbl.cold[i]
            {
                // SAFETY: `vt` matches the exact `T` stored in `tbl.hot[i]`.
                let cloned = unsafe { (vt.clone_fn)(&tbl.hot[i]) };
                staged.push_slot(cloned, vt);
            } else {
                staged.push_empty();
            }
        }
        Self::from_staged(staged)
    }

    /// Returns a new immutable snapshot with `slot` cleared in one pass (O(1) clone with zero
    /// allocations when `slot` is already empty, trimming trailing empty entries back to
    /// [`Self::EMPTY`] when all slots are removed).
    #[must_use]
    pub fn without_slot(&self, slot: ServiceSlotId) -> Self {
        let idx = slot.as_usize();
        let Some(tbl) = self.table.as_deref() else {
            return Self::EMPTY;
        };
        if idx >= tbl.cold.len() || tbl.cold[idx].is_none() {
            return self.clone();
        }
        let mut new_len = tbl.cold.len();
        while new_len > 0 {
            let last = new_len - 1;
            if last != idx && tbl.cold[last].is_some() {
                break;
            }
            new_len -= 1;
        }
        if new_len == 0 {
            return Self::EMPTY;
        }
        let mut staged = StagedTable::with_capacity(new_len);
        for i in 0..new_len {
            if i != idx
                && let Some(vt) = tbl.cold[i]
            {
                // SAFETY: `vt` matches the exact `T` stored in `tbl.hot[i]`.
                let cloned = unsafe { (vt.clone_fn)(&tbl.hot[i]) };
                staged.push_slot(cloned, vt);
            } else {
                staged.push_empty();
            }
        }
        Self::from_staged(staged)
    }

    #[inline(always)]
    fn as_hot_slice(&self) -> &[HotSlotEntry] {
        // SAFETY: `self.hot` is either `&[]` (when `table` is `None`) or points to immutable
        // `SlotTable::hot` kept live by `self.table` for at least `&self`'s borrow lifetime.
        unsafe { &*self.hot }
    }

    /// Lock-free O(1) typed borrow at `T::SLOT` with **0 locks, 0 hashing, 0 atomic refcount
    /// increments, and 0 heap allocations**.
    #[must_use]
    #[inline(always)]
    pub fn get<T: ServiceSlot + ?Sized>(&self) -> Option<&T> {
        self.get_at::<T>(T::SLOT)
    }

    /// Lock-free O(1) typed borrow at an explicit dense [`ServiceSlotId`] with **0 locks,
    /// 0 hashing, 0 atomic refcount increments, and 0 heap allocations**.
    #[must_use]
    #[inline(always)]
    pub fn get_at<T: ?Sized + Send + Sync + 'static>(&self, slot: ServiceSlotId) -> Option<&T> {
        self.as_hot_slice().get(slot.as_usize())?.get_ref::<T>()
    }

    /// Resolves a cached [`ServiceHandle<T>`] once from `T::SLOT` (bumps the single `Arc<T>`
    /// refcount once; subsequent calls through the handle perform 0 lookups and 0 atomic ops).
    #[must_use]
    #[inline]
    pub fn handle<T: ServiceSlot + ?Sized>(&self) -> Option<ServiceHandle<T>> {
        self.handle_at::<T>(T::SLOT)
    }

    /// Resolves a cached [`ServiceHandle<T>`] once from an explicit dense [`ServiceSlotId`].
    #[must_use]
    #[inline]
    pub fn handle_at<T: ?Sized + Send + Sync + 'static>(
        &self,
        slot: ServiceSlotId,
    ) -> Option<ServiceHandle<T>> {
        self.get_arc_at::<T>(slot).map(ServiceHandle::new)
    }

    /// Resolves an owned `Arc<T>` from `T::SLOT` without double-`Arc` indirection.
    #[must_use]
    #[inline]
    pub fn get_arc<T: ServiceSlot + ?Sized>(&self) -> Option<Arc<T>> {
        self.get_arc_at::<T>(T::SLOT)
    }

    /// Resolves an owned `Arc<T>` from an explicit dense [`ServiceSlotId`] without double-`Arc`
    /// indirection.
    #[must_use]
    #[inline]
    pub fn get_arc_at<T: ?Sized + Send + Sync + 'static>(
        &self,
        slot: ServiceSlotId,
    ) -> Option<Arc<T>> {
        self.as_hot_slice().get(slot.as_usize())?.clone_arc::<T>()
    }

    /// Returns `true` if `slot` currently holds any service.
    #[must_use]
    #[inline]
    pub fn contains(&self, slot: ServiceSlotId) -> bool {
        self.as_hot_slice()
            .get(slot.as_usize())
            .is_some_and(|e| !e.is_empty())
    }

    /// Number of occupied service slots in this snapshot.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.as_hot_slice().iter().filter(|e| !e.is_empty()).count()
    }

    /// O(1) check returning `true` iff no service slots are populated.
    #[must_use]
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.table.is_none()
    }

    /// Dense array length allocated for this snapshot (`<= MAX_SERVICE_SLOTS`).
    #[must_use]
    #[inline(always)]
    pub fn slot_capacity(&self) -> usize {
        self.as_hot_slice().len()
    }
}

/// Mutable builder producing an immutable [`ServiceSlots`] snapshot.
#[derive(Default)]
pub struct ServiceSlotsBuilder {
    slots: Vec<Option<OwnedSlot>>,
}

impl ServiceSlotsBuilder {
    #[must_use]
    #[inline]
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Registers `service` at `T::SLOT` (replacing any prior entry at that slot).
    #[inline]
    pub fn insert<T: ServiceSlot + ?Sized>(&mut self, service: Arc<T>) -> &mut Self {
        self.insert_at::<T>(T::SLOT, service)
    }

    /// Registers `service` at an explicit dense [`ServiceSlotId`].
    pub fn insert_at<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        slot: ServiceSlotId,
        service: Arc<T>,
    ) -> &mut Self {
        let idx = slot.as_usize();
        if idx >= self.slots.len() {
            self.slots.resize_with(idx + 1, || None);
        }
        self.slots[idx] = Some(OwnedSlot::new::<T>(service));
        self
    }

    /// Wraps concrete `service` in a single `Arc<T>` and registers it at `T::SLOT`.
    #[inline]
    pub fn insert_value<T: ServiceSlot + Sized>(&mut self, service: T) -> &mut Self {
        self.insert::<T>(Arc::new(service))
    }

    /// Wraps concrete `service` in a single `Arc<T>` and registers it at `slot`.
    #[inline]
    pub fn insert_value_at<T: Send + Sync + 'static>(
        &mut self,
        slot: ServiceSlotId,
        service: T,
    ) -> &mut Self {
        self.insert_at::<T>(slot, Arc::new(service))
    }

    /// Builder-style variant of [`Self::insert`].
    #[must_use]
    #[inline]
    pub fn set<T: ServiceSlot + ?Sized>(mut self, service: Arc<T>) -> Self {
        self.insert::<T>(service);
        self
    }

    /// Builder-style variant of [`Self::insert_at`].
    #[must_use]
    #[inline]
    pub fn set_at<T: ?Sized + Send + Sync + 'static>(
        mut self,
        slot: ServiceSlotId,
        service: Arc<T>,
    ) -> Self {
        self.insert_at::<T>(slot, service);
        self
    }

    /// Builder-style variant of [`Self::insert_value`].
    #[must_use]
    #[inline]
    pub fn set_value<T: ServiceSlot + Sized>(mut self, service: T) -> Self {
        self.insert_value::<T>(service);
        self
    }

    /// Builder-style variant of [`Self::insert_value_at`].
    #[must_use]
    #[inline]
    pub fn set_value_at<T: Send + Sync + 'static>(
        mut self,
        slot: ServiceSlotId,
        service: T,
    ) -> Self {
        self.insert_value_at::<T>(slot, service);
        self
    }

    /// Clears `slot` if populated.
    #[inline]
    pub fn remove(&mut self, slot: ServiceSlotId) -> &mut Self {
        if let Some(entry) = self.slots.get_mut(slot.as_usize()) {
            *entry = None;
        }
        self
    }

    /// Builder-style variant of [`Self::remove`].
    #[must_use]
    #[inline]
    pub fn without(mut self, slot: ServiceSlotId) -> Self {
        self.remove(slot);
        self
    }

    /// Freezes the registered slots into an immutable, lock-free [`ServiceSlots`] snapshot.
    #[must_use]
    pub fn build(mut self) -> ServiceSlots {
        while self.slots.last().is_some_and(Option::is_none) {
            let _ = self.slots.pop();
        }
        if self.slots.is_empty() {
            return ServiceSlots::EMPTY;
        }
        let mut staged = StagedTable::with_capacity(self.slots.len());
        for entry in self.slots {
            match entry {
                Some(owned) => {
                    let (hot, vt) = owned.into_parts();
                    staged.push_slot(hot, vt);
                }
                None => staged.push_empty(),
            }
        }
        ServiceSlots::from_staged(staged)
    }
}
