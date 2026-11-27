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

pub(crate) fn cpp_owned_type(ty: &HybridType) -> String {
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
        HybridType::FixedArray { elem, len } => {
            format!("std::array<{}, {len}>", cpp_owned_type(elem))
        }
    }
}

pub(super) fn cpp_param_type(ty: &HybridType) -> String {
    match ty {
        HybridType::String => "std::string_view".to_string(),
        HybridType::Bytes => "std::span<const uint8_t>".to_string(),
        HybridType::Value => "const DynamicValue&".to_string(),
        HybridType::Struct(name) => format!("const {name}&"),
        HybridType::Optional(inner) => format!("const std::optional<{}>&", cpp_owned_type(inner)),
        HybridType::Vec(_) | HybridType::FixedArray { .. } => format!("const {}&", cpp_owned_type(ty)),
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

/// Which way a POD struct field crosses between the C++ wrapper and its C ABI struct.
#[derive(Clone, Copy)]
pub(super) enum AbiDirection {
    ToAbi,
    FromAbi,
}

/// `dst = src` for one POD field across the C ABI: enums cast, POD structs convert, and `[T; N]`
/// (nested too) copy element by element, so a C++ wrapper is never `memcpy`d onto its C struct.
pub(super) fn emit_cpp_abi_copy(
    out: &mut CodeWriter,
    indent: &str,
    dst: &str,
    src: &str,
    ty: &HybridType,
    dir: AbiDirection,
    depth: usize,
) {
    match (ty, dir) {
        (HybridType::Enum(_), AbiDirection::ToAbi) => {
            writeln!(out, "{indent}{dst} = static_cast<uint32_t>({src});");
        }
        (HybridType::Enum(e), AbiDirection::FromAbi) => {
            writeln!(out, "{indent}{dst} = static_cast<{e}>({src});");
        }
        (HybridType::Struct(_), AbiDirection::ToAbi) => {
            writeln!(out, "{indent}{dst} = {src}.to_abi();");
        }
        (HybridType::Struct(s), AbiDirection::FromAbi) => {
            writeln!(out, "{indent}{dst} = {s}::from_abi({src});");
        }
        (HybridType::FixedArray { elem, len }, _) => {
            let i = format!("_i{depth}");
            writeln!(out, "{indent}for (size_t {i} = 0; {i} < {len}u; ++{i}) {{");
            let (dst, src) = (format!("{dst}[{i}]"), format!("{src}[{i}]"));
            emit_cpp_abi_copy(out, &format!("{indent}  "), &dst, &src, elem, dir, depth + 1);
            writeln!(out, "{indent}}}");
        }
        _ => writeln!(out, "{indent}{dst} = {src};"),
    }
}

pub(super) fn c_abi_field_decl(ty: &HybridType, fname: &str) -> String {
    match ty {
        // `[[T; M]; N]` is `T name[N][M]`: dimensions outermost first.
        HybridType::FixedArray { .. } => {
            let mut dims = String::new();
            let mut elem = ty;
            while let HybridType::FixedArray { elem: inner, len } = elem {
                dims.push_str(&format!("[{len}]"));
                elem = inner;
            }
            format!("{} {fname}{dims}", c_pod_type(elem))
        }
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
            emit_cpp_fill_nve_value(
                out,
                &format!("{indent}  "),
                dst,
                &format!("(*{expr})"),
                inner,
                idx,
            );
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
            writeln!(
                out,
                "{indent}{dst}.data.i64_val = static_cast<int64_t>({expr});"
            );
        }
        HybridType::Enum(_) => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_I64;");
            writeln!(
                out,
                "{indent}{dst}.data.i64_val = static_cast<int64_t>(static_cast<uint32_t>({expr}));"
            );
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_F64;");
            writeln!(
                out,
                "{indent}{dst}.data.f64_val = static_cast<double>({expr});"
            );
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
        HybridType::Value => {
            writeln!(out, "{indent}{dst} = _prop_tree_{idx}.root();");
        }
        // An absent non-direct option is `Null` itself, never an encoded null inside `Bytes`.
        HybridType::Optional(_) => {
            writeln!(out, "{indent}if ({expr}.has_value()) {{");
            writeln!(out, "{indent}  {dst}.tag = NVE_VAL_BYTES;");
            writeln!(
                out,
                "{indent}  {dst}.data.bytes_val = ::nve::hybrid::to_slice(_prop_bin_{idx});"
            );
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  {dst}.tag = NVE_VAL_NULL;");
            writeln!(out, "{indent}}}");
        }
        _ => {
            writeln!(out, "{indent}{dst}.tag = NVE_VAL_BYTES;");
            writeln!(
                out,
                "{indent}{dst}.data.bytes_val = ::nve::hybrid::to_slice(_prop_bin_{idx});"
            );
        }
    }
}

pub(super) fn cpp_prop_default_initializer(
    ty: &HybridType,
    def: Option<&HybridDefaultValue>,
) -> String {
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
