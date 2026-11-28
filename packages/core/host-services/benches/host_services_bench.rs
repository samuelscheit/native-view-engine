//! Criterion micro-benchmarks for `nve-host-services`: `ByteWriter` / `ByteReader` binary framing
//! and URL validation.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_host_services::{validate_url, ByteReader, ByteWriter, MAX_PAYLOAD_BYTES};

    fn bench_bin_codec(c: &mut Criterion) {
        c.bench_function("host_services_bin_roundtrip", |b| {
            b.iter(|| {
                let mut w = ByteWriter::with_capacity(128);
                w.write_u32(black_box(0x0023));
                w.write_u64(black_box(1_700_000_000));
                w.write_bool(true);
                w.write_f32(black_box(2.0));
                w.write_str(black_box("nve://workspace/project/42"))
                    .expect("valid bench string write");
                let bytes = w.into_bytes();

                let mut r = ByteReader::new(&bytes);
                let svc = r.read_u32().expect("read u32");
                let ts = r.read_u64().expect("read u64");
                let ok = r.read_bool().expect("read bool");
                let scale = r.read_f32().expect("read f32");
                let url = r.read_str(MAX_PAYLOAD_BYTES).expect("read str");
                black_box((svc, ts, ok, scale, url.len()))
            });
        });

        let urls = [
            "https://example.com/docs/getting-started",
            "nve://studio/panel/42",
            "mailto:dev@example.com",
            "invalid-without-scheme",
        ];
        c.bench_function("host_services_validate_url_batch", |b| {
            b.iter(|| {
                let mut valid = 0usize;
                for &u in black_box(&urls) {
                    if validate_url(u).is_ok() {
                        valid += 1;
                    }
                }
                black_box(valid)
            });
        });
    }

    criterion_group!(benches, bench_bin_codec);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
