use std::collections::VecDeque;

use super::*;

fn collect_mru_to_lru<K: Copy + Eq + Hash, V, S: BuildHasher>(
    lru: &SlabLru<K, V, S>,
) -> Vec<(K, usize)> {
    let mut out = Vec::new();
    let mut cursor = lru.head;
    let mut prev = NONE;
    while cursor != NONE {
        let slot = lru.slots[cursor as usize]
            .as_ref()
            .expect("linked node must be occupied");
        assert_eq!(slot.prev, prev, "broken prev pointer at slot {cursor}");
        out.push((slot.key, slot.bytes));
        prev = cursor;
        cursor = slot.next;
    }
    assert_eq!(prev, lru.tail, "tail pointer mismatch");
    assert_eq!(out.len(), lru.len());
    assert_eq!(out.len(), lru.index.len());
    let sum_bytes: usize = out.iter().map(|(_, b)| *b).sum();
    assert_eq!(sum_bytes, lru.live_bytes());
    out
}

#[test]
fn lru_evicts_least_recently_used_under_entry_and_byte_caps() {
    let mut lru: SlabLru<u32, u32, IdentityU64BuildHasher> =
        SlabLru::with_initial_capacity_and_hasher(3, 100, 3, IdentityU64BuildHasher);
    lru.insert(1, 10, 30);
    lru.insert(2, 20, 30);
    lru.insert(3, 30, 30);
    assert_eq!(lru.len(), 3);
    assert_eq!(lru.live_bytes(), 90);
    assert_eq!(lru.slot_len(), 3);

    // Promote single/middle entry and verify LRU eviction order.
    assert_eq!(lru.get_mut(&1), Some(&mut 10));
    assert_eq!(lru.insert(4, 40, 30), 1);
    assert!(lru.peek(&2).is_none());
    assert_eq!(lru.peek(&1), Some(&10));
    assert_eq!(lru.peek(&3), Some(&30));
    assert_eq!(lru.peek(&4), Some(&40));
    assert_eq!(lru.live_bytes(), 90);
    assert!(lru.slot_len() <= 3, "free-list must reuse evicted slots");

    // Overwrite existing key preserves its dense slot index even while evicting LRU.
    let (slot_before, _) = lru.get_slot_and_value_mut(&4).expect("slot 4");
    let (slot_after, evicted) = lru.insert_with_slot(4, 44, 50);
    assert_eq!(evicted, 1); // 90 - 30 = 60, +50 = 110 > 100 => evicts LRU (3)
    assert_eq!(slot_after, Some(slot_before), "overwrite must preserve dense slot");
    assert_eq!(lru.peek(&4), Some(&44));
    assert!(lru.peek(&3).is_none());
    assert_eq!(lru.live_bytes(), 80);

    // Byte cap forces multiple evictions when inserting a larger item.
    assert_eq!(lru.insert(5, 50, 70), 2);
    assert_eq!(lru.len(), 1);
    assert_eq!(lru.live_bytes(), 70);
    assert_eq!(lru.peek(&5), Some(&50));
    // Promoting the sole entry keeps links intact.
    assert_eq!(lru.get_mut(&5), Some(&mut 50));
    assert_eq!(collect_mru_to_lru(&lru), vec![(5, 70)]);
}

#[test]
fn remove_and_retain_head_tail_preserve_links_and_slot_reuse() {
    let mut lru: SlabLru<u32, u32, FastBuildHasher> =
        SlabLru::with_initial_capacity_and_hasher(4, 400, 4, FastBuildHasher);
    for i in 1..=4u32 {
        lru.insert(i, i * 10, 50);
    }
    // Order MRU->LRU: 4, 3, 2, 1. Remove middle (3), then retain dropping head (4) and tail (1).
    assert_eq!(lru.remove(&3), Some(30));
    assert_eq!(collect_mru_to_lru(&lru), vec![(4, 50), (2, 50), (1, 50)]);
    assert_eq!(lru.remove(&999), None);

    let dropped = lru.retain(|&k, _| k == 2);
    assert_eq!(dropped, 2);
    assert_eq!(collect_mru_to_lru(&lru), vec![(2, 50)]);

    // Re-inserting 100 items never grows slots beyond max_entries.
    for i in 10..110u32 {
        lru.insert(i, i, 50);
        assert!(lru.slot_len() <= 4, "slots leaked past max_entries: {}", lru.slot_len());
    }
}

#[test]
fn max_bytes_usize_max_evicts_without_overflow_and_keeps_exact_byte_sum() {
    let mut lru: SlabLru<u32, u32> = SlabLru::new(usize::MAX, usize::MAX);
    assert_eq!(lru.max_entries(), MAX_VALID_ENTRIES);
    let big = usize::MAX - 100;
    assert_eq!(lru.insert(1, 1, big), 0);
    assert_eq!(lru.live_bytes(), big);

    // Second insert of 200 bytes overflows `usize::MAX` unless key 1 is evicted!
    assert_eq!(lru.insert(2, 2, 200), 1);
    assert!(lru.peek(&1).is_none());
    assert_eq!(lru.peek(&2), Some(&2));
    assert_eq!(lru.live_bytes(), 200);
    assert_eq!(collect_mru_to_lru(&lru), vec![(2, 200)]);
}

#[test]
fn oversized_item_does_not_wipe_existing_entries() {
    let mut lru: SlabLru<u32, u32> = SlabLru::new(4, 64);
    lru.insert(1, 10, 32);
    assert_eq!(lru.insert(2, 20, 128), 0);
    assert_eq!(lru.len(), 1);
    assert_eq!(lru.live_bytes(), 32);
    assert_eq!(lru.peek(&1), Some(&10));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "IdentityU64Hasher accepts only one")]
fn identity_u64_hasher_panics_on_multi_field_overwrite_in_debug() {
    let mut h = IdentityU64Hasher::default();
    h.write_u64(1);
    h.write_u64(2);
}

#[test]
fn seeded_random_operations_match_vecdeque_lru_oracle() {
    const MAX_ENTRIES: usize = 8;
    const MAX_BYTES: usize = 256;
    let mut lru: SlabLru<u16, u32, IdentityU64BuildHasher> =
        SlabLru::with_initial_capacity_and_hasher(
            MAX_ENTRIES,
            MAX_BYTES,
            MAX_ENTRIES,
            IdentityU64BuildHasher,
        );
    // Oracle stores (key, value, bytes) in MRU -> LRU order.
    let mut oracle: VecDeque<(u16, u32, usize)> = VecDeque::new();
    let mut rng = 0x5EED_CAFE_1234_5678u64;
    let mut next_u64 = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };

    for step in 0..5_000u32 {
        let op = next_u64() % 10;
        let key = (next_u64() % 16) as u16;
        match op {
            0..=4 => {
                let bytes = ((next_u64() % 18) as usize) * 16;
                let val = step;
                lru.insert(key, val, bytes);

                if let Some(pos) = oracle.iter().position(|(k, _, _)| *k == key) {
                    oracle.remove(pos);
                }
                if bytes <= MAX_BYTES {
                    let mut live: usize = oracle.iter().map(|(_, _, b)| *b).sum();
                    while !oracle.is_empty()
                        && (oracle.len() >= MAX_ENTRIES || live > MAX_BYTES - bytes)
                    {
                        if let Some((_, _, b)) = oracle.pop_back() {
                            live -= b;
                        }
                    }
                    oracle.push_front((key, val, bytes));
                }
            }
            5..=7 => {
                let got = lru.get_mut(&key).copied();
                let expected = if let Some(pos) = oracle.iter().position(|(k, _, _)| *k == key) {
                    let item = oracle.remove(pos).expect("valid pos");
                    oracle.push_front(item);
                    Some(item.1)
                } else {
                    None
                };
                assert_eq!(got, expected, "get_mut mismatch at step {step}");
            }
            8 => {
                let got = lru.remove(&key);
                let expected = oracle
                    .iter()
                    .position(|(k, _, _)| *k == key)
                    .and_then(|pos| oracle.remove(pos))
                    .map(|(_, v, _)| v);
                assert_eq!(got, expected, "remove mismatch at step {step}");
            }
            _ => {
                let parity = (next_u64() & 1) as u16;
                lru.retain(|&k, _| (k & 1) == parity);
                oracle.retain(|(k, _, _)| (*k & 1) == parity);
            }
        }

        let actual = collect_mru_to_lru(&lru);
        let expected: Vec<(u16, usize)> = oracle.iter().map(|&(k, _, b)| (k, b)).collect();
        assert_eq!(actual, expected, "LRU order diverged at step {step}");
        assert!(lru.slot_len() <= MAX_ENTRIES);
        assert!(lru.live_bytes() <= MAX_BYTES);
    }
}
