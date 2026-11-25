//! Direction 1 (`import_rn`): React Native Fabric Component / TurboModule spec ->
//! Zero-JS, zero-JSON typed Rust `nve-dioxus` module + thin C++20 Fabric adapter (`cxx/nve-fabric-bridge`).

mod cpp_emitter;
mod rust_emitter;
mod rust_types;

use crate::error::CodegenError;
use crate::ir::{GeneratedFile, ImportRnOutput, ParsedRnSpec, RnComponentSpec, TurboModuleSpec};
use crate::naming::to_snake_case;

use cpp_emitter::{emit_component_cpp, emit_turbomodule_cpp};
use rust_emitter::{emit_component_rust, emit_turbomodule_rust};

/// Generates zero-JS, zero-JSON Rust `nve-dioxus` bindings and a thin C++20 Fabric adapter
/// from a parsed RN spec.
pub fn generate_import_rn(spec: &ParsedRnSpec) -> Result<ImportRnOutput, CodegenError> {
    match spec {
        ParsedRnSpec::Component(comp) => generate_component_import(comp),
        ParsedRnSpec::TurboModule(tm) => generate_turbomodule_import(tm),
    }
}

fn generate_component_import(spec: &RnComponentSpec) -> Result<ImportRnOutput, CodegenError> {
    let mod_name = to_snake_case(&spec.component_name);
    let rust_source = emit_component_rust(spec, &mod_name);
    let cpp_source = emit_component_cpp(spec);

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
    let rust_source = emit_turbomodule_rust(spec, &mod_name);
    let cpp_source = emit_turbomodule_cpp(spec);

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
