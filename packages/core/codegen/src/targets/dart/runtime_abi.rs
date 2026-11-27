//! Dart `dart:ffi` emitter for `nve_runtime_abi.generated.dart` derived directly from
//! `nve_abi::schema::runtime_abi_descriptor()` (the same source as `nve_runtime_abi.generated.h`).
//!
//! Vtable slots and callbacks are C signatures (`name`, return type, `"T name, ..."` parameters)
//! translated mechanically into `dart:ffi` native / Dart typedef pairs.

use super::core_abi::{emit_dart_enum, emit_dart_struct};
use crate::naming::{screaming_snake_to_camel_case, to_camel_case, to_pascal_case};
use crate::schema::hybrid::schema_bridge::from_abi_descriptor;
use crate::writer::CodeWriter;

/// One C function signature: name, return type, and comma-separated `"type name"` parameters.
struct CSignature {
    name: &'static str,
    ret: &'static str,
    params: &'static str,
}

const fn sig(name: &'static str, ret: &'static str, params: &'static str) -> CSignature {
    CSignature { name, ret, params }
}

/// Host callbacks passed through the vtable (`NveRuntimeWakeFn` → `NveRuntimeWakeNative`).
const RUNTIME_CALLBACKS: &[CSignature] = &[sig("NveRuntimeWakeFn", "void", "void* ctx")];

/// `NveRuntimeApi` function-pointer slots in exact `#[repr(C)]` order (after the 16-byte
/// `abi_version` / `struct_size` / `schema_hash` header).
const RUNTIME_API_SLOTS: &[CSignature] = &[
    sig("create", "NveRuntimeHandle", "const NveRuntimeConfig* config"),
    sig("destroy", "void", "NveRuntimeHandle handle"),
    sig("set_wake_callback", "int32_t", "NveRuntimeHandle handle, NveRuntimeWakeFn cb, void* ctx"),
    sig("set_viewport", "int32_t", "NveRuntimeHandle handle, float width, float height, float scale_factor"),
    sig("attach_app_interface", "int32_t", "NveRuntimeHandle handle, NveGetInterfaceFn get_iface"),
    sig("submit_nve2", "int32_t", "NveRuntimeHandle handle, NveBufferSlice frame"),
    sig("tick", "int32_t", "NveRuntimeHandle handle, float dt_s, uint32_t* out_changed"),
    sig("dispatch_events", "int32_t", "NveRuntimeHandle handle, NveEventBatch batch, uint32_t* out_changed"),
    sig("hit_test", "int32_t", "NveRuntimeHandle handle, float x, float y, uint32_t* out_node_id"),
    sig(
        "scroll_by",
        "int32_t",
        "NveRuntimeHandle handle, uint32_t node_id, float dx, float dy, float* out_x, float* out_y",
    ),
    sig("scene_acquire", "int32_t", "NveRuntimeHandle handle, NveRuntimeSceneView* out_view"),
    sig("scene_release", "int32_t", "NveRuntimeHandle handle"),
    sig("get_metrics", "int32_t", "NveRuntimeHandle handle, NveRuntimeMetrics* out_metrics"),
    sig("pair_connect", "int32_t", "NveRuntimeHandle handle, NveStrSlice uri, NveStrSlice token"),
    sig("pair_disconnect", "int32_t", "NveRuntimeHandle handle"),
    sig("pair_is_connected", "uint32_t", "NveRuntimeHandle handle"),
    sig("last_error", "size_t", "uint8_t* out_buf, size_t cap"),
];

/// `(native, dart)` spelling of one C type.
fn dart_ffi_type(c: &str) -> (String, String) {
    let c = c.trim().trim_start_matches("const ").trim_start_matches("struct ").trim();
    if let Some(pointee) = c.strip_suffix('*') {
        let (native, _) = dart_ffi_type(pointee);
        let ptr = format!("ffi.Pointer<{native}>");
        return (ptr.clone(), ptr);
    }
    let scalar = |native: &str, dart: &str| (native.to_owned(), dart.to_owned());
    match c {
        "void" => scalar("ffi.Void", "void"),
        "int32_t" => scalar("ffi.Int32", "int"),
        "uint8_t" => scalar("ffi.Uint8", "int"),
        "uint32_t" => scalar("ffi.Uint32", "int"),
        "uint64_t" => scalar("ffi.Uint64", "int"),
        "size_t" => scalar("ffi.Size", "int"),
        "float" => scalar("ffi.Float", "double"),
        // `NveFooFn` C function pointer → `ffi.Pointer<ffi.NativeFunction<NveFooNative>>`.
        fn_ptr if fn_ptr.ends_with("Fn") => {
            let ty = format!(
                "ffi.Pointer<ffi.NativeFunction<{}Native>>",
                fn_ptr.trim_end_matches("Fn")
            );
            (ty.clone(), ty)
        }
        // Handles and by-value structs keep their generated Dart name.
        named => scalar(named, named),
    }
}

/// `(type, name)` pairs of a C parameter list (`"float* out_x"` → `("float*", "out_x")`).
fn c_params(params: &str) -> Vec<(String, String)> {
    params
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let split = p.rfind(|c: char| c.is_whitespace() || c == '*').expect("C param has a name");
            let (ty, name) = p.split_at(split + 1);
            (ty.trim().to_owned(), name.trim().to_owned())
        })
        .collect()
}

/// Emits `typedef {prefix}{Pascal}Native = ...;` and (when `with_dart`) the matching `...Dart`.
fn emit_typedefs(out: &mut CodeWriter, typedef_base: &str, s: &CSignature, with_dart: bool) {
    let params = c_params(s.params);
    let kinds: &[(usize, &str)] = if with_dart { &[(0, "Native"), (1, "Dart")] } else { &[(0, "Native")] };
    for &(pick, suffix) in kinds {
        let ty = |c: &str| {
            let (native, dart) = dart_ffi_type(c);
            if pick == 0 { native } else { dart }
        };
        let list: Vec<String> =
            params.iter().map(|(t, n)| format!("{} {}", ty(t), to_camel_case(n))).collect();
        writeln!(
            out,
            "typedef {typedef_base}{suffix} = {} Function({});",
            ty(s.ret),
            list.join(", ")
        );
    }
}

/// Generates `packages/core/host-dart/lib/src/nve_runtime_abi.generated.dart` directly from
/// `packages/core/abi/src/runtime.rs`.
pub fn generate_dart_runtime_abi() -> String {
    let spec = from_abi_descriptor(&nve_abi::schema::runtime_abi_descriptor());
    let schema_hash = nve_abi::NVE_RUNTIME_SCHEMA_HASH;
    let mut out = CodeWriter::with_capacity(8 * 1024);

    writeln!(
        out,
        "// @generated by nve-codegen (runtime Dart ABI generator) — DO NOT EDIT MANUALLY."
    );
    writeln!(
        out,
        "// Zero-copy NveRuntimeSceneView, NveNodeRow, NveLayoutRect, and NveRuntimeApi `dart:ffi`"
    );
    writeln!(out, "// bindings generated from packages/core/abi/src/runtime.rs.");
    writeln!(out);
    writeln!(out, "import 'dart:ffi' as ffi;");
    writeln!(out);
    writeln!(out, "import 'key64.dart';");
    writeln!(out, "import 'nve_core_abi.generated.dart';");
    writeln!(out);
    writeln!(
        out,
        "/// Deterministic XXH64 runtime ABI schema hash (`0x{schema_hash:016X}`) as exact u32 halves."
    );
    writeln!(
        out,
        "const Key64 nveRuntimeSchemaHash = Key64(0x{:08X}, 0x{:08X});",
        schema_hash as u32,
        (schema_hash >> 32) as u32
    );
    for c in &spec.consts {
        if let Some(doc) = &c.doc {
            writeln!(out, "/// {doc}");
        }
        writeln!(out, "const int {} = {};", screaming_snake_to_camel_case(&c.name), c.value);
    }
    writeln!(out);

    for e in &spec.enums {
        emit_dart_enum(&mut out, e);
    }
    for s in &spec.structs {
        emit_dart_struct(&mut out, s);
    }

    writeln!(out, "typedef NveRuntimeHandle = ffi.Pointer<ffi.Void>;");
    for cb in RUNTIME_CALLBACKS {
        emit_typedefs(&mut out, cb.name.trim_end_matches("Fn"), cb, false);
    }
    emit_typedefs(
        &mut out,
        "NveRuntimeGetInterface",
        &sig("nve_runtime_get_interface", "NveRuntimeApi*", "uint32_t abi_version"),
        true,
    );
    for slot in RUNTIME_API_SLOTS {
        emit_typedefs(&mut out, &format!("NveRuntime{}", to_pascal_case(slot.name)), slot, true);
    }
    writeln!(out);

    writeln!(
        out,
        "/// Single `NveRuntimeApi` vtable returned by `nve_runtime_get_interface(abi_version)`."
    );
    writeln!(out, "final class NveRuntimeApi extends ffi.Struct {{");
    writeln!(out, "  @ffi.Uint32()");
    writeln!(out, "  external int abiVersion;");
    writeln!(out, "  @ffi.Uint32()");
    writeln!(out, "  external int structSize;");
    writeln!(out, "  @ffi.Uint64()");
    writeln!(out, "  external int schemaHash;");
    for slot in RUNTIME_API_SLOTS {
        writeln!(
            out,
            "  external ffi.Pointer<ffi.NativeFunction<NveRuntime{}Native>> {};",
            to_pascal_case(slot.name),
            to_camel_case(slot.name)
        );
    }
    writeln!(out, "}}");

    out.into_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn vtable_slots_match_rust_repr_c_layout() {
        let header = 16;
        let slots = (size_of::<nve_abi::NveRuntimeApi>() - header) / size_of::<usize>();
        assert_eq!(RUNTIME_API_SLOTS.len(), slots);
        let src = generate_dart_runtime_abi();
        assert!(src.contains("external ffi.Pointer<ffi.NativeFunction<NveRuntimeLastErrorNative>> lastError;"));
        assert!(src.contains("typedef NveRuntimeScrollByDart = int Function(NveRuntimeHandle handle, int nodeId, double dx, double dy, ffi.Pointer<ffi.Float> outX, ffi.Pointer<ffi.Float> outY);"));
        assert!(src.contains("typedef NveRuntimeSetWakeCallbackNative = ffi.Int32 Function(NveRuntimeHandle handle, ffi.Pointer<ffi.NativeFunction<NveRuntimeWakeNative>> cb, ffi.Pointer<ffi.Void> ctx);"));
        assert!(src.contains("typedef NveRuntimeWakeNative = ffi.Void Function(ffi.Pointer<ffi.Void> ctx);"));
        assert!(src.contains("typedef NveRuntimeGetInterfaceDart = ffi.Pointer<NveRuntimeApi> Function(int abiVersion);"));
        assert!(src.contains("@ffi.Array(4)\n  external ffi.Array<ffi.Uint32> tableSet;"));
        assert!(src.contains("external ffi.Pointer<NveTableView> tables;"));
        assert!(src.contains("enum NveRuntimeStatus {"));
    }

    #[test]
    fn c_types_map_to_dart_ffi() {
        assert_eq!(c_params("uint8_t* out_buf, size_t cap"), [
            ("uint8_t*".to_owned(), "out_buf".to_owned()),
            ("size_t".to_owned(), "cap".to_owned())
        ]);
        assert_eq!(dart_ffi_type("const struct NveMapEntry*").0, "ffi.Pointer<NveMapEntry>");
        assert_eq!(dart_ffi_type("uint64_t*").1, "ffi.Pointer<ffi.Uint64>");
        assert_eq!(dart_ffi_type("NveGetInterfaceFn").0, "ffi.Pointer<ffi.NativeFunction<NveGetInterfaceNative>>");
        assert_eq!(dart_ffi_type("float"), ("ffi.Float".to_owned(), "double".to_owned()));
    }
}
