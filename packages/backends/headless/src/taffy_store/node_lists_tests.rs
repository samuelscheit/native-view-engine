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

#[test]
fn empty_lists_and_out_of_range_reads() {
    let mut lists = NodeLists::default();
    assert!(lists.get(0).is_empty());
    assert!(lists.get(usize::MAX).is_empty());
    lists.clear(7);
    lists.release(7);
    lists.extend(7, ids(1, 3));
    assert!(
        lists.get(7).is_empty(),
        "writes past `ensure_len` are ignored"
    );
    lists.compact_if_fragmented();
    lists.compact();
    lists.assert_consistent();

    lists.ensure_len(4);
    lists.extend(2, core::iter::empty());
    assert!(lists.get(2).is_empty());
    assert_eq!(lists.pool_len(), 0);
    lists.assert_consistent();
}

#[test]
fn model_based_random_ops_match_vec_of_vecs() {
    const NODES: usize = 48;
    let mut rng = Rng(0xC0FF_EE00);
    let mut lists = NodeLists::default();
    let mut model: Vec<Vec<NodeId>> = vec![Vec::new(); NODES];
    lists.ensure_len(NODES);

    for step in 0..20_000u32 {
        let idx = rng.next(NODES);
        match rng.next(10) {
            0..=2 => {
                lists.clear(idx);
                model[idx].clear();
            }
            3..=6 => {
                let n = rng.next(9);
                let base = step * 16;
                model[idx].extend(ids(base, n));
                // Odd steps feed a filtered iterator whose upper bound (2n) overestimates its
                // length, like the store-lookup filter in `add_hoisted_children`.
                if step % 2 == 0 {
                    lists.extend(idx, ids(base, n));
                } else {
                    let doubled = (0..2 * n as u32).filter(|i| i % 2 == 0);
                    lists.extend(idx, doubled.map(|i| NodeId(base + i / 2)));
                }
            }
            7 => {
                lists.release(idx);
                model[idx].clear();
            }
            8 => lists.compact_if_fragmented(),
            _ => lists.compact(),
        }
        lists.assert_consistent();
        for (i, expected) in model.iter().enumerate() {
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
    // Warm: every container records up to 6 boxes, then one compaction primes `scratch`.
    for round in 0..4usize {
        for idx in 0..NODES {
            lists.clear(idx);
            lists.extend(idx, ids(idx as u32 * 8, (idx + round) % 7));
        }
        lists.compact();
    }
    let (pool_cap, scratch_cap, pool_len) = (
        lists.pool.capacity(),
        lists.scratch.capacity(),
        lists.pool_len(),
    );

    // Steady state: any container re-records any list no longer than its peak, in any order.
    let mut rng = Rng(7);
    for _ in 0..10_000 {
        let idx = rng.next(NODES);
        lists.clear(idx);
        let peak = (idx..idx + 4).map(|r| r % 7).max().unwrap_or(0);
        lists.extend(idx, ids(idx as u32 * 8, rng.next(peak + 1)));
        lists.compact_if_fragmented();
    }
    lists.assert_consistent();
    assert_eq!(
        lists.pool_len(),
        pool_len,
        "in-place rewrites must not grow the pool"
    );
    assert_eq!(lists.pool.capacity(), pool_cap);
    assert_eq!(lists.scratch.capacity(), scratch_cap);
}

#[test]
fn growth_relocates_then_compaction_restores_index_order() {
    let mut lists = NodeLists::default();
    lists.ensure_len(3);
    lists.extend(0, ids(10, 2));
    lists.extend(1, ids(20, 2));
    // Node 0 outgrows its capacity while not last: moves to the tail, leaving a 2-slot hole.
    lists.extend(0, ids(12, 1));
    assert_eq!(lists.get(0), &[NodeId(10), NodeId(11), NodeId(12)]);
    assert_eq!(lists.dead, 2);
    // Node 0 is now last: grows in place.
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
