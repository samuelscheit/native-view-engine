//! Flat pooled-CSR storage of every containing block's out-of-flow ("hoisted") child list.
//!
//! Taffy rewrites the lists of only the containers it re-lays out, in DFS order of the dirty
//! subtree, while cached containers keep theirs. Strict CSR would need an O(N) rebuild for each
//! such out-of-order rewrite, so each node owns a `(start, len, cap)` span in one shared `pool`:
//!
//! - `clear` keeps the span's capacity, so the common "clear + re-add the same boxes" rewrite of a
//!   dirty container writes in place.
//! - A list outgrowing its capacity grows in place at the pool tail, else moves to the tail and
//!   leaves a dead hole.
//! - Once holes exceed half the pool, [`HoistedLists::compact`] repacks every span in node-index
//!   order (plain CSR plus per-node slack) through a reused scratch buffer.
//!
//! Steady-state relayout therefore performs zero heap allocations and zero per-node `Vec`s.

use nve_store::NodeId;

/// Holes below this many slots are never worth a compaction pass.
const MIN_COMPACT_DEAD_SLOTS: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct HoistedSpan {
    start: u32,
    len: u32,
    cap: u32,
}

impl HoistedSpan {
    #[inline(always)]
    fn range(self) -> core::ops::Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }

    #[inline(always)]
    fn end_of_capacity(self) -> usize {
        (self.start + self.cap) as usize
    }
}

#[inline(always)]
fn pool_offset(offset: usize) -> u32 {
    debug_assert!(
        u32::try_from(offset).is_ok(),
        "hoisted pool exceeds u32 offsets"
    );
    offset as u32
}

/// Per-node out-of-flow child lists packed into one flat `NodeId` pool.
#[derive(Debug, Default)]
pub(super) struct HoistedLists {
    spans: Vec<HoistedSpan>,
    pool: Vec<NodeId>,
    /// Reused destination of [`HoistedLists::compact`], swapped with `pool`.
    scratch: Vec<NodeId>,
    /// Pool slots owned by no span (moved-out or released capacity).
    dead: usize,
}

impl HoistedLists {
    /// Grows the per-node span table to `len` slots (never shrinks).
    pub(super) fn ensure_len(&mut self, len: usize) {
        if self.spans.len() < len {
            self.spans.resize(len, HoistedSpan::default());
        }
    }

    /// Drops every list while keeping all buffer capacity.
    pub(super) fn reset(&mut self) {
        self.spans.clear();
        self.pool.clear();
        self.dead = 0;
    }

    #[inline]
    pub(super) fn get(&self, idx: usize) -> &[NodeId] {
        self.spans
            .get(idx)
            .map_or(&[], |span| &self.pool[span.range()])
    }

    /// Empties `idx`'s list, keeping its capacity for an in-place rewrite.
    #[inline]
    pub(super) fn clear(&mut self, idx: usize) {
        if let Some(span) = self.spans.get_mut(idx) {
            span.len = 0;
        }
    }

    /// Empties `idx`'s list and returns its capacity to the pool (node slot created or removed).
    #[inline]
    pub(super) fn release(&mut self, idx: usize) {
        if let Some(span) = self.spans.get_mut(idx) {
            self.dead += span.cap as usize;
            *span = HoistedSpan::default();
        }
    }

    /// Appends `ids` to `idx`'s list.
    pub(super) fn extend(&mut self, idx: usize, ids: impl Iterator<Item = NodeId>) {
        let Some(&span) = self.spans.get(idx) else {
            return;
        };
        let pool_len = self.pool.len();
        let fits = ids
            .size_hint()
            .1
            .is_some_and(|upper| span.len as usize + upper <= span.cap as usize);

        let next = if fits {
            let mut at = span.range().end;
            for id in ids {
                self.pool[at] = id;
                at += 1;
            }
            HoistedSpan {
                len: pool_offset(at) - span.start,
                ..span
            }
        } else if span.end_of_capacity() == pool_len {
            // Last span in the pool (or an empty pool): drop its slack and grow in place.
            self.pool.truncate(span.range().end);
            self.pool.extend(ids);
            let len = pool_offset(self.pool.len()) - span.start;
            HoistedSpan {
                start: span.start,
                len,
                cap: len,
            }
        } else {
            let start = self.pool.len();
            self.pool.extend_from_within(span.range());
            self.pool.extend(ids);
            self.dead += span.cap as usize;
            let len = pool_offset(self.pool.len() - start);
            HoistedSpan {
                start: pool_offset(start),
                len,
                cap: len,
            }
        };
        // Capacity-less spans are canonical `{0, 0, 0}` so no stale `start` can outlive a tail
        // truncation or compaction and index past the pool.
        self.spans[idx] = if next.cap == 0 {
            HoistedSpan::default()
        } else {
            next
        };
    }

    /// Repacks all spans in node-index order once dead holes exceed half the pool.
    pub(super) fn compact_if_fragmented(&mut self) {
        if self.dead >= MIN_COMPACT_DEAD_SLOTS && self.dead * 2 > self.pool.len() {
            self.compact();
        }
    }

    fn compact(&mut self) {
        self.scratch.clear();
        for span in self.spans.iter_mut().filter(|span| span.cap != 0) {
            let start = self.scratch.len();
            self.scratch.extend_from_slice(&self.pool[span.range()]);
            self.scratch.resize(start + span.cap as usize, NodeId::NULL);
            span.start = pool_offset(start);
        }
        core::mem::swap(&mut self.pool, &mut self.scratch);
        self.dead = 0;
    }

    /// Asserts the pool accounting invariant: span capacities are disjoint, in bounds, and
    /// together with `dead` cover the pool exactly.
    #[cfg(test)]
    pub(super) fn assert_consistent(&self) {
        let mut owned: Vec<(usize, usize)> = self
            .spans
            .iter()
            .filter(|s| s.cap != 0)
            .map(|s| (s.start as usize, s.end_of_capacity()))
            .collect();
        owned.sort_unstable();
        for pair in owned.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "overlapping hoisted spans {pair:?}");
        }
        assert!(self.spans.iter().all(|s| s.len <= s.cap));
        assert!(owned.last().is_none_or(|&(_, end)| end <= self.pool.len()));
        let live: usize = owned.iter().map(|&(s, e)| e - s).sum();
        assert_eq!(
            live + self.dead,
            self.pool.len(),
            "hoisted pool slot accounting"
        );
    }

    #[cfg(test)]
    pub(super) fn pool_len(&self) -> usize {
        self.pool.len()
    }
}

#[cfg(test)]
#[path = "hoisted_tests.rs"]
mod tests;
