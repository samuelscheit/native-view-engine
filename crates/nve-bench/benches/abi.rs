use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nve_abi::{
    decode_events, encode_bin_value_to_vec, encode_events, BinValueView, CommandDecoder,
    NveLayoutApi, NveMapEntry, NveSceneRef, NveSceneTable, NveSizeConstraints, NveSurfaceApi,
    NveValue, SceneNodeInput, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE, NVE_NODE_INTERACTIVE,
    NVE_NODE_KIND_PRESSABLE, NVE_NODE_KIND_ROOT, NVE_NODE_KIND_TEXT, NVE_NODE_LEAF_TEXT,
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
    let encoded = encode_bin_value_to_vec(unsafe { root.as_ref() }).unwrap();

    group.bench_function("value_direct_ffi_read", |b| {
        b.iter(|| {
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
            let v = BinValueView::parse(black_box(&encoded)).unwrap();
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

fn bench_scene_view_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("abi/scene");

    let mut table = NveSceneTable::with_capacity(1000);
    table.begin_frame(1);
    let children_root: Vec<u32> = (2..=1000u32).collect();
    for id in 1..=1000u32 {
        let label = if id == 1 {
            ""
        } else if id % 2 == 0 {
            "Metric Card Value"
        } else {
            "Button Action"
        };
        let children: &[u32] = if id == 1 { &children_root } else { &[] };
        table
            .upsert_node(SceneNodeInput {
                node_id: id,
                kind: if id == 1 {
                    NVE_NODE_KIND_ROOT
                } else if id % 4 == 0 {
                    NVE_NODE_KIND_PRESSABLE
                } else {
                    NVE_NODE_KIND_TEXT
                },
                flags: if id % 4 == 0 {
                    NVE_NODE_INTERACTIVE
                } else {
                    NVE_NODE_LEAF_TEXT
                },
                opacity_u8: 255,
                x: (id % 10) as f32 * 40.0,
                y: (id / 10) as f32 * 24.0,
                width: 120.0,
                height: 36.0,
                bg_rgba: 0xFF1E_293B,
                text_rgba: 0xFFF8_FAFC,
                border_rgba: 0xFF33_4155,
                border_radius: 8.0,
                border_width: 1.0,
                font_size: 14.0,
                z_index: 0,
                text: label,
                children,
            })
            .unwrap();
    }

    // Mutate only 3 nodes out of 1000 for the incremental frame benchmark.
    table.begin_frame(2);
    for &id in &[17u32, 333, 987] {
        table
            .upsert_node(SceneNodeInput {
                node_id: id,
                kind: NVE_NODE_KIND_TEXT,
                flags: NVE_NODE_LEAF_TEXT,
                opacity_u8: 240,
                x: 12.0,
                y: 24.0,
                width: 140.0,
                height: 36.0,
                bg_rgba: 0xFF0F_172A,
                text_rgba: 0xFFFF_FFFF,
                border_rgba: 0xFF38_BDF8,
                border_radius: 10.0,
                border_width: 1.5,
                font_size: 15.0,
                z_index: 2,
                text: "Counter: 42",
                children: &[],
            })
            .unwrap();
    }
    let view = table.as_view();
    assert_eq!(view.node_count, 1000);
    assert_eq!(view.dirty_count, 3);

    group.throughput(Throughput::Elements(3));
    group.bench_function("scene_view_read_dirty_3_of_1000", |b| {
        b.iter(|| {
            let scene = unsafe { NveSceneRef::from_raw(black_box(view)) }.unwrap();
            let mut acc_x = 0.0f32;
            let mut acc_text = 0usize;
            for (_, node) in scene.dirty_nodes() {
                acc_x += node.x + node.width + node.opacity_f32();
                acc_text += scene.node_text(node).unwrap().len();
            }
            black_box((acc_x, acc_text))
        });
    });

    group.throughput(Throughput::Elements(1000));
    group.bench_function("scene_view_read_all_1000", |b| {
        b.iter(|| {
            let scene = unsafe { NveSceneRef::from_raw(black_box(view)) }.unwrap();
            let mut acc_x = 0.0f32;
            let mut acc_text = 0usize;
            for node in scene.nodes() {
                acc_x += node.x + node.width + node.opacity_f32();
                acc_text += scene.node_text(node).unwrap().len();
            }
            black_box((acc_x, acc_text))
        });
    });

    group.finish();
}

criterion_group!(
    abi_benches,
    bench_command_buffer_codec,
    bench_event_ring_codec,
    bench_ffi_interface_overhead,
    bench_zero_copy_value_read,
    bench_scene_view_read
);
criterion_main!(abi_benches);
