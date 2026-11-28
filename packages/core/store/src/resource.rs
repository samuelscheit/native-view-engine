//! Flat-buffer [`ResourceStore`]: refcounted, `xxh64`-deduplicated 4-byte-aligned word blobs
//! backed directly by two contiguous [`SpanPool`] slabs (`SpanPool<u32>` for payload words and
//! `SpanPool<ResourceId>` for transitive child resource references).
//!
//! Zero `Arc<dyn Any>`, zero `downcast_ref`, zero global `CODECS` registry, zero `Box<[u8]>`.
//! In-process readers borrow typed zero-copy `View<'a>` handles directly over the `u32` slab;
//! `NVE2` wire framing copies the exact slab words verbatim.

use std::any::TypeId;
use std::collections::VecDeque;
use std::sync::OnceLock;

use nve_abi::Key64;
use nve_hash::xxh64;
use rustc_hash::FxHashMap;

use crate::id::ResourceId;
use crate::resource_ingress::ValidatedWords;
use crate::span_pool::{Span, SpanPool, MAX_SPAN_LEN};

#[path = "resource_wire.rs"]
mod wire;

const _: () = assert!(
    cfg!(target_endian = "little"),
    "nve-store flat u32 ResourceStore requires a little-endian target"
);

/// Maximum `u32` words permitted in a single flat resource payload (`64 MiB`, well below
/// `u32::MAX / 4` so `NVE2` `u32` byte-length framing can never overflow).
pub const MAX_RESOURCE_WORDS: usize = 16 * 1024 * 1024;

const _: () = assert!(MAX_RESOURCE_WORDS <= (MAX_SPAN_LEN as usize) / 4);

const RES_PENDING_FREE: u16 = 1 << 0;
const RES_IN_FREE: u16 = 1 << 1;
const RES_DEFINED_THIS_FRAME: u16 = 1 << 2;
const RES_CREATED_THIS_FRAME: u16 = 1 << 3;
const RES_RELEASED_THIS_FRAME: u16 = 1 << 4;
const RES_OCCUPIED: u16 = 1 << 5;
const RES_EPHEMERAL: u16 = 1 << 6;
const RES_CHILDREN_HELD: u16 = 1 << 7;

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
/// [`Self::view`] receives [`ValidatedWords<'a, Self>`], which can only be constructed after
/// [`Self::validate`] returned `true`. Implementations may elide redundant per-read UTF-8 scans
/// only on spans explicitly verified inside [`Self::validate`]. [`ResourceStore::get`] validates
/// wire-origin entries once (caching the validated [`TypeId`] in an atomic [`OnceLock`]) and
/// verifies every child [`ResourceId`] reported by [`Self::child_resources`] is tracked in the
/// entry's refcounted child span before returning `Some(Self::view(words))`.
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

    /// Constructs a zero-copy typed view over ingress-validated `words`.
    fn view<'a>(words: ValidatedWords<'a, Self>) -> Self::View<'a>;

    /// Visits every non-none nested [`ResourceId`] embedded in `words` so [`ResourceStore`] can
    /// retain/release transitive dependencies and topologically order `NVE2` `DefineResources`.
    #[inline]
    fn child_resources(_words: &[u32], _visit: &mut dyn FnMut(ResourceId)) {}
}

pub(crate) struct Entry {
    kind: Key64,
    validated_type: OnceLock<TypeId>,
    span: Span<u32>,
    children: Span<ResourceId>,
    refs: u32,
    hash: u64,
    generation: u8,
    flags: u16,
}

impl Default for Entry {
    #[inline]
    fn default() -> Self {
        Self {
            kind: Key64(0),
            validated_type: OnceLock::new(),
            span: Span::EMPTY,
            children: Span::EMPTY,
            refs: 0,
            hash: 0,
            generation: 0,
            flags: 0,
        }
    }
}

/// Flat-buffer refcounted resource store. Index `0` is reserved for [`ResourceId::NONE`].
pub struct ResourceStore {
    entries: Vec<Entry>,
    words: SpanPool<u32>,
    children: SpanPool<ResourceId>,
    dedup: FxHashMap<(u64, u64), ResourceId>,
    /// FIFO so a freed slot is reused as late as possible, maximizing the window in which a stale
    /// `ResourceId` still meets its own (bumped) generation instead of a recycled one.
    free: VecDeque<u32>,
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
            free: VecDeque::new(),
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

    #[inline(always)]
    pub(crate) fn entry(&self, id: ResourceId) -> Option<&Entry> {
        let i = id.index();
        if i == 0 {
            return None;
        }
        let e = self.entries.get(i)?;
        ((e.flags & RES_OCCUPIED) != 0 && e.generation == id.generation()).then_some(e)
    }

    #[inline(always)]
    pub(crate) fn entry_mut(&mut self, id: ResourceId) -> Option<&mut Entry> {
        let i = id.index();
        if i == 0 {
            return None;
        }
        let e = self.entries.get_mut(i)?;
        ((e.flags & RES_OCCUPIED) != 0 && e.generation == id.generation()).then_some(e)
    }

    fn slot(&mut self) -> Option<ResourceId> {
        while let Some(i) = self.free.pop_front() {
            if let Some(e) = self.entries.get_mut(i as usize) {
                let was_in_free = e.flags & RES_IN_FREE != 0;
                e.flags &= !RES_IN_FREE;
                if was_in_free
                    && e.refs == 0
                    && (e.flags & (RES_OCCUPIED | RES_PENDING_FREE)) == 0
                {
                    return Some(ResourceId::new(i, e.generation));
                }
            }
        }
        if self.entries.len() > ResourceId::MAX_INDEX as usize {
            return None;
        }
        let next = self.entries.len() as u32;
        self.entries.push(Entry::default());
        Some(ResourceId::new(next, 0))
    }

    fn finish_intern_scratch<R: FlatResource>(&mut self) -> Option<ResourceId> {
        if self.scratch.len() > MAX_RESOURCE_WORDS || !R::validate(&self.scratch) {
            return None;
        }
        let hash = xxh64(words_as_le_bytes(&self.scratch), R::KIND.0);
        let tid = TypeId::of::<R>();
        if let Some(&rid) = self.dedup.get(&(R::KIND.0, hash))
            && let Some(e) = self.entries.get_mut(rid.index())
            && (e.flags & (RES_OCCUPIED | RES_EPHEMERAL)) == RES_OCCUPIED
            && e.generation == rid.generation()
            && e.refs > 0
            && e.kind == R::KIND
            && e.validated_type.get() == Some(&tid)
            && self.words.get(e.span) == self.scratch.as_slice()
        {
            e.refs = e.refs.checked_add(1)?;
            return Some(rid);
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

        let rid = self.slot()?;
        // Retain every referenced child before populating the new parent slot.
        for idx in 0..self.child_scratch.len() {
            let cid = self.child_scratch[idx];
            self.retain(cid);
        }

        let span = self.words.replace(Span::EMPTY, &self.scratch);
        let child_span = self.children.replace(Span::EMPTY, &self.child_scratch);
        self.entries[rid.index()] = Entry {
            kind: R::KIND,
            validated_type: OnceLock::from(tid),
            span,
            children: child_span,
            refs: 1,
            hash,
            generation: rid.generation(),
            flags: RES_OCCUPIED
                | RES_CHILDREN_HELD
                | RES_DEFINED_THIS_FRAME
                | RES_CREATED_THIS_FRAME,
        };
        self.dedup.entry((R::KIND.0, hash)).or_insert(rid);
        self.defined.push(rid);
        Some(rid)
    }

    fn finish_ephemeral_scratch<R: FlatResource>(&mut self, old: ResourceId) -> Option<ResourceId> {
        if self.scratch.len() > MAX_RESOURCE_WORDS || !R::validate(&self.scratch) {
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
        let can_reuse = self.entry(old).is_some_and(|e| {
            (e.flags & RES_EPHEMERAL) != 0
                && e.refs == 1
                && e.kind == R::KIND
                && e.validated_type.get() == Some(&tid)
        });

        if can_reuse {
            let mut budget = usize::MAX;
            let creates_cycle = self.reaches_child_scratch_budgeted(old.index(), &mut budget);
            if !creates_cycle {
                // Retain new children BEFORE releasing old children so shared children never hit 0.
                for idx in 0..self.child_scratch.len() {
                    let cid = self.child_scratch[idx];
                    self.retain(cid);
                }
                let old_idx = old.index();
                let old_span = self.entries[old_idx].span;
                let old_children = self.entries[old_idx].children;
                let old_flags = self.entries[old_idx].flags;
                self.old_children_scratch.clear();
                self.old_children_scratch
                    .extend_from_slice(self.children.get(old_children));

                let new_span = self.words.replace(old_span, &self.scratch);
                let new_children = self.children.replace(old_children, &self.child_scratch);
                let e = &mut self.entries[old_idx];
                e.span = new_span;
                e.children = new_children;
                e.flags |= RES_OCCUPIED | RES_EPHEMERAL | RES_CHILDREN_HELD;
                if (e.flags & RES_DEFINED_THIS_FRAME) == 0 {
                    e.flags |= RES_DEFINED_THIS_FRAME;
                    self.defined.push(old);
                }
                if (old_flags & RES_CHILDREN_HELD) != 0 {
                    for idx in 0..self.old_children_scratch.len() {
                        let cid = self.old_children_scratch[idx];
                        self.release(cid);
                    }
                }
                return Some(old);
            }
        }

        // Allocate a fresh ephemeral slot (caller releases `old` via `swap_resource` if desired).
        let rid = self.slot()?;
        for idx in 0..self.child_scratch.len() {
            let cid = self.child_scratch[idx];
            self.retain(cid);
        }
        let span = self.words.replace(Span::EMPTY, &self.scratch);
        let child_span = self.children.replace(Span::EMPTY, &self.child_scratch);
        self.entries[rid.index()] = Entry {
            kind: R::KIND,
            validated_type: OnceLock::from(tid),
            span,
            children: child_span,
            refs: 1,
            hash: 0,
            generation: rid.generation(),
            flags: RES_OCCUPIED
                | RES_EPHEMERAL
                | RES_CHILDREN_HELD
                | RES_DEFINED_THIS_FRAME
                | RES_CREATED_THIS_FRAME,
        };
        self.defined.push(rid);
        Some(rid)
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
        let Some(e) = self.entry_mut(id) else {
            return;
        };
        if e.refs > 0 {
            debug_assert!(e.refs < u32::MAX, "ResourceStore: refcount saturated");
            e.refs = e.refs.saturating_add(1);
            return;
        }
        // Slow path: `0 -> 1` transition may need to retain transitive children iteratively.
        self.work_stack.clear();
        self.work_stack.push(id);
        while let Some(cur) = self.work_stack.pop() {
            let Some(entry) = self.entry_mut(cur) else {
                continue;
            };
            let was_zero = entry.refs == 0;
            debug_assert!(entry.refs < u32::MAX, "ResourceStore: refcount saturated");
            entry.refs = entry.refs.saturating_add(1);
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
            let idx = cur.index();
            if idx == 0 {
                continue;
            }
            let Some(e) = self.entries.get_mut(idx) else {
                continue;
            };
            if (e.flags & RES_OCCUPIED) == 0 || e.generation != cur.generation() {
                continue;
            }
            debug_assert!(e.refs > 0, "resource {cur:?} over-released");
            if e.refs == 0 || e.refs == u32::MAX {
                continue;
            }
            e.refs -= 1;
            if e.refs == 0 {
                if (e.flags & RES_EPHEMERAL) == 0
                    && self.dedup.get(&(e.kind.0, e.hash)).copied() == Some(cur)
                {
                    self.dedup.remove(&(e.kind.0, e.hash));
                }
                if (e.flags & RES_PENDING_FREE) == 0 {
                    e.flags |= RES_PENDING_FREE;
                    self.pending_free.push(cur.index() as u32);
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
    /// validated once via `R::validate` + child-span check and cached atomically in `Entry` so
    /// subsequent reads never re-scan payloads.
    #[inline]
    pub fn get<R: FlatResource>(&self, id: ResourceId) -> Option<R::View<'_>> {
        let e = self.entry(id)?;
        if e.kind != R::KIND {
            return None;
        }
        let words = self.words.get(e.span);
        let tid = TypeId::of::<R>();
        if let Some(&cached) = e.validated_type.get() {
            if cached != tid {
                return None;
            }
        } else {
            if !self.validate_wire_entry::<R>(e, words) {
                return None;
            }
            if *e.validated_type.get_or_init(|| tid) != tid {
                return None;
            }
        }
        Some(R::view(ValidatedWords::new_unchecked(words)))
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

    /// Raw flat `u32` words of `id` (if occupied with matching generation).
    #[inline]
    pub fn raw_words(&self, id: ResourceId) -> Option<&[u32]> {
        let e = self.entry(id)?;
        Some(self.words.get(e.span))
    }

    /// Little-endian byte view of `id`'s flat words: the exact payload an `NVE2`
    /// `DefineResources` entry carries and foreign hosts borrow (zero-copy; `None` for
    /// `ResourceId::NONE`, released, recycled, or stale-generation ids). Valid until the store is
    /// next mutated.
    #[inline]
    pub fn payload_bytes(&self, id: ResourceId) -> Option<&[u8]> {
        let e = self.entry(id)?;
        (e.refs > 0).then(|| words_as_le_bytes(self.words.get(e.span)))
    }

    /// Sorted, deduplicated child [`ResourceId`] dependencies owned by `id`.
    #[inline]
    pub fn children_of(&self, id: ResourceId) -> &[ResourceId] {
        match self.entry(id) {
            Some(e) => self.children.get(e.children),
            None => &[],
        }
    }

    pub fn kind(&self, id: ResourceId) -> Option<Key64> {
        self.entry(id).map(|e| e.kind)
    }

    #[inline]
    pub fn is_live(&self, id: ResourceId) -> bool {
        self.entry(id).is_some_and(|e| e.refs > 0)
    }

    /// Returns `true` iff `id` is currently live (`refs > 0`, generation matches) and compatible
    /// with resource type `R` (matching `R::KIND` and any sealed `TypeId`).
    #[inline]
    pub fn is_live_of<R: FlatResource>(&self, id: ResourceId) -> bool {
        self.entry(id).is_some_and(|e| {
            e.refs > 0
                && e.kind == R::KIND
                && e.validated_type.get().is_none_or(|&t| t == TypeId::of::<R>())
        })
    }

    /// Returns `true` iff `id` is currently live (`refs > 0`, generation matches) with `kind`.
    #[inline]
    pub fn is_live_with_kind(&self, id: ResourceId, kind: Key64) -> bool {
        self.entry(id)
            .is_some_and(|e| e.refs > 0 && e.kind == kind)
    }

    /// Number of live resources.
    pub fn live(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| (e.flags & RES_OCCUPIED) != 0 && e.refs > 0)
            .count()
    }
}
