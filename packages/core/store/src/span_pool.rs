//! Flat power-of-two size-class span allocator backing the `ChildPool` ([`ChildSpan`]) and the
//! sealed UTF-8 [`StrArena`] ([`StrSpan`]).
//!
//! A span of `len` elements occupies `class_cap(len)` slots (next power of two, `>= MIN_CAP`), so
//! growth within the class rewrites in place and freed spans are recycled through one free list per
//! class: O(1) alloc/free, zero per-span heap allocations, one contiguous publishable buffer, no
//! compaction pauses.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use crate::id::NodeId;

const MIN_CLASS: u32 = 2; // 4 elements
const MAX_CLASS: u32 = 30; // 1 << 30 elements max per span
const CLASSES: usize = (MAX_CLASS as usize) + 1;
/// Hard cap on any single span's element count (prevents shift overflow in `class_cap`).
pub const MAX_SPAN_LEN: u32 = 1u32 << MAX_CLASS;

/// Capacity class of a span of `len` elements.
#[inline(always)]
const fn class_of(len: u32) -> u32 {
    let clamped = if len > MAX_SPAN_LEN { MAX_SPAN_LEN } else { len };
    let c = u32::BITS - clamped.saturating_sub(1).leading_zeros();
    if c < MIN_CLASS { MIN_CLASS } else { c }
}

/// Slot capacity reserved for a span of `len` elements.
#[inline(always)]
pub const fn class_cap(len: u32) -> u32 {
    1u32 << class_of(len)
}

/// Typed offset/length handle into a [`SpanPool<T>`] (`len == 0` owns nothing).
///
/// Parameterized by element type `T` so [`ChildSpan`] (`Span<NodeId>`) and [`StrSpan`] (`Span<u8>`)
/// cannot be mixed at compile time. Fields are `pub(crate)` so external code cannot forge
/// mid-codepoint [`StrSpan`] slices into a [`StrArena`].
#[repr(C)]
pub struct Span<T> {
    pub(crate) off: u32,
    pub(crate) len: u32,
    _marker: PhantomData<fn() -> T>,
}

/// Span of child [`NodeId`]s inside `NodeStore::children`.
pub type ChildSpan = Span<NodeId>;
/// Validated UTF-8 byte span inside [`StrArena`].
pub type StrSpan = Span<u8>;

const _: () = {
    assert!(size_of::<ChildSpan>() == 8);
    assert!(size_of::<StrSpan>() == 8);
};

impl<T> Clone for Span<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Span<T> {}

impl<T> PartialEq for Span<T> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.off == other.off && self.len == other.len
    }
}

impl<T> Eq for Span<T> {}

impl<T> Hash for Span<T> {
    #[inline(always)]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.off.hash(state);
        self.len.hash(state);
    }
}

impl<T> Default for Span<T> {
    #[inline(always)]
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<T> fmt::Debug for Span<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Span")
            .field("off", &self.off)
            .field("len", &self.len)
            .finish()
    }
}

impl<T> Span<T> {
    pub const EMPTY: Self = Self {
        off: 0,
        len: 0,
        _marker: PhantomData,
    };

    #[inline(always)]
    pub(crate) const fn new(off: u32, len: u32) -> Self {
        Self {
            off,
            len,
            _marker: PhantomData,
        }
    }

    #[inline(always)]
    pub const fn off(self) -> u32 {
        self.off
    }

    #[inline(always)]
    pub const fn len(self) -> u32 {
        self.len
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub const fn range(self) -> core::ops::Range<usize> {
        let start = self.off as usize;
        start..start.saturating_add(self.len as usize)
    }
}

#[derive(Debug, Clone)]
pub struct SpanPool<T: Copy + Default> {
    data: Vec<T>,
    free: [Vec<u32>; CLASSES],
    live: usize,
}

impl<T: Copy + Default> Default for SpanPool<T> {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            free: Default::default(),
            live: 0,
        }
    }
}

impl<T: Copy + Default> SpanPool<T> {
    /// The whole backing buffer (published zero-copy as a scene slab).
    #[inline(always)]
    pub fn data(&self) -> &[T] {
        &self.data
    }

    #[inline(always)]
    pub fn get(&self, span: Span<T>) -> &[T] {
        if span.len == 0 {
            return &[];
        }
        &self.data[span.range()]
    }

    #[inline(always)]
    pub fn try_get(&self, span: Span<T>) -> Option<&[T]> {
        if span.len == 0 {
            return Some(&[]);
        }
        let start = span.off as usize;
        let end = start.checked_add(span.len as usize)?;
        self.data.get(start..end)
    }

    /// Elements currently referenced by live spans.
    #[inline]
    pub fn live(&self) -> usize {
        self.live
    }

    fn alloc(&mut self, len: u32) -> u32 {
        assert!(len <= MAX_SPAN_LEN, "SpanPool: span length {len} exceeds MAX_SPAN_LEN");
        let class = class_of(len) as usize;
        if let Some(off) = self.free[class].pop() {
            return off;
        }
        let cap = 1usize << class;
        let off = u32::try_from(self.data.len()).expect("SpanPool: backing offset exceeds u32::MAX");
        let new_total = self
            .data
            .len()
            .checked_add(cap)
            .expect("SpanPool: backing capacity overflow");
        assert!(
            u32::try_from(new_total).is_ok(),
            "SpanPool: backing slab exceeds u32::MAX elements"
        );
        self.data.resize(new_total, T::default());
        off
    }

    /// Releases `span` (no-op for empty spans).
    #[inline]
    pub fn free(&mut self, span: Span<T>) {
        if span.len != 0 {
            self.live = self.live.saturating_sub(span.len as usize);
            self.free[class_of(span.len) as usize].push(span.off);
        }
    }

    /// Replaces the contents of `span` with `src`, reusing its slots when the size class allows.
    /// Returns the new span.
    pub fn replace(&mut self, span: Span<T>, src: &[T]) -> Span<T> {
        let new_len = u32::try_from(src.len()).expect("SpanPool: span length exceeds u32::MAX");
        assert!(
            new_len <= MAX_SPAN_LEN,
            "SpanPool: span length {new_len} exceeds MAX_SPAN_LEN"
        );
        if new_len == 0 {
            self.free(span);
            return Span::EMPTY;
        }
        let off = if span.len != 0 && class_of(span.len) == class_of(new_len) {
            self.live = self.live.saturating_sub(span.len as usize) + new_len as usize;
            span.off
        } else {
            self.free(span);
            let off = self.alloc(new_len);
            self.live += new_len as usize;
            off
        };
        let start = off as usize;
        let end = start + new_len as usize;
        self.data[start..end].copy_from_slice(src);
        Span::new(off, new_len)
    }

    /// Inserts `elem` at `at` (clamped to `span.len`) in place when the capacity class fits, or
    /// migrates to the next class — zero heap allocations beyond slab growth.
    pub fn insert_at(&mut self, span: Span<T>, at: usize, elem: T) -> Span<T> {
        let old_len = span.len as usize;
        let at = at.min(old_len);
        let new_len = span.len.checked_add(1).expect("SpanPool: span length overflow");
        assert!(
            new_len <= MAX_SPAN_LEN,
            "SpanPool: span length {new_len} exceeds MAX_SPAN_LEN"
        );
        if span.len != 0 && class_of(span.len) == class_of(new_len) {
            let base = span.off as usize;
            self.data.copy_within(base + at..base + old_len, base + at + 1);
            self.data[base + at] = elem;
            self.live += 1;
            Span::new(span.off, new_len)
        } else {
            let new_off = self.alloc(new_len);
            let dst = new_off as usize;
            if span.len != 0 {
                let src = span.off as usize;
                self.data.copy_within(src..src + at, dst);
                self.data.copy_within(src + at..src + old_len, dst + at + 1);
                self.free(span);
            }
            self.data[dst + at] = elem;
            self.live += new_len as usize;
            Span::new(new_off, new_len)
        }
    }

    /// Removes the element at `at` in place, shrinking to a smaller class if the class drops.
    pub fn remove_at(&mut self, span: Span<T>, at: usize) -> Span<T> {
        assert!(at < span.len as usize, "SpanPool::remove_at index {at} out of bounds (len={})", span.len);
        let new_len = span.len - 1;
        if new_len == 0 {
            self.free(span);
            return Span::EMPTY;
        }
        let start = span.off as usize;
        let end = start + span.len as usize;
        if class_of(new_len) == class_of(span.len) {
            self.data.copy_within(start + at + 1..end, start + at);
            self.live = self.live.saturating_sub(1);
            Span::new(span.off, new_len)
        } else {
            let off = self.alloc(new_len);
            let dst = off as usize;
            self.data.copy_within(start..start + at, dst);
            self.data.copy_within(start + at + 1..end, dst + at);
            self.free(span);
            self.live += new_len as usize;
            Span::new(off, new_len)
        }
    }
}

/// Sealed UTF-8 string arena backed by a [`SpanPool<u8>`].
///
/// Every write enters as a valid `&str` inside crate-private mutators, and live row readers
/// (`RowRef`) borrow both the row words and `&StrArena` under one shared `'a` lifetime so hot
/// `str_unchecked` reads are O(1) and 100% sound (Rust's borrow checker statically forbids
/// mutating the arena while any borrowed `&'a str` is alive). Detached [`StrSpan`] lookups use
/// checked UTF-8 validation (`try_str` / `str`).
#[derive(Debug, Clone, Default)]
pub struct StrArena(SpanPool<u8>);

impl StrArena {
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        self.0.data()
    }

    #[inline(always)]
    pub fn live(&self) -> usize {
        self.0.live()
    }

    /// Crate-private O(1) unchecked UTF-8 slice read for live row words owned by the same
    /// [`crate::NodeStore`] (`RowRef` / `RowWriter` only).
    #[inline(always)]
    pub(crate) fn str_unchecked(&self, span: StrSpan) -> &str {
        let bytes = self.0.get(span);
        debug_assert!(core::str::from_utf8(bytes).is_ok());
        // SAFETY: `span` was read directly from a live row of the same `NodeStore` that owns
        // this `StrArena`, every string written to the row entered via `&str` in `put_str`, and
        // `&self` prevents any concurrent mutation/recycling while the returned `&str` lives.
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }

    /// Checked UTF-8 lookup for a detached [`StrSpan`] (never UB even on stale or foreign spans).
    #[inline]
    pub fn try_str(&self, span: StrSpan) -> Option<&str> {
        let bytes = self.0.try_get(span)?;
        core::str::from_utf8(bytes).ok()
    }

    /// Checked UTF-8 lookup for a detached [`StrSpan`], returning `""` if stale or invalid.
    #[inline]
    pub fn str(&self, span: StrSpan) -> &str {
        self.try_str(span).unwrap_or("")
    }

    /// Replaces `span` with `text` if its bytes differ (no-op when equal).
    #[inline]
    pub(crate) fn replace_str(&mut self, span: StrSpan, text: &str) -> StrSpan {
        if self.0.get(span) == text.as_bytes() {
            return span;
        }
        self.0.replace(span, text.as_bytes())
    }

    #[inline]
    pub(crate) fn free(&mut self, span: StrSpan) {
        self.0.free(span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_capacity_is_next_power_of_two_with_floor() {
        assert_eq!(class_cap(1), 4);
        assert_eq!(class_cap(4), 4);
        assert_eq!(class_cap(5), 8);
        assert_eq!(class_cap(8), 8);
        assert_eq!(class_cap(9), 16);
        assert_eq!(class_cap(17), 32);
    }

    #[test]
    fn replace_reuses_slots_within_class_and_recycles_across_classes() {
        let mut p: SpanPool<u32> = SpanPool::default();
        let a = p.replace(Span::EMPTY, &[1, 2, 3]);
        assert_eq!(p.get(a), &[1, 2, 3]);
        let a2 = p.replace(a, &[4, 5, 6, 7]);
        assert_eq!(a2.off(), a.off(), "same class must reuse in place");
        assert_eq!(p.get(a2), &[4, 5, 6, 7]);

        let a3 = p.replace(a2, &[1, 2, 3, 4, 5]);
        assert_ne!(a3.off(), a2.off(), "larger class allocates a new slot");
        assert_eq!(p.live(), 5);

        let b = p.replace(Span::EMPTY, &[9, 8]);
        assert_eq!(b.off(), a2.off(), "freed 4-slot span must be recycled");
        assert_eq!(p.live(), 7);
    }

    #[test]
    fn insert_at_and_remove_at_preserve_order_across_classes() {
        let mut p: SpanPool<u32> = SpanPool::default();
        let mut s = Span::EMPTY;
        for i in 0..6u32 {
            s = p.insert_at(s, i as usize, i * 10);
        }
        assert_eq!(p.get(s), &[0, 10, 20, 30, 40, 50]);
        s = p.insert_at(s, 2, 15);
        assert_eq!(p.get(s), &[0, 10, 15, 20, 30, 40, 50]);
        s = p.remove_at(s, 2);
        assert_eq!(p.get(s), &[0, 10, 20, 30, 40, 50]);
        s = p.remove_at(s, 0);
        s = p.remove_at(s, 0);
        assert_eq!(p.get(s), &[20, 30, 40, 50]);
        assert_eq!(p.live(), 4);
    }

    #[test]
    fn str_arena_skips_identical_writes_and_reads_utf8_zero_cost() {
        let mut s = StrArena::default();
        let a = s.replace_str(StrSpan::EMPTY, "hello 🚀");
        let b = s.replace_str(a, "hello 🚀");
        assert_eq!(a, b);
        assert_eq!(s.str(b), "hello 🚀");
    }
}
