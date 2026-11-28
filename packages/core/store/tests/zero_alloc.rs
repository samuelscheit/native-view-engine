//! Thread-local global-allocator verification proving `0` steady-state heap allocations across
//! `NodeStore` hot paths: `tx.set` / `tx.edit` / `NodeMut` (scalars, `OptWord`, `WideOpt`,
//! `Wide64`, `GridLine`, strings), `tx.set_children`, `tx.remove`, `tx.commit_into`, `NodeRef`
//! multi-reads, `StrArena::str`, `publish_table_views`, and `NVE2` `WireEncoder::encode` +
//! `WireDecoder::apply` into reused buffers.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{
    ChangeSet, EventBatchBuf, EventEnum, EventFlags, FlatResource, GridLine, Length, NodeId,
    NodeStore, NveTable, Res, ResolvedRow, Rgba, RowPatch, RowPatchView, Str, SurfaceEventTracker,
    SurfaceId, TimestampUs, ValidatedWords, WordEnum,
};

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

struct CountingGuard;

impl Drop for CountingGuard {
    fn drop(&mut self) {
        let _ = COUNTING.try_with(|a| a.set(false));
    }
}

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    THREAD_ALLOCS.with(|c| c.set(0));
    COUNTING.with(|a| a.set(true));
    let _guard = CountingGuard;
    let r = f();
    drop(_guard);
    let count = THREAD_ALLOCS.with(Cell::get);
    (count, r)
}

#[test]
fn counting_alloc_positive_probe_detects_heap_allocation() {
    let (allocs, v) = measure_allocs(|| {
        let mut buf = Vec::<u32>::with_capacity(std::hint::black_box(64));
        buf.push(std::hint::black_box(1));
        std::hint::black_box(buf)
    });
    assert!(allocs >= 1, "CountingAlloc must detect heap allocations (saw {allocs})");
    drop(v);
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

#[derive(Debug, Clone)]
struct BenchBlob;

impl FlatResource for BenchBlob {
    const KIND: nve_abi::Key64 = nve_abi::Key64::of("bench.blob");
    type View<'a> = &'a [u32];
    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        out.push(1);
        true
    }
    fn validate(words: &[u32]) -> bool {
        !words.is_empty()
    }
    fn view(words: ValidatedWords<'_, Self>) -> &[u32] {
        words.as_slice()
    }
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "bench.label", storage = "sparse", flags = "publish")]
struct BenchLabel {
    #[effect(measure, paint)]
    text: Str,
    #[effect(paint)]
    blob: Res<BenchBlob>,
    #[effect(paint)]
    patch_res: Res<RowPatch>,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, EventEnum)]
enum BenchEvent {
    Click = 0,
    Hover = 1,
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "bench.events", storage = "sparse", flags = "publish", events = BenchEvent)]
struct BenchEvents {
    #[effect(semantics)]
    disabled: bool,
}

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "bench.typo", storage = "sparse", flags = "publish", inherited)]
struct BenchTypo {
    #[effect(paint)]
    #[default(Rgba::BLACK)]
    color: Rgba,
    #[effect(measure)]
    #[default(16.0)]
    font_size: f32,
    #[effect(measure)]
    #[default(400u16)]
    font_weight: u16,
    #[effect(measure)]
    font_family: Str,
    #[effect(measure)]
    #[no_inherit]
    max_lines: Option<u32>,
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

    // 6. Steady-state branchless ResolvedRow<BenchTypo> inheritance walk + clear_color/color toggle.
    {
        let mut tx = store.tx();
        tx.edit::<BenchTypo>(nodes[0])
            .color(Rgba::WHITE)
            .font_size(22.0)
            .font_weight(700)
            .font_family("Inter")
            .max_lines(Some(3));
        for &n in &nodes[1..32] {
            tx.edit::<BenchTypo>(n).color(Rgba::BLACK);
        }
        tx.commit_into(&mut changes);
    }
    let (allocs_inherit, resolved_sum) = measure_allocs(|| {
        let root_r = ResolvedRow::<BenchTypo>::root(&store);
        let p_r = ResolvedRow::<BenchTypo>::resolve(&store, nodes[0], &root_r);
        let mut acc = 0u64;
        for &n in &nodes[1..] {
            let r = ResolvedRow::<BenchTypo>::resolve(&store, n, &p_r);
            let t = r.get();
            acc = acc
                .wrapping_add(u64::from(t.color().0))
                .wrapping_add(u64::from(t.font_size().to_bits()))
                .wrapping_add(u64::from(t.font_weight()))
                .wrapping_add(t.font_family().len() as u64)
                .wrapping_add(u64::from(t.max_lines().unwrap_or(0)));
        }
        let mut tx = store.tx();
        tx.edit::<BenchTypo>(nodes[1]).clear_color();
        tx.edit::<BenchTypo>(nodes[1]).color(Rgba::BLACK);
        tx.commit_into(&mut changes);
        acc
    });
    assert!(resolved_sum > 0);
    assert_eq!(
        allocs_inherit, 0,
        "steady-state ResolvedRow inheritance walk + clear/set explicit bit must allocate 0 bytes"
    );

    // 7. Flat ResourceStore dedup hit + ephemeral in-place rewrite + RowPatchView::apply_table = 0 allocs.
    let mut patch_words = Vec::with_capacity(32);
    let mut rp = RowPatch::new();
    rp.set(BenchBox::OPACITY, 0.35)
        .set(BenchBox::BG, Rgba::rgb(99, 88, 77))
        .set(BenchBox::OPT_ALIGN, Some(BenchAlign::End))
        .set(BenchBox::OPT_TINT, Some(Rgba::rgb(1, 2, 3)))
        .set(BenchBox::STAMP, 0xCAFE_BABE_1234_5678u64);
    assert!(rp.write_words(&mut patch_words));
    {
        let mut tx = store.tx();
        tx.edit::<BenchLabel>(nodes[0]).blob_with(|out| {
            out.extend_from_slice(&[1, 2, 3, 4]);
            true
        });
        tx.edit::<BenchLabel>(nodes[2]).blob_ephemeral_with(|out| {
            out.extend_from_slice(&[10, 20, 30, 40]);
            true
        });
        tx.commit_into(&mut changes);
    }
    let (allocs_flat_res, flat_acc) = measure_allocs(|| {
        // Dedup hit via `blob_with` on a second node.
        let mut tx = store.tx();
        tx.edit::<BenchLabel>(nodes[1]).blob_with(|out| {
            out.extend_from_slice(&[1, 2, 3, 4]);
            true
        });
        // Ephemeral in-place rewrite within same size class (4 words).
        tx.edit::<BenchLabel>(nodes[2]).blob_ephemeral_with(|out| {
            out.extend_from_slice(&[50, 60, 70, 80]);
            true
        });
        tx.commit_into(&mut changes);

        // Zero-copy view read + branchless RowPatchView::apply_table.
        let view = store
            .resource::<BenchBlob>(store.row::<BenchLabel>(nodes[2]).blob())
            .unwrap();
        let mut row_buf = [0u32; 16];
        let n = BenchBox::DESC.words();
        row_buf[..n].copy_from_slice(BenchBox::DESC.defaults);
        let pv = RowPatchView::from_words(&patch_words).unwrap();
        pv.apply_table::<BenchBox>(&mut row_buf[..n]);
        u64::from(view[0]).wrapping_add(u64::from(row_buf[0]))
    });
    assert!(flat_acc > 0);
    assert_eq!(
        allocs_flat_res, 0,
        "steady-state flat ResourceStore dedup hit, ephemeral in-place rewrite, and RowPatchView::apply_table must allocate 0 bytes"
    );

    // 8b. Steady-state wire-origin `ResourceStore::get`, `tx.apply_row_patch`, and topological
    //     `WireEncoder::encode` with nested parent->child resources = 0 allocs!
    let mut nested_patch = RowPatch::new();
    nested_patch.set(BenchBox::OPACITY, 0.25);
    let mut nested_patch_words = Vec::new();
    assert!(nested_patch.write_words(&mut nested_patch_words));
    let nested_pv = RowPatchView::from_words(&nested_patch_words).unwrap();
    // Warm initial + one in-place nested parent->child rewrite on `store`, `enc`, and `replica`
    // so scratch buffers (`visit_marks`, `work_stack`, `old_children_scratch`, `topo_*`) are warm.
    let (wire_blob_id, synced_child_id) = {
        let mut tx = store.tx();
        let child_id = tx.with_temp_resources(|res| {
            let cid = res.intern(&BenchBlob);
            let mut rp = RowPatch::new();
            rp.set_res_id(BenchLabel::BLOB, cid);
            res.tx()
                .edit::<BenchLabel>(nodes[3])
                .patch_res_ephemeral(rp);
            cid
        });
        tx.commit_into(&mut changes);
        wire_buf.clear();
        enc.encode(&store, &changes, &mut wire_buf);
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire_buf).unwrap();
        rtx.commit_into(&mut rep_changes);

        // Second frame warms in-place ephemeral rewrite + topological encode with an existing child.
        let mut tx2 = store.tx();
        tx2.edit::<BenchLabel>(nodes[3]).patch_res_ephemeral_with(|out| {
            out.extend_from_slice(&[
                1,
                BenchLabel::KEY.lo(),
                BenchLabel::KEY.hi(),
                1,
                u32::from(BenchLabel::BLOB.word()) | (u32::from(nve_store::WORD_OP_RESOURCE) << 16),
                u32::MAX,
                child_id.0,
            ]);
            true
        });
        tx2.commit_into(&mut changes);
        wire_buf.clear();
        enc.encode(&store, &changes, &mut wire_buf);
        let mut rtx2 = replica.tx();
        dec.apply(&mut rtx2, &wire_buf).unwrap();
        rtx2.commit_into(&mut rep_changes);

        // Trigger first-read wire validation once so `OnceLock` caches `TypeId`.
        let pid = replica.row::<BenchLabel>(nodes[3]).patch_res();
        assert!(replica.resource::<RowPatch>(pid).is_some());
        (pid, child_id)
    };
    let (allocs_wire_get, get_acc) = measure_allocs(|| {
        let mut acc = 0usize;
        for _ in 0..100 {
            acc += replica.resource::<RowPatch>(wire_blob_id).unwrap().table_count();
        }
        acc
    });
    assert!(get_acc > 0);
    assert_eq!(
        allocs_wire_get, 0,
        "steady-state wire-origin ResourceStore::get (OnceLock cached) must allocate 0 bytes"
    );

    // Up to 8 temporary handles stay inline in `TempResources` (dedup hits on a warm store).
    let (allocs_temp, ()) = measure_allocs(|| {
        let mut tx = store.tx();
        tx.with_temp_resources(|res| {
            for _ in 0..8 {
                assert!(!res.intern(&BenchBlob).is_none());
            }
        });
        tx.commit_into(&mut changes);
    });
    assert_eq!(allocs_temp, 0, "TempResources with <= 8 temporaries must allocate 0 bytes");

    let (allocs_apply_patch, ()) = measure_allocs(|| {
        let mut tx = store.tx();
        tx.apply_row_patch(nodes[0], nested_pv);
        tx.commit_into(&mut changes);
    });
    assert_eq!(
        allocs_apply_patch, 0,
        "steady-state tx.apply_row_patch must allocate 0 bytes"
    );

    let (allocs_wire_topo, wire_len) = measure_allocs(|| {
        {
            let mut tx = store.tx();
            tx.edit::<BenchLabel>(nodes[3]).patch_res_ephemeral_with(|out| {
                out.extend_from_slice(&[
                    1,
                    BenchLabel::KEY.lo(),
                    BenchLabel::KEY.hi(),
                    1,
                    u32::from(BenchLabel::BLOB.word()) | (u32::from(nve_store::WORD_OP_RESOURCE) << 16),
                    u32::MAX,
                    synced_child_id.0,
                ]);
                true
            });
            tx.commit_into(&mut changes);
        }
        wire_buf.clear();
        enc.encode(&store, &changes, &mut wire_buf);
        wire_buf.len()
    });
    assert!(wire_len > 0);
    assert_eq!(
        allocs_wire_topo, 0,
        "steady-state topological WireEncoder::encode with nested child resource must allocate 0 bytes"
    );

    // 8. Steady-state SurfaceEventTracker::sync + EventBatchBuf + ValidatedEvents = 0 allocs.
    let mut tracker = SurfaceEventTracker::new();
    tracker.track::<BenchEvents>();
    tracker.reserve_nodes(store.capacity());
    let mut ev_buf = EventBatchBuf::with_capacity(16, 256);
    {
        let mut tx = store.tx();
        tx.edit::<BenchEvents>(nodes[0]).listen(BenchEvent::Click);
        tx.commit_into(&mut changes);
        tracker.sync(&store, &changes, |_, _, _| {});
    }
    let (allocs_events, ev_transitions) = measure_allocs(|| {
        let mut t_count = 0usize;
        let mut tx = store.tx();
        tx.edit::<BenchEvents>(nodes[0]).listen(BenchEvent::Hover);
        tx.commit_into(&mut changes);
        tracker.sync(&store, &changes, |_, _, _| {
            t_count += 1;
        });

        let mut tx = store.tx();
        tx.edit::<BenchEvents>(nodes[0]).unlisten(BenchEvent::Hover);
        tx.commit_into(&mut changes);
        tracker.sync(&store, &changes, |_, _, _| {
            t_count += 1;
        });

        ev_buf.clear();
        ev_buf.push_press(SurfaceId::ROOT, nodes[0], TimestampUs(99), 12.0, 34.0);
        ev_buf.push_key_down(
            SurfaceId::ROOT,
            nodes[0],
            TimestampUs(100),
            "Enter",
            "Enter",
            EventFlags::MOD_META,
            13,
        );
        let validated = ev_buf.as_validated();
        for ev in validated.iter() {
            t_count += ev.key().len();
        }
        t_count
    });
    assert!(ev_transitions >= 2);
    assert_eq!(
        allocs_events, 0,
        "steady-state SurfaceEventTracker::sync + EventBatchBuf + ValidatedEvents must allocate 0 bytes"
    );
}
