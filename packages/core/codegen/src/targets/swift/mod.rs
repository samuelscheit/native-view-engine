//! Swift target emitters (SwiftUI `NveExportSpec` export and Hybrid FFI C-interop).

pub mod hybrid;
pub mod swiftui;

pub use hybrid::generate_hybrid_swift;
pub use swiftui::generate_export_swiftui;
