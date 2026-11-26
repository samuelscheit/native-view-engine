use crate::writer::CodeWriter;

use super::*;

pub(super) fn rust_owned_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "()".to_string(),
        HybridType::Bool => "bool".to_string(),
        HybridType::U8 => "u8".to_string(),
        HybridType::U16 => "u16".to_string(),
        HybridType::Usize => "usize".to_string(),
        HybridType::I32 => "i32".to_string(),
        HybridType::U32 => "u32".to_string(),
        HybridType::I64 => "i64".to_string(),
        HybridType::U64 => "u64".to_string(),
        HybridType::F32 => "f32".to_string(),
        HybridType::F64 => "f64".to_string(),
        HybridType::String => "String".to_string(),
        HybridType::Bytes => "Vec<u8>".to_string(),
        HybridType::Optional(inner) => format!("Option<{}>", rust_owned_type(inner)),
        HybridType::Vec(inner) => format!("Vec<{}>", rust_owned_type(inner)),
        HybridType::Result { ok, err } => {
            format!("Result<{}, {}>", rust_owned_type(ok), rust_owned_type(err))
        }
        HybridType::Struct(name) | HybridType::Enum(name) | HybridType::Handle(name) => {
            name.clone()
        }
        HybridType::Value => "NveHybridValue".to_string(),
        HybridType::ConstPtr(inner) => format!(
            "*const {}",
            if matches!(inner.as_ref(), HybridType::Void) {
                "std::ffi::c_void".to_string()
            } else {
                rust_owned_type(inner)
            }
        ),
        HybridType::MutPtr(inner) => format!(
            "*mut {}",
            if matches!(inner.as_ref(), HybridType::Void) {
                "std::ffi::c_void".to_string()
            } else {
                rust_owned_type(inner)
            }
        ),
        HybridType::FixedArray { elem, len } => format!("[{}; {len}]", rust_owned_type(elem)),
    }
}

pub(super) fn rust_param_type(ty: &HybridType) -> String {
    match ty {
        HybridType::String => "&str".to_string(),
        HybridType::Bytes => "&[u8]".to_string(),
        other => rust_owned_type(other),
    }
}

pub(super) fn rust_return_type(ty: &HybridType) -> String {
    rust_owned_type(ty)
}

pub(super) fn rust_c_abi_pod_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Bool => "bool".to_string(),
        HybridType::U8 => "u8".to_string(),
        HybridType::U16 => "u16".to_string(),
        HybridType::Usize => "usize".to_string(),
        HybridType::I32 => "i32".to_string(),
        HybridType::U32 => "u32".to_string(),
        HybridType::I64 => "i64".to_string(),
        HybridType::U64 => "u64".to_string(),
        HybridType::F32 => "f32".to_string(),
        HybridType::F64 => "f64".to_string(),
        HybridType::Enum(_) => "u32".to_string(),
        HybridType::Struct(name) => name.clone(),
        HybridType::FixedArray { elem, len } => format!("[{}; {len}]", rust_c_abi_pod_type(elem)),
        _ => unreachable!("non-POD type passed to rust_c_abi_pod_type"),
    }
}

pub(super) fn rust_c_abi_param_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
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
        | HybridType::Enum(_) => rust_c_abi_pod_type(ty),
        HybridType::Struct(_) if ty.is_pod(spec) => rust_c_abi_pod_type(ty),
        HybridType::String => "NveStrSlice".to_string(),
        HybridType::Bytes => "NveBufferSlice".to_string(),
        HybridType::Value => "NveValue".to_string(),
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            format!("NveHybridOpt{}", inner.pod_opt_suffix().expect("validated POD optional has suffix"))
        }
        _ => "NveBufferSlice".to_string(),
    }
}

pub(super) fn rust_c_abi_return_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Void => "()".to_string(),
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
        | HybridType::Enum(_) => rust_c_abi_pod_type(ty),
        HybridType::Struct(_) if ty.is_pod(spec) => rust_c_abi_pod_type(ty),
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            format!("NveHybridOpt{}", inner.pod_opt_suffix().expect("validated POD optional has suffix"))
        }
        _ => "NveHybridOwnedBytes".to_string(),
    }
}

pub(super) fn rust_c_abi_zero_value(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Bool => "false".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::Enum(_) => "0".to_string(),
        HybridType::F32 | HybridType::F64 => "0.0".to_string(),
        HybridType::Struct(name) if ty.is_pod(spec) => format!("{name}::default()"),
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            let suffix = inner.pod_opt_suffix().expect("validated POD optional has suffix");
            let zero_inner = rust_c_abi_zero_value(inner, spec);
            format!("NveHybridOpt{suffix} {{ is_some: false, value: {zero_inner} }}")
        }
        _ => "NveHybridOwnedBytes::EMPTY".to_string(),
    }
}

pub(super) fn emit_bin_write_expr(out: &mut CodeWriter, indent: &str, buf: &str, expr: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}nve_bin_codec::write_bool({buf}, *{expr});"),
        HybridType::U8 | HybridType::U16 | HybridType::Usize | HybridType::I32 | HybridType::U32 | HybridType::U64 => {
            writeln!(out, "{indent}nve_bin_codec::write_i64({buf}, (*{expr}) as i64);")
        }
        HybridType::I64 => writeln!(out, "{indent}nve_bin_codec::write_i64({buf}, *{expr});"),
        HybridType::F32 => writeln!(out, "{indent}nve_bin_codec::write_f64({buf}, (*{expr}) as f64);"),
        HybridType::F64 => writeln!(out, "{indent}nve_bin_codec::write_f64({buf}, *{expr});"),
        HybridType::Enum(_) => writeln!(out, "{indent}nve_bin_codec::write_i64({buf}, ({expr}).to_u32() as i64);"),
        HybridType::String => writeln!(out, "{indent}nve_bin_codec::write_str({buf}, ({expr}).as_str());"),
        HybridType::Bytes => writeln!(out, "{indent}nve_bin_codec::write_bytes({buf}, ({expr}).as_slice());"),
        HybridType::Struct(_) => writeln!(out, "{indent}({expr}).encode_bin({buf});"),
        HybridType::Value => writeln!(out, "{indent}({buf}).extend_from_slice(&({expr}).bin_bytes);"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}match {expr} {{");
            writeln!(out, "{indent}    Some(_opt_v) => {{");
            emit_bin_write_expr(out, &format!("{indent}        "), buf, "_opt_v", inner);
            writeln!(out, "{indent}    }}");
            writeln!(out, "{indent}    None => nve_bin_codec::write_null({buf}),");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}nve_bin_codec::write_array({buf}, ({expr}).len(), |out| {{");
            writeln!(out, "{indent}    for _elem in ({expr}).iter() {{");
            emit_bin_write_expr(out, &format!("{indent}        "), "out", "_elem", inner);
            writeln!(out, "{indent}    }}");
            writeln!(out, "{indent}}});");
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {
            writeln!(out, "{indent}nve_bin_codec::write_null({buf});");
        }
    }
}

pub(super) fn bin_view_decode_expr(var: &str, ty: &HybridType) -> String {
    match ty {
        HybridType::Bool => format!("{var}.as_bool().ok_or(DecodeError::Truncated)?"),
        HybridType::U8 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as u8"),
        HybridType::U16 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as u16"),
        HybridType::Usize => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as usize"),
        HybridType::I32 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as i32"),
        HybridType::U32 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as u32"),
        HybridType::I64 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)?"),
        HybridType::U64 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as u64"),
        HybridType::F32 => format!("{var}.as_number().ok_or(DecodeError::Truncated)? as f32"),
        HybridType::F64 => format!("{var}.as_number().ok_or(DecodeError::Truncated)?"),
        HybridType::String => format!("{var}.as_str().ok_or(DecodeError::Truncated)?.to_string()"),
        HybridType::Bytes => format!("{var}.as_bytes().ok_or(DecodeError::Truncated)?.to_vec()"),
        HybridType::Enum(name) => format!(
            "{{ let _raw = {var}.as_i64().ok_or(DecodeError::Truncated)? as u32; {name}::from_u32(_raw).ok_or(DecodeError::InvalidEnum(\"{name}\", _raw))? }}"
        ),
        HybridType::Struct(name) => format!("{name}::decode_bin({var})?"),
        HybridType::Value => format!("NveHybridValue {{ bin_bytes: {var}.as_raw_bytes().to_vec() }}"),
        HybridType::Optional(inner) => {
            let inner_expr = bin_view_decode_expr(var, inner);
            format!("if {var}.is_null() {{ None }} else {{ Some({inner_expr}) }}")
        }
        HybridType::Vec(inner) => {
            let elem_expr = bin_view_decode_expr("_item", inner);
            format!(
                "{{ let mut _vec = Vec::new(); for _item in {var}.as_array().ok_or(DecodeError::Truncated)? {{ _vec.push({elem_expr}); }} _vec }}"
            )
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => "()".to_string(),
    }
}

pub(super) fn value_ref_decode_expr(var: &str, ty: &HybridType) -> String {
    match ty {
        HybridType::Bool => format!("{var}.as_bool().ok_or(DecodeError::Truncated)?"),
        HybridType::U8 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as u8"),
        HybridType::U16 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as u16"),
        HybridType::Usize => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as usize"),
        HybridType::I32 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as i32"),
        HybridType::U32 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as u32"),
        HybridType::I64 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)?"),
        HybridType::U64 => format!("{var}.as_i64().ok_or(DecodeError::Truncated)? as u64"),
        HybridType::F32 => format!("{var}.as_number().ok_or(DecodeError::Truncated)? as f32"),
        HybridType::F64 => format!("{var}.as_number().ok_or(DecodeError::Truncated)?"),
        HybridType::String => format!("{var}.as_str().ok_or(DecodeError::Truncated)?.to_string()"),
        HybridType::Bytes => format!("{var}.as_bytes().ok_or(DecodeError::Truncated)?.to_vec()"),
        HybridType::Enum(name) => format!(
            "{{ let _raw = {var}.as_i64().ok_or(DecodeError::Truncated)? as u32; {name}::from_u32(_raw).ok_or(DecodeError::InvalidEnum(\"{name}\", _raw))? }}"
        ),
        HybridType::Struct(name) => format!(
            "if let Some(_raw) = {var}.as_bytes() {{ {name}::decode_bin(nve_bin_codec::parse_view(_raw)?)? }} else {{ {name}::decode_value_ref({var})? }}"
        ),
        HybridType::Value => format!("NveHybridValue::from_ffi_ref({var})?"),
        HybridType::Optional(inner) => {
            let inner_expr = value_ref_decode_expr(var, inner);
            format!("if {var}.is_null() {{ None }} else {{ Some({inner_expr}) }}")
        }
        HybridType::Vec(inner) => {
            let elem_ref_expr = value_ref_decode_expr("_item", inner);
            let elem_bin_expr = bin_view_decode_expr("_item", inner);
            format!(
                "if let Some(_raw) = {var}.as_bytes() {{ let _bv = nve_bin_codec::parse_view(_raw)?; let mut _vec = Vec::new(); for _item in _bv.as_array().ok_or(DecodeError::Truncated)? {{ _vec.push({elem_bin_expr}); }} _vec }} else {{ let mut _vec = Vec::new(); for _item in {var}.as_array().ok_or(DecodeError::Truncated)?.iter() {{ _vec.push({elem_ref_expr}); }} _vec }}"
            )
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => "()".to_string(),
    }
}

pub(super) fn is_direct_nve_value_scalar_or_slice(ty: &HybridType) -> bool {
    matches!(
        ty,
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
            | HybridType::Bytes
    )
}

pub(super) fn direct_nve_value_expr(expr: &str, ty: &HybridType, idx: usize) -> String {
    match ty {
        HybridType::Bool => format!("NveValue::bool({expr})"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => {
            format!("NveValue::i64({expr} as i64)")
        }
        HybridType::F32 | HybridType::F64 => format!("NveValue::f64({expr} as f64)"),
        HybridType::Enum(_) => format!("NveValue::i64({expr}.to_u32() as i64)"),
        HybridType::String => format!("NveValue::from_str({expr}.as_str())"),
        HybridType::Bytes => format!("NveValue::from_bytes({expr}.as_slice())"),
        _ => format!("NveValue::from_bytes(_bin_{idx}.as_slice())"),
    }
}

pub(super) fn rust_prop_default_expr(ty: &HybridType, def: Option<&HybridDefaultValue>) -> String {
    let Some(def) = def else {
        return "Default::default()".to_string();
    };
    if let HybridType::Optional(inner) = ty {
        return match def {
            HybridDefaultValue::None => "None".to_string(),
            other => format!("Some({})", rust_prop_default_expr(inner, Some(other))),
        };
    }
    match (def, ty) {
        (HybridDefaultValue::Bool(b), _) => b.to_string(),
        (HybridDefaultValue::Int(i), HybridType::F32 | HybridType::F64) => format!("{i}.0"),
        (HybridDefaultValue::Int(i), _) => i.to_string(),
        (HybridDefaultValue::UInt(u), HybridType::F32 | HybridType::F64) => format!("{u}.0"),
        (HybridDefaultValue::UInt(u), _) => u.to_string(),
        (HybridDefaultValue::Float(f), _) => format_float_literal(*f),
        (HybridDefaultValue::String(s), _) => {
            format!("\"{}\".to_string()", escape_c_like_string(s))
        }
        (HybridDefaultValue::EnumVariant { enum_name, variant }, _) => {
            format!("{enum_name}::{variant}")
        }
        (HybridDefaultValue::None, _) => "Default::default()".to_string(),
    }
}
