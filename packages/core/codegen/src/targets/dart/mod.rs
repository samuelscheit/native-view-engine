//! Dart / Flutter target emitters (Core & Scene `dart:ffi` ABI, Wire Codecs, Flutter
//! `StatefulWidget` export, and Hybrid FFI `dart:ffi` bindings).

pub mod core_abi;
pub mod flutter;
pub mod hybrid;
pub mod scene_abi;
pub mod wire;

pub use core_abi::generate_dart_core_abi;
pub use scene_abi::generate_dart_scene_abi;
pub use flutter::generate_export_flutter;
pub use hybrid::generate_hybrid_dart;
pub use wire::generate_dart_wire_codecs;
