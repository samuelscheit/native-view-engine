//! Zero-copy aligned `RelocSlice` & `VerifyMode` code generator for Rust and C++20.
//!
//! Takes a declarative Rust [`RelocSchema`] (constructed programmatically by each owning crate
//! without any central `.nve.rs` text files) and emits zero-serialization, zero-hand-written-codec
//! Rust 2024 modules and C++20 headers with ASLR-safe relative slices (`RelocSlice<T>`, `RelocStr`),
//! 3-tier verification (`VerifyMode::{Full, HeaderOnly, Unchecked}`), deterministic FNV-1a64
//! schema hashes, zero-padding POD `as_bytes()`, and compile-time `size_of`/`offset_of` checks.

mod cpp_runtime;
pub mod emit_cpp;
pub mod emit_rust;
pub mod ir;
mod rust_runtime;
mod validate;

pub use emit_cpp::generate_reloc_cpp;
pub use emit_rust::generate_reloc_rust;
pub use ir::{
    FieldLayout, RelocEnum, RelocEnumRepr, RelocEnumVariant, RelocField, RelocSchema, RelocStruct,
    RelocType, StructLayout, RELOC_HEADER_ALIGN, RELOC_HEADER_SIZE, RELOC_MAX_VERIFY_DEPTH,
    RELOC_SLICE_ALIGN, RELOC_SLICE_SIZE,
};

use crate::error::CodegenError;
use crate::ir::GeneratedFile;

/// Emits both the standalone Rust module (`<snake>_reloc.generated.rs`) and C++20 header
/// (`<Pascal>Reloc.generated.hpp`) for one [`RelocSchema`].
pub fn generate_reloc_all(schema: &RelocSchema) -> Result<Vec<GeneratedFile>, CodegenError> {
    Ok(vec![
        generate_reloc_rust(schema)?,
        generate_reloc_cpp(schema)?,
    ])
}
