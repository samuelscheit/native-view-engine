//! Dart / Flutter target emitters (Core & Runtime `dart:ffi` ABI, core table layouts, Flutter
//! `StatefulWidget` export, and Hybrid FFI `dart:ffi` bindings).

pub mod core_abi;
pub mod flutter;
pub mod hybrid;
pub mod runtime_abi;
pub mod tables;

pub use core_abi::generate_dart_core_abi;
pub use flutter::generate_export_flutter;
pub use hybrid::generate_hybrid_dart;
pub use runtime_abi::generate_dart_runtime_abi;
pub use tables::generate_dart_tables;
