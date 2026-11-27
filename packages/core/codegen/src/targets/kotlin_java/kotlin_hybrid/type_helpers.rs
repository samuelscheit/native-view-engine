use crate::writer::CodeWriter;

use super::*;

pub(super) fn kotlin_method_sig(m: &HybridMethodSpec) -> String {
    let mname = kotlin_ident(&to_camel_case(&m.name));
    let params: Vec<String> = m
        .params
        .iter()
        .map(|p| format!("{}: {}", kotlin_ident(&p.name), kotlin_type(&p.ty)))
        .collect();
    let ret = match &m.return_type {
        HybridType::Result { ok, .. } => kotlin_type(ok),
        other => kotlin_type(other),
    };
    if ret == "Unit" {
        format!("fun {mname}({})", params.join(", "))
    } else {
        format!("fun {mname}({}): {ret}", params.join(", "))
    }
}

pub(super) fn kotlin_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "Unit".to_string(),
        HybridType::Bool => "Boolean".to_string(),
        HybridType::U8 | HybridType::U16 | HybridType::I32 | HybridType::U32 => "Int".to_string(),
        HybridType::Usize
        | HybridType::I64
        | HybridType::U64
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => "Long".to_string(),
        HybridType::F32 => "Float".to_string(),
        HybridType::F64 => "Double".to_string(),
        HybridType::String => "String".to_string(),
        HybridType::Bytes => "ByteArray".to_string(),
        HybridType::Optional(inner) => format!("{}?", kotlin_type(inner)),
        HybridType::Vec(inner) | HybridType::FixedArray { elem: inner, .. } => {
            format!("List<{}>", kotlin_type(inner))
        }
        HybridType::Result { ok, .. } => kotlin_type(ok),
        HybridType::Struct(name) | HybridType::Enum(name) => name.clone(),
        HybridType::Value => "NveHybridDynamicValue".to_string(),
    }
}

pub(super) fn kotlin_jni_param_type(ty: &HybridType) -> &'static str {
    match ty {
        HybridType::Bool => "Boolean",
        HybridType::U8
        | HybridType::U16
        | HybridType::I32
        | HybridType::U32
        | HybridType::Enum(_) => "Int",
        HybridType::Usize | HybridType::I64 | HybridType::U64 => "Long",
        HybridType::F32 => "Float",
        HybridType::F64 => "Double",
        HybridType::String => "String",
        _ => "ByteBuffer",
    }
}

pub(super) fn kotlin_jni_return_type(ty: &HybridType) -> &'static str {
    let eff = match ty {
        HybridType::Result { ok, .. } => ok.as_ref(),
        other => other,
    };
    match eff {
        HybridType::Void => "Unit",
        HybridType::Bool => "Boolean",
        HybridType::U8
        | HybridType::U16
        | HybridType::I32
        | HybridType::U32
        | HybridType::Enum(_) => "Int",
        HybridType::Usize | HybridType::I64 | HybridType::U64 => "Long",
        HybridType::F32 => "Float",
        HybridType::F64 => "Double",
        HybridType::String => "String",
        _ => "ByteArray",
    }
}

pub(super) fn jni_cpp_param_type(ty: &HybridType) -> &'static str {
    match ty {
        HybridType::Bool => "jboolean",
        HybridType::U8
        | HybridType::U16
        | HybridType::I32
        | HybridType::U32
        | HybridType::Enum(_) => "jint",
        HybridType::Usize | HybridType::I64 | HybridType::U64 => "jlong",
        HybridType::F32 => "jfloat",
        HybridType::F64 => "jdouble",
        HybridType::String => "jstring",
        _ => "jobject",
    }
}

pub(super) fn jni_cpp_return_type(ty: &HybridType) -> &'static str {
    let eff = match ty {
        HybridType::Result { ok, .. } => ok.as_ref(),
        other => other,
    };
    match eff {
        HybridType::Void => "void",
        HybridType::Bool => "jboolean",
        HybridType::U8
        | HybridType::U16
        | HybridType::I32
        | HybridType::U32
        | HybridType::Enum(_) => "jint",
        HybridType::Usize | HybridType::I64 | HybridType::U64 => "jlong",
        HybridType::F32 => "jfloat",
        HybridType::F64 => "jdouble",
        HybridType::String => "jstring",
        _ => "jbyteArray",
    }
}

pub(super) fn jni_cpp_zero_return(ty: &HybridType) -> &'static str {
    let eff = match ty {
        HybridType::Result { ok, .. } => ok.as_ref(),
        other => other,
    };
    match eff {
        HybridType::Void => "",
        HybridType::Bool => " JNI_FALSE",
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::Enum(_) => " 0",
        HybridType::F32 | HybridType::F64 => " 0.0",
        _ => " nullptr",
    }
}

pub(super) fn kotlin_default_value(
    ty: &HybridType,
    def: Option<&HybridDefaultValue>,
    spec: &HybridFileSpec,
) -> String {
    if let Some(d) = def {
        if let HybridType::Optional(inner) = ty {
            return match d {
                HybridDefaultValue::None => "null".to_string(),
                other => kotlin_default_value(inner, Some(other), spec),
            };
        }
        return match (d, ty) {
            (HybridDefaultValue::Bool(b), _) => b.to_string(),
            (HybridDefaultValue::Int(i), HybridType::I64 | HybridType::U64) => format!("{i}L"),
            (HybridDefaultValue::Int(i), HybridType::F32) => format!("{i}.0f"),
            (HybridDefaultValue::Int(i), HybridType::F64) => format!("{i}.0"),
            (HybridDefaultValue::Int(i), _) => i.to_string(),
            (HybridDefaultValue::UInt(u), HybridType::I64 | HybridType::U64) => format!("{u}L"),
            (HybridDefaultValue::UInt(u), HybridType::F32) => format!("{u}.0f"),
            (HybridDefaultValue::UInt(u), HybridType::F64) => format!("{u}.0"),
            (HybridDefaultValue::UInt(u), _) => u.to_string(),
            (HybridDefaultValue::Float(f), HybridType::F32) => {
                format!("{}f", format_float_literal(*f))
            }
            (HybridDefaultValue::Float(f), _) => format_float_literal(*f),
            (HybridDefaultValue::String(s), _) => format!("\"{}\"", escape_kotlin_string(s)),
            (HybridDefaultValue::EnumVariant { enum_name, variant }, _) => {
                format!("{enum_name}.{variant}")
            }
            (HybridDefaultValue::None, _) => "null".to_string(),
        };
    }

    match ty {
        HybridType::Bool => "false".to_string(),
        HybridType::U8 | HybridType::U16 | HybridType::I32 | HybridType::U32 => "0".to_string(),
        HybridType::Usize
        | HybridType::I64
        | HybridType::U64
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => "0L".to_string(),
        HybridType::F32 => "0.0f".to_string(),
        HybridType::F64 => "0.0".to_string(),
        HybridType::String => "\"\"".to_string(),
        HybridType::Bytes => "ByteArray(0)".to_string(),
        HybridType::Optional(_) => "null".to_string(),
        HybridType::Vec(_) | HybridType::FixedArray { .. } => "emptyList()".to_string(),
        HybridType::Struct(name) => format!("{name}()"),
        HybridType::Enum(name) => {
            if let Some(e) = spec.find_enum(name) {
                format!("{name}.{}", e.variants[0].name)
            } else {
                format!("{name}.fromRaw(0)")
            }
        }
        HybridType::Value => "NveHybridDynamicValue()".to_string(),
        HybridType::Void | HybridType::Result { .. } => "Unit".to_string(),
    }
}

pub(super) fn emit_kotlin_bin_write(
    out: &mut CodeWriter,
    indent: &str,
    w: &str,
    expr: &str,
    ty: &HybridType,
) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr})"),
        HybridType::U8 | HybridType::U16 | HybridType::I32 | HybridType::U32 => {
            writeln!(out, "{indent}{w}.writeLong({expr}.toLong())")
        }
        HybridType::Usize | HybridType::I64 | HybridType::U64 => {
            writeln!(out, "{indent}{w}.writeLong({expr})")
        }
        HybridType::F32 => writeln!(out, "{indent}{w}.writeDouble({expr}.toDouble())"),
        HybridType::F64 => writeln!(out, "{indent}{w}.writeDouble({expr})"),
        HybridType::Enum(_) => writeln!(out, "{indent}{w}.writeLong({expr}.rawValue.toLong())"),
        HybridType::String => writeln!(out, "{indent}{w}.writeString({expr})"),
        HybridType::Bytes => writeln!(out, "{indent}{w}.writeBytes({expr})"),
        HybridType::Struct(_) => writeln!(out, "{indent}{expr}.encodeBin({w})"),
        HybridType::Value => writeln!(out, "{indent}{w}.writeRaw({expr}.binBytes)"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({expr} != null) {{");
            emit_kotlin_bin_write(out, &format!("{indent}    "), w, expr, inner);
            writeln!(out, "{indent}}} else {{ {w}.writeNull() }}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{w}.writeArray({expr}.size) {{ aw ->");
            writeln!(out, "{indent}    for (_el in {expr}) {{");
            emit_kotlin_bin_write(out, &format!("{indent}        "), "aw", "_el", inner);
            writeln!(out, "{indent}    }}");
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

pub(super) fn emit_kotlin_bin_read(
    out: &mut CodeWriter,
    indent: &str,
    r: &str,
    dst: &str,
    ty: &HybridType,
) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.readBool()"),
        HybridType::U8 | HybridType::U16 | HybridType::I32 | HybridType::U32 => {
            writeln!(out, "{indent}{dst} = {r}.readLong().toInt()")
        }
        HybridType::Usize | HybridType::I64 | HybridType::U64 => {
            writeln!(out, "{indent}{dst} = {r}.readLong()")
        }
        HybridType::F32 => writeln!(out, "{indent}{dst} = {r}.readDouble().toFloat()"),
        HybridType::F64 => writeln!(out, "{indent}{dst} = {r}.readDouble()"),
        HybridType::Enum(e) => writeln!(out, "{indent}{dst} = {e}.fromRaw({r}.readLong().toInt())"),
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.readString()"),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = {r}.readBytes()"),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = {s}.decodeBin({r})"),
        HybridType::Value => writeln!(
            out,
            "{indent}{dst} = NveHybridDynamicValue({r}.readRawValue())"
        ),
        HybridType::Optional(inner) => {
            writeln!(
                out,
                "{indent}{dst} = if ({r}.readNullIfPresent()) null else {{"
            );
            writeln!(out, "{indent}    val _ov: {}", kotlin_type(inner));
            emit_kotlin_bin_read(out, &format!("{indent}    "), r, "_ov", inner);
            writeln!(out, "{indent}    _ov");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}run {{");
            writeln!(out, "{indent}    val _n = {r}.beginArray()");
            writeln!(
                out,
                "{indent}    val _list = ArrayList<{}>(_n)",
                kotlin_type(inner)
            );
            writeln!(out, "{indent}    repeat(_n) {{");
            writeln!(out, "{indent}        val _el: {}", kotlin_type(inner));
            emit_kotlin_bin_read(out, &format!("{indent}        "), r, "_el", inner);
            writeln!(out, "{indent}        _list.add(_el)");
            writeln!(out, "{indent}    }}");
            writeln!(out, "{indent}    {dst} = _list");
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
