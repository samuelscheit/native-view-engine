//! TypeScript / Web target emitters (Web Component export, React Native Fabric TS spec export,
//! and Hybrid FFI WASM client).

pub mod export_rn_ts;
pub mod hybrid;
pub mod web_component;

pub use hybrid::generate_hybrid_ts;
pub use web_component::generate_export_web_component;
