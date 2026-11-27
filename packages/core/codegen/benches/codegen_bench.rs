//! Criterion benchmarks for `nve-codegen`: the full checked-in ABI artifact set (latency plus
//! total emitted bytes, reported once) and a realistic hybrid FFI spec across all 6 targets.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_codegen::{
        core_abi_spec, generate_all_abi_files, generate_c_core_abi_h, generate_c_scene_abi_h,
        generate_hybrid_all, HybridTarget,
    };

    fn emitted_bytes(files: &[nve_codegen::GeneratedFile]) -> usize {
        files.iter().map(|f| f.content.len()).sum()
    }

    fn bench_codegen_emit(c: &mut Criterion) {
        let abi = generate_all_abi_files();
        println!("codegen_all_abi_files: {} files, {} bytes", abi.len(), emitted_bytes(&abi));
        let core = core_abi_spec();
        let hybrid = generate_hybrid_all(&core, HybridTarget::ALL).map_or(0, |outs| {
            outs.iter().map(|o| emitted_bytes(&o.files)).sum::<usize>()
        });
        println!("codegen_hybrid_core_abi_all_targets: {hybrid} bytes");

        c.bench_function("codegen_all_abi_files", |b| {
            b.iter(|| black_box(emitted_bytes(&generate_all_abi_files())));
        });
        c.bench_function("codegen_c_core_abi_header", |b| {
            b.iter(|| black_box(generate_c_core_abi_h().len()));
        });
        c.bench_function("codegen_c_scene_abi_header", |b| {
            b.iter(|| black_box(generate_c_scene_abi_h().len()));
        });
        c.bench_function("codegen_hybrid_core_abi_all_targets", |b| {
            b.iter(|| {
                let outputs = generate_hybrid_all(black_box(&core), HybridTarget::ALL);
                black_box(outputs.map_or(0, |outs| outs.len()))
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
