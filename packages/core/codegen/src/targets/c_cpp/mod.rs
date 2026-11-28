//! C11 / C++20 / Objective-C++ target emitters: Hybrid FFI, the React Native TurboModule host
//! interface (`import_rn`), and the React Native iOS / Windows component exports.

pub mod export_rn_ios;
pub mod export_rn_windows;
pub mod hybrid;
pub mod import_rn;

pub use hybrid::generate_hybrid_cpp;
