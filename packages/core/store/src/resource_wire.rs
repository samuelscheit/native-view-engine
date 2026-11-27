//! Wire-replica ingestion (`define_at`), iterative cycle reachability, and frame-boundary
//! housekeeping for [`ResourceStore`].

use nve_abi::Key64;
use nve_hash::xxh64;

use crate::id::ResourceId;
use crate::span_pool::{Span, MAX_SPAN_LEN};

use super::{
    Entry, ResourceStore, RES_CHILDREN_HELD, RES_CREATED_THIS_FRAME, RES_DEFINED_THIS_FRAME,
    RES_IN_FREE, RES_OCCUPIED, RES_PENDING_FREE, RES_RELEASED_THIS_FRAME, RES_VALIDATED,
};

impl ResourceStore {
    /// Iterative cycle check: returns `true` iff `start` reaches `target` through `children` edges.
    pub(super) fn reaches(&mut self, start: ResourceId, target: ResourceId) -> bool {
        if start == target {
            return true;
        }
        if start.is_none() || target.is_none() {
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
        self.work_stack.clear();
        self.work_stack.push(start);
        while let Some(cur) = self.work_stack.pop() {
            let idx = cur.0 as usize;
            let Some(e) = self.entries.get(idx) else {
                continue;
            };
            if (e.flags & RES_OCCUPIED) == 0 || self.visit_marks[idx] == epoch {
                continue;
            }
            self.visit_marks[idx] = epoch;
            for &kid in self.children.get(e.children) {
                if kid == target {
                    return true;
                }
                if !kid.is_none() {
                    self.work_stack.push(kid);
                }
            }
        }
        false
    }

    /// Number of resource slots (live, free, and the reserved slot `0`).
    pub(crate) fn slots(&self) -> usize {
        self.entries.len()
    }

    /// Whether `id` holds a defined payload (occupied and not yet recycled).
    pub(crate) fn is_defined(&self, id: ResourceId) -> bool {
        !id.is_none()
            && self
                .entries
                .get(id.0 as usize)
                .is_some_and(|e| (e.flags & RES_OCCUPIED) != 0)
    }

    pub(crate) fn is_ephemeral(&self, id: ResourceId) -> bool {
        self.entries
            .get(id.0 as usize)
            .is_some_and(|e| (e.flags & RES_OCCUPIED) != 0 && e.ephemeral)
    }

    /// Mirrors a resource defined by an `NVE2` wire frame at exactly `id` (replica stores).
    ///
    /// Rejects `id == 0`, non-4-byte-aligned payloads, length overflow, null/missing child ids,
    /// and cyclic child graphs.
    pub(crate) fn define_at(
        &mut self,
        id: ResourceId,
        kind: Key64,
        ephemeral: bool,
        child_ids: &[ResourceId],
        bytes: &[u8],
    ) -> bool {
        let i = id.0 as usize;
        if i == 0 || !bytes.len().is_multiple_of(4) {
            return false;
        }
        let word_len = bytes.len() / 4;
        if word_len > MAX_SPAN_LEN as usize || child_ids.len() > MAX_SPAN_LEN as usize {
            return false;
        }

        // Validate every child dependency before mutating any state.
        self.child_scratch.clear();
        for &cid in child_ids {
            if cid.is_none() || cid == id || !self.is_defined(cid) {
                return false;
            }
            self.child_scratch.push(cid);
        }
        self.child_scratch.sort_unstable();
        self.child_scratch.dedup();
        if i < self.entries.len() && (self.entries[i].flags & RES_OCCUPIED) != 0 {
            for idx in 0..self.child_scratch.len() {
                let cid = self.child_scratch[idx];
                if self.reaches(cid, id) {
                    return false;
                }
            }
        }

        self.scratch.clear();
        self.scratch.reserve(word_len);
        for &chunk in bytes.as_chunks::<4>().0 {
            self.scratch.push(u32::from_le_bytes(chunk));
        }

        while self.entries.len() <= i {
            self.entries.push(Entry::default());
        }
        let prev = self.entries[i];
        if !prev.ephemeral
            && (prev.flags & RES_OCCUPIED) != 0
            && self.dedup.get(&(prev.kind.0, prev.hash)).copied() == Some(id.0)
        {
            self.dedup.remove(&(prev.kind.0, prev.hash));
        }

        // If redefining a live entry (`refs > 0`), retain new children BEFORE releasing old ones.
        let refs = prev.refs;
        if refs > 0 {
            for idx in 0..self.child_scratch.len() {
                let cid = self.child_scratch[idx];
                self.retain(cid);
            }
            self.old_children_scratch.clear();
            self.old_children_scratch
                .extend_from_slice(self.children.get(prev.children));
        }

        let new_span = self.words.replace(prev.span, &self.scratch);
        let new_children = self.children.replace(prev.children, &self.child_scratch);

        let already_defined = (prev.flags & RES_DEFINED_THIS_FRAME) != 0;
        let was_pending = (prev.flags & RES_PENDING_FREE) != 0;
        let preserved = prev.flags & (RES_IN_FREE | RES_PENDING_FREE | RES_RELEASED_THIS_FRAME);
        let hash = if ephemeral { 0 } else { xxh64(bytes, kind.0) };
        let mut flags = RES_OCCUPIED | RES_DEFINED_THIS_FRAME | preserved;
        if refs == 0 {
            flags |= RES_PENDING_FREE;
        } else {
            flags |= RES_CHILDREN_HELD;
        }

        self.entries[i] = Entry {
            kind,
            type_id: None,
            span: new_span,
            children: new_children,
            refs,
            hash,
            ephemeral,
            flags,
        };

        if refs > 0 && (prev.flags & RES_CHILDREN_HELD) != 0 {
            for idx in 0..self.old_children_scratch.len() {
                let cid = self.old_children_scratch[idx];
                self.release(cid);
            }
        }
        if refs == 0 && !was_pending {
            self.pending_free.push(id.0);
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

    /// Frees slots released during the previous frame and returns their spans to the `SpanPool`s.
    pub(crate) fn end_frame(&mut self) {
        for &id in &self.defined {
            if let Some(e) = self.entries.get_mut(id.0 as usize) {
                e.flags &= !(RES_DEFINED_THIS_FRAME | RES_CREATED_THIS_FRAME);
            }
        }
        for &id in &self.released {
            if let Some(e) = self.entries.get_mut(id.0 as usize) {
                e.flags &= !RES_RELEASED_THIS_FRAME;
            }
        }
        for i in self.pending_free.drain(..) {
            let e = &mut self.entries[i as usize];
            e.flags &= !(RES_PENDING_FREE | RES_CREATED_THIS_FRAME | RES_RELEASED_THIS_FRAME);
            if e.refs == 0 {
                if !e.ephemeral
                    && (e.flags & RES_OCCUPIED) != 0
                    && self.dedup.get(&(e.kind.0, e.hash)).copied() == Some(i)
                {
                    self.dedup.remove(&(e.kind.0, e.hash));
                }
                let span = e.span;
                let children = e.children;
                e.span = Span::EMPTY;
                e.children = Span::EMPTY;
                e.type_id = None;
                e.kind = Key64(0);
                e.hash = 0;
                e.ephemeral = false;
                e.flags &= !(RES_OCCUPIED | RES_VALIDATED | RES_CHILDREN_HELD);
                self.words.free(span);
                self.children.free(children);
                if (e.flags & RES_IN_FREE) == 0 {
                    e.flags |= RES_IN_FREE;
                    self.free.push(i);
                }
            }
        }
        self.defined.clear();
        self.released.clear();
    }
}
