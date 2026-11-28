//! Kotlin & Java target emitters (Jetpack Compose `NveExportSpec`, Android RN Fabric ViewManager export, and Kotlin/JNI Hybrid FFI).

pub mod compose;
pub mod export_rn_android;
mod kotlin_bin_codec;
pub mod kotlin_hybrid;

pub use compose::generate_export_compose;
pub use kotlin_hybrid::generate_hybrid_kotlin;
