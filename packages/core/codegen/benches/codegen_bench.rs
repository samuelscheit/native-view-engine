//! Criterion benchmarks for `nve-codegen` ABI header & multi-target wire codec emission.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_codegen::{
        generate_all_wire_codec_files, generate_c_core_abi_h, generate_c_scene_abi_h,
    };

    fn bench_codegen_emit(c: &mut Criterion) {
        c.bench_function("codegen_c_core_abi_header", |b| {
            b.iter(|| black_box(generate_c_core_abi_h().len()));
        });
        c.bench_function("codegen_c_scene_abi_header", |b| {
            b.iter(|| black_box(generate_c_scene_abi_h().len()));
        });
        c.bench_function("codegen_all_wire_codec_files", |b| {
            b.iter(|| {
                let files = generate_all_wire_codec_files();
                black_box(files.iter().map(|f| f.content.len()).sum::<usize>())
            });
        });
    }

    criterion_group!(benches, bench_codegen_emit);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
