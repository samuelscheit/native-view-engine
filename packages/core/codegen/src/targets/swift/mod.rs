//! Swift target emitters (Wire Codecs, SwiftUI `NveExportSpec`, and Hybrid FFI C-interop).

pub mod hybrid;
pub mod swiftui;
pub mod wire;

pub use hybrid::generate_hybrid_swift;
pub use swiftui::generate_export_swiftui;
pub use wire::generate_swift_wire_codecs;
