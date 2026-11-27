//! Criterion micro-benchmarks for `nve-store` hot paths:
//! - `StrArena::str` (16 B and 4 KB strings)
//! - `row_write` (branchless scalar word writes)
//! - `node_multi_read` (`NodeRef` validated single-check multi-table reads)
//! - `subtree_remove` (zero-alloc iterative subtree removal)
//! - `commit_into` (dirty-list drain + ChangeSet refill)
//! - `nve2_roundtrip` (table-major wire encode + replica decode)

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
use std::hint::black_box;

use criterion::{criterion_group, BatchSize, Criterion};
use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{
    ChangeSet, GridLine, InheritedTable, Length, NodeId, NodeStore, NveTable, ResolvedRow, Rgba,
    Str, WordEnum,
};

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
    max_lines: Option<u32>,
}

fn bench_str_arena_read(c: &mut Criterion) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (n16, n4k) = {
        let mut tx = store.tx();
        let a = tx.create();
        tx.edit::<BenchLabel>(a).text("0123456789abcdef");
        let b = tx.create();
        let big = "a".repeat(4096);
        tx.edit::<BenchLabel>(b).text(&big);
        tx.commit_into(&mut changes);
        (a, b)
    };
    let s16 = store.row::<BenchLabel>(n16).text_span();
    let s4k = store.row::<BenchLabel>(n4k).text_span();

    c.bench_function("row_ref_str_read_16b", |b| {
        b.iter(|| black_box(store.row::<BenchLabel>(black_box(n16)).text()).len());
    });
    c.bench_function("row_ref_str_read_4kb", |b| {
        b.iter(|| black_box(store.row::<BenchLabel>(black_box(n4k)).text()).len());
    });
    c.bench_function("detached_str_read_16b", |b| {
        b.iter(|| black_box(store.text(black_box(s16))).len());
    });
    c.bench_function("detached_str_read_4kb", |b| {
        b.iter(|| black_box(store.text(black_box(s4k))).len());
    });
}

fn bench_row_write_and_commit(c: &mut Criterion) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let nodes: Vec<NodeId> = {
        let mut tx = store.tx();
        let ids: Vec<NodeId> = (0..1000)
            .map(|i| {
                let mut nm = tx.spawn();
                nm.edit::<BenchBox>()
                    .width(Length::points(i as f32))
                    .opacity(1.0);
                nm.id()
            })
            .collect();
        tx.set_children(NodeId::ROOT, &ids);
        tx.commit_into(&mut changes);
        ids
    };

    let mut tick = 0u32;
    c.bench_function("row_write_1000_nodes", |b| {
        b.iter(|| {
            tick = tick.wrapping_add(1);
            let mut tx = store.tx();
            for &n in &nodes {
                tx.set(n, BenchBox::OPACITY, (tick & 255) as f32 / 255.0);
            }
            tx.commit_into(&mut changes);
            black_box(changes.touched.len())
        });
    });

    c.bench_function("node_multi_read_1000_nodes", |b| {
        b.iter(|| {
            let mut sum = 0.0f32;
            for &n in &nodes {
                let nr = store.get(n).expect("live bench node");
                let r = nr.row::<BenchBox>();
                sum += r.opacity() + f32::from(r.bg().r());
                sum += nr.children().len() as f32;
            }
            black_box(sum)
        });
    });

    c.bench_function("opt_wide_64_grid_write_1000_nodes", |b| {
        b.iter(|| {
            tick = tick.wrapping_add(1);
            let mut tx = store.tx();
            let toggle = tick & 1 == 0;
            for &n in &nodes {
                tx.edit::<BenchBox>(n)
                    .grid_col(if toggle { GridLine::Span(2) } else { GridLine::Line(3) })
                    .opt_align(if toggle { Some(BenchAlign::Center) } else { None })
                    .opt_tint(if toggle { Some(Rgba::rgb(12, 34, 56)) } else { None })
                    .opt_ratio(if toggle { Some(1.777) } else { None })
                    .stamp(u64::from(tick) << 32 | u64::from(tick));
            }
            tx.commit_into(&mut changes);
            black_box(changes.touched.len())
        });
    });

    // Populate non-default OptWord/WideOpt/Wide64/GridLine values once so the read benchmark
    // works standalone when filtered out without mutating inside the benchmark closure.
    {
        let mut tx = store.tx();
        for (i, &n) in nodes.iter().enumerate() {
            tx.edit::<BenchBox>(n)
                .grid_col(if i & 1 == 0 { GridLine::Span(2) } else { GridLine::Line(3) })
                .opt_align(if i & 1 == 0 { Some(BenchAlign::Center) } else { None })
                .opt_tint(if i & 1 == 0 { Some(Rgba::rgb(12, 34, 56)) } else { None })
                .opt_ratio(Some(1.777))
                .stamp((i as u64) << 32 | i as u64);
        }
        tx.commit_into(&mut changes);
    }

    c.bench_function("opt_wide_64_grid_read_1000_nodes", |b| {
        b.iter(|| {
            let mut acc = 0u64;
            for &n in &nodes {
                let r = store.row::<BenchBox>(n);
                acc = acc
                    .wrapping_add(u64::from(r.grid_col().to_word()))
                    .wrapping_add(u64::from(r.opt_align().map_or(0, |a| a as u32)))
                    .wrapping_add(u64::from(r.opt_tint().map_or(0, |c| c.0)))
                    .wrapping_add(u64::from(r.opt_ratio().map_or(0, f32::to_bits)))
                    .wrapping_add(r.stamp());
            }
            black_box(acc)
        });
    });

    c.bench_function("commit_into_100_dirty", |b| {
        b.iter_batched(
            || {
                tick = tick.wrapping_add(1);
                let val = (tick & 255) as f32 / 255.0;
                // Dirty 100 rows in setup (outside timed measurement) by cloning a tiny pre-dirtied store?
                // Even cleaner: mutate inside setup? `Transaction` borrows `&mut store`, so time
                // pure `commit_into` on a freshly prepared store copy of the 100-node subset.
                let mut s = NodeStore::new();
                let mut c0 = ChangeSet::default();
                let mut ids = Vec::with_capacity(100);
                {
                    let mut tx = s.tx();
                    for _ in 0..100 {
                        let mut nm = tx.spawn();
                        nm.edit::<BenchBox>().opacity(1.0);
                        ids.push(nm.id());
                    }
                    tx.commit_into(&mut c0);
                }
                (s, ids, val, c0)
            },
            |(mut s, ids, val, mut c0)| {
                let mut tx = s.tx();
                for &n in &ids {
                    tx.set(n, BenchBox::OPACITY, val);
                }
                tx.commit_into(&mut c0);
                black_box(c0.touched.len())
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_subtree_remove(c: &mut Criterion) {
    c.bench_function("subtree_remove_32_nodes", |b| {
        b.iter_batched(
            || {
                let mut store = NodeStore::new();
                let mut changes = ChangeSet::default();
                let mut tx = store.tx();
                let root = tx.create();
                let mut kids = [NodeId::NULL; 31];
                for k in &mut kids {
                    let mut nm = tx.spawn();
                    nm.edit::<BenchBox>().opacity(0.9);
                    *k = nm.id();
                }
                tx.set_children(root, &kids);
                tx.append_child(NodeId::ROOT, root);
                tx.commit_into(&mut changes);
                (store, root, changes)
            },
            |(mut store, root, mut changes)| {
                let mut tx = store.tx();
                tx.remove(root);
                tx.commit_into(&mut changes);
                black_box(changes.removed.len())
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_nve2_roundtrip(c: &mut Criterion) {
    let mut store = NodeStore::new();
    let mut replica = NodeStore::replica();
    let mut changes = ChangeSet::default();
    let mut rep_changes = ChangeSet::default();
    let mut enc = WireEncoder::default();
    let mut dec = WireDecoder::default();
    let mut buf = Vec::with_capacity(64 * 1024);

    let nodes: Vec<NodeId> = {
        let mut tx = store.tx();
        let ids: Vec<NodeId> = (0..100)
            .map(|i| {
                let mut nm = tx.spawn();
                nm.edit::<BenchBox>().width(Length::points(i as f32));
                nm.edit::<BenchLabel>().text("hello-wire");
                nm.id()
            })
            .collect();
        tx.set_children(NodeId::ROOT, &ids);
        tx.commit_into(&mut changes);
        ids
    };
    enc.encode(&store, &changes, &mut buf);
    {
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &buf).expect("decode initial bench wire frame");
        rtx.commit_into(&mut rep_changes);
    }

    let mut tick = 0u32;
    c.bench_function("nve2_roundtrip_100_dirty_scalars", |b| {
        b.iter(|| {
            tick = tick.wrapping_add(1);
            let mut tx = store.tx();
            for &n in &nodes {
                tx.set(n, BenchBox::OPACITY, (tick & 255) as f32 / 255.0);
            }
            tx.commit_into(&mut changes);
            buf.clear();
            enc.encode(&store, &changes, &mut buf);
            let mut rtx = replica.tx();
            dec.apply(&mut rtx, &buf).expect("decode scalar bench wire frame");
            rtx.commit_into(&mut rep_changes);
            black_box(buf.len())
        });
    });

    // Mixed workload: words + strings + children reorder in one frame.
    let mut reversed: Vec<NodeId> = nodes.iter().copied().rev().collect();
    c.bench_function("nve2_roundtrip_mixed_words_str_children", |b| {
        b.iter(|| {
            tick = tick.wrapping_add(1);
            let mut tx = store.tx();
            for &n in &nodes[..25] {
                tx.set(n, BenchBox::OPACITY, (tick & 255) as f32 / 255.0);
            }
            for &n in &nodes[25..50] {
                tx.edit::<BenchLabel>(n).text(if tick & 1 == 0 {
                    "mixed-even"
                } else {
                    "mixed-odds"
                });
            }
            let order = if tick & 1 == 0 { &reversed } else { &nodes };
            tx.set_children(NodeId::ROOT, order);
            tx.commit_into(&mut changes);
            buf.clear();
            enc.encode(&store, &changes, &mut buf);
            let mut rtx = replica.tx();
            dec.apply(&mut rtx, &buf).expect("decode mixed bench wire frame");
            rtx.commit_into(&mut rep_changes);
            black_box(buf.len())
        });
    });
    let _ = &mut reversed;
}

fn bench_inherited_resolve(c: &mut Criterion) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (parent, nodes): (NodeId, Vec<NodeId>) = {
        let mut tx = store.tx();
        let p = tx.create();
        tx.edit::<BenchTypo>(p)
            .color(Rgba::WHITE)
            .font_size(20.0)
            .font_weight(700)
            .font_family("Inter")
            .max_lines(Some(2));
        let ids: Vec<NodeId> = (0..1000)
            .map(|i| {
                let n = tx.create();
                if i % 3 == 0 {
                    tx.edit::<BenchTypo>(n).color(Rgba::rgb(i as u8, 128, 255));
                }
                n
            })
            .collect();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &ids);
        tx.commit_into(&mut changes);
        (p, ids)
    };

    c.bench_function("inherited_resolved_row_1000_nodes", |b| {
        b.iter(|| {
            let root_r = ResolvedRow::<BenchTypo>::root(&store);
            let p_r = ResolvedRow::<BenchTypo>::resolve(&store, black_box(parent), &root_r);
            let mut acc = 0u64;
            for &n in &nodes {
                let r = ResolvedRow::<BenchTypo>::resolve(&store, black_box(n), &p_r);
                let t = r.get();
                acc = acc
                    .wrapping_add(u64::from(t.color().0))
                    .wrapping_add(u64::from(t.font_size().to_bits()))
                    .wrapping_add(u64::from(t.font_weight()));
            }
            black_box(acc)
        });
    });

    let p_words = ResolvedRow::<BenchTypo>::resolve(&store, parent, &ResolvedRow::root(&store))
        .into_words();
    let c_words = ResolvedRow::<BenchTypo>::resolve(&store, nodes[0], &ResolvedRow::root(&store))
        .into_words();
    c.bench_function("inherited_resolve_arrays_branchless", |b| {
        b.iter(|| {
            let mut out = <BenchTypo as InheritedTable>::default_words();
            BenchTypo::resolve_inherited_arrays(black_box(&c_words), black_box(&p_words), &mut out);
            black_box(out)
        });
    });
}

criterion_group!(
    benches,
    bench_str_arena_read,
    bench_row_write_and_commit,
    bench_subtree_remove,
    bench_nve2_roundtrip,
    bench_inherited_resolve
);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
