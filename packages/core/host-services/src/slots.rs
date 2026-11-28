//! Lock-free, immutable snapshot-friendly dense service slot table ([`ServiceSlots`]) and
//! cached typed handles ([`ServiceHandle`]).
//!
//! Replaces per-call `RwLock<BTreeMap<u32, Arc<Arc<dyn Backend>>>>` lookups with direct
//! dense-index array access over inline fat pointers (`*const T` from `Arc::into_raw`):
//! - **0 locks** (`RwLock`-free immutable `Arc<[Option<SlotEntry>]>` snapshot)
//! - **0 `BTreeMap` / hash lookups** (indexed directly by validated [`ServiceSlotId`])
//! - **0 double-`Arc` allocations** (`*const T` stored inline in `MaybeUninit<[*const (); 2]>`)
//! - **0 atomic refcount increments** on [`ServiceSlots::get`] (`&T` borrowed directly)
//! - **O(1) snapshot cloning** (`ServiceSlots::clone` is a single `Arc::clone`) and cheap
//!   copy-on-write per-surface/test overrides via [`ServiceSlots::with_override`].

use core::any::TypeId;
use core::fmt;
use core::mem::{align_of, size_of, MaybeUninit};
use core::ops::Deref;
use core::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

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
    #[inline(always)]
    pub const fn try_new(index: u16) -> Option<Self> {
        if (index as usize) < Self::MAX_SLOTS {
            Some(Self(index))
        } else {
            None
        }
    }

    /// Allocates a process-unique dense slot ID in `SLOT_USER_BASE..MAX_SERVICE_SLOTS` for
    /// dynamic / third-party modules or isolated tests. Returns `None` once exhausted.
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

    #[inline(always)]
    pub const fn raw(self) -> u16 {
        self.0
    }

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
pub struct ServiceHandle<T: ?Sized + Send + Sync + 'static> {
    inner: Arc<T>,
}

impl<T: ?Sized + Send + Sync + 'static> ServiceHandle<T> {
    #[inline]
    pub fn new(service: Arc<T>) -> Self {
        Self { inner: service }
    }

    #[inline]
    pub fn from_value(service: T) -> Self
    where
        T: Sized,
    {
        Self {
            inner: Arc::new(service),
        }
    }

    #[inline]
    pub fn as_arc(&self) -> &Arc<T> {
        &self.inner
    }

    #[inline]
    pub fn into_arc(self) -> Arc<T> {
        self.inner
    }

    #[inline]
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.inner)
    }

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

/// Inline type-erased single-`Arc` slot storing `*const T` (thin or fat pointer) directly without
/// double-`Arc` heap indirection.
struct SlotEntry {
    type_id: TypeId,
    raw_ptr: MaybeUninit<[*const (); 2]>,
    clone_fn: unsafe fn(&SlotEntry) -> SlotEntry,
    drop_fn: unsafe fn(&mut SlotEntry),
}

// SAFETY: `SlotEntry::new` enforces `T: ?Sized + Send + Sync + 'static`, so the owned `Arc<T>`
// pointer is both `Send` and `Sync` and only yields `&T` or cloned `Arc<T>`.
unsafe impl Send for SlotEntry {}
// SAFETY: `SlotEntry::new` enforces `T: ?Sized + Send + Sync + 'static`, so shared `&SlotEntry`
// access safely yields `&T` and clones `Arc<T>` across threads.
unsafe impl Sync for SlotEntry {}

impl SlotEntry {
    fn new<T: ?Sized + Send + Sync + 'static>(service: Arc<T>) -> Self {
        const {
            assert!(size_of::<*const T>() <= size_of::<[*const (); 2]>());
            assert!(align_of::<*const T>() <= align_of::<[*const (); 2]>());
        }
        let ptr: *const T = Arc::into_raw(service);
        let mut raw_ptr = MaybeUninit::<[*const (); 2]>::uninit();
        // SAFETY: Compile-time asserts above guarantee `raw_ptr` fits `*const T` in size and
        // alignment; writing `*const T` preserves full pointer provenance (thin or fat).
        unsafe {
            raw_ptr.as_mut_ptr().cast::<*const T>().write(ptr);
        }
        Self {
            type_id: TypeId::of::<T>(),
            raw_ptr,
            clone_fn: clone_slot_entry::<T>,
            drop_fn: drop_slot_entry::<T>,
        }
    }

    #[inline(always)]
    unsafe fn read_ptr<T: ?Sized + Send + Sync + 'static>(&self) -> *const T {
        const {
            assert!(size_of::<*const T>() <= size_of::<[*const (); 2]>());
            assert!(align_of::<*const T>() <= align_of::<[*const (); 2]>());
        }
        // SAFETY: Caller guarantees `self.type_id == TypeId::of::<T>()`, so `self.raw_ptr` was
        // initialized via `write::<*const T>` with a live `Arc::into_raw::<T>` pointer.
        unsafe { self.raw_ptr.as_ptr().cast::<*const T>().read() }
    }

    #[inline(always)]
    fn get_ref<T: ?Sized + Send + Sync + 'static>(&self) -> Option<&T> {
        if self.type_id != TypeId::of::<T>() {
            return None;
        }
        // SAFETY: `type_id` matches `T`; `self` owns a strong `Arc<T>` count keeping `*ptr` valid
        // for at least `&self`'s borrow lifetime.
        unsafe { Some(&*self.read_ptr::<T>()) }
    }

    #[inline]
    fn clone_arc<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        if self.type_id != TypeId::of::<T>() {
            return None;
        }
        // SAFETY: `type_id` matches `T`; incrementing the strong count before `Arc::from_raw`
        // mints an independent owned `Arc<T>` while `self` retains its own strong reference.
        unsafe {
            let ptr = self.read_ptr::<T>();
            Arc::increment_strong_count(ptr);
            Some(Arc::from_raw(ptr))
        }
    }
}

unsafe fn clone_slot_entry<T: ?Sized + Send + Sync + 'static>(entry: &SlotEntry) -> SlotEntry {
    // SAFETY: `clone_slot_entry::<T>` is only installed by `SlotEntry::new::<T>`.
    let ptr: *const T = unsafe { entry.read_ptr::<T>() };
    // SAFETY: `entry` owns one strong reference to `ptr`.
    unsafe { Arc::increment_strong_count(ptr) };
    let mut raw_ptr = MaybeUninit::<[*const (); 2]>::uninit();
    // SAFETY: Size/align checked at compile time in `SlotEntry::new::<T>` and `read_ptr::<T>`.
    unsafe {
        raw_ptr.as_mut_ptr().cast::<*const T>().write(ptr);
    }
    SlotEntry {
        type_id: entry.type_id,
        raw_ptr,
        clone_fn: entry.clone_fn,
        drop_fn: entry.drop_fn,
    }
}

unsafe fn drop_slot_entry<T: ?Sized + Send + Sync + 'static>(entry: &mut SlotEntry) {
    // SAFETY: `drop_slot_entry::<T>` is only installed by `SlotEntry::new::<T>` and runs once in
    // `Drop::drop`.
    let ptr: *const T = unsafe { entry.read_ptr::<T>() };
    // SAFETY: Reclaims the single strong reference surrendered by `Arc::into_raw`.
    drop(unsafe { Arc::from_raw(ptr) });
}

impl Clone for SlotEntry {
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: `self.clone_fn` matches the exact `T` stored in `self.raw_ptr`.
        unsafe { (self.clone_fn)(self) }
    }
}

impl Drop for SlotEntry {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: `self.drop_fn` matches the exact `T` stored in `self.raw_ptr` and runs once.
        unsafe { (self.drop_fn)(self) }
    }
}

/// Immutable, snapshot-friendly dense slot table for lock-free O(1) typed service lookup.
///
/// Cloning `ServiceSlots` is an O(1) `Arc::clone`. Per-surface overrides or unit-test mocks
/// produce a new immutable snapshot via [`Self::with_override`] or [`Self::to_builder`] without
/// mutating sibling surfaces or taking any locks on the read path.
#[derive(Clone, Default)]
pub struct ServiceSlots {
    slots: Option<Arc<[Option<SlotEntry>]>>,
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
    pub const EMPTY: Self = Self { slots: None };

    #[inline]
    pub const fn empty() -> Self {
        Self::EMPTY
    }

    #[inline]
    pub fn builder() -> ServiceSlotsBuilder {
        ServiceSlotsBuilder::new()
    }

    /// Creates a mutable builder pre-populated with shallow clones of this snapshot's slots
    /// (bumps each occupied service's `Arc` refcount; never clones underlying services).
    pub fn to_builder(&self) -> ServiceSlotsBuilder {
        ServiceSlotsBuilder {
            slots: self.as_slice().to_vec(),
        }
    }

    /// Returns a new immutable snapshot with `T::SLOT` replaced by `service` (leaving `self`
    /// untouched for sibling surfaces or parallel tests).
    pub fn with_override<T: ServiceSlot + ?Sized>(&self, service: Arc<T>) -> Self {
        self.to_builder().set(service).build()
    }

    /// Returns a new immutable snapshot with `slot` replaced by `service`.
    pub fn with_override_at<T: ?Sized + Send + Sync + 'static>(
        &self,
        slot: ServiceSlotId,
        service: Arc<T>,
    ) -> Self {
        self.to_builder().set_at(slot, service).build()
    }

    /// Returns a new immutable snapshot with `slot` cleared.
    pub fn without_slot(&self, slot: ServiceSlotId) -> Self {
        self.to_builder().without(slot).build()
    }

    #[inline(always)]
    fn as_slice(&self) -> &[Option<SlotEntry>] {
        match &self.slots {
            Some(arc) => arc,
            None => &[],
        }
    }

    /// Lock-free O(1) typed borrow at `T::SLOT` with **0 locks, 0 hashing, 0 atomic refcount
    /// increments, and 0 heap allocations**.
    #[inline(always)]
    pub fn get<T: ServiceSlot + ?Sized>(&self) -> Option<&T> {
        self.get_at::<T>(T::SLOT)
    }

    /// Lock-free O(1) typed borrow at an explicit dense [`ServiceSlotId`] with **0 locks,
    /// 0 hashing, 0 atomic refcount increments, and 0 heap allocations**.
    #[inline(always)]
    pub fn get_at<T: ?Sized + Send + Sync + 'static>(&self, slot: ServiceSlotId) -> Option<&T> {
        self.as_slice()
            .get(slot.as_usize())?
            .as_ref()?
            .get_ref::<T>()
    }

    /// Resolves a cached [`ServiceHandle<T>`] once from `T::SLOT` (bumps the single `Arc<T>`
    /// refcount once; subsequent calls through the handle perform 0 lookups and 0 atomic ops).
    #[inline]
    pub fn handle<T: ServiceSlot + ?Sized>(&self) -> Option<ServiceHandle<T>> {
        self.handle_at::<T>(T::SLOT)
    }

    /// Resolves a cached [`ServiceHandle<T>`] once from an explicit dense [`ServiceSlotId`].
    #[inline]
    pub fn handle_at<T: ?Sized + Send + Sync + 'static>(
        &self,
        slot: ServiceSlotId,
    ) -> Option<ServiceHandle<T>> {
        self.get_arc_at::<T>(slot).map(ServiceHandle::new)
    }

    /// Resolves an owned `Arc<T>` from `T::SLOT` without double-`Arc` indirection.
    #[inline]
    pub fn get_arc<T: ServiceSlot + ?Sized>(&self) -> Option<Arc<T>> {
        self.get_arc_at::<T>(T::SLOT)
    }

    /// Resolves an owned `Arc<T>` from an explicit dense [`ServiceSlotId`] without double-`Arc`
    /// indirection.
    #[inline]
    pub fn get_arc_at<T: ?Sized + Send + Sync + 'static>(
        &self,
        slot: ServiceSlotId,
    ) -> Option<Arc<T>> {
        self.as_slice()
            .get(slot.as_usize())?
            .as_ref()?
            .clone_arc::<T>()
    }

    /// Returns `true` if `slot` currently holds any service.
    #[inline]
    pub fn contains(&self, slot: ServiceSlotId) -> bool {
        self.as_slice()
            .get(slot.as_usize())
            .is_some_and(Option::is_some)
    }

    /// Number of occupied service slots in this snapshot.
    #[inline]
    pub fn len(&self) -> usize {
        self.as_slice().iter().filter(|e| e.is_some()).count()
    }

    /// Returns `true` if no service slots are populated.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Dense array length allocated for this snapshot (`<= MAX_SERVICE_SLOTS`).
    #[inline]
    pub fn slot_capacity(&self) -> usize {
        self.as_slice().len()
    }
}

/// Mutable builder producing an immutable [`ServiceSlots`] snapshot.
#[derive(Default)]
pub struct ServiceSlotsBuilder {
    slots: Vec<Option<SlotEntry>>,
}

impl ServiceSlotsBuilder {
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
        self.slots[idx] = Some(SlotEntry::new::<T>(service));
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
    #[inline]
    pub fn set<T: ServiceSlot + ?Sized>(mut self, service: Arc<T>) -> Self {
        self.insert::<T>(service);
        self
    }

    /// Builder-style variant of [`Self::insert_at`].
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
    #[inline]
    pub fn set_value<T: ServiceSlot + Sized>(mut self, service: T) -> Self {
        self.insert_value::<T>(service);
        self
    }

    /// Builder-style variant of [`Self::insert_value_at`].
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
    #[inline]
    pub fn without(mut self, slot: ServiceSlotId) -> Self {
        self.remove(slot);
        self
    }

    /// Freezes the registered slots into an immutable, lock-free [`ServiceSlots`] snapshot.
    pub fn build(mut self) -> ServiceSlots {
        while self.slots.last().is_some_and(Option::is_none) {
            let _ = self.slots.pop();
        }
        if self.slots.is_empty() {
            return ServiceSlots::EMPTY;
        }
        ServiceSlots {
            slots: Some(Arc::from(self.slots.into_boxed_slice())),
        }
    }
}
