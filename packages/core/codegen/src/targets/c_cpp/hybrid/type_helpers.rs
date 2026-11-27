use crate::writer::CodeWriter;

use super::*;

pub(super) fn cpp_method_signature(method: &HybridMethodSpec) -> String {
    let mname = cpp_ident(&to_camel_case(&method.name));
    let ret = match &method.return_type {
        HybridType::Result { ok, .. } => cpp_owned_type(ok),
        other => cpp_owned_type(other),
    };
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| format!("{} {}", cpp_param_type(&p.ty), cpp_ident(&p.name)))
        .collect();
    format!("{ret} {mname}({})", params.join(", "))
}

pub(super) fn cpp_owned_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "void".to_string(),
        HybridType::Bool => "bool".to_string(),
        HybridType::U8 => "uint8_t".to_string(),
        HybridType::U16 => "uint16_t".to_string(),
        HybridType::Usize => "size_t".to_string(),
        HybridType::I32 => "int32_t".to_string(),
        HybridType::U32 => "uint32_t".to_string(),
        HybridType::I64 => "int64_t".to_string(),
        HybridType::U64 => "uint64_t".to_string(),
        HybridType::F32 => "float".to_string(),
        HybridType::F64 => "double".to_string(),
        HybridType::String => "std::string".to_string(),
        HybridType::Bytes => "std::vector<uint8_t>".to_string(),
        HybridType::Optional(inner) => format!("std::optional<{}>", cpp_owned_type(inner)),
        HybridType::Vec(inner) => format!("std::vector<{}>", cpp_owned_type(inner)),
        HybridType::Result { ok, .. } => cpp_owned_type(ok),
        HybridType::Struct(name) | HybridType::Enum(name) | HybridType::Handle(name) => {
            name.clone()
        }
        HybridType::Value => "DynamicValue".to_string(),
        HybridType::ConstPtr(inner) => format!("const {}*", cpp_owned_type(inner)),
        HybridType::MutPtr(inner) => format!("{}*", cpp_owned_type(inner)),
        HybridType::FixedArray { elem, len } => format!("std::array<{}, {len}>", cpp_owned_type(elem)),
    }
}

pub(super) fn cpp_param_type(ty: &HybridType) -> String {
    match ty {
        HybridType::String => "std::string_view".to_string(),
        HybridType::Bytes => "std::span<const uint8_t>".to_string(),
        HybridType::Value => "NveValue".to_string(),
        HybridType::Struct(name) => format!("const {name}&"),
        HybridType::Optional(inner) => format!("const std::optional<{}>&", cpp_owned_type(inner)),
        HybridType::Vec(inner) => format!("const std::vector<{}>&", cpp_owned_type(inner)),
        other => cpp_owned_type(other),
    }
}

pub(super) fn c_pod_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Bool => "bool".to_string(),
        HybridType::U8 => "uint8_t".to_string(),
        HybridType::U16 => "uint16_t".to_string(),
        HybridType::Usize => "size_t".to_string(),
        HybridType::I32 => "int32_t".to_string(),
        HybridType::U32 => "uint32_t".to_string(),
        HybridType::I64 => "int64_t".to_string(),
        HybridType::U64 => "uint64_t".to_string(),
        HybridType::F32 => "float".to_string(),
        HybridType::F64 => "double".to_string(),
        HybridType::Enum(name) | HybridType::Struct(name) => format!("NveHybrid{name}"),
        _ => unreachable!("non-POD type in c_pod_type"),
    }
}

pub(super) fn c_abi_field_decl(ty: &HybridType, fname: &str) -> String {
    match ty {
        HybridType::FixedArray { elem, len } => format!("{} {fname}[{len}]", c_pod_type(elem)),
        HybridType::ConstPtr(inner) => match inner.as_ref() {
            HybridType::Void => format!("const void* {fname}"),
            HybridType::Struct(name) => format!("const struct NveHybrid{name}* {fname}"),
            other => format!("const {}* {fname}", c_pod_type(other)),
        },
        HybridType::MutPtr(inner) => match inner.as_ref() {
            HybridType::Void => format!("void* {fname}"),
            HybridType::Struct(name) => format!("struct NveHybrid{name}* {fname}"),
            other => format!("{}* {fname}", c_pod_type(other)),
        },
        other => format!("{} {fname}", c_pod_type(other)),
    }
}

pub(super) fn is_c_abi_struct(s: &HybridStructSpec, spec: &HybridFileSpec) -> bool {
    if s.is_union || s.fields.is_empty() {
        return false;
    }
    s.fields.iter().all(|f| match &f.ty {
        HybridType::ConstPtr(inner) | HybridType::MutPtr(inner) => {
            matches!(inner.as_ref(), HybridType::Void) || inner.is_pod(spec)
        }
        other => other.is_pod(spec),
    })
}

pub(super) fn c_fn_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Void => "void".to_string(),
        HybridType::ConstPtr(inner) => match inner.as_ref() {
            HybridType::Void => "const void*".to_string(),
            HybridType::Struct(name) => format!("const NveHybrid{name}*"),
            other => format!("const {}*", c_pod_type(other)),
        },
        HybridType::MutPtr(inner) => match inner.as_ref() {
            HybridType::Void => "void*".to_string(),
            HybridType::Struct(name) => format!("NveHybrid{name}*"),
            other => format!("{}*", c_pod_type(other)),
        },
        other => c_param_type(other, spec),
    }
}

pub(super) fn c_param_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if let Some(suffix) = ty.optional_pod_suffix(spec) {
        return format!("NveHybridOpt{suffix}");
    }
    match ty {
        HybridType::Bool
        | HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::F32
        | HybridType::F64
        | HybridType::Enum(_) => c_pod_type(ty),
        HybridType::Struct(_) if ty.is_pod(spec) => c_pod_type(ty),
        HybridType::String => "NveStrSlice".to_string(),
        HybridType::Bytes => "NveBufferSlice".to_string(),
        HybridType::Value => "NveValue".to_string(),
        _ => "NveBufferSlice".to_string(),
    }
}

pub(super) fn c_return_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if let Some(suffix) = ty.optional_pod_suffix(spec) {
        return format!("NveHybridOpt{suffix}");
    }
    match ty {
        HybridType::Void => "void".to_string(),
        HybridType::Bool
        | HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::F32
        | HybridType::F64
        | HybridType::Enum(_) => c_pod_type(ty),
        HybridType::Struct(_) if ty.is_pod(spec) => c_pod_type(ty),
        _ => "NveHybridOwnedBytes".to_string(),
    }
}

pub(super) fn emit_cpp_bin_write(out: &mut CodeWriter, indent: &str, w: &str, expr: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.write_bool({expr});"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => {
            writeln!(out, "{indent}{w}.write_i64(static_cast<int64_t>({expr}));");
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{w}.write_f64(static_cast<double>({expr}));");
        }
        HybridType::Enum(_) => {
            writeln!(out, "{indent}{w}.write_i64(static_cast<int64_t>(static_cast<uint32_t>({expr})));");
        }
        HybridType::String => writeln!(out, "{indent}{w}.write_str({expr});"),
        HybridType::Bytes => writeln!(out, "{indent}{w}.write_bytes({expr});"),
        HybridType::Struct(_) => writeln!(out, "{indent}{expr}.encode_bin({w});"),
        HybridType::Value => writeln!(out, "{indent}{w}.write_raw({expr}.bin_bytes);"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({expr}.has_value()) {{");
            emit_cpp_bin_write(out, &format!("{indent}  "), w, &format!("(*{expr})"), inner);
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  {w}.write_null();");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{w}.write_array(static_cast<uint32_t>({expr}.size()), [&](BinWriter& _aw) {{");
            writeln!(out, "{indent}  for (const auto& _elem : {expr}) {{");
            emit_cpp_bin_write(out, &format!("{indent}    "), "_aw", "_elem", inner);
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}}});");
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {
            writeln!(out, "{indent}{w}.write_null();");
        }
    }
}

pub(super) fn emit_cpp_bin_read(out: &mut CodeWriter, indent: &str, r: &str, dst: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.read_bool();"),
        HybridType::U8 => writeln!(out, "{indent}{dst} = static_cast<uint8_t>({r}.read_i64());"),
        HybridType::U16 => writeln!(out, "{indent}{dst} = static_cast<uint16_t>({r}.read_i64());"),
        HybridType::Usize => writeln!(out, "{indent}{dst} = static_cast<size_t>({r}.read_i64());"),
        HybridType::I32 => writeln!(out, "{indent}{dst} = static_cast<int32_t>({r}.read_i64());"),
        HybridType::U32 => writeln!(out, "{indent}{dst} = static_cast<uint32_t>({r}.read_i64());"),
        HybridType::I64 => writeln!(out, "{indent}{dst} = {r}.read_i64();"),
        HybridType::U64 => writeln!(out, "{indent}{dst} = static_cast<uint64_t>({r}.read_i64());"),
        HybridType::F32 => writeln!(out, "{indent}{dst} = static_cast<float>({r}.read_f64());"),
        HybridType::F64 => writeln!(out, "{indent}{dst} = {r}.read_f64();"),
        HybridType::Enum(e) => {
            writeln!(out, "{indent}{dst} = static_cast<{e}>(static_cast<uint32_t>({r}.read_i64()));");
        }
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.read_str();"),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = {r}.read_bytes();"),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = {s}::decode_bin({r});"),
        HybridType::Value => writeln!(out, "{indent}{dst} = DynamicValue{{{r}.read_raw_value()}};"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({r}.read_null_if_present()) {{");
            writeln!(out, "{indent}  {dst} = std::nullopt;");
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  {} _opt_v{{}};", cpp_owned_type(inner));
            emit_cpp_bin_read(out, &format!("{indent}  "), r, "_opt_v", inner);
            writeln!(out, "{indent}  {dst} = std::move(_opt_v);");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  const uint32_t _n = {r}.begin_array();");
            writeln!(out, "{indent}  {dst}.clear();");
            writeln!(out, "{indent}  {dst}.reserve(_n);");
            writeln!(out, "{indent}  for (uint32_t _i = 0; _i < _n; ++_i) {{");
            writeln!(out, "{indent}    {} _elem{{}};", cpp_owned_type(inner));
            emit_cpp_bin_read(out, &format!("{indent}    "), r, "_elem", inner);
            writeln!(out, "{indent}    {dst}.push_back(std::move(_elem));");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}}}");
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {}
    }
}

pub(super) fn cpp_prop_is_direct_nve_value(ty: &HybridType) -> bool {
    match ty {
        HybridType::Bool
        | HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::F32
        | HybridType::F64
        | HybridType::Enum(_)
        | HybridType::String
        | HybridType::Bytes => true,
        HybridType::Optional(inner) => cpp_prop_is_direct_nve_value(inner),
        _ => false,
    }
}

pub(super) fn emit_cpp_fill_nve_value(
    out: &mut CodeWriter,
    indent: &str,
    dst: &str,
    expr: &str,
    ty: &HybridType,
    idx: usize,
) {
    match ty {
        HybridType::Optional(inner) if cpp_prop_is_direct_nve_value(inner) => {
            writeln!(out, "{indent}if ({expr}.has_value()) {{");
            emit_cpp_fill_nve_value(out, &format!("{indent}  "), dst, &format!("(*{expr})"), inner, idx);
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  {dst}.tag = NVE_VAL_NULL;");
            writeln!(out, "{indent}}}");
        }
        HybridType::Bool => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_BOOL;");
            writeln!(out, "{indent}{dst}.data.bool_val = {expr} ? 1u : 0u;");
        }
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_I64;");
            writeln!(out, "{indent}{dst}.data.i64_val = static_cast<int64_t>({expr});");
        }
        HybridType::Enum(_) => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_I64;");
            writeln!(out, "{indent}{dst}.data.i64_val = static_cast<int64_t>(static_cast<uint32_t>({expr}));");
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_F64;");
            writeln!(out, "{indent}{dst}.data.f64_val = static_cast<double>({expr});");
        }
        HybridType::String => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_STR;");
            writeln!(
                out,
                "{indent}{dst}.data.str_val = NveStrSlice{{reinterpret_cast<const uint8_t*>({expr}.data()), {expr}.size()}};"
            );
        }
        HybridType::Bytes => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_BYTES;");
            writeln!(
                out,
                "{indent}{dst}.data.bytes_val = NveBufferSlice{{{expr}.data(), {expr}.size()}};"
            );
        }
        _ => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_BYTES;");
            writeln!(out, "{indent}{dst}.data.bytes_val = _prop_bin_{idx}.slice();");
        }
    }
}

pub(super) fn cpp_prop_default_initializer(ty: &HybridType, def: Option<&HybridDefaultValue>) -> String {
    let Some(def) = def else {
        return "{}".to_string();
    };
    if let HybridType::Optional(inner) = ty {
        return match def {
            HybridDefaultValue::None => "std::nullopt".to_string(),
            other => cpp_prop_default_initializer(inner, Some(other)),
        };
    }
    match (def, ty) {
        (HybridDefaultValue::Bool(b), _) => b.to_string(),
        (HybridDefaultValue::Int(i), HybridType::F32) => format!("{i}.0f"),
        (HybridDefaultValue::Int(i), HybridType::F64) => format!("{i}.0"),
        (HybridDefaultValue::Int(i), _) if *i == i64::MIN => "INT64_MIN".to_string(),
        (HybridDefaultValue::Int(i), HybridType::I64) => format!("{i}LL"),
        (HybridDefaultValue::Int(i), _) => i.to_string(),
        (HybridDefaultValue::UInt(u), HybridType::F32) => format!("{u}.0f"),
        (HybridDefaultValue::UInt(u), HybridType::F64) => format!("{u}.0"),
        (HybridDefaultValue::UInt(u), HybridType::U64) => format!("{u}ULL"),
        (HybridDefaultValue::UInt(u), _) => format!("{u}u"),
        (HybridDefaultValue::Float(f), HybridType::F32) => format!("{}f", format_float_literal(*f)),
        (HybridDefaultValue::Float(f), _) => format_float_literal(*f),
        (HybridDefaultValue::String(s), _) => format!("\"{}\"", escape_c_like_string(s)),
        (HybridDefaultValue::EnumVariant { enum_name, variant }, _) => {
            format!("{enum_name}::{variant}")
        }
        (HybridDefaultValue::None, _) => "{}".to_string(),
    }
}
