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

/// The C / C++ spelling of a scalar (`bool`, `<stdint.h>` integers, `size_t`, `float`, `double`).
fn c_scalar_type(ty: &HybridType) -> Option<&'static str> {
    Some(match ty {
        HybridType::Bool => "bool",
        HybridType::U8 => "uint8_t",
        HybridType::U16 => "uint16_t",
        HybridType::Usize => "size_t",
        HybridType::I32 => "int32_t",
        HybridType::U32 => "uint32_t",
        HybridType::I64 => "int64_t",
        HybridType::U64 => "uint64_t",
        HybridType::F32 => "float",
        HybridType::F64 => "double",
        _ => return None,
    })
}

pub(crate) fn cpp_owned_type(ty: &HybridType) -> String {
    match ty {
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
        // Every scalar, and `void` (the one remaining type without a scalar spelling).
        scalar_or_void => c_scalar_type(scalar_or_void).unwrap_or("void").to_string(),
    }
}

pub(super) fn cpp_param_type(ty: &HybridType) -> String {
    match ty {
        HybridType::String => "std::string_view".to_string(),
        HybridType::Bytes => "std::span<const uint8_t>".to_string(),
        HybridType::Value => "const DynamicValue&".to_string(),
        HybridType::Struct(name) => format!("const {name}&"),
        HybridType::Optional(inner) => format!("const std::optional<{}>&", cpp_owned_type(inner)),
        HybridType::Vec(_) | HybridType::FixedArray { .. } => {
            format!("const {}&", cpp_owned_type(ty))
        }
        other => cpp_owned_type(other),
    }
}

/// The C spelling of a type with a C value form: scalars, enums (`uint32_t` typedefs), and POD
/// structs. `None` for every other type (`[T; N]` crosses as encoded bytes, never by value).
pub(super) fn c_value_type(ty: &HybridType, spec: &HybridFileSpec) -> Option<String> {
    match ty {
        HybridType::Enum(name) => Some(format!("NveHybrid{name}")),
        HybridType::Struct(name) if ty.is_pod(spec) => Some(format!("NveHybrid{name}")),
        scalar => c_scalar_type(scalar).map(str::to_owned),
    }
}

/// [`c_value_type`], or the error naming the type that has no C value form.
pub(super) fn require_c_value_type(
    ty: &HybridType,
    spec: &HybridFileSpec,
) -> Result<String, CodegenError> {
    c_value_type(ty, spec).ok_or_else(|| {
        CodegenError::validation(format!(
            "hybrid C ABI: `{}` has no C value form",
            ty.canonical_signature()
        ))
    })
}

/// The C pointee of a struct field `*const T` / `*mut T`: `void`, a POD struct by tag (complete
/// before or after this struct), or a scalar / enum. `None` for `[T; N]` and every other type, so
/// the struct stays C++-only.
fn c_field_pointee_type(inner: &HybridType, spec: &HybridFileSpec) -> Option<String> {
    match inner {
        HybridType::Void => Some("void".to_string()),
        HybridType::Struct(name) if inner.is_pod(spec) => Some(format!("struct NveHybrid{name}")),
        other => c_value_type(other, spec),
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
            let i = format!("_nve_i{depth}");
            writeln!(out, "{indent}for (size_t {i} = 0; {i} < {len}u; ++{i}) {{");
            let (dst, src) = (format!("{dst}[{i}]"), format!("{src}[{i}]"));
            emit_cpp_abi_copy(
                out,
                &format!("{indent}  "),
                &dst,
                &src,
                elem,
                dir,
                depth + 1,
            );
            writeln!(out, "{indent}}}");
        }
        _ => writeln!(out, "{indent}{dst} = {src};"),
    }
}

/// One C struct field declaration, `None` when `ty` has no C form (the struct then stays C++-only).
fn c_abi_field_decl(ty: &HybridType, fname: &str, spec: &HybridFileSpec) -> Option<String> {
    match ty {
        // `[[T; M]; N]` is `T name[N][M]`: dimensions outermost first.
        HybridType::FixedArray { .. } => {
            let (dims, elem) = ty.fixed_array_dims();
            let dims: String = dims.iter().map(|len| format!("[{len}]")).collect();
            Some(format!("{} {fname}{dims}", c_value_type(elem, spec)?))
        }
        HybridType::ConstPtr(inner) => {
            Some(format!("const {}* {fname}", c_field_pointee_type(inner, spec)?))
        }
        HybridType::MutPtr(inner) => Some(format!("{}* {fname}", c_field_pointee_type(inner, spec)?)),
        other => Some(format!("{} {fname}", c_value_type(other, spec)?)),
    }
}

/// The C field declarations of `s` when it is a C ABI struct (non-empty, not a union, and every
/// field has a C form), else `None`.
pub(super) fn c_abi_struct_fields(
    s: &HybridStructSpec,
    spec: &HybridFileSpec,
) -> Option<Vec<String>> {
    if s.is_union || s.fields.is_empty() {
        return None;
    }
    s.fields
        .iter()
        .map(|f| c_abi_field_decl(&f.ty, &c_field_ident(&f.name), spec))
        .collect()
}

/// A static C function's parameter / return type: values, `void`, and pointers to them.
pub(super) fn c_fn_type(ty: &HybridType, spec: &HybridFileSpec) -> Result<String, CodegenError> {
    let pointer = |inner: &HybridType, constness: &str| {
        let pointee = match inner {
            HybridType::Void => Some("void".to_string()),
            // Any struct: C functions also take opaque handles to non-POD ones.
            HybridType::Struct(name) => Some(format!("NveHybrid{name}")),
            other => c_value_type(other, spec),
        };
        pointee
            .map(|pointee| format!("{constness}{pointee}*"))
            .ok_or_else(|| {
                CodegenError::validation(format!(
                    "hybrid C function: pointer to `{}` has no C form",
                    inner.canonical_signature()
                ))
            })
    };
    match ty {
        HybridType::Void => Ok("void".to_string()),
        HybridType::ConstPtr(inner) => pointer(inner, "const "),
        HybridType::MutPtr(inner) => pointer(inner, ""),
        other => Ok(c_param_type(other, spec)),
    }
}

pub(super) fn c_param_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if let Some(suffix) = ty.optional_pod_suffix(spec) {
        return format!("NveHybridOpt{suffix}");
    }
    if let Some(value) = c_value_type(ty, spec) {
        return value;
    }
    match ty {
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
        other => c_value_type(other, spec).unwrap_or_else(|| "NveHybridOwnedBytes".to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::hybrid::HybridFieldSpec;

    /// Unvalidated specs reach the helpers in-crate: a pointer to `[T; N]` has no C form, so the
    /// struct stays C++-only and a C function signature fails instead of panicking.
    #[test]
    fn pointers_to_fixed_arrays_have_no_c_form() {
        let array_ptr = HybridType::ConstPtr(Box::new(HybridType::FixedArray {
            elem: Box::new(HybridType::U32),
            len: 4,
        }));
        let spec = HybridFileSpec {
            structs: vec![HybridStructSpec {
                name: "Holder".to_owned(),
                fields: vec![HybridFieldSpec {
                    name: "values".to_owned(),
                    ty: array_ptr.clone(),
                    doc: None,
                    offset: None,
                    offset_64: None,
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(c_abi_struct_fields(&spec.structs[0], &spec), None);
        assert!(c_fn_type(&array_ptr, &spec).is_err());
        assert!(require_c_value_type(&array_ptr, &spec).is_err());
        let u32_ptr = HybridType::MutPtr(Box::new(HybridType::U32));
        assert_eq!(c_fn_type(&u32_ptr, &spec).ok().as_deref(), Some("uint32_t*"));
    }
}
