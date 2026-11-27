//! Criterion benchmarks for `nve-codegen`: every generated host file (ABI, the 17 core tables,
//! XXH64 vectors) across 5 languages, the merged C ABI header alone, the core tables alone, and a
//! realistic hybrid FFI spec across all 6 targets. Emitted byte counts are printed once.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{Criterion, criterion_group};
    use nve_abi::schema::core_abi_descriptor;
    use nve_codegen::{
        GeneratedFile, HybridTarget, from_abi_descriptor, generate_all_abi_files,
        generate_c_abi_header, generate_hybrid_all, generate_table_files,
    };
    use nve_components::tables::CORE_TABLE_DESCS;

    fn emitted_bytes(files: &[GeneratedFile]) -> usize {
        files.iter().map(|f| f.content.len()).sum()
    }

    fn bench_codegen_emit(c: &mut Criterion) {
        let all = generate_all_abi_files(CORE_TABLE_DESCS).unwrap_or_default();
        println!(
            "codegen_all_abi_files_5_langs: {} files, {} bytes",
            all.len(),
            emitted_bytes(&all)
        );
        let header = generate_c_abi_header().map_or(0, |h| h.len());
        println!("codegen_c_abi_header: {header} bytes");
        let tables = generate_table_files(CORE_TABLE_DESCS).unwrap_or_default();
        println!(
            "codegen_tables_17_core: {} tables, {} files, {} bytes",
            CORE_TABLE_DESCS.len(),
            tables.len(),
            emitted_bytes(&tables)
        );
        let core = from_abi_descriptor(&core_abi_descriptor());
        let hybrid = generate_hybrid_all(&core, HybridTarget::ALL).map_or(0, |outs| {
            outs.iter().map(|o| emitted_bytes(&o.files)).sum::<usize>()
        });
        println!("codegen_hybrid_core_abi_all_targets: {hybrid} bytes");

        c.bench_function("codegen_all_abi_files_5_langs", |b| {
            b.iter(|| {
                black_box(
                    generate_all_abi_files(black_box(CORE_TABLE_DESCS)).map_or(0, |f| f.len()),
                )
            });
        });
        c.bench_function("codegen_c_abi_header", |b| {
            b.iter(|| black_box(generate_c_abi_header().map_or(0, |h| h.len())));
        });
        c.bench_function("codegen_tables_17_core", |b| {
            b.iter(|| {
                black_box(generate_table_files(black_box(CORE_TABLE_DESCS)).map_or(0, |f| f.len()))
            });
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
