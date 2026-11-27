//! C11 / C++20 / Objective-C++ target emitters: Hybrid FFI, React Native import adapters, and
//! React Native C++ / iOS / Windows exports.

pub mod export_rn_cpp;
pub mod export_rn_ios;
pub mod export_rn_windows;
pub mod hybrid;
pub mod import_rn;

pub use hybrid::generate_hybrid_cpp;
