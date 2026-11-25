//! Rust target emitters (Reloc slabs, `import_rn` Dioxus bindings, and Hybrid FFI vtables/types).

pub mod hybrid;
pub mod import_rn;
pub mod reloc;
mod reloc_runtime;

pub use hybrid::generate_hybrid_rust;
pub use reloc::generate_reloc_rust;
