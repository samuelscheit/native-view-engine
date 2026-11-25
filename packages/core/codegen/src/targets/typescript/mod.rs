//! TypeScript / Web target emitters (Core & Scene ABI, Wire Codecs, Web Component export,
//! React Native Fabric TS spec export, and Hybrid FFI WASM client).

pub mod core_abi;
pub mod export_rn_ts;
pub mod hybrid;
pub mod web_component;
pub mod wire;

pub use core_abi::{generate_ts_core_abi, generate_ts_scene_view};
pub use hybrid::generate_hybrid_ts;
pub use web_component::generate_export_web_component;
pub use wire::generate_ts_wire_codecs;
