//! C11 / C++20 / Objective-C++ target emitters (Core & Scene ABI, Hybrid FFI, React Native import adapters, and React Native C++/iOS/Windows exports).

pub mod core_abi;
pub mod export_rn_cpp;
pub mod export_rn_ios;
pub mod export_rn_windows;
pub mod hybrid;
pub mod import_rn;
pub mod scene_abi;

pub use core_abi::generate_c_core_abi_h;
pub use scene_abi::generate_c_scene_abi_h;
pub use hybrid::generate_hybrid_cpp;
