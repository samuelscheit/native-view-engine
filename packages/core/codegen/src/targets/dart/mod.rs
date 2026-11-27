//! Dart / Flutter target emitters (Core & Scene `dart:ffi` ABI, Flutter
//! `StatefulWidget` export, and Hybrid FFI `dart:ffi` bindings).

pub mod core_abi;
pub mod flutter;
pub mod hybrid;
pub mod scene_abi;

pub use core_abi::generate_dart_core_abi;
pub use scene_abi::generate_dart_scene_abi;
pub use flutter::generate_export_flutter;
pub use hybrid::generate_hybrid_dart;
