//! Target-language code emitters organized by output language, plus unified multi-target
//! dispatchers for Core/Scene ABI, Wire Codecs, Hybrid FFI, Reloc slabs, and RN import/export.

pub mod c_cpp;
pub mod dart;
pub mod hash;
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
use crate::schema::reloc::RelocSchema;

pub use c_cpp::{
    generate_c_core_abi_h, generate_c_scene_abi_h, generate_cpp_wire_codecs, generate_hybrid_cpp,
    generate_reloc_cpp,
};
pub use dart::{
    generate_dart_core_abi, generate_dart_scene_abi, generate_dart_wire_codecs,
    generate_export_flutter, generate_hybrid_dart,
};
pub use kotlin_java::{
    generate_export_compose, generate_hybrid_kotlin, generate_java_core_abi,
    generate_java_scene_buffer, generate_java_wire_codecs,
};
pub use rust::{generate_hybrid_rust, generate_reloc_rust};
pub use swift::{generate_export_swiftui, generate_hybrid_swift, generate_swift_wire_codecs};
pub use typescript::{
    generate_export_web_component, generate_hybrid_ts, generate_ts_core_abi,
    generate_ts_scene_view, generate_ts_wire_codecs,
};

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
// Unified Core ABI (5) + Scene ABI (5) + Wire Codec (5) = 15 Artifacts
// ============================================================================

/// Generates all 5 checked-in Core ABI files keyed by workspace-relative path.
pub fn generate_all_core_abi_files() -> Vec<GeneratedFile> {
    let c_header = generate_c_core_abi_h();
    vec![
        GeneratedFile {
            relative_path: "cxx/nve-fabric-bridge/include/nve_abi.generated.h".to_owned(),
            content: c_header.clone(),
        },
        GeneratedFile {
            relative_path: "hosts/swiftui-kit/Sources/CNveAbi/include/nve_abi.generated.h"
                .to_owned(),
            content: c_header,
        },
        GeneratedFile {
            relative_path: "hosts/flutter-plugin/lib/src/nve_core_abi.generated.dart".to_owned(),
            content: generate_dart_core_abi(),
        },
        GeneratedFile {
            relative_path:
                "hosts/android-shell/android-app/src/dev/nve/android/generated/NveCoreAbi.java"
                    .to_owned(),
            content: generate_java_core_abi(),
        },
        GeneratedFile {
            relative_path: "hosts/web-shell/nve_core_abi.generated.ts".to_owned(),
            content: generate_ts_core_abi(),
        },
    ]
}

/// Generates all 5 checked-in Scene ABI files keyed by workspace-relative path.
pub fn generate_all_scene_abi_files() -> Vec<GeneratedFile> {
    let c_header = generate_c_scene_abi_h();
    vec![
        GeneratedFile {
            relative_path: "cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h".to_owned(),
            content: c_header.clone(),
        },
        GeneratedFile {
            relative_path: "hosts/swiftui-kit/Sources/CNveAbi/include/nve_scene_abi.generated.h"
                .to_owned(),
            content: c_header,
        },
        GeneratedFile {
            relative_path: "hosts/flutter-plugin/lib/src/nve_scene_abi.generated.dart".to_owned(),
            content: generate_dart_scene_abi(),
        },
        GeneratedFile {
            relative_path:
                "hosts/android-shell/android-app/src/dev/nve/android/generated/NveSceneBuffer.java"
                    .to_owned(),
            content: generate_java_scene_buffer(),
        },
        GeneratedFile {
            relative_path: "hosts/web-shell/nve_scene_view.generated.ts".to_owned(),
            content: generate_ts_scene_view(),
        },
    ]
}

/// Generates all 5 checked-in cross-language wire codec artifacts keyed by workspace-relative path.
pub fn generate_all_wire_codec_files() -> Vec<GeneratedFile> {
    vec![
        GeneratedFile {
            relative_path: "cxx/nve-fabric-bridge/include/nve_wire_codecs.generated.hpp".to_owned(),
            content: generate_cpp_wire_codecs(),
        },
        GeneratedFile {
            relative_path: "hosts/swiftui-kit/Sources/NveSwiftUI/NveWireCodecs.generated.swift"
                .to_owned(),
            content: generate_swift_wire_codecs(),
        },
        GeneratedFile {
            relative_path: "hosts/flutter-plugin/lib/src/nve_wire_codecs.generated.dart".to_owned(),
            content: generate_dart_wire_codecs(),
        },
        GeneratedFile {
            relative_path: "hosts/web-shell/nve_wire_codecs.generated.ts".to_owned(),
            content: generate_ts_wire_codecs(),
        },
        GeneratedFile {
            relative_path:
                "hosts/android-shell/android-app/src/dev/nve/android/generated/NveWireCodecs.java"
                    .to_owned(),
            content: generate_java_wire_codecs(),
        },
    ]
}

/// Generates every Core ABI (5) + Scene ABI (5) + Wire Codecs (5) + XXH64 mirror (6) artifact.
pub fn generate_all_abi_files() -> Vec<GeneratedFile> {
    let mut files = generate_all_core_abi_files();
    files.extend(generate_all_scene_abi_files());
    files.extend(generate_all_wire_codec_files());
    files.extend(hash::generate_all_hash_files());
    files
}

// ============================================================================
// Reloc Zero-Copy Slab Dispatcher
// ============================================================================

/// Emits both the standalone Rust module (`<snake>_reloc.generated.rs`) and C++20 header
/// (`<Pascal>Reloc.generated.hpp`) for one [`RelocSchema`].
pub fn generate_reloc_all(schema: &RelocSchema) -> Result<Vec<GeneratedFile>, CodegenError> {
    Ok(vec![
        generate_reloc_rust(schema)?,
        generate_reloc_cpp(schema)?,
    ])
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
    let cpp_source = c_cpp::import_rn::emit_component_cpp(spec);

    Ok(ImportRnOutput {
        spec: ParsedRnSpec::Component(spec.clone()),
        rust_module: GeneratedFile {
            relative_path: format!("{mod_name}.generated.rs"),
            content: rust_source,
        },
        cpp_adapter: GeneratedFile {
            relative_path: format!("{mod_name}_fabric_adapter.generated.hpp"),
            content: cpp_source,
        },
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
        cpp_adapter: GeneratedFile {
            relative_path: format!("{mod_name}_turbomodule_adapter.generated.hpp"),
            content: cpp_source,
        },
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
    let cpp_bridge_hpp = GeneratedFile {
        relative_path: format!("{name}NveBridge.generated.hpp"),
        content: c_cpp::export_rn_cpp::emit_cpp_bridge_hpp(spec),
    };
    let cpp_bridge_cpp = GeneratedFile {
        relative_path: format!("{name}NveBridge.generated.cpp"),
        content: c_cpp::export_rn_cpp::emit_cpp_bridge_cpp(spec),
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
        cpp_bridge_hpp,
        cpp_bridge_cpp,
        ios_component_view_mm,
        android_view_manager_kt,
        windows_component_view_hpp,
    })
}
