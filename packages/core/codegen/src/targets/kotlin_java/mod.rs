//! Kotlin & Java target emitters (Java Core & Scene ABI, Jetpack Compose
//! `NveExportSpec`, Android RN Fabric ViewManager export, and Kotlin/JNI Hybrid FFI).

pub mod compose;
pub mod export_rn_android;
pub mod java_core_abi;
pub mod java_tables;
pub mod kotlin_hybrid;

pub use compose::generate_export_compose;
pub use java_core_abi::{generate_java_core_abi, generate_java_scene_buffer};
pub use java_tables::{generate_java_tables, generate_java_tables_file, JAVA_TABLES_PATH};
pub use kotlin_hybrid::generate_hybrid_kotlin;
