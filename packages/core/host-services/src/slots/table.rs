//! Split hot/cold storage backing [`super::ServiceSlots`]: 32-byte cache-line-aligned
//! [`HotSlotEntry`] (`type_id` + inline thin/fat pointer) paired with a cold [`SlotVtable`] slice
//! touched only during snapshot cloning and drop.

use core::any::TypeId;
use core::marker::PhantomData;
use core::mem::{align_of, size_of, ManuallyDrop, MaybeUninit};
use std::sync::Arc;

/// Private `!Send + !Sync` sentinel whose `TypeId` marks empty [`HotSlotEntry`] slots.
///
/// Because every stored service requires `T: Send + Sync`, no caller type can ever collide with
/// `TypeId::of::<EmptySlotSentinel>()`, allowing empty-slot and type-mismatch rejection in one
/// branch without adding an `Option` discriminant byte to the 32-byte hot entry.
struct EmptySlotSentinel(PhantomData<*const ()>);

/// Hot 32-byte (exact half cache line on 64-bit) lookup entry storing `(type_id, raw_ptr)`.
///
/// Ownership vtables live in a parallel cold slice so `get()` touches strictly 32 bytes.
#[repr(C)]
pub(super) struct HotSlotEntry {
    type_id: TypeId,
    raw_ptr: MaybeUninit<[*const (); 2]>,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<HotSlotEntry>() == 32);
    assert!(align_of::<HotSlotEntry>() == 8);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<HotSlotEntry>() <= 32);
};

// SAFETY: Every populated `HotSlotEntry` stores an `Arc::into_raw::<T>` pointer where
// `T: ?Sized + Send + Sync + 'static`, and empty entries hold no pointer.
unsafe impl Send for HotSlotEntry {}

// SAFETY: Shared references to `HotSlotEntry` only yield `&T` or clone `Arc<T>` for `T: Sync`.
unsafe impl Sync for HotSlotEntry {}

pub(super) struct SlotVtable {
    pub(super) clone_fn: unsafe fn(&HotSlotEntry) -> HotSlotEntry,
    pub(super) drop_fn: unsafe fn(&mut HotSlotEntry),
}

const fn vtable_for<T: ?Sized + Send + Sync + 'static>() -> &'static SlotVtable {
    &SlotVtable {
        clone_fn: clone_hot_entry::<T>,
        drop_fn: drop_hot_entry::<T>,
    }
}

impl HotSlotEntry {
    #[inline(always)]
    pub(super) fn empty() -> Self {
        Self {
            type_id: TypeId::of::<EmptySlotSentinel>(),
            raw_ptr: MaybeUninit::uninit(),
        }
    }

    pub(super) fn new<T: ?Sized + Send + Sync + 'static>(
        service: Arc<T>,
    ) -> (Self, &'static SlotVtable) {
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
        (
            Self {
                type_id: TypeId::of::<T>(),
                raw_ptr,
            },
            vtable_for::<T>(),
        )
    }

    #[inline(always)]
    pub(super) fn is_empty(&self) -> bool {
        self.type_id == TypeId::of::<EmptySlotSentinel>()
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
    pub(super) fn get_ref<T: ?Sized + Send + Sync + 'static>(&self) -> Option<&T> {
        if self.type_id != TypeId::of::<T>() {
            return None;
        }
        // SAFETY: `type_id` matches `T` (cannot be `EmptySlotSentinel` since `T: Send + Sync`);
        // the owning table holds a strong `Arc<T>` count valid for at least `&self`'s lifetime.
        unsafe { Some(&*self.read_ptr::<T>()) }
    }

    #[inline]
    pub(super) fn clone_arc<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        if self.type_id != TypeId::of::<T>() {
            return None;
        }
        // SAFETY: `type_id` matches `T`; incrementing the strong count before `Arc::from_raw`
        // mints an independent owned `Arc<T>` while the table retains its strong reference.
        unsafe {
            let ptr = self.read_ptr::<T>();
            Arc::increment_strong_count(ptr);
            Some(Arc::from_raw(ptr))
        }
    }
}

unsafe fn clone_hot_entry<T: ?Sized + Send + Sync + 'static>(entry: &HotSlotEntry) -> HotSlotEntry {
    // SAFETY: `clone_hot_entry::<T>` is only installed by `HotSlotEntry::new::<T>`.
    let ptr: *const T = unsafe { entry.read_ptr::<T>() };
    // SAFETY: `entry` owns one strong reference to `ptr`.
    unsafe { Arc::increment_strong_count(ptr) };
    let mut raw_ptr = MaybeUninit::<[*const (); 2]>::uninit();
    // SAFETY: Size/align checked at compile time in `HotSlotEntry::new::<T>` and `read_ptr::<T>`.
    unsafe {
        raw_ptr.as_mut_ptr().cast::<*const T>().write(ptr);
    }
    HotSlotEntry {
        type_id: entry.type_id,
        raw_ptr,
    }
}

unsafe fn drop_hot_entry<T: ?Sized + Send + Sync + 'static>(entry: &mut HotSlotEntry) {
    // SAFETY: `drop_hot_entry::<T>` is only installed by `HotSlotEntry::new::<T>` and runs once.
    let ptr: *const T = unsafe { entry.read_ptr::<T>() };
    // SAFETY: Reclaims the single strong reference surrendered by `Arc::into_raw`.
    drop(unsafe { Arc::from_raw(ptr) });
}

/// Owned hot/cold entry pair used inside [`super::ServiceSlotsBuilder`].
pub(super) struct OwnedSlot {
    pub(super) hot: HotSlotEntry,
    pub(super) vtable: &'static SlotVtable,
}

impl OwnedSlot {
    pub(super) fn new<T: ?Sized + Send + Sync + 'static>(service: Arc<T>) -> Self {
        let (hot, vtable) = HotSlotEntry::new(service);
        Self { hot, vtable }
    }

    pub(super) fn into_parts(self) -> (HotSlotEntry, &'static SlotVtable) {
        let me = ManuallyDrop::new(self);
        // SAFETY: Reading `me.hot` out of `ManuallyDrop` transfers ownership without running Drop.
        let hot = unsafe { core::ptr::read(&me.hot) };
        (hot, me.vtable)
    }
}

impl Drop for OwnedSlot {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: `self.vtable` matches the exact `T` stored in `self.hot`.
        unsafe { (self.vtable.drop_fn)(&mut self.hot) };
    }
}

/// Immutable hot/cold storage backing [`super::ServiceSlots`].
pub(super) struct SlotTable {
    pub(super) hot: Box<[HotSlotEntry]>,
    pub(super) cold: Box<[Option<&'static SlotVtable>]>,
}

impl Drop for SlotTable {
    fn drop(&mut self) {
        for (hot, cold) in self.hot.iter_mut().zip(self.cold.iter()) {
            if let Some(vt) = cold {
                // SAFETY: `vt` was paired with `hot` at construction and runs once on drop.
                unsafe { (vt.drop_fn)(hot) };
            }
        }
    }
}

/// Drop-safe staging buffer ensuring one-pass snapshot construction never leaks on unwind.
pub(super) struct StagedTable {
    hot: Vec<HotSlotEntry>,
    cold: Vec<Option<&'static SlotVtable>>,
}

impl StagedTable {
    pub(super) fn with_capacity(cap: usize) -> Self {
        Self {
            hot: Vec::with_capacity(cap),
            cold: Vec::with_capacity(cap),
        }
    }

    #[inline]
    pub(super) fn push_empty(&mut self) {
        self.hot.push(HotSlotEntry::empty());
        self.cold.push(None);
    }

    #[inline]
    pub(super) fn push_slot(&mut self, hot: HotSlotEntry, vt: &'static SlotVtable) {
        self.hot.push(hot);
        self.cold.push(Some(vt));
    }

    pub(super) fn into_arc(self) -> Option<Arc<SlotTable>> {
        let me = ManuallyDrop::new(self);
        // SAFETY: `me` is `ManuallyDrop`, transferring ownership of both vectors into `SlotTable`.
        let hot = unsafe { core::ptr::read(&me.hot) }.into_boxed_slice();
        // SAFETY: Same as above.
        let cold = unsafe { core::ptr::read(&me.cold) }.into_boxed_slice();
        if hot.is_empty() {
            None
        } else {
            Some(Arc::new(SlotTable { hot, cold }))
        }
    }
}

impl Drop for StagedTable {
    fn drop(&mut self) {
        for (hot, cold) in self.hot.iter_mut().zip(self.cold.iter()) {
            if let Some(vt) = cold {
                // SAFETY: Every pushed populated slot owns one strong `Arc` reference.
                unsafe { (vt.drop_fn)(hot) };
            }
        }
    }
}
