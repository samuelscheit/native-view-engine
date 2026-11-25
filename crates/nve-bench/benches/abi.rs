use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nve_abi::{
    decode_events, encode_events, CommandDecoder, NveLayoutApi, NveSizeConstraints, NveSurfaceApi,
    NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
use nve_bench::{build_event_batch, build_node_tree_cmd, nve_bench_get_interface};

fn bench_command_buffer_codec(c: &mut Criterion) {
    let mut encode_group = c.benchmark_group("abi/command_buffer_encode");
    for &nodes in &[100usize, 1_000, 10_000] {
        encode_group.throughput(Throughput::Elements(nodes as u64));
        encode_group.bench_with_input(BenchmarkId::from_parameter(nodes), &nodes, |b, &n| {
            b.iter(|| black_box(build_node_tree_cmd(black_box(n), 1)));
        });
    }
    encode_group.finish();

    let mut decode_group = c.benchmark_group("abi/command_buffer_decode");
    for &nodes in &[100usize, 1_000, 10_000] {
        let encoded = build_node_tree_cmd(nodes, 1);
        decode_group.throughput(Throughput::Bytes(encoded.len() as u64));
        decode_group.bench_with_input(BenchmarkId::from_parameter(nodes), &encoded, |b, bytes| {
            b.iter(|| {
                let decoded = CommandDecoder::new(black_box(bytes))
                    .unwrap()
                    .decode_all()
                    .unwrap();
                black_box(decoded)
            });
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
            b.iter(|| black_box(decode_events(black_box(bytes)).unwrap()));
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
    let layout_api = unsafe {
        let ptr = nve_bench_get_interface(NVE_IFACE_LAYOUT) as *const NveLayoutApi;
        assert!(!ptr.is_null());
        *ptr
    };
    let handle = unsafe { (surface_api.create)(1) };

    group.bench_function("lookup_interface", |b| {
        b.iter(|| unsafe { black_box(nve_bench_get_interface(black_box(NVE_IFACE_LAYOUT))) });
    });

    let constraints = NveSizeConstraints::loose(393.0, 852.0);
    group.bench_function("scalar_measure", |b| {
        b.iter(|| unsafe {
            black_box((layout_api.measure)(
                black_box(handle),
                black_box(constraints),
            ))
        });
    });

    group.bench_function("scalar_hit_test", |b| {
        b.iter(|| unsafe {
            black_box((layout_api.hit_test)(
                black_box(handle),
                black_box(120.0),
                black_box(80.0),
                black_box(1),
            ))
        });
    });

    group.bench_function("surface_render_slice", |b| {
        let mut frame = 1u64;
        b.iter(|| {
            frame += 1;
            unsafe { black_box((surface_api.render)(black_box(handle), black_box(frame))) }
        });
    });

    unsafe { (surface_api.destroy)(handle) };
    group.finish();
}

criterion_group!(
    abi_benches,
    bench_command_buffer_codec,
    bench_event_ring_codec,
    bench_ffi_interface_overhead
);
criterion_main!(abi_benches);
