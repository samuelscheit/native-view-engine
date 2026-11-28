//! Target-language code emitters organized by output language, plus unified multi-target
//! dispatchers for Hybrid FFI and RN import/export. Host ABI data lives in [`crate::abi`].

pub mod c_cpp;
pub mod dart;
pub mod kotlin_java;
pub mod rust;
pub mod swift;
pub mod typescript;

use std::path::{Path, PathBuf};

use crate::error::CodegenError;
use crate::naming::to_snake_case;
use crate::schema::export::{
    write_generated_files, ExportRnOutput, GeneratedFile, ImportRnOutput, NveExportSpec,
    ParsedRnSpec, RnComponentSpec, TurboModuleSpec,
};
use crate::schema::hybrid::{HybridFileSpec, HybridTarget};

pub use c_cpp::generate_hybrid_cpp;
pub use dart::{generate_export_flutter, generate_hybrid_dart};
pub use kotlin_java::{generate_export_compose, generate_hybrid_kotlin};
pub use rust::generate_hybrid_rust;
pub use swift::{generate_export_swiftui, generate_hybrid_swift};
pub use typescript::{generate_export_web_component, generate_hybrid_ts};

// ============================================================================
// Hybrid FFI Dispatchers
// ============================================================================

/// Generated files emitted for one [`HybridTarget`].
#[derive(Debug, Clone, PartialEq)]
pub struct HybridGenerateOutput {
    pub target: HybridTarget,
    pub files: Vec<GeneratedFile>,
}

impl HybridGenerateOutput {
    /// Writes all generated files for this target into `out_dir`.
    pub fn write_to_dir(&self, out_dir: &Path) -> Result<Vec<PathBuf>, CodegenError> {
        let refs: Vec<&GeneratedFile> = self.files.iter().collect();
        write_generated_files(out_dir, &refs)
    }
}

/// Generates binary Hybrid FFI bindings for a single requested [`HybridTarget`].
pub fn generate_hybrid_target(
    spec: &HybridFileSpec,
    target: HybridTarget,
) -> Result<HybridGenerateOutput, CodegenError> {
    spec.validate()?;
    let files = match target {
        HybridTarget::Rust => generate_hybrid_rust(spec)?,
        HybridTarget::Cpp => generate_hybrid_cpp(spec)?,
        HybridTarget::Swift => generate_hybrid_swift(spec)?,
        HybridTarget::Kotlin => generate_hybrid_kotlin(spec)?,
        HybridTarget::Dart => generate_hybrid_dart(spec)?,
        HybridTarget::TypeScript => generate_hybrid_ts(spec)?,
    };
    Ok(HybridGenerateOutput { target, files })
}

/// Generates binary Hybrid FFI bindings for all requested [`HybridTarget`]s (or
/// [`HybridTarget::ALL`]).
pub fn generate_hybrid_all(
    spec: &HybridFileSpec,
    targets: &[HybridTarget],
) -> Result<Vec<HybridGenerateOutput>, CodegenError> {
    spec.validate()?;
    let mut outputs = Vec::with_capacity(targets.len());
    for &target in targets {
        outputs.push(generate_hybrid_target(spec, target)?);
    }
    Ok(outputs)
}

// ============================================================================
// React Native Fabric Import / Export Dispatchers
// ============================================================================

/// Generates zero-JS, binary FFI Rust `nve-dioxus` bindings and a thin C++20 Fabric adapter
/// from a parsed RN spec.
pub fn generate_import_rn(spec: &ParsedRnSpec) -> Result<ImportRnOutput, CodegenError> {
    match spec {
        ParsedRnSpec::Component(comp) => generate_component_import(comp),
        ParsedRnSpec::TurboModule(tm) => generate_turbomodule_import(tm),
    }
}

fn generate_component_import(spec: &RnComponentSpec) -> Result<ImportRnOutput, CodegenError> {
    let mod_name = to_snake_case(&spec.component_name);
    let rust_source = rust::import_rn::emit_component_rust(spec, &mod_name);

    Ok(ImportRnOutput {
        spec: ParsedRnSpec::Component(spec.clone()),
        rust_module: GeneratedFile {
            relative_path: format!("{mod_name}.generated.rs"),
            content: rust_source,
        },
        cpp_turbomodule_host: None,
    })
}

fn generate_turbomodule_import(spec: &TurboModuleSpec) -> Result<ImportRnOutput, CodegenError> {
    let mod_name = to_snake_case(&spec.module_name);
    let rust_source = rust::import_rn::emit_turbomodule_rust(spec, &mod_name);
    let cpp_source = c_cpp::import_rn::emit_turbomodule_cpp(spec);

    Ok(ImportRnOutput {
        spec: ParsedRnSpec::TurboModule(spec.clone()),
        rust_module: GeneratedFile {
            relative_path: format!("{mod_name}_module.generated.rs"),
            content: rust_source,
        },
        cpp_turbomodule_host: Some(GeneratedFile {
            relative_path: format!("{mod_name}_turbomodule_host.generated.hpp"),
            content: cpp_source,
        }),
    })
}

/// Generates a complete drop-in binary React Native Fabric NativeComponent package for an NVE component.
pub fn generate_export_rn(spec: &NveExportSpec) -> Result<ExportRnOutput, CodegenError> {
    spec.validate()?;

    let name = &spec.name;
    let native_component_ts = GeneratedFile {
        relative_path: format!("{name}NativeComponent.generated.ts"),
        content: typescript::export_rn_ts::emit_native_component_ts(spec),
    };
    let ios_component_view_mm = GeneratedFile {
        relative_path: format!("{name}ComponentView.generated.mm"),
        content: c_cpp::export_rn_ios::emit_ios_component_view_mm(spec),
    };
    let android_view_manager_kt = GeneratedFile {
        relative_path: format!("{name}ViewManager.generated.kt"),
        content: kotlin_java::export_rn_android::emit_android_view_manager_kt(spec),
    };
    let windows_component_view_hpp = GeneratedFile {
        relative_path: format!("{name}RnwComponentView.generated.hpp"),
        content: c_cpp::export_rn_windows::emit_windows_rnw_component_view_hpp(spec),
    };

    Ok(ExportRnOutput {
        spec: spec.clone(),
        native_component_ts,
        ios_component_view_mm,
        android_view_manager_kt,
        windows_component_view_hpp,
    })
}
