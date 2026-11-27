use crate::writer::CodeWriter;

use super::*;

pub(super) fn spec_needs_dart_ffi(spec: &HybridFileSpec) -> bool {
    !spec.modules.is_empty()
        || !spec.views.is_empty()
        || !spec.collect_pod_option_types().is_empty()
        || spec.structs.iter().any(|s| s.is_pod(spec))
        || spec_any_type(spec, type_needs_dart_ffi)
}

pub(super) fn spec_needs_nve_host_ffi(spec: &HybridFileSpec) -> bool {
    !spec.modules.is_empty() || !spec.views.is_empty()
}

pub(super) fn spec_needs_typed_data(spec: &HybridFileSpec) -> bool {
    spec_any_type(spec, type_contains_bytes)
}

fn spec_any_type(spec: &HybridFileSpec, pred: fn(&HybridType) -> bool) -> bool {
    let any_method = |methods: &[HybridMethodSpec]| {
        methods
            .iter()
            .any(|m| pred(&m.return_type) || m.params.iter().any(|p| pred(&p.ty)))
    };
    spec.structs
        .iter()
        .any(|s| s.fields.iter().any(|f| pred(&f.ty)))
        || spec.modules.iter().any(|m| any_method(&m.methods))
        || spec.views.iter().any(|v| {
            v.props.iter().any(|p| pred(&p.ty))
                || v.events
                    .iter()
                    .any(|ev| ev.params.iter().any(|p| pred(&p.ty)))
                || any_method(&v.methods)
        })
}

fn type_contains_bytes(ty: &HybridType) -> bool {
    match ty {
        HybridType::Bytes => true,
        HybridType::Optional(inner)
        | HybridType::Vec(inner)
        | HybridType::FixedArray { elem: inner, .. } => type_contains_bytes(inner),
        HybridType::Result { ok, .. } => type_contains_bytes(ok),
        _ => false,
    }
}

fn type_needs_dart_ffi(ty: &HybridType) -> bool {
    match ty {
        HybridType::ConstPtr(_) | HybridType::MutPtr(_) | HybridType::Handle(_) => true,
        HybridType::Optional(inner)
        | HybridType::Vec(inner)
        | HybridType::FixedArray { elem: inner, .. } => type_needs_dart_ffi(inner),
        HybridType::Result { ok, err } => type_needs_dart_ffi(ok) || type_needs_dart_ffi(err),
        _ => false,
    }
}

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
        HybridType::Value => "DynamicValue".to_string(),
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

pub(super) fn dart_ffi_field_annotation(ty: &HybridType) -> Option<String> {
    match ty {
        HybridType::Bool => Some("@ffi.Bool()".to_string()),
        HybridType::U8 => Some("@ffi.Uint8()".to_string()),
        HybridType::U16 => Some("@ffi.Uint16()".to_string()),
        HybridType::Usize => Some("@ffi.Size()".to_string()),
        HybridType::I32 => Some("@ffi.Int32()".to_string()),
        HybridType::U32 | HybridType::Enum(_) => Some("@ffi.Uint32()".to_string()),
        HybridType::I64 => Some("@ffi.Int64()".to_string()),
        HybridType::U64 => Some("@ffi.Uint64()".to_string()),
        HybridType::F32 => Some("@ffi.Float()".to_string()),
        HybridType::F64 => Some("@ffi.Double()".to_string()),
        HybridType::FixedArray { len, .. } => Some(format!("@ffi.Array({len})")),
        _ => None,
    }
}

fn dart_ffi_array_element_type(elem: &HybridType) -> Result<String, CodegenError> {
    match elem {
        HybridType::Bool => Ok("ffi.Bool".to_string()),
        HybridType::U8 => Ok("ffi.Uint8".to_string()),
        HybridType::U16 => Ok("ffi.Uint16".to_string()),
        HybridType::Usize => Ok("ffi.Size".to_string()),
        HybridType::I32 => Ok("ffi.Int32".to_string()),
        HybridType::U32 | HybridType::Enum(_) => Ok("ffi.Uint32".to_string()),
        HybridType::I64 => Ok("ffi.Int64".to_string()),
        HybridType::U64 => Ok("ffi.Uint64".to_string()),
        HybridType::F32 => Ok("ffi.Float".to_string()),
        HybridType::F64 => Ok("ffi.Double".to_string()),
        HybridType::Struct(name) => Ok(format!("NveHybrid{name}")),
        other => Err(CodegenError::validation(format!(
            "unsupported Dart FFI fixed-array element type '{}'",
            other.canonical_signature()
        ))),
    }
}

pub(super) fn dart_ffi_struct_field_type(ty: &HybridType) -> Result<String, CodegenError> {
    match ty {
        HybridType::Bool => Ok("bool".to_string()),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::Enum(_) => Ok("int".to_string()),
        HybridType::F32 | HybridType::F64 => Ok("double".to_string()),
        HybridType::Struct(name) => Ok(format!("NveHybrid{name}")),
        HybridType::FixedArray { elem, .. } => {
            Ok(format!("ffi.Array<{}>", dart_ffi_array_element_type(elem)?))
        }
        other => Err(CodegenError::validation(format!(
            "unsupported Dart FFI POD struct field type '{}'",
            other.canonical_signature()
        ))),
    }
}

pub(super) fn dart_native_value_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if let Some(suffix) = ty.optional_pod_suffix(spec) {
        return format!("_NveHybridOpt{suffix}");
    }
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
        _ => "HybridOwnedBytes".to_string(),
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
        _ if ty.optional_pod_suffix(spec).is_some() => dart_native_value_type(ty, spec),
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

/// Emits a 64-bit constant as a web-safe `Key64(lo, hi)` matching `nve_abi_const.generated.dart`.
pub(super) fn dart_key64_const(v: u64) -> String {
    let lo = v as u32;
    let hi = (v >> 32) as u32;
    format!("Key64(0x{lo:08X}, 0x{hi:08X})")
}

pub(super) fn dart_is_const_expr(
    ty: &HybridType,
    def: Option<&HybridDefaultValue>,
    spec: &HybridFileSpec,
) -> bool {
    if matches!(ty, HybridType::Optional(_)) {
        return match def {
            None | Some(HybridDefaultValue::None) => true,
            Some(_) => matches!(
                ty,
                HybridType::Optional(inner) if dart_is_const_expr(inner, def, spec)
            ),
        };
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
        | HybridType::String
        | HybridType::Enum(_) => true,
        HybridType::Struct(name) => spec.find_struct(name).is_some_and(|s| {
            s.fields
                .iter()
                .all(|f| dart_is_const_expr(&f.ty, None, spec))
        }),
        _ => false,
    }
}

pub(super) fn dart_default_value(
    ty: &HybridType,
    def: Option<&HybridDefaultValue>,
    spec: &HybridFileSpec,
) -> String {
    dart_default_value_ctx(ty, def, spec, false)
}

fn dart_default_value_ctx(
    ty: &HybridType,
    def: Option<&HybridDefaultValue>,
    spec: &HybridFileSpec,
    in_const: bool,
) -> String {
    if let Some(d) = def {
        if let HybridType::Optional(inner) = ty {
            return match d {
                HybridDefaultValue::None => "null".to_string(),
                other => dart_default_value_ctx(inner, Some(other), spec, in_const),
            };
        }
        return match d.scalar_literal(ty) {
            Ok(lit) => lit,
            Err(NonScalarDefault::String(s)) => format!("'{}'", escape_dart_single_quoted_string(s)),
            Err(NonScalarDefault::EnumVariant { enum_name, variant }) => {
                format!("{enum_name}.{}", dart_ident(variant))
            }
            Err(NonScalarDefault::None) => "null".to_string(),
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
        HybridType::Vec(inner) => format!("<{}>[]", dart_type(inner)),
        HybridType::FixedArray { elem, len } => format!(
            "List<{}>.generate({len}, (_) => {})",
            dart_type(elem),
            dart_default_value_ctx(elem, None, spec, false),
        ),
        HybridType::Struct(name) => {
            if let Some(s) = spec.find_struct(name) {
                let is_const = dart_is_const_expr(ty, None, spec);
                let child_in_const = in_const || is_const;
                let args: Vec<String> = s
                    .fields
                    .iter()
                    .map(|f| {
                        format!(
                            "{}: {}",
                            dart_ident(&f.name),
                            dart_default_value_ctx(&f.ty, None, spec, child_in_const)
                        )
                    })
                    .collect();
                if is_const && !in_const {
                    format!("const {name}({})", args.join(", "))
                } else {
                    format!("{name}({})", args.join(", "))
                }
            } else {
                format!("{name}()")
            }
        }
        HybridType::Enum(name) => {
            if let Some(first) = spec.find_enum(name).and_then(|e| e.variants.first()) {
                format!("{name}.{}", dart_ident(&first.name))
            } else {
                format!("{name}.fromRaw(0)")
            }
        }
        HybridType::Value => "DynamicValue.nullValue".to_string(),
        HybridType::ConstPtr(_) | HybridType::MutPtr(_) | HybridType::Handle(_) => {
            "ffi.nullptr".to_string()
        }
        HybridType::Void | HybridType::Result { .. } => "null".to_string(),
    }
}

pub(super) fn emit_dart_bin_write(
    out: &mut CodeWriter,
    indent: &str,
    w: &str,
    expr: &str,
    ty: &HybridType,
) {
    let d = indent.len();
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr});"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => writeln!(out, "{indent}{w}.writeI64({expr});"),
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{w}.writeF64({expr});")
        }
        HybridType::Enum(_) => writeln!(out, "{indent}{w}.writeI64({expr}.rawValue);"),
        HybridType::String => writeln!(out, "{indent}{w}.writeStr({expr});"),
        HybridType::Bytes => writeln!(out, "{indent}{w}.writeBytes({expr});"),
        HybridType::Struct(_) => writeln!(out, "{indent}{expr}.encodeBin({w});"),
        HybridType::Value => writeln!(out, "{indent}{w}.raw({expr}.bytes);"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  final _nveOv{d} = {expr};");
            writeln!(out, "{indent}  if (_nveOv{d} != null) {{");
            emit_dart_bin_write(
                out,
                &format!("{indent}    "),
                w,
                &format!("_nveOv{d}"),
                inner,
            );
            writeln!(out, "{indent}  }} else {{");
            writeln!(out, "{indent}    {w}.writeNull();");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  final _nveArr{d} = {w}.beginArray({expr}.length);");
            writeln!(out, "{indent}  for (final _nveEl{d} in {expr}) {{");
            emit_dart_bin_write(
                out,
                &format!("{indent}    "),
                w,
                &format!("_nveEl{d}"),
                inner,
            );
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  {w}.end(_nveArr{d});");
            writeln!(out, "{indent}}}");
        }
        HybridType::FixedArray { elem: inner, len } => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  if ({expr}.length != {len}) {{");
            writeln!(
                out,
                "{indent}    throw RangeError.value({expr}.length, null, 'expected fixed length {len}');"
            );
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  final _nveArr{d} = {w}.beginArray({len});");
            writeln!(out, "{indent}  for (final _nveEl{d} in {expr}) {{");
            emit_dart_bin_write(
                out,
                &format!("{indent}    "),
                w,
                &format!("_nveEl{d}"),
                inner,
            );
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  {w}.end(_nveArr{d});");
            writeln!(out, "{indent}}}");
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => writeln!(out, "{indent}{w}.writeNull();"),
    }
}

pub(super) fn emit_dart_bin_read(
    out: &mut CodeWriter,
    indent: &str,
    r: &str,
    dst: &str,
    ty: &HybridType,
) {
    let d = indent.len();
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.readBool();"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => writeln!(out, "{indent}{dst} = {r}.readI64();"),
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{dst} = {r}.readF64();")
        }
        HybridType::Enum(e) => {
            writeln!(out, "{indent}{dst} = {e}.fromRaw({r}.readI64());")
        }
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.readStr();"),
        HybridType::Bytes => {
            writeln!(out, "{indent}{dst} = Uint8List.fromList({r}.readBytes());")
        }
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = {s}.decodeBin({r});"),
        HybridType::Value => {
            writeln!(out, "{indent}{dst} = DynamicValue.read({r});")
        }
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({r}.readNull()) {{");
            writeln!(out, "{indent}  {dst} = null;");
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  late final {} _nveOv{d};", dart_type(inner));
            emit_dart_bin_read(
                out,
                &format!("{indent}  "),
                r,
                &format!("_nveOv{d}"),
                inner,
            );
            writeln!(out, "{indent}  {dst} = _nveOv{d};");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            let inner_ty = dart_type(inner);
            writeln!(
                out,
                "{indent}{dst} = {r}.readArrayScope<List<{inner_ty}>>((_nveN{d}) {{"
            );
            writeln!(out, "{indent}  final _nveList{d} = <{inner_ty}>[];");
            writeln!(
                out,
                "{indent}  for (var _nveI{d} = 0; _nveI{d} < _nveN{d}; _nveI{d}++) {{"
            );
            writeln!(out, "{indent}    late final {inner_ty} _nveEl{d};");
            emit_dart_bin_read(
                out,
                &format!("{indent}    "),
                r,
                &format!("_nveEl{d}"),
                inner,
            );
            writeln!(out, "{indent}    _nveList{d}.add(_nveEl{d});");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  return _nveList{d};");
            writeln!(out, "{indent}}});");
        }
        HybridType::FixedArray { elem: inner, len } => {
            let inner_ty = dart_type(inner);
            writeln!(
                out,
                "{indent}{dst} = {r}.readArrayScope<List<{inner_ty}>>((_nveN{d}) {{"
            );
            writeln!(out, "{indent}  if (_nveN{d} != {len}) {{");
            writeln!(
                out,
                "{indent}    throw BinCodecException(_nveN{d} < {len} ? BinErrorKind.truncated : BinErrorKind.tooLarge, 'expected fixed length {len}, got $_nveN{d}');"
            );
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  final _nveList{d} = <{inner_ty}>[];");
            writeln!(
                out,
                "{indent}  for (var _nveI{d} = 0; _nveI{d} < {len}; _nveI{d}++) {{"
            );
            writeln!(out, "{indent}    late final {inner_ty} _nveEl{d};");
            emit_dart_bin_read(
                out,
                &format!("{indent}    "),
                r,
                &format!("_nveEl{d}"),
                inner,
            );
            writeln!(out, "{indent}    _nveList{d}.add(_nveEl{d});");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  return _nveList{d};");
            writeln!(out, "{indent}}});");
        }
        HybridType::ConstPtr(_) | HybridType::MutPtr(_) | HybridType::Handle(_) => {
            writeln!(out, "{indent}{r}.skipValue();");
            writeln!(out, "{indent}{dst} = ffi.nullptr;");
        }
        HybridType::Void | HybridType::Result { .. } => writeln!(out, "{indent}{r}.skipValue();"),
    }
}
