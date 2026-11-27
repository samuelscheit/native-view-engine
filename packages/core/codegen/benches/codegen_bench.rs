//! Criterion benchmarks for `nve-codegen`: every generated host file (ABI, the 17 core tables,
//! XXH64 vectors) across 5 languages, the merged C ABI header alone, the core tables alone, the
//! `syn` hybrid spec frontend, and a realistic hybrid spec across all 6 targets. Emitted byte
//! counts are printed once; every benchmark observes its output bytes.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{Criterion, criterion_group};
    use nve_codegen::{
        GeneratedFile, HybridTarget, generate_all_abi_files, generate_c_abi_header,
        generate_hybrid_all, generate_table_files, parse_hybrid_rust_spec,
    };
    use nve_components::tables::{CORE_TABLE_DESCS, CORE_WORD_CONSTS};

    /// Modules, views (props, events, methods), structs, enums, and C functions.
    const REALISTIC_HYBRID_SPEC: &str = include_str!("../tests/fixtures/realistic_hybrid.nve.rs");

    fn bytes(files: &[GeneratedFile]) -> usize {
        files.iter().map(|f| f.content.len()).sum()
    }

    fn bench_codegen_emit(c: &mut Criterion) {
        let all = generate_all_abi_files(CORE_TABLE_DESCS, CORE_WORD_CONSTS).expect("generate_all_abi_files");
        println!(
            "codegen_all_abi_files_5_langs: {} files, {} bytes",
            all.len(),
            bytes(&all)
        );
        let header = generate_c_abi_header().expect("generate_c_abi_header");
        println!("codegen_c_abi_header: {} bytes", header.len());
        let tables = generate_table_files(CORE_TABLE_DESCS, CORE_WORD_CONSTS).expect("generate_table_files");
        println!(
            "codegen_tables_17_core: {} tables, {} bytes",
            CORE_TABLE_DESCS.len(),
            bytes(&tables)
        );
        let spec =
            parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("parse the realistic hybrid spec");
        let hybrid = generate_hybrid_all(&spec, HybridTarget::ALL).expect("generate_hybrid_all");
        let hybrid_bytes: usize = hybrid.iter().map(|o| bytes(&o.files)).sum();
        assert!(hybrid_bytes > 0, "hybrid codegen emitted nothing");
        println!(
            "parse_hybrid_rust_spec: {} source bytes; codegen_hybrid_all_6_targets: {hybrid_bytes} bytes",
            REALISTIC_HYBRID_SPEC.len()
        );

        c.bench_function("codegen_all_abi_files_5_langs", |b| {
            b.iter(|| {
                bytes(&generate_all_abi_files(black_box(CORE_TABLE_DESCS), CORE_WORD_CONSTS).expect("abi files"))
            });
        });
        c.bench_function("codegen_c_abi_header", |b| {
            b.iter(|| generate_c_abi_header().expect("C header").len());
        });
        c.bench_function("codegen_tables_17_core", |b| {
            b.iter(|| bytes(&generate_table_files(black_box(CORE_TABLE_DESCS), CORE_WORD_CONSTS).expect("tables")));
        });
        c.bench_function("parse_hybrid_rust_spec", |b| {
            b.iter(|| {
                parse_hybrid_rust_spec(black_box(REALISTIC_HYBRID_SPEC))
                    .expect("parse")
                    .structs
                    .len()
            });
        });
        c.bench_function("codegen_hybrid_all_6_targets", |b| {
            b.iter(|| {
                let outputs =
                    generate_hybrid_all(black_box(&spec), HybridTarget::ALL).expect("hybrid");
                outputs.iter().map(|o| bytes(&o.files)).sum::<usize>()
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
