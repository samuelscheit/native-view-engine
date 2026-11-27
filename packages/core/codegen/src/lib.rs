//! Bi-directional & multi-target Native View Engine (NVE) code generator.
//!
//! Organized as a compiler pipeline:
//! - [`abi`]: Askama-rendered host ABI data (Core + Scene ABI, word-table field specs, golden
//!   XXH64 vectors) for the static `packages/core/host-*` runtimes.
//! - [`schema`]: Intermediate representations and validation (`export`, `hybrid`).
//! - [`frontend`]: Input specification parsers (`rn_spec`, and `hybrid_spec` on `syn` 2).
//! - [`targets`]: Output-language code emitters (`c_cpp`, `rust`, `swift`, `kotlin_java`, `dart`,
//!   `typescript`) plus unified multi-target dispatchers.
//!
//! Dependencies: `nve-abi` / `nve-hash` (ABI descriptors, XXH64), `serde` derives on the IR,
//! `askama` (templates), and `syn` / `proc-macro2` (the hybrid spec frontend). Reading specs
//! from JSON files is the CLI's job (`nve-cli`), so this crate never links `serde_json`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod abi;
pub mod error;
pub mod frontend;
pub mod naming;
pub mod schema;
pub mod targets;
pub(crate) mod writer;

use std::path::Path;

pub use abi::{
    generate_abi_files, generate_all_abi_files, generate_c_abi_header, generate_hash_vector_files,
    generate_table_files,
};
pub use error::CodegenError;
pub use frontend::{
    parse_hybrid_rust_spec, parse_hybrid_rust_spec_file, parse_rn_spec, parse_rn_spec_file,
};
pub use schema::export::{
    CommandSpec, ComponentOptions, DefaultValue, EventBubbling, EventSpec, ExportBackend,
    ExportComposeOutput, ExportFlutterOutput, ExportRnOutput, ExportSwiftUiOutput, ExportTarget,
    ExportWebComponentOutput, GeneratedFile, ImportRnOutput, MultiTargetExportOutput,
    NveExportSpec, ObjectField, ParamSpec, ParsedRnSpec, PropSpec, PropType, RnComponentSpec,
    TurboModuleMethod, TurboModuleSpec,
};
pub use schema::hybrid::{
    compute_hybrid_interface_id, HybridAliasSpec, HybridConstSpec, HybridDefaultValue,
    HybridEnumSpec, HybridEnumVariant, HybridEventSpec, HybridFieldSpec, HybridFileSpec,
    HybridMethodSpec, HybridModuleSpec, HybridParamSpec, HybridPropSpec, HybridStructSpec,
    HybridTarget, HybridType, HybridViewSpec, NVE_HYBRID_ABI_VERSION, NVE_HYBRID_INTERFACE_ID_BASE,
};
pub use targets::{
    generate_export_compose, generate_export_flutter, generate_export_rn,
    generate_export_swiftui, generate_export_web_component, generate_hybrid_all,
    generate_hybrid_cpp, generate_hybrid_dart, generate_hybrid_kotlin, generate_hybrid_rust,
    generate_hybrid_swift, generate_hybrid_target, generate_hybrid_ts, generate_import_rn,
    HybridGenerateOutput,
};

// ===========================================================================
// Direction 1: `import_rn` (RN -> NVE)
// ===========================================================================

/// Parse RN spec source text and emit zero-JS Rust + C++20 Fabric adapter files.
pub fn import_rn_source(source: &str) -> Result<ImportRnOutput, CodegenError> {
    let parsed = parse_rn_spec(source)?;
    generate_import_rn(&parsed)
}

/// CLI entrypoint (`nve codegen import-rn`): parse a `.ts`/`.js` spec file on disk
/// (following RN re-export wrappers automatically) and emit Rust + C++20 files.
pub fn import_rn_file(spec_path: &Path) -> Result<ImportRnOutput, CodegenError> {
    let parsed = parse_rn_spec_file(spec_path)?;
    generate_import_rn(&parsed)
}

// ===========================================================================
// Direction 2: Target-Specific Exporters
// ===========================================================================

/// Export an `NveExportSpec` to a drop-in React Native Fabric NativeComponent package (6 files).
pub fn export_rn_spec(spec: &NveExportSpec) -> Result<ExportRnOutput, CodegenError> {
    generate_export_rn(spec)
}

/// Export an `NveExportSpec` to a typed Flutter `StatefulWidget` using `dart:ffi` + `nve_flutter`.
pub fn export_flutter(spec: &NveExportSpec) -> Result<ExportFlutterOutput, CodegenError> {
    generate_export_flutter(spec)
}

/// Export an `NveExportSpec` to a typed SwiftUI `View` wrapping `NveSwiftUI.NveSurfaceRepresentable`.
pub fn export_swiftui(spec: &NveExportSpec) -> Result<ExportSwiftUiOutput, CodegenError> {
    generate_export_swiftui(spec)
}

/// Export an `NveExportSpec` to a typed Jetpack Compose `@Composable` wrapping `dev.nve.compose.NveSurface`.
pub fn export_compose(spec: &NveExportSpec) -> Result<ExportComposeOutput, CodegenError> {
    generate_export_compose(spec)
}

/// Export an `NveExportSpec` to a typed Custom Element `<nve-...>` + `.d.ts` wrapping `wasm-bindgen`.
pub fn export_web_component(
    spec: &NveExportSpec,
) -> Result<ExportWebComponentOutput, CodegenError> {
    generate_export_web_component(spec)
}

// ===========================================================================
// Unified Multi-Target Export API (`nve codegen export --target ...`)
// ===========================================================================

/// Exports a single `NveExportSpec` to one requested [`ExportTarget`].
pub fn export_target(
    spec: &NveExportSpec,
    target: ExportTarget,
) -> Result<MultiTargetExportOutput, CodegenError> {
    let files = match target {
        ExportTarget::ReactNative => generate_export_rn(spec)?
            .files()
            .into_iter()
            .cloned()
            .collect(),
        ExportTarget::Flutter => generate_export_flutter(spec)?
            .files()
            .into_iter()
            .cloned()
            .collect(),
        ExportTarget::SwiftUi => generate_export_swiftui(spec)?
            .files()
            .into_iter()
            .cloned()
            .collect(),
        ExportTarget::Compose => generate_export_compose(spec)?
            .files()
            .into_iter()
            .cloned()
            .collect(),
        ExportTarget::WebComponent => generate_export_web_component(spec)?
            .files()
            .into_iter()
            .cloned()
            .collect(),
    };
    Ok(MultiTargetExportOutput {
        spec: spec.clone(),
        target,
        files,
    })
}

/// Exports a single `NveExportSpec` across multiple [`ExportTarget`]s (or [`ExportTarget::ALL`]).
pub fn export_all_targets(
    spec: &NveExportSpec,
    targets: &[ExportTarget],
) -> Result<Vec<MultiTargetExportOutput>, CodegenError> {
    let mut outputs = Vec::with_capacity(targets.len());
    for &target in targets {
        outputs.push(export_target(spec, target)?);
    }
    Ok(outputs)
}
