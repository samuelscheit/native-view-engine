//! Criterion micro-benchmarks for `nve-codec` (`ByteWriter`/`ByteReader` + `#[derive(NveCodec)]`).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion, Throughput};
    use nve_codec::{ByteReader, ByteWriter, NveCodec, NveDecode, NveEncode, SliceWriter};

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

        w.clear();
        sample.encode(&mut w).expect("encode");
        let small_bytes = w.len() as u64;

        let mut g_small = c.benchmark_group("codec_small");
        g_small.throughput(Throughput::Bytes(small_bytes));
        g_small.bench_function("struct_reused_roundtrip", |b| {
            b.iter(|| {
                w.clear();
                black_box(&sample).encode(&mut w).expect("encode");
                let mut r = ByteReader::new(black_box(w.as_slice()));
                let dec = BenchRecord::decode(&mut r).expect("decode");
                r.finish().expect("finish");
                black_box(dec)
            });
        });
        g_small.throughput(Throughput::Bytes(40));
        g_small.bench_function("slice_writer_40b_header", |b| {
            let mut dst = [0u8; 40];
            let fields = (1u8, 0u8, 90u16, 1920u32, 1080u32, 7680u32, 0u32, 123_456_789u64, 42u64);
            b.iter(|| {
                let (ver, fmt, rot, width, height, stride, res, ts, handle) = black_box(fields);
                let mut sw = SliceWriter::new(black_box(&mut dst));
                sw.write_raw(black_box(b"NVCF")).expect("magic");
                sw.write_u8(ver).expect("v");
                sw.write_u8(fmt).expect("fmt");
                sw.write_u16(rot).expect("rot");
                sw.write_u32(width).expect("w");
                sw.write_u32(height).expect("h");
                sw.write_u32(stride).expect("stride");
                sw.write_u32(res).expect("res");
                sw.write_u64(ts).expect("ts");
                sw.write_u64(handle).expect("handle");
                sw.finish().expect("finish");
                black_box(dst[39])
            });
        });
        g_small.finish();

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
        batch_buf.write_len(batch.len()).expect("len");
        for item in &batch {
            item.encode(&mut batch_buf).expect("encode");
        }
        let batch_bytes = batch_buf.len() as u64;

        let mut g_batch = c.benchmark_group("codec_realistic");
        g_batch.throughput(Throughput::Bytes(batch_bytes));
        g_batch.bench_function("128_records_roundtrip", |b| {
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
        g_batch.finish();

        // String & byte-slice throughput benchmark (64 strings + 4 KiB blob).
        let strings: Vec<&str> = (0..64).map(|_| "nve-codec-utf8-throughput-sample").collect();
        let blob = vec![0xABu8; 4096];
        let mut str_buf = ByteWriter::with_capacity(8192);
        str_buf.write_len(strings.len()).expect("len");
        for s in &strings {
            str_buf.write_str(s).expect("str");
        }
        str_buf.write_bytes(&blob).expect("blob");
        let str_bytes = str_buf.len() as u64;

        let mut g_str = c.benchmark_group("codec_slices_strings");
        g_str.throughput(Throughput::Bytes(str_bytes));
        g_str.bench_function("64_strings_and_4k_blob_roundtrip", |b| {
            b.iter(|| {
                str_buf.clear();
                str_buf.write_len(strings.len()).expect("len");
                for s in black_box(&strings) {
                    str_buf.write_str(s).expect("str");
                }
                str_buf.write_bytes(black_box(&blob)).expect("blob");
                let mut r = ByteReader::new(black_box(str_buf.as_slice()));
                let n = r.read_len(1024, 4).expect("len");
                let mut total = 0usize;
                for _ in 0..n {
                    total += r.read_str(256).expect("str").len();
                }
                total += r.read_bytes(8192).expect("blob").len();
                r.finish().expect("finish");
                black_box(total)
            });
        });
        g_str.finish();
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
