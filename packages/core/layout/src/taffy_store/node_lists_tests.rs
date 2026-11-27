use super::NodeLists;
use nve_store::NodeId;

/// Deterministic SplitMix64 stream for model-based tests.
struct Rng(u64);

impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % bound as u64) as usize
    }
}

fn ids(base: u32, n: usize) -> impl Iterator<Item = NodeId> {
    (0..n as u32).map(move |i| NodeId(base + i))
}

/// Iterator that reports an arbitrary (possibly lying) `size_hint`.
struct LyingHint<I> {
    inner: I,
    hint: (usize, Option<usize>),
}

impl<I: Iterator<Item = NodeId>> Iterator for LyingHint<I> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.hint
    }
}

fn lying(
    base: u32,
    n: usize,
    hint: (usize, Option<usize>),
) -> LyingHint<impl Iterator<Item = NodeId>> {
    LyingHint {
        inner: ids(base, n),
        hint,
    }
}

#[test]
fn empty_lists_tolerate_out_of_range_reads() {
    let mut lists = NodeLists::default();
    assert!(lists.get(0).is_empty());
    assert!(lists.get(usize::MAX).is_empty());
    lists.compact_if_fragmented();
    lists.compact();
    lists.assert_consistent();

    lists.ensure_len(4);
    lists.extend(2, core::iter::empty());
    assert!(lists.get(2).is_empty());
    assert!(
        lists.get(4).is_empty(),
        "reads past `ensure_len` stay tolerant"
    );
    assert_eq!(lists.pool_len(), 0);
    lists.assert_consistent();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "past ensure_len")]
fn clear_past_ensure_len_is_a_debug_bug() {
    NodeLists::default().clear(7);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "past ensure_len")]
fn release_past_ensure_len_is_a_debug_bug() {
    NodeLists::default().release(7);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "past ensure_len")]
fn extend_past_ensure_len_is_a_debug_bug() {
    NodeLists::default().extend(7, ids(1, 3));
}

#[test]
fn lying_size_hints_never_overflow_or_overwrite_neighbours() {
    let mut lists = NodeLists::default();
    lists.ensure_len(3);
    lists.extend(0, ids(10, 4));
    lists.extend(1, ids(20, 4));
    lists.extend(2, ids(30, 4));
    lists.clear(1);
    // Upper bound `usize::MAX`: no `len + upper` overflow, items still land in place.
    lists.extend(1, lying(40, 3, (0, Some(usize::MAX))));
    assert_eq!(lists.get(1), &[NodeId(40), NodeId(41), NodeId(42)]);
    // Under-reporting hint (claims 0 items, yields 9): must spill to the tail, never overwrite
    // node 2's span that follows node 1 in the pool.
    lists.extend(1, lying(50, 9, (0, Some(0))));
    assert_eq!(lists.get(1).len(), 12);
    assert_eq!(&lists.get(1)[3..], &ids(50, 9).collect::<Vec<_>>()[..]);
    assert_eq!(lists.get(0), &ids(10, 4).collect::<Vec<_>>()[..]);
    assert_eq!(lists.get(2), &ids(30, 4).collect::<Vec<_>>()[..]);
    lists.assert_consistent();
}

#[test]
fn model_based_random_ops_match_vec_of_vecs() {
    let mut nodes = 16usize;
    let mut rng = Rng(0xC0FF_EE00);
    let mut lists = NodeLists::default();
    let mut model: Vec<Vec<NodeId>> = vec![Vec::new(); 64];
    lists.ensure_len(nodes);

    for step in 0..30_000u32 {
        // Mid-run growth of the node table (store capacity grows).
        if step % 5_000 == 4_999 && nodes < 64 {
            nodes += 16;
            lists.ensure_len(nodes);
        }
        let idx = rng.next(nodes);
        match rng.next(12) {
            0..=2 => {
                lists.clear(idx);
                model[idx].clear();
            }
            3..=6 => {
                let n = rng.next(9);
                let base = step * 16;
                model[idx].extend(ids(base, n));
                match step % 4 {
                    0 => lists.extend(idx, ids(base, n)),
                    // Over-estimating filter, like the store lookup in `add_hoisted_children`.
                    1 => {
                        let doubled = (0..2 * n as u32).filter(|i| i % 2 == 0);
                        lists.extend(idx, doubled.map(|i| NodeId(base + i / 2)));
                    }
                    2 => lists.extend(idx, lying(base, n, (0, Some(usize::MAX)))),
                    _ => lists.extend(idx, lying(base, n, (0, Some(0)))),
                }
            }
            7 => {
                lists.release(idx);
                model[idx].clear();
            }
            8 => lists.compact_if_fragmented(),
            9 => lists.compact(),
            10 if step % 97 == 0 => {
                lists.reset();
                lists.ensure_len(nodes);
                model.iter_mut().for_each(Vec::clear);
            }
            _ => {}
        }
        lists.assert_consistent();
        for (i, expected) in model.iter().enumerate().take(nodes) {
            assert_eq!(
                lists.get(i),
                expected.as_slice(),
                "node {i} after step {step}"
            );
        }
    }
}

#[test]
fn steady_state_rewrites_keep_pool_capacity_and_length() {
    const NODES: usize = 256;
    let mut lists = NodeLists::default();
    lists.ensure_len(NODES);
    // Warm: lists grow round over round, so non-last spans outgrow their capacity and relocate
    // (via over-estimating filters); one compaction per round primes `scratch`.
    let mut relocated = false;
    for round in 0..4usize {
        for idx in 0..NODES {
            lists.clear(idx);
            let n = (idx + round) % 7;
            let filtered = (0..2 * n as u32).filter(|i| i % 2 == 0);
            lists.extend(idx, filtered.map(|i| NodeId(idx as u32 * 8 + i / 2)));
        }
        relocated |= lists.dead > 0;
        lists.compact();
    }
    assert!(relocated, "warm-up must exercise relocation");

    // Steady state: any container re-records any list no longer than its peak, in any order,
    // through exact, over-estimating, and lying iterators. Emptied lists lose their slack at
    // compaction and relocate once when refilled, so the pool breathes between compactions;
    // after one settling pass its buffers never reallocate again.
    let mut rng = Rng(7);
    let mut churn = |lists: &mut NodeLists, steps: usize| {
        for step in 0..steps {
            let idx = rng.next(NODES);
            lists.clear(idx);
            let peak = (idx..idx + 4).map(|r| r % 7).max().unwrap_or(0);
            let n = rng.next(peak + 1);
            let base = idx as u32 * 8;
            match step % 3 {
                0 => lists.extend(idx, ids(base, n)),
                1 => lists.extend(idx, lying(base, n, (0, Some(0)))),
                _ => lists.extend(idx, lying(base, n, (0, Some(usize::MAX)))),
            }
            lists.compact_if_fragmented();
        }
    };
    churn(&mut lists, 20_000);
    // `compact` swaps `pool` and `scratch`, so compare the pair of buffer capacities.
    let caps = |l: &NodeLists| {
        let (a, b) = (l.pool.capacity(), l.scratch.capacity());
        (a.min(b), a.max(b))
    };
    let settled = caps(&lists);
    churn(&mut lists, 20_000);
    lists.assert_consistent();
    assert_eq!(
        caps(&lists),
        settled,
        "settled rewrites must never reallocate"
    );
    assert!(lists.pool_len() <= settled.1);
}

#[test]
fn compaction_threshold_needs_64_dead_slots_and_majority_dead() {
    let mut lists = NodeLists::default();
    lists.ensure_len(4);
    lists.extend(0, ids(0, 100));
    lists.extend(1, ids(200, 40));
    lists.extend(2, ids(300, 64));
    lists.extend(3, ids(400, 10));
    let pool = lists.pool_len();
    assert_eq!(pool, 214);

    // 40 dead (> half of nothing, but < 64): skipped.
    lists.release(1);
    lists.compact_if_fragmented();
    assert_eq!((lists.dead, lists.pool_len()), (40, pool));

    // 104 dead, 104 * 2 <= 214: skipped.
    lists.release(2);
    lists.compact_if_fragmented();
    assert_eq!((lists.dead, lists.pool_len()), (104, pool));

    // 114 dead, 114 * 2 > 214: compacts to the 100 live slots.
    lists.release(3);
    lists.compact_if_fragmented();
    assert_eq!((lists.dead, lists.pool_len()), (0, 100));
    assert_eq!(lists.get(0), &ids(0, 100).collect::<Vec<_>>()[..]);
    lists.assert_consistent();

    // A small pool never compacts below 64 dead slots, even when entirely dead.
    let mut small = NodeLists::default();
    small.ensure_len(1);
    small.extend(0, ids(0, 63));
    small.release(0);
    small.compact_if_fragmented();
    assert_eq!((small.dead, small.pool_len()), (63, 63));
}

#[test]
fn compaction_reclaims_slack_of_emptied_lists() {
    let mut lists = NodeLists::default();
    lists.ensure_len(2);
    lists.extend(0, ids(0, 8));
    lists.extend(1, ids(10, 8));
    lists.clear(0);
    lists.compact();
    assert_eq!(
        lists.pool_len(),
        8,
        "an emptied list keeps no slack across compaction"
    );
    assert_eq!(lists.get(1), &ids(10, 8).collect::<Vec<_>>()[..]);
    lists.assert_consistent();
}

#[test]
fn growth_relocates_then_compaction_restores_index_order() {
    let mut lists = NodeLists::default();
    lists.ensure_len(3);
    lists.extend(0, ids(10, 2));
    lists.extend(1, ids(20, 2));
    // Node 0 outgrows its capacity while not last: moves to the tail with doubled capacity
    // (2 -> 4, one `NULL` pad slot), leaving a 2-slot hole.
    lists.extend(0, ids(12, 1));
    assert_eq!(lists.get(0), &[NodeId(10), NodeId(11), NodeId(12)]);
    assert_eq!((lists.spans[0].len, lists.spans[0].cap), (3, 4));
    assert_eq!(lists.dead, 2);
    // The doubled capacity absorbs the next item in place.
    lists.extend(0, ids(13, 1));
    assert_eq!(lists.pool_len(), 2 + 2 + 4);
    lists.compact();
    assert_eq!(lists.dead, 0);
    assert_eq!(lists.pool_len(), 6);
    assert_eq!(
        lists.spans[0].start, 0,
        "compaction packs spans in node-index order"
    );
    assert_eq!(lists.spans[1].start, 4);
    assert_eq!(lists.get(1), &[NodeId(20), NodeId(21)]);
    lists.release(0);
    assert_eq!(lists.dead, 4);
    lists.assert_consistent();
}
