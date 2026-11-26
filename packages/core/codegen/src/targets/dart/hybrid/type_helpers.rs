use crate::writer::CodeWriter;

use super::*;

pub(super) fn dart_method_sig(m: &HybridMethodSpec) -> String {
    let mname = dart_ident(&to_camel_case(&m.name));
    let ret = match &m.return_type {
        HybridType::Result { ok, .. } => dart_type(ok),
        other => dart_type(other),
    };
    let params: Vec<String> = m
        .params
        .iter()
        .map(|p| format!("{} {}", dart_type(&p.ty), dart_ident(&p.name)))
        .collect();
    format!("{ret} {mname}({})", params.join(", "))
}

pub(super) fn dart_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "void".to_string(),
        HybridType::Bool => "bool".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => "int".to_string(),
        HybridType::F32 | HybridType::F64 => "double".to_string(),
        HybridType::String => "String".to_string(),
        HybridType::Bytes => "Uint8List".to_string(),
        HybridType::Optional(inner) => format!("{}?", dart_type(inner)),
        HybridType::Vec(inner) | HybridType::FixedArray { elem: inner, .. } => {
            format!("List<{}>", dart_type(inner))
        }
        HybridType::Result { ok, .. } => dart_type(ok),
        HybridType::Struct(name) | HybridType::Enum(name) => name.clone(),
        HybridType::Value => "NveHybridDynamicValue".to_string(),
        HybridType::ConstPtr(_) | HybridType::MutPtr(_) | HybridType::Handle(_) => {
            "ffi.Pointer<ffi.Void>".to_string()
        }
    }
}

pub(super) fn dart_type_non_nullable(ty: &HybridType) -> String {
    match ty {
        HybridType::Optional(inner) => dart_type(inner),
        other => dart_type(other),
    }
}

pub(super) fn dart_ffi_field_annotation(ty: &HybridType) -> Option<&'static str> {
    match ty {
        HybridType::Bool => Some("@ffi.Bool()"),
        HybridType::U8 => Some("@ffi.Uint8()"),
        HybridType::U16 => Some("@ffi.Uint16()"),
        HybridType::Usize => Some("@ffi.Size()"),
        HybridType::I32 => Some("@ffi.Int32()"),
        HybridType::U32 | HybridType::Enum(_) => Some("@ffi.Uint32()"),
        HybridType::I64 => Some("@ffi.Int64()"),
        HybridType::U64 => Some("@ffi.Uint64()"),
        HybridType::F32 => Some("@ffi.Float()"),
        HybridType::F64 => Some("@ffi.Double()"),
        _ => None,
    }
}

pub(super) fn dart_ffi_struct_field_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Bool => "bool".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::Enum(_) => "int".to_string(),
        HybridType::F32 | HybridType::F64 => "double".to_string(),
        HybridType::Struct(name) => format!("NveHybrid{name}"),
        _ => unreachable!("non-POD field in dart_ffi_struct_field_type"),
    }
}

pub(super) fn dart_native_value_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Void => "ffi.Void".to_string(),
        HybridType::Bool => "ffi.Bool".to_string(),
        HybridType::U8 => "ffi.Uint8".to_string(),
        HybridType::U16 => "ffi.Uint16".to_string(),
        HybridType::Usize => "ffi.Size".to_string(),
        HybridType::I32 => "ffi.Int32".to_string(),
        HybridType::U32 | HybridType::Enum(_) => "ffi.Uint32".to_string(),
        HybridType::I64 => "ffi.Int64".to_string(),
        HybridType::U64 => "ffi.Uint64".to_string(),
        HybridType::F32 => "ffi.Float".to_string(),
        HybridType::F64 => "ffi.Double".to_string(),
        HybridType::Struct(name) if ty.is_pod(spec) => format!("NveHybrid{name}"),
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            format!("NveHybridOpt{}", inner.pod_opt_suffix().expect("validated POD optional has suffix"))
        }
        _ => "NveHybridOwnedBytes".to_string(),
    }
}

pub(super) fn dart_native_param_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::String => "NveStrSlice".to_string(),
        HybridType::Bytes => "NveBufferSlice".to_string(),
        HybridType::Value => "NveValue".to_string(),
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
        | HybridType::Enum(_) => dart_native_value_type(ty, spec),
        HybridType::Struct(_) if ty.is_pod(spec) => dart_native_value_type(ty, spec),
        HybridType::Optional(inner) if inner.is_pod(spec) => dart_native_value_type(ty, spec),
        _ => "NveBufferSlice".to_string(),
    }
}

pub(super) fn dart_ffi_call_param_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Bool => "bool".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::Enum(_) => "int".to_string(),
        HybridType::F32 | HybridType::F64 => "double".to_string(),
        other => dart_native_param_type(other, spec),
    }
}

pub(super) fn dart_native_return_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Result { .. } => "ffi.Int32".to_string(),
        other => dart_native_value_type(other, spec),
    }
}

pub(super) fn dart_ffi_call_return_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Result { .. } => "int".to_string(),
        HybridType::Void => "void".to_string(),
        HybridType::Bool => "bool".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::Enum(_) => "int".to_string(),
        HybridType::F32 | HybridType::F64 => "double".to_string(),
        other => dart_native_value_type(other, spec),
    }
}

pub(super) fn dart_u64_const(v: u64) -> String {
    // Dart signed 64-bit int literal syntax: represent values >= 2^63 cleanly using signed two's complement.
    let signed = v as i64;
    if signed < 0 {
        format!("{signed} /* 0x{v:016X} */")
    } else {
        format!("0x{v:016X}")
    }
}

pub(super) fn dart_is_const_expr(ty: &HybridType, def: Option<&HybridDefaultValue>) -> bool {
    if matches!(ty, HybridType::Optional(_)) {
        return match def {
            None | Some(HybridDefaultValue::None) => true,
            Some(_) => matches!(
                ty,
                HybridType::Optional(inner) if dart_is_const_expr(inner, def)
            ),
        };
    }
    matches!(
        ty,
        HybridType::Bool
            | HybridType::I32
            | HybridType::U32
            | HybridType::I64
            | HybridType::U64
            | HybridType::F32
            | HybridType::F64
            | HybridType::String
            | HybridType::Enum(_)
    )
}

pub(super) fn dart_default_value(
    ty: &HybridType,
    def: Option<&HybridDefaultValue>,
    spec: &HybridFileSpec,
) -> String {
    if let Some(d) = def {
        if let HybridType::Optional(inner) = ty {
            return match d {
                HybridDefaultValue::None => "null".to_string(),
                other => dart_default_value(inner, Some(other), spec),
            };
        }
        return match (d, ty) {
            (HybridDefaultValue::Bool(b), _) => b.to_string(),
            (HybridDefaultValue::Int(i), HybridType::F32 | HybridType::F64) => format!("{i}.0"),
            (HybridDefaultValue::Int(i), _) => i.to_string(),
            (HybridDefaultValue::UInt(u), HybridType::F32 | HybridType::F64) => format!("{u}.0"),
            (HybridDefaultValue::UInt(u), _) => u.to_string(),
            (HybridDefaultValue::Float(f), _) => format_float_literal(*f),
            (HybridDefaultValue::String(s), _) => format!("'{}'", escape_dart_single_quoted_string(s)),
            (HybridDefaultValue::EnumVariant { enum_name, variant }, _) => {
                format!("{enum_name}.{}", dart_ident(variant))
            }
            (HybridDefaultValue::None, _) => "null".to_string(),
        };
    }

    match ty {
        HybridType::Bool => "false".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => "0".to_string(),
        HybridType::F32 | HybridType::F64 => "0.0".to_string(),
        HybridType::String => "''".to_string(),
        HybridType::Bytes => "Uint8List(0)".to_string(),
        HybridType::Optional(_) => "null".to_string(),
        HybridType::Vec(inner) | HybridType::FixedArray { elem: inner, .. } => {
            format!("<{}>[]", dart_type(inner))
        }
        HybridType::Struct(name) => {
            if let Some(s) = spec.find_struct(name) {
                let args: Vec<String> = s
                    .fields
                    .iter()
                    .map(|f| {
                        format!(
                            "{}: {}",
                            dart_ident(&f.name),
                            dart_default_value(&f.ty, None, spec)
                        )
                    })
                    .collect();
                format!("{name}({})", args.join(", "))
            } else {
                format!("{name}()")
            }
        }
        HybridType::Enum(name) => {
            if let Some(e) = spec.find_enum(name) {
                format!("{name}.{}", dart_ident(&e.variants[0].name))
            } else {
                format!("{name}.fromRaw(0)")
            }
        }
        HybridType::Value => "NveHybridDynamicValue()".to_string(),
        HybridType::ConstPtr(_) | HybridType::MutPtr(_) | HybridType::Handle(_) => {
            "ffi.nullptr".to_string()
        }
        HybridType::Void | HybridType::Result { .. } => "null".to_string(),
    }
}

pub(super) fn emit_dart_bin_write(out: &mut CodeWriter, indent: &str, w: &str, expr: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr});"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => writeln!(out, "{indent}{w}.writeInt64({expr});"),
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{w}.writeDouble({expr});")
        }
        HybridType::Enum(_) => writeln!(out, "{indent}{w}.writeInt64({expr}.rawValue);"),
        HybridType::String => writeln!(out, "{indent}{w}.writeString({expr});"),
        HybridType::Bytes => writeln!(out, "{indent}{w}.writeBytes({expr});"),
        HybridType::Struct(_) => writeln!(out, "{indent}{expr}.encodeBin({w});"),
        HybridType::Value => writeln!(out, "{indent}{w}.writeRaw({expr}.binBytes);"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}final _ov = {expr};");
            writeln!(out, "{indent}if (_ov != null) {{");
            emit_dart_bin_write(out, &format!("{indent}  "), w, "_ov", inner);
            writeln!(out, "{indent}}} else {{ {w}.writeNull(); }}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{w}.writeArray({expr}.length, (aw) {{");
            writeln!(out, "{indent}  for (final _el in {expr}) {{");
            emit_dart_bin_write(out, &format!("{indent}    "), "aw", "_el", inner);
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}}});");
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {}
    }
}

pub(super) fn emit_dart_bin_read(out: &mut CodeWriter, indent: &str, r: &str, dst: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.readBool();"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => writeln!(out, "{indent}{dst} = {r}.readInt64();"),
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{dst} = {r}.readDouble();")
        }
        HybridType::Enum(e) => {
            writeln!(out, "{indent}{dst} = {e}.fromRaw({r}.readInt64());")
        }
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.readString();"),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = {r}.readBytes();"),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = {s}.decodeBin({r});"),
        HybridType::Value => {
            writeln!(out, "{indent}{dst} = NveHybridDynamicValue({r}.readRawValue());")
        }
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({r}.readNullIfPresent()) {{");
            writeln!(out, "{indent}  {dst} = null;");
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  late final {} _ov;", dart_type(inner));
            emit_dart_bin_read(out, &format!("{indent}  "), r, "_ov", inner);
            writeln!(out, "{indent}  {dst} = _ov;");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  final _n = {r}.beginArray();");
            writeln!(out, "{indent}  final _list = <{}>[];", dart_type(inner));
            writeln!(out, "{indent}  for (var _i = 0; _i < _n; _i++) {{");
            writeln!(out, "{indent}    late final {} _el;", dart_type(inner));
            emit_dart_bin_read(out, &format!("{indent}    "), r, "_el", inner);
            writeln!(out, "{indent}    _list.add(_el);");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  {dst} = _list;");
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
