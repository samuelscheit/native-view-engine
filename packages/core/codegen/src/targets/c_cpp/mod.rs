//! C11 / C++20 / Objective-C++ target emitters (Core & Scene ABI, Wire Codecs, Reloc slabs,
//! Hybrid FFI, React Native import adapters, and React Native C++/iOS/Windows exports).

pub mod core_abi;
pub mod export_rn_cpp;
pub mod export_rn_ios;
pub mod export_rn_windows;
pub mod hybrid;
pub mod import_rn;
pub mod reloc;
mod reloc_runtime;
pub mod scene_abi;
pub mod wire;

pub use core_abi::generate_c_core_abi_h;
pub use scene_abi::generate_c_scene_abi_h;
pub use hybrid::generate_hybrid_cpp;
pub use reloc::generate_reloc_cpp;
pub use wire::generate_cpp_wire_codecs;

use crate::writer::CodeWriter;

/// Emits the C11/C++ `NVE_STATIC_ASSERT(cond, msg)` portability macro guarded by `#ifndef`.
pub(super) fn emit_static_assert_macro(out: &mut CodeWriter) {
    writeln!(out, "#ifndef NVE_STATIC_ASSERT");
    writeln!(out, "#if defined(__cplusplus)");
    writeln!(out, "#define NVE_STATIC_ASSERT(cond, msg) static_assert((cond), msg)");
    writeln!(
        out,
        "#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L"
    );
    writeln!(
        out,
        "#define NVE_STATIC_ASSERT(cond, msg) _Static_assert((cond), msg)"
    );
    writeln!(out, "#else");
    writeln!(out, "#define NVE_STATIC_ASSERT(cond, msg)");
    writeln!(out, "#endif");
    writeln!(out, "#endif");
}
