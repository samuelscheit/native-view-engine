//! Dart / Flutter target emitters (Flutter `StatefulWidget` export and Hybrid FFI `dart:ffi`
//! bindings).

pub mod flutter;
pub mod hybrid;

pub use flutter::generate_export_flutter;
pub use hybrid::generate_hybrid_dart;
