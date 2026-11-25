use std::fmt::Write as _;

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

pub(super) fn c_param_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
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
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            format!("NveHybridOpt{}", inner.pod_opt_suffix().unwrap())
        }
        _ => "NveBufferSlice".to_string(),
    }
}

pub(super) fn c_return_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
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
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            format!("NveHybridOpt{}", inner.pod_opt_suffix().unwrap())
        }
        _ => "NveHybridOwnedBytes".to_string(),
    }
}

pub(super) fn emit_cpp_bin_write(out: &mut String, indent: &str, w: &str, expr: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.write_bool({expr});").unwrap(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => {
            writeln!(out, "{indent}{w}.write_i64(static_cast<int64_t>({expr}));").unwrap();
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{w}.write_f64(static_cast<double>({expr}));").unwrap();
        }
        HybridType::Enum(_) => {
            writeln!(out, "{indent}{w}.write_i64(static_cast<int64_t>(static_cast<uint32_t>({expr})));").unwrap();
        }
        HybridType::String => writeln!(out, "{indent}{w}.write_str({expr});").unwrap(),
        HybridType::Bytes => writeln!(out, "{indent}{w}.write_bytes({expr});").unwrap(),
        HybridType::Struct(_) => writeln!(out, "{indent}{expr}.encode_bin({w});").unwrap(),
        HybridType::Value => writeln!(out, "{indent}{w}.write_raw({expr}.bin_bytes);").unwrap(),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({expr}.has_value()) {{").unwrap();
            emit_cpp_bin_write(out, &format!("{indent}  "), w, &format!("(*{expr})"), inner);
            writeln!(out, "{indent}}} else {{").unwrap();
            writeln!(out, "{indent}  {w}.write_null();").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{w}.write_array(static_cast<uint32_t>({expr}.size()), [&](BinWriter& _aw) {{").unwrap();
            writeln!(out, "{indent}  for (const auto& _elem : {expr}) {{").unwrap();
            emit_cpp_bin_write(out, &format!("{indent}    "), "_aw", "_elem", inner);
            writeln!(out, "{indent}  }}").unwrap();
            writeln!(out, "{indent}}});").unwrap();
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {
            writeln!(out, "{indent}{w}.write_null();").unwrap();
        }
    }
}

pub(super) fn emit_cpp_bin_read(out: &mut String, indent: &str, r: &str, dst: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.read_bool();").unwrap(),
        HybridType::U8 => writeln!(out, "{indent}{dst} = static_cast<uint8_t>({r}.read_i64());").unwrap(),
        HybridType::U16 => writeln!(out, "{indent}{dst} = static_cast<uint16_t>({r}.read_i64());").unwrap(),
        HybridType::Usize => writeln!(out, "{indent}{dst} = static_cast<size_t>({r}.read_i64());").unwrap(),
        HybridType::I32 => writeln!(out, "{indent}{dst} = static_cast<int32_t>({r}.read_i64());").unwrap(),
        HybridType::U32 => writeln!(out, "{indent}{dst} = static_cast<uint32_t>({r}.read_i64());").unwrap(),
        HybridType::I64 => writeln!(out, "{indent}{dst} = {r}.read_i64();").unwrap(),
        HybridType::U64 => writeln!(out, "{indent}{dst} = static_cast<uint64_t>({r}.read_i64());").unwrap(),
        HybridType::F32 => writeln!(out, "{indent}{dst} = static_cast<float>({r}.read_f64());").unwrap(),
        HybridType::F64 => writeln!(out, "{indent}{dst} = {r}.read_f64();").unwrap(),
        HybridType::Enum(e) => {
            writeln!(out, "{indent}{dst} = static_cast<{e}>(static_cast<uint32_t>({r}.read_i64()));").unwrap();
        }
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.read_str();").unwrap(),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = {r}.read_bytes();").unwrap(),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = {s}::decode_bin({r});").unwrap(),
        HybridType::Value => writeln!(out, "{indent}{dst} = DynamicValue{{{r}.read_raw_value()}};").unwrap(),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({r}.read_null_if_present()) {{").unwrap();
            writeln!(out, "{indent}  {dst} = std::nullopt;").unwrap();
            writeln!(out, "{indent}}} else {{").unwrap();
            writeln!(out, "{indent}  {} _opt_v{{}};", cpp_owned_type(inner)).unwrap();
            emit_cpp_bin_read(out, &format!("{indent}  "), r, "_opt_v", inner);
            writeln!(out, "{indent}  {dst} = std::move(_opt_v);").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{{").unwrap();
            writeln!(out, "{indent}  const uint32_t _n = {r}.begin_array();").unwrap();
            writeln!(out, "{indent}  {dst}.clear();").unwrap();
            writeln!(out, "{indent}  {dst}.reserve(_n);").unwrap();
            writeln!(out, "{indent}  for (uint32_t _i = 0; _i < _n; ++_i) {{").unwrap();
            writeln!(out, "{indent}    {} _elem{{}};", cpp_owned_type(inner)).unwrap();
            emit_cpp_bin_read(out, &format!("{indent}    "), r, "_elem", inner);
            writeln!(out, "{indent}    {dst}.push_back(std::move(_elem));").unwrap();
            writeln!(out, "{indent}  }}").unwrap();
            writeln!(out, "{indent}}}").unwrap();
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
    out: &mut String,
    indent: &str,
    dst: &str,
    expr: &str,
    ty: &HybridType,
    idx: usize,
) {
    match ty {
        HybridType::Optional(inner) if cpp_prop_is_direct_nve_value(inner) => {
            writeln!(out, "{indent}if ({expr}.has_value()) {{").unwrap();
            emit_cpp_fill_nve_value(out, &format!("{indent}  "), dst, &format!("(*{expr})"), inner, idx);
            writeln!(out, "{indent}}} else {{").unwrap();
            writeln!(out, "{indent}  {dst}.tag = NVE_VAL_NULL;").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::Bool => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_BOOL;").unwrap();
            writeln!(out, "{indent}{dst}.data.bool_val = {expr} ? 1u : 0u;").unwrap();
        }
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_I64;").unwrap();
            writeln!(out, "{indent}{dst}.data.i64_val = static_cast<int64_t>({expr});").unwrap();
        }
        HybridType::Enum(_) => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_I64;").unwrap();
            writeln!(out, "{indent}{dst}.data.i64_val = static_cast<int64_t>(static_cast<uint32_t>({expr}));").unwrap();
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_F64;").unwrap();
            writeln!(out, "{indent}{dst}.data.f64_val = static_cast<double>({expr});").unwrap();
        }
        HybridType::String => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_STR;").unwrap();
            writeln!(
                out,
                "{indent}{dst}.data.str_val = NveStrSlice{{reinterpret_cast<const uint8_t*>({expr}.data()), {expr}.size()}};"
            )
            .unwrap();
        }
        HybridType::Bytes => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_BYTES;").unwrap();
            writeln!(
                out,
                "{indent}{dst}.data.bytes_val = NveBufferSlice{{{expr}.data(), {expr}.size()}};"
            )
            .unwrap();
        }
        _ => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_BYTES;").unwrap();
            writeln!(out, "{indent}{dst}.data.bytes_val = _prop_bin_{idx}.slice();").unwrap();
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
