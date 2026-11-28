use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use nve_abi::{
    decode_events, encode_bin_value_to_vec, encode_events, BinValueView, NveLayoutApi, NveMapEntry,
    NveSizeConstraints, NveSurfaceApi, NveValue, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
use nve_bench::{build_event_batch, build_node_tree_cmd, nve_bench_get_interface};
use nve_components::tables::register_core_tables;
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, NodeStore};

fn bench_nve2_wire_codec(c: &mut Criterion) {
    register_core_tables();

    let mut encode_group = c.benchmark_group("abi/nve2_wire_encode");
    for &nodes in &[100usize, 1_000, 10_000] {
        encode_group.throughput(Throughput::Elements(nodes as u64));
        encode_group.bench_with_input(BenchmarkId::from_parameter(nodes), &nodes, |b, &n| {
            b.iter(|| black_box(build_node_tree_cmd(black_box(n), 1)));
        });
    }
    encode_group.finish();

    let mut decode_group = c.benchmark_group("abi/nve2_wire_decode");
    for &nodes in &[100usize, 1_000, 10_000] {
        let encoded = build_node_tree_cmd(nodes, 1);
        decode_group.throughput(Throughput::Bytes(encoded.len() as u64));
        decode_group.bench_with_input(BenchmarkId::from_parameter(nodes), &encoded, |b, bytes| {
            b.iter_batched(
                || (NodeStore::replica(), WireDecoder::default(), ChangeSet::default()),
                |(mut store, mut dec, mut changes)| {
                    {
                        let mut tx = store.tx();
                        dec.apply(&mut tx, black_box(bytes))
                            .expect("decode valid bench wire frame");
                        tx.commit_into(&mut changes);
                    }
                    black_box(changes.created.len())
                },
                BatchSize::SmallInput,
            );
        });
    }
    decode_group.finish();
}

fn bench_event_ring_codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("abi/event_ring");
    for &count in &[16usize, 256] {
        let events = build_event_batch(count);
        let encoded = encode_events(&events);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("encode", count), &events, |b, evs| {
            b.iter(|| black_box(encode_events(black_box(evs))));
        });
        group.bench_with_input(BenchmarkId::new("decode", count), &encoded, |b, bytes| {
            b.iter(|| black_box(decode_events(black_box(bytes)).expect("decode valid bench events")));
        });
    }
    group.finish();
}

fn bench_ffi_interface_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("abi/nve_get_interface");

    // SAFETY: `nve_bench_get_interface` is exported by `nve_engine::export_nve_app!` in `nve-bench`.
    let surface_api = unsafe {
        let ptr = nve_bench_get_interface(NVE_IFACE_SURFACE) as *const NveSurfaceApi;
        assert!(!ptr.is_null());
        *ptr
    };
    // SAFETY: `nve_bench_get_interface` exports a static `NveLayoutApi` table for `NVE_IFACE_LAYOUT`.
    let layout_api = unsafe {
        let ptr = nve_bench_get_interface(NVE_IFACE_LAYOUT) as *const NveLayoutApi;
        assert!(!ptr.is_null());
        *ptr
    };
    // SAFETY: `surface_api.create` returns an opaque handle valid until `surface_api.destroy`.
    let handle = unsafe { (surface_api.create)(1) };

    group.bench_function("lookup_interface", |b| {
        b.iter(|| {
            // SAFETY: Static C ABI interface lookup with a valid interface id.
            unsafe { black_box(nve_bench_get_interface(black_box(NVE_IFACE_LAYOUT))) }
        });
    });

    let constraints = NveSizeConstraints::loose(393.0, 852.0);
    group.bench_function("scalar_measure", |b| {
        b.iter(|| {
            // SAFETY: `handle` is live until destroyed after this benchmark group.
            unsafe {
                black_box((layout_api.measure)(
                    black_box(handle),
                    black_box(constraints),
                ))
            }
        });
    });

    group.bench_function("scalar_hit_test", |b| {
        b.iter(|| {
            // SAFETY: `handle` is live until destroyed after this benchmark group.
            unsafe {
                black_box((layout_api.hit_test)(
                    black_box(handle),
                    black_box(120.0),
                    black_box(80.0),
                    black_box(1),
                ))
            }
        });
    });

    group.bench_function("surface_render_slice", |b| {
        let mut frame = 1u64;
        b.iter(|| {
            frame += 1;
            // SAFETY: `handle` is live until destroyed after this benchmark group.
            unsafe { black_box((surface_api.render)(black_box(handle), black_box(frame))) }
        });
    });

    // SAFETY: `handle` was created by `surface_api.create` and is destroyed once here.
    unsafe { (surface_api.destroy)(handle) };
    group.finish();
}

fn bench_zero_copy_value_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("abi/value");

    let velocity = [NveValue::f64(12.5), NveValue::f64(-4.25)];
    let entries = [
        NveMapEntry::new("event", NveValue::from_str("onGestureUpdate")),
        NveMapEntry::new("x", NveValue::f64(142.25)),
        NveMapEntry::new("y", NveValue::f64(84.5)),
        NveMapEntry::new("active", NveValue::bool(true)),
        NveMapEntry::new("velocity", NveValue::array(&velocity)),
    ];
    let root = NveValue::map(&entries);
    // SAFETY: `entries` and `velocity` outlive `root` for this entire function.
    let encoded = encode_bin_value_to_vec(unsafe { root.as_ref() })
        .expect("encode valid bench NveValue");

    group.bench_function("value_direct_ffi_read", |b| {
        b.iter(|| {
            // SAFETY: Backing `entries` and `velocity` arrays stay live across all iterations.
            let v = unsafe { black_box(&root).as_ref() };
            let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
            let x = v.get("x").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let active = v.get("active").and_then(|x| x.as_bool()).unwrap_or(false);
            let vx = v
                .get("velocity")
                .and_then(|a| a.as_array())
                .and_then(|a| a.get(0))
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            black_box((ev.len(), x + vx, active))
        });
    });

    group.bench_function("value_binary_view_read", |b| {
        b.iter(|| {
            let v = BinValueView::parse(black_box(&encoded)).expect("parse valid bench BinValue");
            let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
            let x = v.get("x").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let active = v.get("active").and_then(|x| x.as_bool()).unwrap_or(false);
            let vx = v
                .get("velocity")
                .and_then(|a| a.array_at(0))
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            black_box((ev.len(), x + vx, active))
        });
    });

    group.finish();
}

criterion_group!(
    abi_benches,
    bench_nve2_wire_codec,
    bench_event_ring_codec,
    bench_ffi_interface_overhead,
    bench_zero_copy_value_read
);
criterion_main!(abi_benches);
