use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nve_rpc::{validate_cbor, CborCodec, CborLimits, RpcCodec, Tag64};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode, Serialize, Deserialize)]
#[cbor(map)]
struct IssueSummary {
    #[n(0)]
    id: u64,
    #[n(1)]
    title: String,
    #[n(2)]
    state: u8,
    #[n(3)]
    priority: Option<u8>,
    #[n(4)]
    comment_count: u32,
}

fn sample_issues(count: usize) -> Vec<IssueSummary> {
    (0..count)
        .map(|i| IssueSummary {
            id: i as u64 + 1,
            title: format!("Issue #{i}: refine layout & server hydration"),
            state: (i % 3) as u8,
            priority: (i % 2 == 0).then_some((i % 4) as u8),
            comment_count: (i * 3) as u32,
        })
        .collect()
}

fn bench_cbor_vs_json(c: &mut Criterion) {
    let codec = CborCodec::default();
    let mut group = c.benchmark_group("nve_rpc_codec");

    for count in [1usize, 50, 500] {
        let issues = sample_issues(count);
        let mut cbor_buf = Vec::with_capacity(count * 64);
        codec.encode_into(&issues, &mut cbor_buf).expect("encode cbor fixture");
        let json_buf = serde_json::to_vec(&issues).expect("encode json fixture");
        eprintln!(
            "[wire-size] count={count}: cbor={} B, json={} B ({:.1}% of JSON)",
            cbor_buf.len(),
            json_buf.len(),
            (cbor_buf.len() as f64 / json_buf.len() as f64) * 100.0
        );

        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(BenchmarkId::new("cbor_encode", count), &issues, |b, data| {
            let mut scratch = Vec::with_capacity(cbor_buf.len() + 64);
            b.iter(|| {
                scratch.clear();
                codec.encode_into(black_box(data), &mut scratch).expect("encode cbor");
                black_box(scratch.len())
            });
        });

        group.bench_with_input(BenchmarkId::new("json_encode", count), &issues, |b, data| {
            let mut scratch = Vec::with_capacity(json_buf.len() + 64);
            b.iter(|| {
                scratch.clear();
                serde_json::to_writer(&mut scratch, black_box(data)).expect("encode json");
                black_box(scratch.len())
            });
        });

        group.bench_with_input(
            BenchmarkId::new("cbor_validate_and_decode", count),
            &cbor_buf,
            |b, wire| {
                b.iter(|| {
                    let decoded: Vec<IssueSummary> =
                        codec.decode_bounded(black_box(wire)).expect("decode cbor");
                    black_box(decoded.len())
                });
            },
        );

        group.bench_with_input(BenchmarkId::new("json_decode", count), &json_buf, |b, wire| {
            b.iter(|| {
                let decoded: Vec<IssueSummary> =
                    serde_json::from_slice(black_box(wire)).expect("decode json");
                black_box(decoded.len())
            });
        });

        group.bench_with_input(
            BenchmarkId::new("cbor_validate_only", count),
            &cbor_buf,
            |b, wire| {
                b.iter(|| {
                    validate_cbor(black_box(wire), CborLimits::DEFAULT).expect("validate cbor");
                });
            },
        );
    }

    // Worst-case validator benchmark: 64-deep nested arrays (at default max_depth).
    let mut deep_64 = vec![0x81u8; 64];
    deep_64.push(0x00);
    group.bench_function("cbor_validate_worst_depth_64", |b| {
        b.iter(|| {
            validate_cbor(black_box(&deep_64), CborLimits::DEFAULT).expect("validate deep_64")
        });
    });

    group.bench_function("tag64_keyed", |b| {
        b.iter(|| black_box(Tag64::keyed(black_box("issue"), black_box(42u64))));
    });

    group.finish();
}

criterion_group!(benches, bench_cbor_vs_json);
criterion_main!(benches);
