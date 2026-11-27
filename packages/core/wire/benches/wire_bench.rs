//! Criterion micro-benchmarks for `nve-wire`: constant-time Bearer token auth and `NveWireFrame`
//! v2 XXH64 framing roundtrip.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_wire::{
        generate_bearer_token, verify_bearer_token, NveWireFrame, NveWireFrameRef, NveWireKind,
        DEFAULT_MAX_WIRE_FRAME_BYTES,
    };

    fn bench_wire_auth_and_framing(c: &mut Criterion) {
        let token = generate_bearer_token();
        let wrong = generate_bearer_token();
        let frame = NveWireFrame::new(NveWireKind::CommandDelta, 7, vec![0xABu8; 1024]);
        let mut buf = Vec::with_capacity(1088);

        c.bench_function("wire_verify_bearer_token_match", |b| {
            b.iter(|| black_box(verify_bearer_token(black_box(&token), black_box(&token))));
        });

        c.bench_function("wire_verify_bearer_token_mismatch", |b| {
            b.iter(|| black_box(verify_bearer_token(black_box(&token), black_box(&wrong))));
        });

        c.bench_function("wire_frame_1kb_xxh64_roundtrip", |b| {
            b.iter(|| {
                black_box(&frame)
                    .encode_into(&mut buf)
                    .expect("valid wire frame encode");
                let decoded = NveWireFrameRef::decode(&buf, DEFAULT_MAX_WIRE_FRAME_BYTES)
                    .expect("valid wire frame decode");
                black_box(decoded.payload.len())
            });
        });
    }

    criterion_group!(benches, bench_wire_auth_and_framing);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
