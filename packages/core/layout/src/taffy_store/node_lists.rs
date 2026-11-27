//! Flat pooled-CSR per-node `NodeId` lists: each containing block's out-of-flow ("hoisted")
//! boxes, and the `display: contents`-flattened box children of their parents.
//!
//! Lists are rewritten for only some nodes in arbitrary order (Taffy re-lays out only dirty
//! containers, in DFS order, while cached ones keep theirs). Strict CSR would need an O(N)
//! rebuild per out-of-order rewrite, so each node owns a `(start, len, cap)` span in one `pool`:
//!
//! - `clear` keeps the span's capacity, so the common "clear + re-add the same boxes" rewrite of a
//!   dirty container writes in place.
//! - A list outgrowing its capacity grows in place at the pool tail, else moves to the tail and
//!   leaves a dead hole; either way its capacity at least doubles (amortized O(1) growth).
//! - Once holes exceed half the pool, [`NodeLists::compact`] repacks every non-empty span in
//!   node-index order (plain CSR plus per-node slack) through a reused scratch buffer.
//!
//! Steady-state rewrites therefore perform zero heap allocations and zero per-node `Vec`s.

use nve_store::NodeId;

/// Holes below this many slots are never worth a compaction pass.
const MIN_COMPACT_DEAD_SLOTS: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ListSpan {
    start: u32,
    len: u32,
    cap: u32,
}

impl ListSpan {
    #[inline(always)]
    fn range(self) -> core::ops::Range<usize> {
        let start = self.start as usize;
        start..start + self.len as usize
    }

    #[inline(always)]
    fn end_of_capacity(self) -> usize {
        self.start as usize + self.cap as usize
    }
}

/// Spans store `u32` offsets (12-byte spans). The bound is enforced at the pool's only growth
/// point, [`NodeLists::extend`], so every narrowing through [`narrow`] is in range.
const MAX_POOL_LEN: usize = u32::MAX as usize;

#[inline(always)]
fn narrow(v: usize) -> u32 {
    debug_assert!(
        v <= MAX_POOL_LEN,
        "node list offset {v} escaped the pool bound"
    );
    v as u32
}

/// Per-node `NodeId` lists packed into one flat pool.
#[derive(Debug, Default)]
pub(super) struct NodeLists {
    spans: Vec<ListSpan>,
    pool: Vec<NodeId>,
    /// Reused destination of [`NodeLists::compact`], swapped with `pool`.
    scratch: Vec<NodeId>,
    /// Pool slots owned by no span (moved-out or released capacity).
    dead: usize,
}

impl NodeLists {
    /// Grows the per-node span table to `len` slots (never shrinks).
    pub(super) fn ensure_len(&mut self, len: usize) {
        if self.spans.len() < len {
            self.spans.resize(len, ListSpan::default());
        }
    }

    /// Drops every list while keeping all buffer capacity.
    pub(super) fn reset(&mut self) {
        self.spans.clear();
        self.pool.clear();
        self.scratch.clear();
        self.dead = 0;
    }

    /// `idx`'s list; empty for slots past [`NodeLists::ensure_len`].
    #[inline]
    pub(super) fn get(&self, idx: usize) -> &[NodeId] {
        self.spans
            .get(idx)
            .map_or(&[], |span| &self.pool[span.range()])
    }

    /// Empties `idx`'s list, keeping its capacity for an in-place rewrite. The capacity stays
    /// owned (not dead) until the list is refilled, released, or [`NodeLists::compact`] reclaims
    /// the slack of every list still empty at that point.
    #[inline]
    pub(super) fn clear(&mut self, idx: usize) {
        debug_assert!(
            idx < self.spans.len(),
            "clear of node slot {idx} past ensure_len"
        );
        if let Some(span) = self.spans.get_mut(idx) {
            span.len = 0;
        }
    }

    /// Empties `idx`'s list and returns its capacity to the pool (node slot created or removed).
    #[inline]
    pub(super) fn release(&mut self, idx: usize) {
        debug_assert!(
            idx < self.spans.len(),
            "release of node slot {idx} past ensure_len"
        );
        if let Some(span) = self.spans.get_mut(idx) {
            self.dead += span.cap as usize;
            *span = ListSpan::default();
        }
    }

    /// Appends `ids` to `idx`'s list. Fills the span's spare capacity first, then (without ever
    /// trusting `Iterator::size_hint`) grows in place at the pool tail or moves the list to the
    /// tail, doubling its capacity so repeated single-item growth stays amortized O(1).
    pub(super) fn extend(&mut self, idx: usize, ids: impl IntoIterator<Item = NodeId>) {
        debug_assert!(
            idx < self.spans.len(),
            "extend of node slot {idx} past ensure_len"
        );
        let Some(&span) = self.spans.get(idx) else {
            return;
        };
        let mut ids = ids.into_iter();
        let start = span.start as usize;
        let cap = span.cap as usize;
        let mut len = span.len as usize;
        let overflow = loop {
            match ids.next() {
                None => break None,
                Some(id) if len < cap => {
                    self.pool[start + len] = id;
                    len += 1;
                }
                Some(id) => break Some(id),
            }
        };
        let Some(first_overflow) = overflow else {
            self.spans[idx].len = narrow(len);
            return;
        };
        let in_place = span.end_of_capacity() == self.pool.len();
        let new_start = if in_place {
            // Last span in the pool (or an empty pool): drop its slack and grow in place.
            self.pool.truncate(start + len);
            start
        } else {
            let tail = self.pool.len();
            self.pool.extend_from_within(start..start + len);
            self.dead += cap;
            tail
        };
        self.pool.push(first_overflow);
        self.pool.extend(ids);
        let new_len = self.pool.len() - new_start;
        if self.pool.len() > MAX_POOL_LEN {
            // Beyond `u32` offsets (> 4 Gi node ids, unreachable before memory runs out): keep
            // the list as filled so far and drop the excess ids rather than corrupt offsets.
            debug_assert!(false, "node list pool exceeds u32 offsets");
            if in_place {
                self.pool.truncate(start + len);
                self.pool.resize(start + cap, NodeId::NULL);
            } else {
                self.pool.truncate(new_start);
                self.dead -= cap;
            }
            self.spans[idx].len = narrow(len);
            return;
        }
        // Doubling headroom is clamped to the offset bound; the ids themselves fit.
        let new_cap = new_len
            .max(cap.saturating_mul(2))
            .min(MAX_POOL_LEN - new_start);
        self.pool.resize(new_start + new_cap, NodeId::NULL);
        self.spans[idx] = ListSpan {
            start: narrow(new_start),
            len: narrow(new_len),
            cap: narrow(new_cap),
        };
    }

    /// Repacks all spans in node-index order once dead holes exceed half the pool.
    pub(super) fn compact_if_fragmented(&mut self) {
        if self.dead >= MIN_COMPACT_DEAD_SLOTS && self.dead * 2 > self.pool.len() {
            self.compact();
        }
    }

    /// Repacks every span in node-index order. Non-empty spans keep their slack (in-place
    /// rewrites stay cheap); empty spans give theirs back and become canonical `{0, 0, 0}`, so
    /// slack of lists that emptied out is reclaimed instead of pinned forever.
    fn compact(&mut self) {
        self.scratch.clear();
        for span in &mut self.spans {
            if span.len == 0 {
                *span = ListSpan::default();
                continue;
            }
            let start = self.scratch.len();
            self.scratch.extend_from_slice(&self.pool[span.range()]);
            self.scratch.resize(start + span.cap as usize, NodeId::NULL);
            span.start = narrow(start);
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
            assert!(pair[0].1 <= pair[1].0, "overlapping list spans {pair:?}");
        }
        assert!(self.spans.iter().all(|s| s.len <= s.cap));
        assert!(
            self.spans
                .iter()
                .all(|s| s.cap != 0 || (s.start == 0 && s.len == 0)),
            "capacity-less spans must be canonical {{0, 0, 0}}"
        );
        assert!(owned.last().is_none_or(|&(_, end)| end <= self.pool.len()));
        let live: usize = owned.iter().map(|&(s, e)| e - s).sum();
        assert_eq!(
            live + self.dead,
            self.pool.len(),
            "node list pool slot accounting"
        );
    }

    #[cfg(test)]
    pub(super) fn dead_slots(&self) -> usize {
        self.dead
    }

    #[cfg(test)]
    pub(super) fn capacity_of(&self, idx: usize) -> usize {
        self.spans.get(idx).map_or(0, |s| s.cap as usize)
    }

    #[cfg(test)]
    pub(super) fn pool_len(&self) -> usize {
        self.pool.len()
    }
}

#[cfg(test)]
#[path = "node_lists_tests.rs"]
mod tests;
