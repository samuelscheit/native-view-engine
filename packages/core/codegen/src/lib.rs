//! Bi-directional & multi-target Native View Engine (NVE) code generator.
//!
//! Organized as a compiler pipeline:
//! - [`schema`]: Intermediate representations, layout computation, validation, and colocated
//!   `#[derive(NveAbi)]` descriptor bridges (`export`, `hybrid`, `reloc`, `wire`).
//! - [`frontend`]: Input specification lexers and parsers (`rn_spec`, `hybrid_rust`).
//! - [`targets`]: Output-language code emitters (`c_cpp`, `rust`, `swift`, `kotlin_java`, `dart`,
//!   `typescript`) plus unified multi-target dispatchers.
//!
//! Zero external dependencies beyond `std`, `serde`, and `serde_json`.

pub mod error;
pub mod frontend;
pub mod naming;
pub mod schema;
pub mod targets;
pub(crate) mod writer;

use std::path::Path;

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
    compute_hybrid_interface_id, compute_scene_schema_hash, core_abi_schema_hash, core_abi_spec,
    from_abi_descriptor, scene_abi_spec, HybridAliasSpec, HybridConstSpec, HybridDefaultValue,
    HybridEnumSpec, HybridEnumVariant, HybridEventSpec, HybridFieldSpec, HybridFileSpec,
    HybridMethodSpec, HybridModuleSpec, HybridParamSpec, HybridPropSpec, HybridStructSpec,
    HybridTarget, HybridType, HybridViewSpec, NVE_CORE_ABI_SCHEMA_HASH, NVE_HYBRID_ABI_VERSION,
    NVE_HYBRID_INTERFACE_ID_BASE, NVE_RENDER_NODE_ALIGN, NVE_RENDER_NODE_SIZE,
    NVE_SCENE_SCHEMA_HASH,
};
pub use schema::reloc::{
    FieldLayout, RelocEnum, RelocEnumRepr, RelocEnumVariant, RelocField, RelocSchema, RelocStruct,
    RelocType, StructLayout, RELOC_HEADER_ALIGN, RELOC_HEADER_SIZE, RELOC_MAX_VERIFY_DEPTH,
    RELOC_SLICE_ALIGN, RELOC_SLICE_SIZE,
};
pub use schema::wire::{wire_codecs_schema_hash, NVE_WIRE_CODECS_SCHEMA_HASH};
pub use targets::{
    generate_all_abi_files, generate_all_core_abi_files, generate_all_scene_abi_files,
    generate_all_wire_codec_files, generate_c_core_abi_h, generate_c_scene_abi_h,
    generate_cpp_wire_codecs, generate_dart_core_abi, generate_dart_scene_abi,
    generate_dart_wire_codecs, generate_export_compose, generate_export_flutter,
    generate_export_rn, generate_export_swiftui, generate_export_web_component,
    generate_hybrid_all, generate_hybrid_cpp, generate_hybrid_dart, generate_hybrid_kotlin,
    generate_hybrid_rust, generate_hybrid_swift, generate_hybrid_target, generate_hybrid_ts,
    generate_import_rn, generate_java_core_abi, generate_java_scene_buffer,
    generate_java_wire_codecs, generate_reloc_all, generate_reloc_cpp, generate_reloc_rust,
    generate_swift_wire_codecs, generate_ts_core_abi, generate_ts_scene_view,
    generate_ts_wire_codecs, HybridGenerateOutput,
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

/// CLI entrypoint (`nve codegen export`): reads a JSON `NveExportSpec` file and generates
/// bindings for every requested [`ExportTarget`].
pub fn export_json_file_for_targets(
    spec_json_path: &Path,
    targets: &[ExportTarget],
) -> Result<Vec<MultiTargetExportOutput>, CodegenError> {
    let spec = NveExportSpec::from_json_file(spec_json_path)?;
    export_all_targets(&spec, targets)
}
