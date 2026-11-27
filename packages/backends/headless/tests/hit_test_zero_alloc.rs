//! `hit_test` / `hit_test_interaction` never allocate in steady state: uniform `z_index`
//! siblings walk in DOM order, mixed ones sort `(z, index)` keys in a stack buffer (≤ 64
//! siblings) or in scratch kept by the backend across queries (warmed once).

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
#[path = "support/z_siblings.rs"]
mod z_siblings;

use counting_alloc::count_allocs;
use z_siblings::ZSiblings;

#[test]
fn counting_alloc_probe_fires_on_this_thread_only() {
    counting_alloc::assert_counts_only_this_thread();
}

#[test]
fn hit_testing_z_ordered_siblings_never_allocates() {
    for (n, distinct_z, warm) in [(300, false, false), (64, true, false), (300, true, true)] {
        let fixture = ZSiblings::new(n, distinct_z);
        if warm {
            assert!(fixture.hit_bottom(), "warm-up query");
        }
        let (all_hit, allocs) = count_allocs(|| (0..20).all(|_| fixture.hit_bottom()));
        assert!(
            all_hit,
            "{n} siblings, distinct z {distinct_z}: wrong target"
        );
        assert_eq!(allocs, 0, "{n} siblings, distinct z {distinct_z}");
    }
}
