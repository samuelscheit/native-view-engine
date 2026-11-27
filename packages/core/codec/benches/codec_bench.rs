//! Criterion micro-benchmarks for `nve-codec` (`ByteWriter`/`ByteReader` + `#[derive(NveCodec)]`).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_codec::{ByteReader, ByteWriter, NveCodec, NveDecode, NveEncode};

    #[repr(u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, NveCodec)]
    enum BenchKind {
        Click = 1,
        Input = 2,
        Scroll = 3,
    }

    #[derive(Debug, Clone, PartialEq, NveCodec)]
    struct BenchRecord {
        kind: BenchKind,
        seq: u64,
        flags: u32,
        x: f32,
        y: f32,
        active: bool,
    }

    fn bench_codec(c: &mut Criterion) {
        let mut w = ByteWriter::with_capacity(4096);
        let sample = BenchRecord {
            kind: BenchKind::Input,
            seq: 1_700_000_042,
            flags: 0x0003_000F,
            x: 128.5,
            y: 256.25,
            active: true,
        };

        c.bench_function("codec_small_struct_reused_roundtrip", |b| {
            b.iter(|| {
                w.clear();
                sample.encode(&mut w).expect("encode");
                let mut r = ByteReader::new(black_box(w.as_slice()));
                let dec = BenchRecord::decode(&mut r).expect("decode");
                r.finish().expect("finish");
                black_box(dec)
            });
        });

        let batch: Vec<BenchRecord> = (0..128)
            .map(|i| BenchRecord {
                kind: BenchKind::Scroll,
                seq: i,
                flags: i as u32,
                x: i as f32 * 1.5,
                y: i as f32 * 2.25,
                active: (i & 1) == 0,
            })
            .collect();
        let mut batch_buf = ByteWriter::with_capacity(8192);

        c.bench_function("codec_realistic_128_records_roundtrip", |b| {
            b.iter(|| {
                batch_buf.clear();
                batch_buf.write_len(batch.len()).expect("len");
                for item in black_box(&batch) {
                    item.encode(&mut batch_buf).expect("encode");
                }
                let mut r = ByteReader::new(black_box(batch_buf.as_slice()));
                let n = r.read_len(4096, BenchRecord::MIN_ENCODED_BYTES).expect("len");
                let mut acc = 0u64;
                for _ in 0..n {
                    let rec = BenchRecord::decode(&mut r).expect("decode");
                    acc = acc.wrapping_add(rec.seq);
                }
                r.finish().expect("finish");
                black_box(acc)
            });
        });
    }

    criterion_group!(benches, bench_codec);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
