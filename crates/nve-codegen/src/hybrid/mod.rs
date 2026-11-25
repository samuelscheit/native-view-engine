//! Nitro-inspired, Rust-first typed Hybrid FFI code generator for `native-view-engine`.
//!
//! Turns a declarative Rust specification (`.rs` file or programmatic [`HybridFileSpec`]) into
//! compile-time-enforced, zero-JSON, direct-FFI bindings across 6 targets:
//! - [`HybridTarget::Rust`]: trait contracts, `#[repr(C)]` POD structs/enums, direct C-ABI
//!   vtables (`NveHybrid<Name>VTable`), and zero-copy `NveMapEntry`/`NveDirectEvent` view helpers
//! - [`HybridTarget::Cpp`]: pure C11 ABI header (`<Stem>HybridAbi.h`) + C++20 Nitro-style
//!   abstract `Hybrid<Name>Spec` and RAII `<Name>Client` (`<Stem>HybridSpec.hpp`)
//! - [`HybridTarget::Swift`]: direct C-interop Swift 5.9+ `Hybrid<Name>Spec` protocol &
//!   `<Name>HybridClient` (zero `@objc`, zero `NSObject`, zero JSON)
//! - [`HybridTarget::Kotlin`]: Kotlin `Hybrid<Name>Spec` + C++20 JNI glue over primitives &
//!   `DirectByteBuffer` (zero `org.json`)
//! - [`HybridTarget::Dart`]: standalone `dart:ffi` `NveHybrid<Name>VTable` + `Hybrid<Name>Spec`
//!   + `<Name>HybridClient` (zero JSON, zero external Dart packages)
//! - [`HybridTarget::TypeScript`]: typed TS interfaces, enums, and direct `DataView`/`Uint8Array`
//!   WASM linear-memory client (zero JSON)

pub mod emit_cpp;
pub mod emit_dart;
pub mod emit_kotlin;
pub mod emit_rust;
pub mod emit_swift;
pub mod emit_ts;
pub mod ir;
pub mod parser;

use std::path::{Path, PathBuf};

use crate::error::CodegenError;
use crate::ir::{write_generated_files, GeneratedFile};

pub use emit_cpp::generate_hybrid_cpp;
pub use emit_dart::generate_hybrid_dart;
pub use emit_kotlin::generate_hybrid_kotlin;
pub use emit_rust::generate_hybrid_rust;
pub use emit_swift::generate_hybrid_swift;
pub use emit_ts::generate_hybrid_ts;
pub use ir::{
    compute_hybrid_interface_id, fnv1a_64, fnv1a_64_update, HybridDefaultValue, HybridEnumSpec,
    HybridEnumVariant, HybridEventSpec, HybridFieldSpec, HybridFileSpec, HybridMethodSpec,
    HybridModuleSpec, HybridParamSpec, HybridPropSpec, HybridStructSpec, HybridTarget, HybridType,
    HybridViewSpec, FNV1A_64_OFFSET_BASIS, NVE_HYBRID_ABI_VERSION, NVE_HYBRID_INTERFACE_ID_BASE,
    NVE_HYBRID_INTERFACE_ID_MASK,
};
pub use parser::{parse_hybrid_rust_spec, parse_hybrid_rust_spec_file};

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

/// Generates zero-JSON Hybrid FFI bindings for a single requested [`HybridTarget`].
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

/// Generates zero-JSON Hybrid FFI bindings for all requested [`HybridTarget`]s (or
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
