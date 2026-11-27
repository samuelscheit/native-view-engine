use crate::writer::CodeWriter;

pub use super::scene_abi::generate_dart_scene_abi;
use crate::naming::{screaming_snake_to_camel_case, to_camel_case, to_pascal_case};
use crate::schema::hybrid::core_abi::{
    core_abi_spec, format_enum_value, module_c_struct_name, HOST_STATUS_VARIANTS,
};
use crate::schema::hybrid::{HybridAliasSpec, HybridType};

fn resolve_alias_target<'a>(ty: &'a HybridType, aliases: &'a [HybridAliasSpec]) -> &'a HybridType {
    match ty {
        HybridType::Handle(name) => aliases
            .iter()
            .find(|a| a.name == *name)
            .map_or(ty, |a| &a.target),
        _ => ty,
    }
}

pub(super) fn dart_core_ffi_annotation(
    ty: &HybridType,
    aliases: &[HybridAliasSpec],
) -> Option<&'static str> {
    match resolve_alias_target(ty, aliases) {
        HybridType::Bool => Some("@ffi.Bool()"),
        HybridType::U8 => Some("@ffi.Uint8()"),
        HybridType::U16 => Some("@ffi.Uint16()"),
        HybridType::Usize => Some("@ffi.Size()"),
        HybridType::I32 => Some("@ffi.Int32()"),
        HybridType::U32 => Some("@ffi.Uint32()"),
        HybridType::I64 => Some("@ffi.Int64()"),
        HybridType::U64 => Some("@ffi.Uint64()"),
        HybridType::F32 => Some("@ffi.Float()"),
        HybridType::F64 => Some("@ffi.Double()"),
        _ => None,
    }
}

pub(super) fn dart_core_native_type(ty: &HybridType, aliases: &[HybridAliasSpec]) -> String {
    if let HybridType::Handle(name) = ty {
        if name == "NveHostInvokeFn" {
            return "ffi.Pointer<ffi.NativeFunction<NveHostInvokeNative>>".to_string();
        }
        if name == "NveHostChannelPublishFn" {
            return "ffi.Pointer<ffi.NativeFunction<NveHostChannelPublishNative>>".to_string();
        }
    }
    match resolve_alias_target(ty, aliases) {
        HybridType::Void => "ffi.Void".to_string(),
        HybridType::Bool => "ffi.Bool".to_string(),
        HybridType::U8 => "ffi.Uint8".to_string(),
        HybridType::U16 => "ffi.Uint16".to_string(),
        HybridType::Usize => "ffi.Size".to_string(),
        HybridType::I32 => "ffi.Int32".to_string(),
        HybridType::U32 => "ffi.Uint32".to_string(),
        HybridType::I64 => "ffi.Int64".to_string(),
        HybridType::U64 => "ffi.Uint64".to_string(),
        HybridType::F32 => "ffi.Float".to_string(),
        HybridType::F64 => "ffi.Double".to_string(),
        HybridType::Value => "NveValue".to_string(),
        HybridType::Struct(name) => name.clone(),
        HybridType::Handle(_) => "ffi.Pointer<ffi.Void>".to_string(),
        HybridType::ConstPtr(inner) | HybridType::MutPtr(inner) => {
            format!("ffi.Pointer<{}>", dart_core_native_type(inner, aliases))
        }
        other => unreachable!("unsupported core ABI Dart native type: {other:?}"),
    }
}

pub(super) fn dart_core_struct_field_type(ty: &HybridType, aliases: &[HybridAliasSpec]) -> String {
    if let HybridType::Handle(name) = ty {
        if name == "NveHostInvokeFn" {
            return "ffi.Pointer<ffi.NativeFunction<NveHostInvokeNative>>".to_string();
        }
        if name == "NveHostChannelPublishFn" {
            return "ffi.Pointer<ffi.NativeFunction<NveHostChannelPublishNative>>".to_string();
        }
    }
    match resolve_alias_target(ty, aliases) {
        HybridType::Bool => "bool".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => "int".to_string(),
        HybridType::F32 | HybridType::F64 => "double".to_string(),
        HybridType::Value => "NveValue".to_string(),
        HybridType::Struct(name) => name.clone(),
        HybridType::Handle(_) => "ffi.Pointer<ffi.Void>".to_string(),
        HybridType::ConstPtr(inner) | HybridType::MutPtr(inner) => {
            format!("ffi.Pointer<{}>", dart_core_native_type(inner, aliases))
        }
        other => unreachable!("unsupported core ABI Dart struct field type: {other:?}"),
    }
}

fn dart_enum_variant_ident(variant_name: &str) -> String {
    let camel = to_camel_case(variant_name);
    match camel.as_str() {
        "null" => "nullValue".to_string(),
        "bool" => "boolValue".to_string(),
        "default" => "defaultValue".to_string(),
        _ => camel,
    }
}

// ============================================================================
// Core ABI Target 3: Dart `dart:ffi` (`hosts/flutter-plugin/lib/src/nve_core_abi.generated.dart`)
// ============================================================================

/// Generates `hosts/flutter-plugin/lib/src/nve_core_abi.generated.dart` directly from
/// `packages/core/abi/src/schema.rs`.
pub fn generate_dart_core_abi() -> String {
    let spec = core_abi_spec();
    let schema_hash = crate::schema::hybrid::core_abi::core_abi_schema_hash();
    let signed_hash = schema_hash as i64;
    let mut out = CodeWriter::with_capacity(8 * 1024);

    writeln!(
        out,
        "// @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY."
    );
    writeln!(
        out,
        "// Single-source NVE Core C ABI `dart:ffi` bindings generated from packages/core/abi/src/schema.rs."
    );
    writeln!(out);
    writeln!(out, "import 'dart:ffi' as ffi;");
    writeln!(out);
    writeln!(
        out,
        "/// Deterministic XXH64 Core ABI schema hash (`0x{schema_hash:016X}`)."
    );
    writeln!(out, "const int nveCoreAbiSchemaHash = {signed_hash};");
    writeln!(
        out,
        "const String nveCoreAbiSchemaHashHex = '0x{schema_hash:016X}';"
    );
    writeln!(out);

    // Core constants
    for c in &spec.consts {
        let dart_name = screaming_snake_to_camel_case(&c.name);
        if c.hex {
            writeln!(out, "const int {dart_name} = 0x{:X};", c.value);
        } else {
            writeln!(out, "const int {dart_name} = {};", c.value);
        }
    }
    writeln!(out);

    // Modular interface IDs
    writeln!(
        out,
        "/// Modular interface IDs passed to `nve_get_interface(uint32_t)`."
    );
    for m in &spec.modules {
        let pascal = to_pascal_case(&m.name);
        writeln!(out, "const int nveIface{pascal} = {};", m.interface_id());
    }
    writeln!(out);

    // Core enums
    for e in &spec.enums {
        writeln!(out, "enum {} {{", e.name);
        for (i, v) in e.variants.iter().enumerate() {
            let vname = dart_enum_variant_ident(&v.name);
            let term = if i + 1 == e.variants.len() { ";" } else { "," };
            writeln!(out, "  {vname}({}){term}", format_enum_value(v.value));
        }
        writeln!(out);
        writeln!(out, "  const {}(this.code);", e.name);
        writeln!(out, "  final int code;");
        writeln!(out);
        if let Some(fallback) = &e.dart_fallback {
            let fb_ident = dart_enum_variant_ident(fallback);
            writeln!(out, "  static {} fromCode(int code) {{", e.name);
            writeln!(out, "    return switch (code) {{");
            for v in &e.variants {
                let vname = dart_enum_variant_ident(&v.name);
                if vname == fb_ident {
                    continue;
                }
                writeln!(
                    out,
                    "      {} => {}.{vname},",
                    format_enum_value(v.value),
                    e.name
                );
            }
            writeln!(out, "      _ => {}.{fb_ident},", e.name);
            writeln!(out, "    }};");
            writeln!(out, "  }}");
        } else {
            writeln!(out, "  static {}? fromCode(int code) {{", e.name);
            writeln!(out, "    return switch (code) {{");
            for v in &e.variants {
                let vname = dart_enum_variant_ident(&v.name);
                writeln!(
                    out,
                    "      {} => {}.{vname},",
                    format_enum_value(v.value),
                    e.name
                );
            }
            writeln!(out, "      _ => null,");
            writeln!(out, "    }};");
            writeln!(out, "  }}");
        }
        writeln!(out, "}}");
        writeln!(out);
    }

    // Signed host-capability status enum (`NVE_IFACE_HOST = 5`)
    writeln!(out, "enum NveHostStatus {{");
    for (i, &(variant, code)) in HOST_STATUS_VARIANTS.iter().enumerate() {
        let vname = dart_enum_variant_ident(variant);
        let term = if i + 1 == HOST_STATUS_VARIANTS.len() {
            ";"
        } else {
            ","
        };
        writeln!(out, "  {vname}({code}){term}");
    }
    writeln!(out);
    writeln!(out, "  const NveHostStatus(this.code);");
    writeln!(out, "  final int code;");
    writeln!(out);
    writeln!(out, "  static NveHostStatus? fromCode(int code) {{");
    writeln!(out, "    return switch (code) {{");
    for &(variant, code) in HOST_STATUS_VARIANTS {
        let vname = dart_enum_variant_ident(variant);
        writeln!(out, "      {code} => NveHostStatus.{vname},");
    }
    writeln!(out, "      _ => null,");
    writeln!(out, "    }};");
    writeln!(out, "  }}");
    writeln!(out, "}}");
    writeln!(out);

    // Core structs & union
    for s in &spec.structs {
        if s.name == "NveHostCallbacks" {
            writeln!(out, "typedef NveHostInvokeNative = ffi.Int32 Function(");
            writeln!(out, "  ffi.Pointer<ffi.Void> hostCtx,");
            writeln!(out, "  ffi.Uint32 serviceId,");
            writeln!(out, "  ffi.Uint32 methodId,");
            writeln!(out, "  ffi.Uint64 requestId,");
            writeln!(out, "  NveBufferSlice request,");
            writeln!(out, "  ffi.Pointer<ffi.Uint8> outBuf,");
            writeln!(out, "  ffi.Size outCap,");
            writeln!(out, "  ffi.Pointer<ffi.Size> outLen,");
            writeln!(out, ");");
            writeln!(
                out,
                "typedef NveHostChannelPublishNative = ffi.Int32 Function("
            );
            writeln!(out, "  ffi.Pointer<ffi.Void> hostCtx,");
            writeln!(out, "  ffi.Uint32 channelId,");
            writeln!(out, "  ffi.Uint32 flags,");
            writeln!(out, "  NveBufferSlice payload,");
            writeln!(out, ");");
            writeln!(out);
        }
        let base = if s.is_union { "ffi.Union" } else { "ffi.Struct" };
        writeln!(out, "final class {} extends {base} {{", s.name);
        for f in &s.fields {
            let dart_field = to_camel_case(&f.name);
            match &f.ty {
                HybridType::FixedArray { elem, len } => {
                    writeln!(out, "  @ffi.Array({len})");
                    writeln!(
                        out,
                        "  external ffi.Array<{}> {dart_field};",
                        dart_core_native_type(elem, &spec.aliases)
                    );
                }
                _ => {
                    if let Some(ann) = dart_core_ffi_annotation(&f.ty, &spec.aliases) {
                        writeln!(out, "  {ann}");
                    }
                    writeln!(
                        out,
                        "  external {} {dart_field};",
                        dart_core_struct_field_type(&f.ty, &spec.aliases)
                    );
                }
            }
        }
        writeln!(out, "}}");
        writeln!(out);
    }

    // Native function typedefs for modular interfaces + nve_get_interface
    for m in &spec.modules {
        for method in &m.methods {
            let td_name = format!("Nve{}Native", to_pascal_case(&method.name));
            let ret = dart_core_native_type(&method.return_type, &spec.aliases);
            let mut params = Vec::new();
            if method.has_self {
                params.push("ffi.Pointer<ffi.Void> handle".to_string());
            }
            for p in &method.params {
                params.push(format!(
                    "{} {}",
                    dart_core_native_type(&p.ty, &spec.aliases),
                    to_camel_case(&p.name)
                ));
            }
            writeln!(out, "typedef {td_name} = {ret} Function(");
            for p in &params {
                writeln!(out, "  {p},");
            }
            writeln!(out, ");");
        }
    }
    writeln!(
        out,
        "typedef NveGetInterfaceNative = ffi.Pointer<ffi.Void> Function(ffi.Uint32 interfaceId);"
    );
    writeln!(out);

    // Modular interface vtable structs
    for m in &spec.modules {
        let c_name = module_c_struct_name(m);
        writeln!(out, "final class {c_name} extends ffi.Struct {{");
        for method in &m.methods {
            let td_name = format!("Nve{}Native", to_pascal_case(&method.name));
            let field_name = to_camel_case(&method.name);
            writeln!(
                out,
                "  external ffi.Pointer<ffi.NativeFunction<{td_name}>> {field_name};"
            );
        }
        writeln!(out, "}}");
        writeln!(out);
    }

    out.into_string()
}



