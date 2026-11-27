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

/// The decode local of struct field `name`: `_f_` + the unescaped Kotlin name.
pub(super) fn field_local(name: &str) -> String {
    format!("_f_{}", kotlin_ident(name).trim_matches('`'))
}

/// Rejects types the Kotlin / JNI target cannot carry: raw pointers, handles, and fixed arrays
/// have no `NveBinValue` codec or JNI mapping, and an empty codec arm would corrupt map framing.
pub(super) fn check_kotlin_supported(spec: &HybridFileSpec) -> Result<(), CodegenError> {
    fn check(ty: &HybridType, at: &str) -> Result<(), CodegenError> {
        match ty {
            HybridType::ConstPtr(_)
            | HybridType::MutPtr(_)
            | HybridType::Handle(_)
            | HybridType::FixedArray { .. } => Err(CodegenError::validation(format!(
                "hybrid Kotlin target: `{at}` has type `{}`, which Kotlin / JNI cannot carry",
                ty.canonical_signature()
            ))),
            HybridType::Optional(inner) | HybridType::Vec(inner) => check(inner, at),
            HybridType::Result { ok, .. } => check(ok, at),
            _ => Ok(()),
        }
    }
    let check_methods = |owner: &str, methods: &[HybridMethodSpec]| {
        methods.iter().try_for_each(|m| {
            let at = format!("{owner}::{}", m.name);
            m.params.iter().try_for_each(|p| check(&p.ty, &at))?;
            check(&m.return_type, &at)
        })
    };
    for s in &spec.structs {
        for f in &s.fields {
            check(&f.ty, &format!("{}.{}", s.name, f.name))?;
        }
    }
    for m in &spec.modules {
        check_methods(&m.name, &m.methods)?;
    }
    for v in &spec.views {
        for p in &v.props {
            check(&p.ty, &format!("{}.{}", v.name, p.name))?;
        }
        for e in &v.events {
            for p in &e.params {
                check(&p.ty, &format!("{}::{}", v.name, e.name))?;
            }
        }
        check_methods(&v.name, &v.methods)?;
    }
    Ok(())
}

/// A `const val` initializer of the `u32` bits as a Kotlin `Int` (a hex literal above
/// `Int.MAX_VALUE`, or `.toInt()` of one, is not a constant).
pub(super) fn kotlin_int_literal(bits: u32) -> String {
    match bits.cast_signed() {
        i32::MIN => "Int.MIN_VALUE".to_string(),
        v => v.to_string(),
    }
}

/// A `const val` initializer of the `u64` bits as a Kotlin `Long`.
pub(super) fn kotlin_long_literal(bits: u64) -> String {
    match bits.cast_signed() {
        i64::MIN => "Long.MIN_VALUE".to_string(),
        v => format!("{v}L"),
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
        HybridType::Value => "DynamicValue".to_string(),
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
            // Integers keep their bits: `u32` / `u64` above the signed range and `i64::MIN` are
            // not Kotlin literals, so they go through the signed `const` spellings.
            (HybridDefaultValue::Int(i), HybridType::I64 | HybridType::U64 | HybridType::Usize) => {
                kotlin_long_literal(i.cast_unsigned())
            }
            (HybridDefaultValue::Int(i), HybridType::F32) => format!("{i}.0f"),
            (HybridDefaultValue::Int(i), HybridType::F64) => format!("{i}.0"),
            (HybridDefaultValue::Int(i), _) => kotlin_int_literal(*i as u32),
            (HybridDefaultValue::UInt(u), HybridType::I64 | HybridType::U64 | HybridType::Usize) => {
                kotlin_long_literal(*u)
            }
            (HybridDefaultValue::UInt(u), HybridType::F32) => format!("{u}.0f"),
            (HybridDefaultValue::UInt(u), HybridType::F64) => format!("{u}.0"),
            (HybridDefaultValue::UInt(u), _) => kotlin_int_literal(*u as u32),
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
        HybridType::Value => "DynamicValue.NULL".to_string(),
        HybridType::Void | HybridType::Result { .. } => "Unit".to_string(),
    }
}

/// Writes `expr` (of type `ty`) to `dev.nve.host.BinWriter` `w`. Locals carry the nesting `depth`,
/// so nested sequences never shadow each other.
pub(super) fn emit_kotlin_bin_write(
    out: &mut CodeWriter,
    indent: &str,
    w: &str,
    expr: &str,
    ty: &HybridType,
    depth: usize,
) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr})"),
        HybridType::U8 | HybridType::U16 | HybridType::I32 => {
            writeln!(out, "{indent}{w}.writeI64({expr}.toLong())")
        }
        // `u32` bits live in an `Int`: zero-extend, like Rust and C++ (`0xFFFFFFFF`, never `-1`).
        HybridType::U32 => writeln!(out, "{indent}{w}.writeI64({expr}.toLong() and 0xFFFFFFFFL)"),
        HybridType::Usize | HybridType::I64 | HybridType::U64 => {
            writeln!(out, "{indent}{w}.writeI64({expr})")
        }
        HybridType::F32 => writeln!(out, "{indent}{w}.writeF64({expr}.toDouble())"),
        HybridType::F64 => writeln!(out, "{indent}{w}.writeF64({expr})"),
        HybridType::Enum(_) => {
            writeln!(out, "{indent}{w}.writeI64({expr}.rawValue.toLong() and 0xFFFFFFFFL)")
        }
        HybridType::String => writeln!(out, "{indent}{w}.writeStr({expr})"),
        HybridType::Bytes => writeln!(out, "{indent}{w}.writeBytes({expr})"),
        HybridType::Struct(_) => writeln!(out, "{indent}{expr}.encodeBin({w})"),
        HybridType::Value => writeln!(out, "{indent}{expr}.writeTo({w})"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({expr} != null) {{");
            emit_kotlin_bin_write(out, &format!("{indent}    "), w, expr, inner, depth + 1);
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}    {w}.writeNull()");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}run {{");
            writeln!(out, "{indent}    val _a{depth} = {w}.beginArray({expr}.size)");
            writeln!(out, "{indent}    for (_e{depth} in {expr}) {{");
            emit_kotlin_bin_write(
                out,
                &format!("{indent}        "),
                w,
                &format!("_e{depth}"),
                inner,
                depth + 1,
            );
            writeln!(out, "{indent}    }}");
            writeln!(out, "{indent}    {w}.end(_a{depth})");
            writeln!(out, "{indent}}}");
        }
        // Never values (`Void`, `Result` is unwrapped by callers) or rejected up front by
        // `check_kotlin_supported`.
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {}
    }
}

/// Reads a `ty` from `dev.nve.host.BinReader` `r` into `dst` (a declared `val` or `var`).
pub(super) fn emit_kotlin_bin_read(
    out: &mut CodeWriter,
    indent: &str,
    r: &str,
    dst: &str,
    ty: &HybridType,
    depth: usize,
) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.readBool()"),
        HybridType::U8 | HybridType::U16 | HybridType::I32 | HybridType::U32 => {
            writeln!(out, "{indent}{dst} = {r}.readI64().toInt()")
        }
        HybridType::Usize | HybridType::I64 | HybridType::U64 => {
            writeln!(out, "{indent}{dst} = {r}.readI64()")
        }
        HybridType::F32 => writeln!(out, "{indent}{dst} = {r}.readF64().toFloat()"),
        HybridType::F64 => writeln!(out, "{indent}{dst} = {r}.readF64()"),
        HybridType::Enum(e) => writeln!(out, "{indent}{dst} = {e}.fromRaw({r}.readI64().toInt())"),
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.readStr()"),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = {r}.readBytes()"),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = {s}.decodeBin({r})"),
        HybridType::Value => writeln!(out, "{indent}{dst} = DynamicValue.read({r})"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}{dst} = if ({r}.readNull()) null else {{");
            writeln!(out, "{indent}    val _ov{depth}: {}", kotlin_type(inner));
            emit_kotlin_bin_read(
                out,
                &format!("{indent}    "),
                r,
                &format!("_ov{depth}"),
                inner,
                depth + 1,
            );
            writeln!(out, "{indent}    _ov{depth}");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            // The reserve is capped (C++ `MAX_DECODE_RESERVE`): a crafted count cannot force a huge
            // allocation before the payload proves its entries exist.
            let elem = kotlin_type(inner);
            writeln!(out, "{indent}run {{");
            writeln!(out, "{indent}    val _a{depth} = {r}.beginArray()");
            writeln!(
                out,
                "{indent}    val _list{depth} = ArrayList<{elem}>(minOf(_a{depth}.count, 1024))"
            );
            writeln!(out, "{indent}    repeat(_a{depth}.count) {{");
            writeln!(out, "{indent}        val _el{depth}: {elem}");
            emit_kotlin_bin_read(
                out,
                &format!("{indent}        "),
                r,
                &format!("_el{depth}"),
                inner,
                depth + 1,
            );
            writeln!(out, "{indent}        _list{depth}.add(_el{depth})");
            writeln!(out, "{indent}    }}");
            writeln!(out, "{indent}    {r}.end(_a{depth})");
            writeln!(out, "{indent}    {dst} = _list{depth}");
            writeln!(out, "{indent}}}");
        }
        // Never values, or rejected up front by `check_kotlin_supported`.
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {}
    }
}
