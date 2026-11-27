//! Flat-buffer [`ResourceStore`]: refcounted, `xxh64`-deduplicated 4-byte-aligned word blobs
//! backed directly by two contiguous [`SpanPool`] slabs (`SpanPool<u32>` for payload words and
//! `SpanPool<ResourceId>` for transitive child resource references).
//!
//! Zero `Arc<dyn Any>`, zero `downcast_ref`, zero global `CODECS` registry, zero `Box<[u8]>`.
//! In-process readers borrow typed zero-copy `View<'a>` handles directly over the `u32` slab;
//! `NVE2` wire framing copies the exact slab words verbatim.

use std::any::TypeId;

use nve_abi::Key64;
use nve_hash::xxh64;
use rustc_hash::FxHashMap;

use crate::id::ResourceId;
use crate::span_pool::{Span, SpanPool, MAX_SPAN_LEN};

#[path = "resource_wire.rs"]
mod wire;

const _: () = assert!(
    cfg!(target_endian = "little"),
    "nve-store flat u32 ResourceStore requires a little-endian target"
);

const RES_PENDING_FREE: u8 = 1 << 0;
const RES_IN_FREE: u8 = 1 << 1;
const RES_DEFINED_THIS_FRAME: u8 = 1 << 2;
const RES_CREATED_THIS_FRAME: u8 = 1 << 3;
const RES_RELEASED_THIS_FRAME: u8 = 1 << 4;
const RES_OCCUPIED: u8 = 1 << 5;
const RES_VALIDATED: u8 = 1 << 6;
const RES_CHILDREN_HELD: u8 = 1 << 7;

/// Zero-copy little-endian byte view of a `u32` word slice (used for `xxh64` hashing, wire copy,
/// and length-prefixed byte/UTF-8 payloads).
#[inline(always)]
pub fn words_as_le_bytes(words: &[u32]) -> &[u8] {
    // SAFETY: Compile-time assert guarantees little-endian; `u32` has no uninitialized padding
    // bytes, `u8` has alignment 1, and `core::mem::size_of_val(words) <= isize::MAX`.
    unsafe {
        core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), core::mem::size_of_val(words))
    }
}

/// A 4-byte-aligned flat-word blob stored directly inside [`ResourceStore`]'s contiguous
/// `SpanPool<u32>` slab.
///
/// # Soundness Contract
///
/// Every [`Self::view`] implementation MUST be 100% safe Rust (no unchecked indexing or
/// `from_utf8_unchecked`) so even direct calls on arbitrary slices can never cause UB.
/// [`ResourceStore::get`] validates wire-origin entries with [`Self::validate`] and verifies that
/// every child [`ResourceId`] reported by [`Self::child_resources`] is tracked in the entry's
/// refcounted child span before returning `Some(Self::view(words))`.
pub trait FlatResource: 'static {
    /// Stable kind identity (`Key64::of("nve.res.box_shadows")`).
    const KIND: Key64;

    /// Zero-copy borrowed view directly over the flat `u32` slab words.
    type View<'a>;

    /// Serializes `self` into flat `u32` words in `out` (returning `false` if an encode-side limit
    /// is exceeded).
    fn write_words(&self, out: &mut Vec<u32>) -> bool;

    /// Validates tags, lengths, bounds, UTF-8, and nested payloads of `words`.
    fn validate(words: &[u32]) -> bool;

    /// Constructs a zero-copy typed view over already-validated `words`.
    fn view<'a>(words: &'a [u32]) -> Self::View<'a>;

    /// Visits every non-none nested [`ResourceId`] embedded in `words` so [`ResourceStore`] can
    /// retain/release transitive dependencies and topologically order `NVE2` `DefineResources`.
    #[inline]
    fn child_resources(_words: &[u32], _visit: &mut dyn FnMut(ResourceId)) {}
}

#[derive(Clone, Copy)]
struct Entry {
    kind: Key64,
    type_id: Option<TypeId>,
    span: Span<u32>,
    children: Span<ResourceId>,
    refs: u32,
    hash: u64,
    ephemeral: bool,
    flags: u8,
}

impl Default for Entry {
    #[inline]
    fn default() -> Self {
        Self {
            kind: Key64(0),
            type_id: None,
            span: Span::EMPTY,
            children: Span::EMPTY,
            refs: 0,
            hash: 0,
            ephemeral: false,
            flags: 0,
        }
    }
}

/// Flat-buffer refcounted resource store. Index `0` is reserved for [`ResourceId::NONE`].
pub struct ResourceStore {
    entries: Vec<Entry>,
    words: SpanPool<u32>,
    children: SpanPool<ResourceId>,
    dedup: FxHashMap<(u64, u64), u32>,
    free: Vec<u32>,
    pending_free: Vec<u32>,
    pub(crate) defined: Vec<ResourceId>,
    pub(crate) released: Vec<ResourceId>,
    scratch: Vec<u32>,
    child_scratch: Vec<ResourceId>,
    old_children_scratch: Vec<ResourceId>,
    work_stack: Vec<ResourceId>,
    visit_marks: Vec<u32>,
    visit_epoch: u32,
}

impl Default for ResourceStore {
    fn default() -> Self {
        Self {
            entries: vec![Entry::default()],
            words: SpanPool::default(),
            children: SpanPool::default(),
            dedup: FxHashMap::default(),
            free: Vec::new(),
            pending_free: Vec::new(),
            defined: Vec::new(),
            released: Vec::new(),
            scratch: Vec::new(),
            child_scratch: Vec::new(),
            old_children_scratch: Vec::new(),
            work_stack: Vec::new(),
            visit_marks: Vec::new(),
            visit_epoch: 0,
        }
    }
}

impl ResourceStore {
    /// Whole contiguous payload word slab (publishable zero-copy).
    #[inline(always)]
    pub fn word_slab(&self) -> &[u32] {
        self.words.data()
    }

    /// Number of live `u32` words currently occupied in the payload slab.
    #[inline(always)]
    pub fn words_live(&self) -> usize {
        self.words.live()
    }

    /// Number of live child `ResourceId` edges currently occupied in the child slab.
    #[inline(always)]
    pub fn child_edges_live(&self) -> usize {
        self.children.live()
    }

    fn slot(&mut self) -> u32 {
        while let Some(i) = self.free.pop() {
            if let Some(e) = self.entries.get_mut(i as usize) {
                let was_in_free = e.flags & RES_IN_FREE != 0;
                e.flags &= !RES_IN_FREE;
                if was_in_free
                    && e.refs == 0
                    && (e.flags & (RES_OCCUPIED | RES_PENDING_FREE)) == 0
                {
                    return i;
                }
            }
        }
        let next = u32::try_from(self.entries.len()).expect("ResourceStore: slot count exceeds u32::MAX");
        self.entries.push(Entry::default());
        next
    }

    fn finish_intern_scratch<R: FlatResource>(&mut self) -> Option<ResourceId> {
        if self.scratch.len() > MAX_SPAN_LEN as usize || !R::validate(&self.scratch) {
            return None;
        }
        let hash = xxh64(words_as_le_bytes(&self.scratch), R::KIND.0);
        let tid = TypeId::of::<R>();
        if let Some(&i) = self.dedup.get(&(R::KIND.0, hash))
            && let Some(e) = self.entries.get_mut(i as usize)
            && (e.flags & RES_OCCUPIED) != 0
            && !e.ephemeral
            && e.refs > 0
            && e.kind == R::KIND
            && e.type_id == Some(tid)
            && self.words.get(e.span) == self.scratch.as_slice()
        {
            e.refs = e.refs.checked_add(1)?;
            return Some(ResourceId(i));
        }

        // Collect, sort, dedup, and validate every child resource edge.
        let Self {
            scratch,
            child_scratch,
            ..
        } = self;
        child_scratch.clear();
        R::child_resources(scratch, &mut |cid| {
            if !cid.is_none() {
                child_scratch.push(cid);
            }
        });
        child_scratch.sort_unstable();
        child_scratch.dedup();
        if child_scratch.len() > MAX_SPAN_LEN as usize {
            return None;
        }
        for &cid in &self.child_scratch {
            if !self.is_live(cid) {
                return None;
            }
        }

        // Retain every referenced child before inserting the new parent slot.
        for idx in 0..self.child_scratch.len() {
            let cid = self.child_scratch[idx];
            self.retain(cid);
        }

        let i = self.slot();
        let span = self.words.replace(Span::EMPTY, &self.scratch);
        let child_span = self.children.replace(Span::EMPTY, &self.child_scratch);
        self.entries[i as usize] = Entry {
            kind: R::KIND,
            type_id: Some(tid),
            span,
            children: child_span,
            refs: 1,
            hash,
            ephemeral: false,
            flags: RES_OCCUPIED
                | RES_VALIDATED
                | RES_CHILDREN_HELD
                | RES_DEFINED_THIS_FRAME
                | RES_CREATED_THIS_FRAME,
        };
        self.dedup.entry((R::KIND.0, hash)).or_insert(i);
        self.defined.push(ResourceId(i));
        Some(ResourceId(i))
    }

    fn finish_ephemeral_scratch<R: FlatResource>(&mut self, old: ResourceId) -> Option<ResourceId> {
        if self.scratch.len() > MAX_SPAN_LEN as usize || !R::validate(&self.scratch) {
            return None;
        }
        let Self {
            scratch,
            child_scratch,
            ..
        } = self;
        child_scratch.clear();
        R::child_resources(scratch, &mut |cid| {
            if !cid.is_none() {
                child_scratch.push(cid);
            }
        });
        child_scratch.sort_unstable();
        child_scratch.dedup();
        if child_scratch.len() > MAX_SPAN_LEN as usize {
            return None;
        }
        for &cid in &self.child_scratch {
            if !self.is_live(cid) {
                return None;
            }
        }

        let tid = TypeId::of::<R>();
        let can_reuse = !old.is_none()
            && self.entries.get(old.0 as usize).is_some_and(|e| {
                (e.flags & RES_OCCUPIED) != 0
                    && e.ephemeral
                    && e.refs == 1
                    && e.kind == R::KIND
                    && e.type_id == Some(tid)
            });

        if can_reuse {
            // Fast path: 0 children => cycle impossible.
            let mut creates_cycle = false;
            for idx in 0..self.child_scratch.len() {
                let cid = self.child_scratch[idx];
                if cid == old || self.reaches(cid, old) {
                    creates_cycle = true;
                    break;
                }
            }
            if !creates_cycle {
                // Retain new children BEFORE releasing old children so shared children never hit 0.
                for idx in 0..self.child_scratch.len() {
                    let cid = self.child_scratch[idx];
                    self.retain(cid);
                }
                let old_entry = self.entries[old.0 as usize];
                self.old_children_scratch.clear();
                self.old_children_scratch
                    .extend_from_slice(self.children.get(old_entry.children));

                let new_span = self.words.replace(old_entry.span, &self.scratch);
                let new_children = self
                    .children
                    .replace(old_entry.children, &self.child_scratch);
                let e = &mut self.entries[old.0 as usize];
                e.span = new_span;
                e.children = new_children;
                e.flags |= RES_OCCUPIED | RES_VALIDATED | RES_CHILDREN_HELD;
                if (e.flags & RES_DEFINED_THIS_FRAME) == 0 {
                    e.flags |= RES_DEFINED_THIS_FRAME;
                    self.defined.push(old);
                }
                if (old_entry.flags & RES_CHILDREN_HELD) != 0 {
                    for idx in 0..self.old_children_scratch.len() {
                        let cid = self.old_children_scratch[idx];
                        self.release(cid);
                    }
                }
                return Some(old);
            }
        }

        // Allocate a fresh ephemeral slot (caller releases `old` via `swap_resource` if desired).
        for idx in 0..self.child_scratch.len() {
            let cid = self.child_scratch[idx];
            self.retain(cid);
        }
        let i = self.slot();
        let span = self.words.replace(Span::EMPTY, &self.scratch);
        let child_span = self.children.replace(Span::EMPTY, &self.child_scratch);
        self.entries[i as usize] = Entry {
            kind: R::KIND,
            type_id: Some(tid),
            span,
            children: child_span,
            refs: 1,
            hash: 0,
            ephemeral: true,
            flags: RES_OCCUPIED
                | RES_VALIDATED
                | RES_CHILDREN_HELD
                | RES_DEFINED_THIS_FRAME
                | RES_CREATED_THIS_FRAME,
        };
        self.defined.push(ResourceId(i));
        Some(ResourceId(i))
    }

    /// Interns `words` directly (0 allocations on dedup hit), returning `None` if `R::validate`
    /// fails, `words.len() > MAX_SPAN_LEN`, or any embedded child resource is not live.
    pub fn try_intern_words<R: FlatResource>(&mut self, words: &[u32]) -> Option<ResourceId> {
        self.scratch.clear();
        self.scratch.extend_from_slice(words);
        self.finish_intern_scratch::<R>()
    }

    /// Interns via a closure writing flat words directly into the reusable scratch buffer (0 heap
    /// allocations on dedup hit once scratch is warm).
    pub fn try_intern_with<R: FlatResource>(
        &mut self,
        write: impl FnOnce(&mut Vec<u32>) -> bool,
    ) -> Option<ResourceId> {
        self.scratch.clear();
        if !write(&mut self.scratch) {
            return None;
        }
        self.finish_intern_scratch::<R>()
    }

    /// Interns `value`, returning `None` if encoding exceeds limits, fails validation, or references
    /// a dead child resource.
    pub fn try_intern<R: FlatResource>(&mut self, value: &R) -> Option<ResourceId> {
        self.try_intern_with::<R>(|out| value.write_words(out))
    }

    /// Interns `value`, returning [`ResourceId::NONE`] if validation or encode-side limits fail.
    #[inline]
    pub fn intern<R: FlatResource>(&mut self, value: &R) -> ResourceId {
        self.try_intern(value).unwrap_or(ResourceId::NONE)
    }

    /// Rewrites ephemeral `old` in place via a zero-alloc scratch closure when uniquely owned and
    /// acyclic; otherwise allocates a fresh ephemeral resource.
    pub fn try_set_ephemeral_with<R: FlatResource>(
        &mut self,
        old: ResourceId,
        write: impl FnOnce(&mut Vec<u32>) -> bool,
    ) -> Option<ResourceId> {
        self.scratch.clear();
        if !write(&mut self.scratch) {
            return None;
        }
        self.finish_ephemeral_scratch::<R>(old)
    }

    /// Rewrites ephemeral `old` in place when uniquely owned; otherwise allocates a fresh ephemeral
    /// resource. Returns `None` if encoding/validation fails.
    pub fn try_set_ephemeral<R: FlatResource>(
        &mut self,
        old: ResourceId,
        value: &R,
    ) -> Option<ResourceId> {
        self.try_set_ephemeral_with::<R>(old, |out| value.write_words(out))
    }

    /// Rewrites ephemeral `old` in place (falling back to [`ResourceId::NONE`] on invalid input).
    #[inline]
    pub fn set_ephemeral<R: FlatResource>(&mut self, old: ResourceId, value: &R) -> ResourceId {
        self.try_set_ephemeral(old, value).unwrap_or(ResourceId::NONE)
    }

    /// Adds a reference to `id`. When `refs` transitions `0 -> 1` on a wire-defined resource with
    /// children, iteratively retains all transitive child resources.
    pub fn retain(&mut self, id: ResourceId) {
        if id.is_none() {
            return;
        }
        let Some(e) = self.entries.get_mut(id.0 as usize) else {
            return;
        };
        if (e.flags & RES_OCCUPIED) == 0 {
            return;
        }
        if e.refs > 0 {
            e.refs = e.refs.checked_add(1).expect("ResourceStore: refcount overflow");
            return;
        }
        // Slow path: `0 -> 1` transition may need to retain transitive children iteratively.
        self.work_stack.clear();
        self.work_stack.push(id);
        while let Some(cur) = self.work_stack.pop() {
            let Some(entry) = self.entries.get_mut(cur.0 as usize) else {
                continue;
            };
            if (entry.flags & RES_OCCUPIED) == 0 {
                continue;
            }
            let was_zero = entry.refs == 0;
            entry.refs = entry.refs.checked_add(1).expect("ResourceStore: refcount overflow");
            if was_zero && (entry.flags & RES_CHILDREN_HELD) == 0 {
                entry.flags |= RES_CHILDREN_HELD;
                let kids = entry.children;
                if !kids.is_empty() {
                    self.work_stack.extend_from_slice(self.children.get(kids));
                }
            }
        }
    }

    /// Drops a reference iteratively; when `refs` reaches `0`, cascades releases to transitive
    /// children and schedules slot/span recycling at the next frame boundary.
    pub fn release(&mut self, id: ResourceId) {
        if id.is_none() {
            return;
        }
        self.work_stack.clear();
        self.work_stack.push(id);
        while let Some(cur) = self.work_stack.pop() {
            if cur.is_none() {
                continue;
            }
            let Some(e) = self.entries.get_mut(cur.0 as usize) else {
                continue;
            };
            debug_assert!(e.refs > 0, "resource {cur:?} over-released");
            if e.refs == 0 {
                continue;
            }
            e.refs -= 1;
            if e.refs == 0 {
                if !e.ephemeral
                    && self.dedup.get(&(e.kind.0, e.hash)).copied() == Some(cur.0)
                {
                    self.dedup.remove(&(e.kind.0, e.hash));
                }
                if (e.flags & RES_PENDING_FREE) == 0 {
                    e.flags |= RES_PENDING_FREE;
                    self.pending_free.push(cur.0);
                }
                if (e.flags & RES_CREATED_THIS_FRAME) == 0
                    && (e.flags & RES_RELEASED_THIS_FRAME) == 0
                {
                    e.flags |= RES_RELEASED_THIS_FRAME;
                    self.released.push(cur);
                }
                if (e.flags & RES_CHILDREN_HELD) != 0 {
                    e.flags &= !RES_CHILDREN_HELD;
                    let kids = e.children;
                    if !kids.is_empty() {
                        self.work_stack.extend_from_slice(self.children.get(kids));
                    }
                }
            }
        }
    }

    /// Zero-copy typed view over `id`'s flat `u32` words.
    ///
    /// Local entries were sealed at intern time (`O(1)` slice load); wire-origin entries are
    /// validated via `R::validate` and verified against their refcounted child list before
    /// constructing `R::view`.
    #[inline]
    pub fn get<R: FlatResource>(&self, id: ResourceId) -> Option<R::View<'_>> {
        if id.is_none() {
            return None;
        }
        let e = self.entries.get(id.0 as usize)?;
        if (e.flags & RES_OCCUPIED) == 0 || e.kind != R::KIND {
            return None;
        }
        if e.type_id.is_some_and(|t| t != TypeId::of::<R>()) {
            return None;
        }
        let words = self.words.get(e.span);
        if (e.flags & RES_VALIDATED) == 0 && !self.validate_wire_entry::<R>(e, words) {
            return None;
        }
        Some(R::view(words))
    }

    #[cold]
    fn validate_wire_entry<R: FlatResource>(&self, e: &Entry, words: &[u32]) -> bool {
        if !R::validate(words) {
            return false;
        }
        let tracked = self.children.get(e.children);
        let mut ok = true;
        R::child_resources(words, &mut |cid| {
            if !cid.is_none() && (tracked.binary_search(&cid).is_err() || !self.is_defined(cid)) {
                ok = false;
            }
        });
        ok
    }

    /// Raw flat `u32` words of `id` (if occupied).
    #[inline]
    pub fn raw_words(&self, id: ResourceId) -> Option<&[u32]> {
        if id.is_none() {
            return None;
        }
        let e = self.entries.get(id.0 as usize)?;
        ((e.flags & RES_OCCUPIED) != 0).then(|| self.words.get(e.span))
    }

    /// Sorted, deduplicated child [`ResourceId`] dependencies owned by `id`.
    #[inline]
    pub fn children_of(&self, id: ResourceId) -> &[ResourceId] {
        if id.is_none() {
            return &[];
        }
        match self.entries.get(id.0 as usize) {
            Some(e) if (e.flags & RES_OCCUPIED) != 0 => self.children.get(e.children),
            _ => &[],
        }
    }

    pub fn kind(&self, id: ResourceId) -> Option<Key64> {
        let e = self.entries.get(id.0 as usize)?;
        ((e.flags & RES_OCCUPIED) != 0).then_some(e.kind)
    }

    #[inline]
    pub fn is_live(&self, id: ResourceId) -> bool {
        self.entries
            .get(id.0 as usize)
            .is_some_and(|e| (e.flags & RES_OCCUPIED) != 0 && e.refs > 0)
    }

    /// Number of live resources.
    pub fn live(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| (e.flags & RES_OCCUPIED) != 0 && e.refs > 0)
            .count()
    }
}
