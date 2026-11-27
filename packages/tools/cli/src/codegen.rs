//! `nve codegen` — host ABI data, hybrid FFI specs, React Native import, and multi-target export.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use nve_codegen::{ExportTarget, HybridTarget, NveExportSpec};

#[derive(Subcommand)]
pub(crate) enum CodegenAction {
    /// Regenerate (or check for drift in) the host ABI, word-table, and XXH64 vector files of
    /// `packages/core/host-*`.
    Abi {
        /// Verify checked-in artifacts match generated output byte-for-byte without writing files.
        #[arg(long, default_value_t = false)]
        check: bool,
        /// Workspace root directory where relative artifact paths are resolved.
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// Parse a declarative Rust `#[nve_hybrid_*]` spec file and emit binary FFI bindings.
    Hybrid {
        /// Path to the declarative Rust hybrid spec file (`.rs` or `.nve.rs`).
        spec: PathBuf,
        /// Target language(s): `rust`, `cpp`, `swift`, `kotlin`, `dart`, `ts`, or `all` (comma-separated allowed).
        #[arg(long, default_value = "all")]
        target: String,
        /// Output root directory (each target writes into `<out>/<target>/`).
        #[arg(long, default_value = "generated/hybrid")]
        out: PathBuf,
    },
    /// Parse a React Native `*NativeComponent.{ts,js}` or `Native*.{ts,js}` spec and emit zero-JS Rust + C++20 bindings.
    ImportRn {
        /// Path to the React Native TypeScript/Flow spec file.
        spec: PathBuf,
        /// Output directory for generated Rust and C++20 files.
        #[arg(long, default_value = "generated/import-rn")]
        out: PathBuf,
    },
    /// Export an `NveExportSpec` JSON file into native wrappers for RN, Flutter, SwiftUI, Compose, and/or Web.
    Export {
        /// Path to the `NveExportSpec` JSON file.
        spec: PathBuf,
        /// Target framework(s): `rn`, `flutter`, `swiftui`, `compose`, `web`, or `all` (comma-separated allowed).
        #[arg(long, default_value = "all")]
        target: String,
        /// Output root directory (each target writes into `<out>/<target>/`).
        #[arg(long, default_value = "generated/export")]
        out: PathBuf,
    },
}

pub(crate) fn parse_export_targets(target: &str) -> Result<Vec<ExportTarget>, String> {
    if target.trim().eq_ignore_ascii_case("all") {
        return Ok(ExportTarget::ALL.to_vec());
    }
    let mut out = Vec::new();
    for part in target.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return Err("empty target name in comma-separated list".into());
        }
        let parsed = ExportTarget::from_name(trimmed)
            .map_err(|e| format!("invalid target '{trimmed}': {e}"))?;
        out.push(parsed);
    }
    Ok(out)
}

pub(crate) fn parse_hybrid_targets(target: &str) -> Result<Vec<HybridTarget>, String> {
    if target.trim().eq_ignore_ascii_case("all") {
        return Ok(HybridTarget::ALL.to_vec());
    }
    let mut out = Vec::new();
    for part in target.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return Err("empty hybrid target name in comma-separated list".into());
        }
        let parsed = HybridTarget::from_name(trimmed).ok_or_else(|| {
            format!(
                "invalid hybrid target '{trimmed}' (expected rust, cpp, swift, kotlin, dart, ts, or all)"
            )
        })?;
        out.push(parsed);
    }
    Ok(out)
}

fn write_or_check_generated(
    files: &[nve_codegen::GeneratedFile],
    out_root: &Path,
    check: bool,
    regen_hint: &str,
) -> Result<Vec<PathBuf>, String> {
    let mut drifted = Vec::new();
    let mut touched = Vec::with_capacity(files.len());

    for file in files {
        let full_path = out_root.join(&file.relative_path);
        if check {
            match std::fs::read_to_string(&full_path) {
                Ok(existing) if existing.replace("\r\n", "\n") == file.content => {
                    touched.push(full_path)
                }
                Ok(_) => drifted.push(format!("{} (content drifted)", file.relative_path)),
                Err(e) => drifted.push(format!("{} ({e})", file.relative_path)),
            }
        } else {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
            }
            std::fs::write(&full_path, &file.content)
                .map_err(|e| format!("write {}: {e}", full_path.display()))?;
            touched.push(full_path);
        }
    }

    if !drifted.is_empty() {
        return Err(format!(
            "ABI drift detected in {} file(s):\n  - {}\nRun `{regen_hint}` to regenerate.",
            drifted.len(),
            drifted.join("\n  - ")
        ));
    }
    Ok(touched)
}

/// Published `nve.canvas` tables (one mirror per host language; core tables come from
/// `nve-components`).
const CANVAS_TABLES: &[&nve_store::TableDesc] = &[&<nve_canvas::Canvas as nve_store::NveTable>::DESC];

/// Capability-module table mirrors for C/C++ hosts. They are generated here (tools may link
/// modules) because core `nve-codegen` stays module-free.
fn module_table_headers() -> Result<Vec<nve_codegen::GeneratedFile>, nve_codegen::CodegenError> {
    Ok(vec![nve_codegen::generate_module_table_header(
        "canvas",
        CANVAS_TABLES,
        "packages/modules/canvas/cxx/include/nve/canvas/nve_canvas_tables.generated.h".to_owned(),
    )?])
}

pub(crate) fn run_codegen_abi(out_root: &Path, check: bool) -> Result<Vec<PathBuf>, String> {
    use nve_components::tables::{CORE_TABLE_DESCS, CORE_WORD_CONSTS};
    let mut files = nve_codegen::generate_all_abi_files(CORE_TABLE_DESCS, CORE_WORD_CONSTS)
        .map_err(|e| format!("ABI codegen failed: {e}"))?;
    files.extend(module_table_headers().map_err(|e| format!("module table codegen failed: {e}"))?);
    files.extend(nve_codegen::generate_java_host_files(CANVAS_TABLES));
    write_or_check_generated(&files, out_root, check, "cargo nve codegen abi")
}

/// Reads and validates an `NveExportSpec` JSON file. JSON parsing lives here, in the CLI, so the
/// `nve-codegen` library itself never links `serde_json`.
pub(crate) fn load_export_spec(path: &Path) -> Result<NveExportSpec, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let spec: NveExportSpec =
        serde_json::from_str(&json).map_err(|e| format!("invalid NveExportSpec JSON in {}: {e}", path.display()))?;
    spec.validate().map_err(|e| format!("invalid NveExportSpec in {}: {e}", path.display()))?;
    Ok(spec)
}

pub(crate) fn run_codegen_hybrid(
    spec_path: &Path,
    target: &str,
    out_root: &Path,
) -> Result<Vec<(HybridTarget, PathBuf, Vec<PathBuf>)>, String> {
    let targets = parse_hybrid_targets(target)?;
    let spec = nve_codegen::parse_hybrid_rust_spec_file(spec_path)
        .map_err(|e| format!("hybrid spec parse failed for {}: {e}", spec_path.display()))?;
    let outputs = nve_codegen::generate_hybrid_all(&spec, &targets)
        .map_err(|e| format!("hybrid codegen failed for {}: {e}", spec_path.display()))?;

    let mut summary = Vec::with_capacity(outputs.len());
    for bundle in outputs {
        let target_dir = out_root.join(bundle.target.dir_name());
        let written = bundle
            .write_to_dir(&target_dir)
            .map_err(|e| format!("failed writing {} output: {e}", bundle.target.dir_name()))?;
        summary.push((bundle.target, target_dir, written));
    }
    Ok(summary)
}

pub(crate) fn cmd_codegen(action: CodegenAction) {
    match action {
        CodegenAction::Abi { check, out } => match run_codegen_abi(&out, check) {
            Ok(paths) => {
                let verb = if check { "Verified" } else { "Generated" };
                println!(
                    "{verb} {} host ABI / table / XXH64 vector file(s) in {} (core_hash=0x{:016X}, scene_hash=0x{:016X}){}",
                    paths.len(),
                    out.display(),
                    nve_abi::NVE_CORE_ABI_SCHEMA_HASH,
                    nve_abi::NVE_SCENE_SCHEMA_HASH,
                    if check { "." } else { ":" }
                );
                if !check {
                    for path in paths {
                        println!("  - {}", path.display());
                    }
                }
            }
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        },
        CodegenAction::Hybrid { spec, target, out } => {
            let summary = run_codegen_hybrid(&spec, &target, &out).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            for (tgt, target_dir, written) in summary {
                println!(
                    "[{}] wrote {} file(s) -> {}",
                    tgt.dir_name(),
                    written.len(),
                    target_dir.display()
                );
                for path in written {
                    println!("  - {}", path.display());
                }
            }
        }
        CodegenAction::ImportRn { spec, out } => {
            let generated = nve_codegen::import_rn_file(&spec).unwrap_or_else(|e| {
                eprintln!("import-rn failed for {}: {e}", spec.display());
                std::process::exit(1);
            });
            let written = generated.write_to_dir(&out).unwrap_or_else(|e| {
                eprintln!("failed writing output to {}: {e}", out.display());
                std::process::exit(1);
            });
            println!("Generated {} files in {}:", written.len(), out.display());
            for path in written {
                println!("  - {}", path.display());
            }
        }
        CodegenAction::Export { spec, target, out } => {
            let targets = parse_export_targets(&target).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let export_spec = load_export_spec(&spec).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let bundles = nve_codegen::export_all_targets(&export_spec, &targets).unwrap_or_else(|e| {
                eprintln!("export failed for {}: {e}", spec.display());
                std::process::exit(1);
            });
            for bundle in bundles {
                let target_dir = out.join(bundle.target.dir_name());
                let written = bundle.write_to_dir(&target_dir).unwrap_or_else(|e| {
                    eprintln!("failed writing {} output: {e}", bundle.target.dir_name());
                    std::process::exit(1);
                });
                println!(
                    "[{}] wrote {} file(s) -> {}",
                    bundle.target.dir_name(),
                    written.len(),
                    target_dir.display()
                );
                for path in written {
                    println!("  - {}", path.display());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codegen_target_parsers_and_abi_hybrid_end_to_end() {
        assert_eq!(parse_export_targets("all").unwrap(), ExportTarget::ALL);
        assert_eq!(parse_export_targets(" ALL ").unwrap(), ExportTarget::ALL);
        assert_eq!(
            parse_export_targets("rn, flutter , web").unwrap(),
            vec![
                ExportTarget::ReactNative,
                ExportTarget::Flutter,
                ExportTarget::WebComponent,
            ]
        );
        assert!(parse_export_targets("unknown-framework").is_err());
        assert!(parse_export_targets("rn,").is_err());

        assert_eq!(parse_hybrid_targets("all").unwrap(), HybridTarget::ALL);
        assert_eq!(parse_hybrid_targets(" ALL ").unwrap(), HybridTarget::ALL);
        assert_eq!(
            parse_hybrid_targets("rust,cpp,swift,kotlin,dart,ts").unwrap(),
            HybridTarget::ALL
        );
        assert!(parse_hybrid_targets("unknown-target").is_err());
        assert!(parse_hybrid_targets("rust,").is_err());

        let tmp = std::env::temp_dir().join(format!(
            "nve_cli_codegen_e2e_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        assert!(run_codegen_abi(&tmp, true).is_err());
        let written = run_codegen_abi(&tmp, false).expect("write every host ABI file");
        assert_eq!(written.len(), 21, "core + canvas module mirrors + Java host files");
        assert!(run_codegen_abi(&tmp, true).is_ok());

        let target_file = tmp.join("packages/core/host-ts/src/generated/nve_abi.generated.ts");
        std::fs::write(&target_file, "// drifted\n").unwrap();
        let drift_err = run_codegen_abi(&tmp, true).unwrap_err();
        assert!(drift_err.contains("packages/core/host-ts/src/generated/nve_abi.generated.ts"));

        // `codegen export` JSON loading: valid specs load, broken JSON and invalid specs are refused.
        let export_json = tmp.join("export.json");
        let spec = NveExportSpec::new("NveCounterCard", nve_codegen::ExportBackend::Fabric);
        std::fs::write(&export_json, serde_json::to_string_pretty(&spec).unwrap()).unwrap();
        assert_eq!(load_export_spec(&export_json).unwrap().name, "NveCounterCard");
        std::fs::write(&export_json, "{invalid json").unwrap();
        assert!(load_export_spec(&export_json).unwrap_err().contains("invalid NveExportSpec JSON"));
        let invalid = NveExportSpec::new("123BadName", nve_codegen::ExportBackend::Fabric);
        std::fs::write(&export_json, serde_json::to_string_pretty(&invalid).unwrap()).unwrap();
        assert!(load_export_spec(&export_json).unwrap_err().contains("invalid NveExportSpec in"));
        assert!(load_export_spec(&tmp.join("missing.json")).unwrap_err().contains("read "));

        let spec_file = tmp.join("greeter.nve.rs");
        std::fs::write(
            &spec_file,
            r#"
#[nve_hybrid_module(name = "Greeter")]
pub trait GreeterSpec {
    fn hello(&self, name: &str) -> String;
}
"#,
        )
        .unwrap();
        let hybrid_out = tmp.join("hybrid_out");
        let summary = run_codegen_hybrid(&spec_file, "all", &hybrid_out).expect("hybrid gen");
        assert_eq!(summary.len(), 6);
        assert!(hybrid_out.join("rust/greeter_hybrid.generated.rs").exists());
        assert!(hybrid_out.join("cpp/GreeterHybridAbi.generated.h").exists());
        assert!(hybrid_out.join("cpp/GreeterHybridSpec.generated.hpp").exists());
        assert!(hybrid_out.join("swift/GreeterHybridSpec.generated.swift").exists());
        assert!(hybrid_out.join("kotlin/GreeterHybridSpec.generated.kt").exists());
        assert!(hybrid_out.join("kotlin/GreeterHybridJni.generated.cpp").exists());
        assert!(hybrid_out.join("dart/greeter_hybrid.generated.dart").exists());
        assert!(hybrid_out.join("ts/greeter_hybrid.generated.ts").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
