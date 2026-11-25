//! Bi-directional & multi-target Native View Engine (NVE) code generator.
//!
//! Zero external dependencies beyond `std`, `serde`, and `serde_json`.
//!
//! # Direction 1 — `import_rn` (React Native -> NVE)
//! Parses React Native Fabric `*NativeComponent.{ts,js}` and TurboModule `Native*.{ts,js}`
//! specifications (both TypeScript and Flow syntax) and emits:
//! - A zero-JS typed Rust `nve-dioxus` component module writing `nve_abi::RawPropValue` directly
//! - A thin C++20 Fabric prop/event adapter compatible with `cxx/nve-fabric-bridge`
//!
//! # Direction 2 — Multi-target Export (Single `NveExportSpec` -> Every Host Platform)
//! Takes a single [`NveExportSpec`] (Rust struct or JSON) and exports drop-in native bindings for:
//! - **React Native Fabric** (`export_rn`): `<Name>NativeComponent.ts`, `<Name>NveBridge.hpp/.cpp`,
//!   `<Name>ComponentView.mm` (`RCTNveComponentView`), `<Name>ViewManager.kt` (`NveNativeBridge`)
//! - **Flutter** (`export_flutter`): typed Dart `StatefulWidget` + `dart:ffi` binding wrapping
//!   `package:nve_flutter/nve_flutter.dart` (`NveTextureHostWidget` / `NvePlatformViewWidget`)
//! - **SwiftUI** (`export_swiftui`): typed SwiftUI `View` wrapping `NveSwiftUI.NveSurfaceRepresentable`
//! - **Jetpack Compose** (`export_compose`): typed Kotlin `@Composable` wrapping `dev.nve.compose.NveSurface`
//! - **Web Components** (`export_web_component`): typed Custom Element `<nve-...>` `.js` + `.d.ts`
//!   wrapping `wasm-bindgen`

pub mod error;
pub mod export_compose;
pub mod export_flutter;
pub mod export_rn;
pub mod export_swiftui;
pub mod export_web;
pub mod hybrid;
pub mod import_rn;
pub mod ir;
pub mod lexer;
pub mod naming;
pub mod parser;
pub mod reloc;
pub mod wire_codecs;

use std::path::Path;

pub use error::CodegenError;
pub use export_compose::generate_export_compose;
pub use export_flutter::generate_export_flutter;
pub use export_rn::generate_export_rn;
pub use export_swiftui::generate_export_swiftui;
pub use export_web::generate_export_web_component;
pub use hybrid::{
    compute_hybrid_interface_id, compute_scene_schema_hash, core_abi_schema_hash, core_abi_spec,
    generate_all_abi_files, generate_all_core_abi_files, generate_all_scene_abi_files,
    generate_c_core_abi_h, generate_c_scene_abi_h, generate_dart_core_abi, generate_dart_scene_abi,
    generate_hybrid_all, generate_hybrid_cpp, generate_hybrid_dart, generate_hybrid_kotlin,
    generate_hybrid_rust, generate_hybrid_swift, generate_hybrid_target, generate_hybrid_ts,
    generate_java_core_abi, generate_java_scene_buffer, generate_ts_core_abi,
    generate_ts_scene_view, parse_hybrid_rust_spec, parse_hybrid_rust_spec_file, scene_abi_spec,
    HybridAliasSpec, HybridConstSpec, HybridDefaultValue, HybridEnumSpec, HybridEnumVariant,
    HybridEventSpec, HybridFieldSpec, HybridFileSpec, HybridGenerateOutput, HybridMethodSpec,
    HybridModuleSpec, HybridParamSpec, HybridPropSpec, HybridStructSpec, HybridTarget, HybridType,
    HybridViewSpec, NVE_CORE_ABI_SCHEMA_HASH, NVE_CORE_ABI_SPEC_SOURCE, NVE_HYBRID_ABI_VERSION,
    NVE_HYBRID_INTERFACE_ID_BASE, NVE_RENDER_NODE_ALIGN, NVE_RENDER_NODE_SIZE,
    NVE_SCENE_ABI_SPEC_SOURCE, NVE_SCENE_SCHEMA_HASH,
};
pub use import_rn::generate_import_rn;
pub use ir::{
    CommandSpec, ComponentOptions, DefaultValue, EventBubbling, EventSpec, ExportBackend,
    ExportComposeOutput, ExportFlutterOutput, ExportRnOutput, ExportSwiftUiOutput, ExportTarget,
    ExportWebComponentOutput, GeneratedFile, ImportRnOutput, MultiTargetExportOutput,
    NveExportSpec, ObjectField, ParamSpec, ParsedRnSpec, PropSpec, PropType, RnComponentSpec,
    TurboModuleMethod, TurboModuleSpec,
};
pub use parser::{parse_rn_spec, parse_rn_spec_file};
pub use reloc::{
    generate_reloc_all, generate_reloc_cpp, generate_reloc_rust, FieldLayout, RelocEnum,
    RelocEnumRepr, RelocEnumVariant, RelocField, RelocSchema, RelocStruct, RelocType,
    StructLayout, RELOC_HEADER_ALIGN, RELOC_HEADER_SIZE, RELOC_MAX_VERIFY_DEPTH,
    RELOC_SLICE_ALIGN, RELOC_SLICE_SIZE,
};
pub use wire_codecs::{
    fnv1a64, fnv1a64_pair, fnv1a64_update, generate_all_wire_codec_files,
    generate_cpp_wire_codecs, generate_dart_wire_codecs, generate_java_wire_codecs,
    generate_swift_wire_codecs, generate_ts_wire_codecs, wire_codecs_schema_hash,
    NVE_WIRE_CODECS_SCHEMA_HASH,
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
