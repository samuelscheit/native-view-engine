//! Thread-local global-allocator verification proving `0` steady-state heap allocations across
//! `NodeStore` hot paths: `tx.set` / `tx.edit` / `NodeMut` (scalars, `OptWord`, `WideOpt`,
//! `Wide64`, `GridLine`, strings), `tx.set_children`, `tx.remove`, `tx.commit_into`, `NodeRef`
//! multi-reads, `StrArena::str`, `publish_table_views`, and `NVE2` `WireEncoder::encode` +
//! `WireDecoder::apply` into reused buffers.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{ChangeSet, GridLine, Length, NodeId, NodeStore, NveTable, Rgba, Str, WordEnum};

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

#[inline]
fn record_alloc() {
    let _ = COUNTING.try_with(|active| {
        if active.get() {
            let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get() + 1));
        }
    });
}

// SAFETY: Delegates directly to `std::alloc::System` while incrementing a thread-local counter.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_alloc();
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_alloc();
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    THREAD_ALLOCS.with(|c| c.set(0));
    COUNTING.with(|a| a.set(true));
    let r = f();
    COUNTING.with(|a| a.set(false));
    let count = THREAD_ALLOCS.with(Cell::get);
    (count, r)
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, WordEnum)]
enum BenchAlign {
    #[default]
    Start = 0,
    Center = 1,
    End = 2,
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
    #[effect(layout)]
    grid_col: GridLine,
    #[effect(layout)]
    opt_align: Option<BenchAlign>,
    #[effect(paint)]
    opt_tint: Option<Rgba>,
    #[effect(layout)]
    opt_ratio: Option<f32>,
    #[effect(paint)]
    stamp: u64,
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
                .opacity(1.0)
                .grid_col(GridLine::Span(2))
                .opt_align(Some(BenchAlign::Center))
                .opt_tint(Some(Rgba::rgb(255, 0, 128)))
                .opt_ratio(Some(1.5))
                .stamp(u64::from(i));
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

    // 1. Steady-state scalar, OptWord, WideOpt, Wide64, GridLine & string mutations.
    let (allocs_mutate, ()) = measure_allocs(|| {
        let mut tx = store.tx();
        for (idx, &n) in nodes.iter().enumerate() {
            let mut nm = tx.node_mut(n).unwrap();
            nm.edit::<BenchBox>()
                .opacity(0.5 + (idx as f32) * 0.001)
                .bg(Rgba::rgb(idx as u8, 40, 80))
                .grid_col(GridLine::Line((idx as i16) - 16))
                .opt_align(if idx & 1 == 0 { Some(BenchAlign::End) } else { None })
                .opt_tint(if idx & 1 == 0 { Some(Rgba::rgb(1, 2, 3)) } else { None })
                .opt_ratio(Some(1.0 + idx as f32 * 0.1))
                .stamp(0x1000_0000_0000_0000u64 | idx as u64);
            nm.edit::<BenchLabel>().text("updated-label-txt");
        }
        tx.set(nodes[0], BenchBox::WIDTH, Length::points(222.0));
        tx.set(nodes[0], BenchBox::OPT_RATIO, None);
        tx.set(nodes[0], BenchBox::STAMP, 0xDEAD_BEEF_CAFE_BABEu64);
        tx.commit_into(&mut changes);
    });
    assert_eq!(allocs_mutate, 0, "steady-state tx mutate + commit_into must allocate 0 bytes");

    // 2. Steady-state NodeRef multi-reads covering OptWord, WideOpt, Wide64, GridLine, &str.
    let (allocs_read, sum) = measure_allocs(|| {
        let mut acc = 0u64;
        for &n in &nodes {
            let nr = store.get(n).unwrap();
            let b = nr.row::<BenchBox>();
            let l = nr.try_row::<BenchLabel>().unwrap();
            acc = acc
                .wrapping_add(u64::from(b.bg().0))
                .wrapping_add(u64::from(b.grid_col().to_word()))
                .wrapping_add(u64::from(b.opt_align().map_or(0, |a| a as u32)))
                .wrapping_add(u64::from(b.opt_tint().map_or(0, |c| c.0)))
                .wrapping_add(u64::from(b.opt_ratio().map_or(0, f32::to_bits)))
                .wrapping_add(b.stamp())
                .wrapping_add(l.text().len() as u64)
                .wrapping_add(nr.children().len() as u64);
        }
        acc
    });
    assert!(sum > 0);
    assert_eq!(
        allocs_read, 0,
        "NodeRef multi-read (OptWord/WideOpt/Wide64/GridLine/&str) must allocate 0 bytes"
    );

    // 3. Steady-state publish_table_views into reused buffers.
    let (allocs_pub, ()) = measure_allocs(|| {
        store.publish_table_views(&changes, &mut table_views, &mut dirty_scratch);
    });
    assert_eq!(allocs_pub, 0, "publish_table_views must allocate 0 bytes");
    assert!(table_views.len() >= 2);

    // 4. Steady-state NVE2 wire encode + decode covering SetWords + SetStrings + SetChildren + Detach.
    {
        let mut tx = store.tx();
        tx.set(nodes[0], BenchBox::OPACITY, 0.66);
        tx.set(nodes[0], BenchBox::OPT_TINT, Some(Rgba::rgb(9, 8, 7)));
        tx.set(nodes[0], BenchBox::STAMP, 42u64);
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
        tx.set(nodes[0], BenchBox::OPT_TINT, None);
        tx.set(nodes[0], BenchBox::OPT_ALIGN, Some(BenchAlign::Start));
        tx.set(nodes[0], BenchBox::GRID_COL, GridLine::Span(4));
        tx.set(nodes[0], BenchBox::STAMP, 99u64);
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
        "steady-state NVE2 encode + decode (OptWord/WideOpt/Wide64/GridLine + strings + children + detach) must allocate 0 bytes"
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
