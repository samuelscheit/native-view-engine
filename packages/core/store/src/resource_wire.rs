//! Wire-replica ingestion (`define_at`), iterative cycle reachability, and frame-boundary
//! housekeeping for [`ResourceStore`].

use std::sync::OnceLock;

use nve_abi::Key64;
use nve_hash::xxh64;

use crate::id::ResourceId;
use crate::span_pool::{Span, MAX_SPAN_LEN};

use super::{
    Entry, ResourceStore, MAX_RESOURCE_WORDS, RES_CHILDREN_HELD, RES_CREATED_THIS_FRAME,
    RES_DEFINED_THIS_FRAME, RES_EPHEMERAL, RES_IN_FREE, RES_OCCUPIED, RES_PENDING_FREE,
    RES_RELEASED_THIS_FRAME,
};

impl ResourceStore {
    /// Single-epoch multi-source iterative cycle check over `self.child_scratch`.
    ///
    /// Returns `true` iff any node in `self.child_scratch` reaches `target_idx` OR the per-frame
    /// `budget` of inspected edges is exhausted (bounding worst-case CPU on hostile wire
    /// redefinitions over dense DAGs). Each node is marked before it is pushed, so it is expanded
    /// at most once.
    pub(super) fn reaches_child_scratch_budgeted(
        &mut self,
        target_idx: usize,
        budget: &mut usize,
    ) -> bool {
        if target_idx == 0 || self.child_scratch.is_empty() {
            return false;
        }
        self.visit_epoch = self.visit_epoch.wrapping_add(1);
        if self.visit_epoch == 0 {
            self.visit_marks.fill(0);
            self.visit_epoch = 1;
        }
        if self.visit_marks.len() < self.entries.len() {
            self.visit_marks.resize(self.entries.len(), 0);
        }
        let epoch = self.visit_epoch;
        let Self {
            entries,
            children,
            child_scratch,
            work_stack,
            visit_marks,
            ..
        } = self;
        work_stack.clear();
        // Inspects one edge: `true` when it reaches `target_idx` or exhausts `budget`; otherwise
        // marks and schedules an unvisited live child.
        let mut edge = |kid: ResourceId, work_stack: &mut Vec<ResourceId>| -> bool {
            let Some(left) = budget.checked_sub(1) else {
                return true;
            };
            *budget = left;
            let idx = kid.index();
            if idx == target_idx {
                return true;
            }
            let live = entries.get(idx).is_some_and(|e| {
                (e.flags & RES_OCCUPIED) != 0 && e.generation == kid.generation()
            });
            if live && !kid.is_none() && visit_marks[idx] != epoch {
                visit_marks[idx] = epoch;
                work_stack.push(kid);
            }
            false
        };
        for &cid in child_scratch.iter() {
            if edge(cid, work_stack) {
                return true;
            }
        }
        while let Some(cur) = work_stack.pop() {
            let span = entries[cur.index()].children;
            for &kid in children.get(span) {
                if edge(kid, work_stack) {
                    return true;
                }
            }
        }
        false
    }

    /// Number of resource slots (live, free, and the reserved slot `0`).
    pub(crate) fn slots(&self) -> usize {
        self.entries.len()
    }

    /// Whether `id` holds a defined payload (occupied, matching generation, not yet recycled).
    pub(crate) fn is_defined(&self, id: ResourceId) -> bool {
        self.entry(id).is_some()
    }

    pub(crate) fn is_ephemeral(&self, id: ResourceId) -> bool {
        self.entry(id)
            .is_some_and(|e| (e.flags & RES_EPHEMERAL) != 0)
    }

    /// Convenience wrapper for tests calling `define_at_with_budget` directly.
    #[cfg(test)]
    pub(crate) fn define_at(
        &mut self,
        id: ResourceId,
        kind: Key64,
        ephemeral: bool,
        child_ids: &[ResourceId],
        bytes: &[u8],
    ) -> bool {
        let mut budget = 1_000_000;
        self.define_at_with_budget(id, kind, ephemeral, child_ids, bytes, &mut budget)
    }

    /// Mirrors a resource defined by an `NVE2` wire frame at exactly `id` (replica stores).
    ///
    /// Rejects `index == 0`, non-4-byte-aligned payloads, length overflow, null/missing child ids,
    /// live generation mismatches, and cyclic (or cycle-budget-exceeding) child graphs.
    pub(crate) fn define_at_with_budget(
        &mut self,
        id: ResourceId,
        kind: Key64,
        ephemeral: bool,
        child_ids: &[ResourceId],
        bytes: &[u8],
        cycle_budget: &mut usize,
    ) -> bool {
        let i = id.index();
        let g = id.generation();
        if i == 0 || !bytes.len().is_multiple_of(4) {
            return false;
        }
        let word_len = bytes.len() / 4;
        if word_len > MAX_RESOURCE_WORDS || child_ids.len() > MAX_SPAN_LEN as usize {
            return false;
        }

        // Validate every child dependency before mutating any state.
        self.child_scratch.clear();
        for &cid in child_ids {
            if cid.is_none() || cid.index() == i || !self.is_defined(cid) {
                return false;
            }
            self.child_scratch.push(cid);
        }
        self.child_scratch.sort_unstable();
        self.child_scratch.dedup();
        if i < self.entries.len() && (self.entries[i].flags & RES_OCCUPIED) != 0 {
            if self.entries[i].generation != g {
                return false;
            }
            if self.reaches_child_scratch_budgeted(i, cycle_budget) {
                return false;
            }
        }

        self.scratch.clear();
        self.scratch.reserve(word_len);
        for &chunk in bytes.as_chunks::<4>().0 {
            self.scratch.push(u32::from_le_bytes(chunk));
        }

        let old_len = self.entries.len();
        if old_len <= i {
            self.entries.resize_with(i + 1, Entry::default);
            for skipped in old_len..i {
                self.entries[skipped].flags |= RES_IN_FREE;
                self.free.push_back(skipped as u32);
            }
        }
        let prev_flags = self.entries[i].flags;
        let prev_refs = self.entries[i].refs;
        let prev_kind = self.entries[i].kind;
        let prev_hash = self.entries[i].hash;
        let prev_gen = self.entries[i].generation;
        let prev_span = self.entries[i].span;
        let prev_children = self.entries[i].children;
        let prev_rid = ResourceId::new(i as u32, prev_gen);

        if (prev_flags & (RES_OCCUPIED | RES_EPHEMERAL)) == RES_OCCUPIED
            && self.dedup.get(&(prev_kind.0, prev_hash)).copied() == Some(prev_rid)
        {
            self.dedup.remove(&(prev_kind.0, prev_hash));
        }

        // If redefining a live entry (`refs > 0`), retain new children BEFORE releasing old ones.
        if prev_refs > 0 {
            for idx in 0..self.child_scratch.len() {
                let cid = self.child_scratch[idx];
                self.retain(cid);
            }
            self.old_children_scratch.clear();
            self.old_children_scratch
                .extend_from_slice(self.children.get(prev_children));
        }

        let new_span = self.words.replace(prev_span, &self.scratch);
        let new_children = self.children.replace(prev_children, &self.child_scratch);

        let already_defined = (prev_flags & RES_DEFINED_THIS_FRAME) != 0;
        let was_pending = (prev_flags & RES_PENDING_FREE) != 0;
        let preserved = prev_flags & (RES_IN_FREE | RES_PENDING_FREE | RES_RELEASED_THIS_FRAME);
        let hash = if ephemeral { 0 } else { xxh64(bytes, kind.0) };
        let mut flags = RES_OCCUPIED | RES_DEFINED_THIS_FRAME | preserved;
        if ephemeral {
            flags |= RES_EPHEMERAL;
        }
        if prev_refs == 0 {
            flags |= RES_PENDING_FREE;
        } else {
            flags |= RES_CHILDREN_HELD;
        }

        self.entries[i] = Entry {
            kind,
            validated_type: OnceLock::new(),
            span: new_span,
            children: new_children,
            refs: prev_refs,
            hash,
            generation: g,
            flags,
        };

        if prev_refs > 0 && (prev_flags & RES_CHILDREN_HELD) != 0 {
            for idx in 0..self.old_children_scratch.len() {
                let cid = self.old_children_scratch[idx];
                self.release(cid);
            }
        }
        if prev_refs == 0 && !was_pending {
            self.pending_free.push(i as u32);
        }
        if !already_defined {
            self.defined.push(id);
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn free_len(&self) -> usize {
        self.free.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_free_len(&self) -> usize {
        self.pending_free.len()
    }

    /// Frees slots released during the previous frame, bumping each freed slot's generation to
    /// prevent ABA reuse and returning their spans to the `SpanPool`s.
    pub(crate) fn end_frame(&mut self) {
        for &id in &self.defined {
            if let Some(e) = self.entries.get_mut(id.index()) {
                e.flags &= !(RES_DEFINED_THIS_FRAME | RES_CREATED_THIS_FRAME);
            }
        }
        for &id in &self.released {
            if let Some(e) = self.entries.get_mut(id.index()) {
                e.flags &= !RES_RELEASED_THIS_FRAME;
            }
        }
        for i in self.pending_free.drain(..) {
            let e = &mut self.entries[i as usize];
            e.flags &= !(RES_PENDING_FREE | RES_CREATED_THIS_FRAME | RES_RELEASED_THIS_FRAME);
            if e.refs == 0 {
                let cur_rid = ResourceId::new(i, e.generation);
                if (e.flags & (RES_OCCUPIED | RES_EPHEMERAL)) == RES_OCCUPIED
                    && self.dedup.get(&(e.kind.0, e.hash)).copied() == Some(cur_rid)
                {
                    self.dedup.remove(&(e.kind.0, e.hash));
                }
                if (e.flags & RES_OCCUPIED) != 0 {
                    e.generation = e.generation.wrapping_add(1);
                }
                let span = e.span;
                let children = e.children;
                e.span = Span::EMPTY;
                e.children = Span::EMPTY;
                e.validated_type = OnceLock::new();
                e.kind = Key64(0);
                e.hash = 0;
                e.flags &= !(RES_OCCUPIED | RES_EPHEMERAL | RES_CHILDREN_HELD);
                self.words.free(span);
                self.children.free(children);
                if (e.flags & RES_IN_FREE) == 0 {
                    e.flags |= RES_IN_FREE;
                    self.free.push_back(i);
                }
            }
        }
        self.defined.clear();
        self.released.clear();
    }
}
