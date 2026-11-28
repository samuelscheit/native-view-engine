//! Criterion micro-benchmarks for `nve-wire-core`: `NveNodePatchDelta` encode/decode and
//! `NveWireFrame` v2 XXH64 framing roundtrip.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{DurationMs, NodeId};
    use nve_wire_core::{
        parse_cert_hash_hex, NveNodePatch, NveNodePatchDeltaBuilder, NveNodePatchDeltaView,
        NveWireFrame, NveWireFrameRef, NveWireKind, DEFAULT_MAX_WIRE_FRAME_BYTES,
    };

    fn bench_wire_core(c: &mut Criterion) {
        let mut builder = NveNodePatchDeltaBuilder::with_capacity(32, 256);
        for id in 1..=16u32 {
            let nid = NodeId::new(id, 0);
            builder
                .set_text(nid, "status: active")
                .set_opacity(nid, 0.85, DurationMs(120));
        }
        let delta_bytes = builder.encode().expect("valid bench patch delta");
        let frame = NveWireFrame::new(NveWireKind::NodePatchDelta, 42, delta_bytes.clone());
        let wire_bytes = frame.encode().expect("valid bench wire frame");
        let mut scratch = Vec::with_capacity(wire_bytes.len());
        let mut patch_scratch: Vec<NveNodePatch> = Vec::with_capacity(32);
        let cert_hex = "ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89:ab:cd:ef:01:23:45:67:89";

        c.bench_function("wire_core_node_patch_delta_encode_32", |b| {
            b.iter(|| {
                let encoded = black_box(&builder)
                    .encode()
                    .expect("valid bench delta encode");
                black_box(encoded.len())
            });
        });

        c.bench_function("wire_core_node_patch_delta_decode_32", |b| {
            b.iter(|| {
                let view = NveNodePatchDeltaView::decode(black_box(&delta_bytes))
                    .expect("valid bench delta decode");
                black_box(view.patches().len())
            });
        });

        c.bench_function("wire_core_node_patch_delta_decode_into_32", |b| {
            b.iter(|| {
                let strs = NveNodePatchDeltaView::decode_into(
                    black_box(&delta_bytes),
                    &mut patch_scratch,
                )
                .expect("valid bench delta decode_into");
                black_box((patch_scratch.len(), strs.len()))
            });
        });

        c.bench_function("wire_core_parse_cert_hash_hex", |b| {
            b.iter(|| {
                let hash = parse_cert_hash_hex(black_box(cert_hex)).expect("valid cert hash");
                black_box(hash[0])
            });
        });

        c.bench_function("wire_core_frame_xxh64_encode_decode", |b| {
            b.iter(|| {
                black_box(&frame)
                    .encode_into(&mut scratch)
                    .expect("valid bench frame encode");
                let decoded =
                    NveWireFrameRef::decode(&scratch, DEFAULT_MAX_WIRE_FRAME_BYTES)
                        .expect("valid bench frame decode");
                black_box(decoded.payload.len())
            });
        });
    }

    criterion_group!(benches, bench_wire_core);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
