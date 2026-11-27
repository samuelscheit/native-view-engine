//! `NveMutationWriter` performs ZERO heap allocations in steady state: same-value and changed
//! attribute updates (standalone + inline `style`), keyed child reorders, subtree removal, and
//! template re-instantiation from a warm `NodeStore` slot pool, all driven through
//! `dioxus_core::WriteMutations`.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
#[path = "support/writer_list.rs"]
mod writer_list;

use std::hint::black_box;

use counting_alloc::count_allocs;
use nve_dioxus::NveMutationWriter;
use writer_list::{
    ITEMS, changed_attributes, item, keyed_reorder, mount_list, remove_and_reinstantiate,
    same_value_attributes, values,
};

#[test]
fn counting_alloc_probe_fires_on_this_thread_only() {
    let (_, own) = count_allocs(|| black_box(Vec::<u64>::with_capacity(32)));
    assert!(
        own >= 1,
        "allocator hook must count this thread's allocations"
    );
    let (_, with_child_alloc) = count_allocs(|| {
        std::thread::spawn(|| black_box(vec![0u8; 4096]).len())
            .join()
            .unwrap_or(0)
    });
    let (_, spawn_only) = count_allocs(|| std::thread::spawn(|| 0usize).join().unwrap_or(0));
    assert_eq!(
        with_child_alloc, spawn_only,
        "child-thread allocations leaked into this count"
    );
}

fn assert_zero_steady_state(name: &str, mut frame: impl FnMut(usize)) {
    // Warm-up: two full cycles grow every scratch buffer, table, and slot pool to steady size.
    for i in 0..ITEMS * 4 {
        frame(i);
    }
    let (_, allocs) = count_allocs(|| {
        for i in 0..ITEMS * 8 {
            frame(i);
        }
    });
    assert_eq!(
        allocs,
        0,
        "{name}: {allocs} allocations over {} steady-state frames",
        ITEMS * 8
    );
}

#[test]
fn writer_steady_state_mutations_allocate_nothing() {
    let mut writer = NveMutationWriter::new(1);
    mount_list(&mut writer);
    let v = values();
    let first = writer.resolve_el(item(0)).expect("item 0 bound");
    assert_eq!(writer.store().children(first).len(), 2);

    assert_zero_steady_state("same-value attributes", |i| {
        same_value_attributes(&mut writer, &v, i)
    });
    assert_zero_steady_state("changed attributes", |i| {
        changed_attributes(&mut writer, &v, i)
    });
    assert_zero_steady_state("keyed reorder", |i| keyed_reorder(&mut writer, i));
    assert_zero_steady_state("remove + re-instantiate", |i| {
        remove_and_reinstantiate(&mut writer, i)
    });

    // Every item is still bound to a live 4-node subtree and listed exactly once.
    let container = writer
        .resolve_el(dioxus_core::ElementId(1))
        .expect("container bound");
    let children = writer.store().children(container).to_vec();
    assert_eq!(children.len(), ITEMS);
    for k in 0..ITEMS {
        let node = writer.resolve_el(item(k)).expect("item bound");
        assert!(writer.store().is_live(node));
        assert!(children.contains(&node));
        assert_eq!(writer.element_id_of(node.0), Some(item(k)));
    }
}
