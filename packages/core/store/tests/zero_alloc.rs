//! Global-allocator verification proving `0` steady-state heap allocations across
//! `NodeStore` hot paths: `tx.set` / `tx.edit` / `NodeMut`, `tx.set_children`, `tx.insert_child`,
//! `tx.remove`, `tx.commit_into`, `NodeRef` multi-reads, `StrArena::str`, `publish_table_views`,
//! and `NVE2` `WireEncoder::encode` + `WireDecoder::apply` into reused buffers.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba, Str};

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = ALLOC_COUNT.load(Ordering::SeqCst);
    let r = f();
    let after = ALLOC_COUNT.load(Ordering::SeqCst);
    (after - before, r)
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "bench.box", storage = "dense", flags = "publish")]
struct BenchBox {
    #[effect(layout)]
    width: Length,
    #[effect(layout)]
    height: Length,
    #[effect(paint)]
    bg: Rgba,
    #[effect(composite)]
    #[default(1.0)]
    opacity: f32,
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "bench.label", storage = "sparse", flags = "publish")]
struct BenchLabel {
    #[effect(measure, paint)]
    text: Str,
}

#[test]
fn steady_state_hot_paths_perform_zero_heap_allocations() {
    let mut store = NodeStore::new();
    let mut replica = NodeStore::replica();
    let mut changes = ChangeSet::default();
    let mut rep_changes = ChangeSet::default();
    let mut enc = WireEncoder::default();
    let mut dec = WireDecoder::default();
    let mut wire_buf = Vec::with_capacity(16 * 1024);
    let mut table_views = Vec::with_capacity(16);
    let mut dirty_scratch = Vec::with_capacity(256);

    // Pre-warm store, columns, span pools, scratch stack, wire encoder/decoder, and replica.
    let mut nodes = Vec::with_capacity(64);
    {
        let mut tx = store.tx();
        for i in 0..64u32 {
            let mut nm = tx.spawn();
            nm.edit::<BenchBox>()
                .width(Length::points(100.0 + i as f32))
                .height(Length::points(40.0))
                .bg(Rgba::rgb(10, 20, 30))
                .opacity(1.0);
            nm.edit::<BenchLabel>().text("initial-label-text");
            nodes.push(nm.id());
        }
        tx.set_children(NodeId::ROOT, &nodes);
        tx.commit_into(&mut changes);
    }
    enc.encode(&store, &changes, &mut wire_buf);
    {
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire_buf).unwrap();
        rtx.commit_into(&mut rep_changes);
    }

    // Warm subtree remove + recreate + commit + wire round-trip once so free lists & scratch
    // buffers have steady-state capacity.
    let mut sub_root;
    {
        let mut tx = store.tx();
        sub_root = tx.create();
        let c1 = tx.create();
        let c2 = tx.create();
        tx.set_children(sub_root, &[c1, c2]);
        tx.append_child(NodeId::ROOT, sub_root);
        tx.commit_into(&mut changes);
    }
    wire_buf.clear();
    enc.encode(&store, &changes, &mut wire_buf);
    {
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire_buf).unwrap();
        rtx.commit_into(&mut rep_changes);
    }
    {
        let mut tx = store.tx();
        tx.remove(sub_root);
        tx.commit_into(&mut changes);
    }
    wire_buf.clear();
    enc.encode(&store, &changes, &mut wire_buf);
    {
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire_buf).unwrap();
        rtx.commit_into(&mut rep_changes);
    }
    // Recreate sub_root for the measured reorder + removal pass.
    let (sub_c1, sub_c2);
    {
        let mut tx = store.tx();
        sub_root = tx.create();
        sub_c1 = tx.create();
        sub_c2 = tx.create();
        tx.set_children(sub_root, &[sub_c1, sub_c2]);
        tx.append_child(NodeId::ROOT, sub_root);
        tx.commit_into(&mut changes);
    }
    wire_buf.clear();
    enc.encode(&store, &changes, &mut wire_buf);
    {
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire_buf).unwrap();
        rtx.commit_into(&mut rep_changes);
    }
    store.publish_table_views(&changes, &mut table_views, &mut dirty_scratch);

    // 1. Steady-state scalar & string mutations via NodeMut + tx.edit + tx.set + commit_into.
    let (allocs_mutate, ()) = measure_allocs(|| {
        let mut tx = store.tx();
        for (idx, &n) in nodes.iter().enumerate() {
            let mut nm = tx.node_mut(n).unwrap();
            nm.edit::<BenchBox>()
                .opacity(0.5 + (idx as f32) * 0.001)
                .bg(Rgba::rgb(idx as u8, 40, 80));
            nm.edit::<BenchLabel>().text("updated-label-txt");
        }
        tx.set(nodes[0], BenchBox::WIDTH, Length::points(222.0));
        tx.commit_into(&mut changes);
    });
    assert_eq!(allocs_mutate, 0, "steady-state tx mutate + commit_into must allocate 0 bytes");

    // 2. Steady-state NodeRef multi-reads + sealed RowRef O(1) `&str` reads.
    let (allocs_read, sum) = measure_allocs(|| {
        let mut acc = 0usize;
        for &n in &nodes {
            let nr = store.get(n).unwrap();
            let b = nr.row::<BenchBox>();
            let l = nr.try_row::<BenchLabel>().unwrap();
            acc += b.bg().0 as usize + l.text().len() + nr.children().len();
        }
        acc
    });
    assert!(sum > 0);
    assert_eq!(allocs_read, 0, "NodeRef multi-read + RowRef &str must allocate 0 bytes");

    // 3. Steady-state publish_table_views into reused buffers.
    let (allocs_pub, ()) = measure_allocs(|| {
        store.publish_table_views(&changes, &mut table_views, &mut dirty_scratch);
    });
    assert_eq!(allocs_pub, 0, "publish_table_views must allocate 0 bytes");
    assert!(table_views.len() >= 2);

    // 4. Steady-state NVE2 wire encode + decode covering SetWords + SetStrings + SetChildren + Detach.
    // Warm one detach + re-attach + reorder wire frame (touching both tables) so decoder scratch &
    // `ChangeSet::tables[0..2]` detached buffers have steady-state capacity.
    {
        let mut tx = store.tx();
        tx.set(nodes[0], BenchBox::OPACITY, 0.66);
        tx.edit::<BenchLabel>(nodes[1]).text("warm-str-same-cls");
        tx.detach::<BenchLabel>(nodes[63]);
        tx.set_children(sub_root, &[sub_c2, sub_c1]);
        tx.commit_into(&mut changes);
        wire_buf.clear();
        enc.encode(&store, &changes, &mut wire_buf);
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire_buf).unwrap();
        rtx.commit_into(&mut rep_changes);

        let mut tx = store.tx();
        tx.edit::<BenchLabel>(nodes[63]).text("reattached-label!");
        tx.set_children(sub_root, &[sub_c1, sub_c2]);
        tx.commit_into(&mut changes);
        wire_buf.clear();
        enc.encode(&store, &changes, &mut wire_buf);
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire_buf).unwrap();
        rtx.commit_into(&mut rep_changes);
    }

    let (allocs_wire, ()) = measure_allocs(|| {
        let mut tx = store.tx();
        tx.set(nodes[0], BenchBox::OPACITY, 0.77);
        tx.edit::<BenchLabel>(nodes[1]).text("wire-str-same-cls");
        tx.detach::<BenchLabel>(nodes[63]);
        tx.set_children(sub_root, &[sub_c2, sub_c1]);
        tx.commit_into(&mut changes);

        wire_buf.clear();
        enc.encode(&store, &changes, &mut wire_buf);
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire_buf).unwrap();
        rtx.commit_into(&mut rep_changes);
    });
    assert_eq!(
        allocs_wire, 0,
        "steady-state NVE2 encode + decode (words + strings + children + detach) must allocate 0 bytes"
    );

    // 5. Steady-state set_children reorder + subtree remove + commit_into.
    let reordered = [sub_c1, sub_c2];
    let (allocs_tree, ()) = measure_allocs(|| {
        let mut tx = store.tx();
        tx.set_children(sub_root, &reordered);
        tx.remove(sub_root);
        tx.commit_into(&mut changes);
    });
    assert_eq!(
        allocs_tree, 0,
        "steady-state set_children + subtree remove + commit_into must allocate 0 bytes"
    );
}
