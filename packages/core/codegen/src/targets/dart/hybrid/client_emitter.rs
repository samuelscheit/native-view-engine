use crate::writer::CodeWriter;

use super::type_helpers::*;
use super::*;

pub(super) fn emit_dart_client(
    out: &mut CodeWriter,
    pascal: &str,
    interface_id: u32,
    schema_hash: u64,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
    spec: &HybridFileSpec,
) {
    let mut methods_body = CodeWriter::default();
    if let Some(v) = view {
        emit_dart_set_props(&mut methods_body, pascal, v);
    }

    for m in methods {
        emit_dart_client_method(&mut methods_body, pascal, m, spec);
    }

    let needs_free_bytes = methods_body.contains("_freeBytes");
    let needs_handle_guard = methods_body.contains("_nveHandle");

    writeln!(
        out,
        "class {pascal}HybridClient implements Hybrid{pascal}Spec {{"
    );
    writeln!(
        out,
        "  static const int interfaceId = 0x{interface_id:08X};"
    );
    writeln!(
        out,
        "  static const Key64 schemaHash = {};",
        dart_key64_const(schema_hash)
    );
    writeln!(out);
    writeln!(out, "  final ffi.Pointer<NveHybrid{pascal}VTable> _vtable;");
    writeln!(out, "  ffi.Pointer<ffi.Void> _handle;");
    writeln!(out, "  late final HybridDestroyDart _destroy;");
    if needs_free_bytes {
        writeln!(out, "  late final HybridFreeBytesDart _freeBytes;");
    }
    if view.is_some() {
        writeln!(out, "  late final HybridSetPropsDirectDart _setPropsDirect;");
    }
    writeln!(out);

    writeln!(
        out,
        "  static {pascal}HybridClient resolve(ffi.DynamicLibrary library) {{"
    );
    writeln!(
        out,
        "    final _nveRaw = hybridResolveInterface(library, interfaceId, '{pascal}');"
    );
    writeln!(
        out,
        "    return {pascal}HybridClient.fromVtable(_nveRaw.cast<NveHybrid{pascal}VTable>());"
    );
    writeln!(out, "  }}");
    writeln!(out);

    writeln!(
        out,
        "  {pascal}HybridClient.fromVtable(this._vtable) : _handle = ffi.nullptr {{"
    );
    writeln!(out, "    if (_vtable == ffi.nullptr) {{");
    writeln!(
        out,
        "      throw const HybridException(HybridStatus.nullHandle, 'hybrid vtable for {pascal} is null');"
    );
    writeln!(out, "    }}");
    writeln!(
        out,
        "    hybridVerifyVtable(_vtable.ref.abiVersion, _vtable.ref.schemaHash, schemaHash, '{pascal}');"
    );
    writeln!(
        out,
        "    _destroy = _vtable.ref.destroy.asFunction<HybridDestroyDart>();"
    );
    if needs_free_bytes {
        writeln!(
            out,
            "    _freeBytes = _vtable.ref.freeBytes.asFunction<HybridFreeBytesDart>();"
        );
    }
    if view.is_some() {
        writeln!(
            out,
            "    _setPropsDirect = _vtable.ref.setPropsDirect.asFunction<HybridSetPropsDirectDart>();"
        );
    }
    writeln!(
        out,
        "    final _nveCreate = _vtable.ref.create.asFunction<HybridCreateDart>();"
    );
    writeln!(out, "    _handle = _nveCreate();");
    writeln!(out, "    if (_handle == ffi.nullptr) {{");
    writeln!(
        out,
        "      throw const HybridException(HybridStatus.nullHandle, 'failed to create {pascal} hybrid instance');"
    );
    writeln!(out, "    }}");
    writeln!(out, "  }}");
    writeln!(out);

    writeln!(out, "  void dispose() {{");
    writeln!(out, "    if (_handle != ffi.nullptr) {{");
    writeln!(out, "      _destroy(_handle);");
    writeln!(out, "      _handle = ffi.nullptr;");
    writeln!(out, "    }}");
    writeln!(out, "  }}");
    writeln!(out);

    if needs_handle_guard {
        writeln!(out, "  ffi.Pointer<ffi.Void> get _nveHandle {{");
        writeln!(out, "    if (_handle == ffi.nullptr) {{");
        writeln!(
            out,
            "      throw const HybridException(HybridStatus.nullHandle, '{pascal}HybridClient has been disposed');"
        );
        writeln!(out, "    }}");
        writeln!(out, "    return _handle;");
        writeln!(out, "  }}");
        writeln!(out);
    }

    out.push_str(&methods_body);
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_dart_direct_prop_write(
    out: &mut CodeWriter,
    key_lit: &str,
    expr: &str,
    ty: &HybridType,
    idx: usize,
) {
    match ty {
        HybridType::Bool => writeln!(out, "    _nveB.writeBool('{key_lit}', {expr});"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => writeln!(out, "    _nveB.writeInt64('{key_lit}', {expr});"),
        HybridType::Enum(_) => {
            writeln!(out, "    _nveB.writeInt64('{key_lit}', {expr}.rawValue);")
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "    _nveB.writeDouble('{key_lit}', {expr});")
        }
        HybridType::String => {
            writeln!(out, "    _nveB.writeString('{key_lit}', {expr});")
        }
        HybridType::Bytes => writeln!(out, "    _nveB.writeBytes('{key_lit}', {expr});"),
        HybridType::Value => {
            writeln!(out, "    _nveB.writeDynamicValue('{key_lit}', {expr});")
        }
        HybridType::Optional(inner) => match inner.as_ref() {
            HybridType::Bool => {
                writeln!(out, "    _nveB.writeBool('{key_lit}', {expr});")
            }
            HybridType::U8
            | HybridType::U16
            | HybridType::Usize
            | HybridType::I32
            | HybridType::U32
            | HybridType::I64
            | HybridType::U64 => {
                writeln!(out, "    _nveB.writeInt64('{key_lit}', {expr});")
            }
            HybridType::Enum(_) => {
                writeln!(
                    out,
                    "    _nveB.writeInt64('{key_lit}', {expr}?.rawValue);"
                )
            }
            HybridType::F32 | HybridType::F64 => {
                writeln!(out, "    _nveB.writeDouble('{key_lit}', {expr});")
            }
            HybridType::String => {
                writeln!(out, "    _nveB.writeString('{key_lit}', {expr});")
            }
            HybridType::Bytes => {
                writeln!(out, "    _nveB.writeBytes('{key_lit}', {expr});")
            }
            HybridType::Value => {
                writeln!(out, "    _nveB.writeDynamicValue('{key_lit}', {expr});")
            }
            other => {
                writeln!(out, "    final _nveOpt{idx} = {expr};");
                writeln!(out, "    if (_nveOpt{idx} == null) {{");
                writeln!(out, "      _nveB.writeNull('{key_lit}');");
                writeln!(out, "    }} else {{");
                writeln!(out, "      final _nveBw{idx} = BinWriter();");
                emit_dart_bin_write(
                    out,
                    "      ",
                    &format!("_nveBw{idx}"),
                    &format!("_nveOpt{idx}"),
                    other,
                );
                writeln!(
                    out,
                    "      _nveB.writeBytes('{key_lit}', _nveBw{idx}.bytes);"
                );
                writeln!(out, "    }}");
            }
        },
        other => {
            writeln!(out, "    {{");
            writeln!(out, "      final _nveBw{idx} = BinWriter();");
            emit_dart_bin_write(out, "      ", &format!("_nveBw{idx}"), expr, other);
            writeln!(
                out,
                "      _nveB.writeBytes('{key_lit}', _nveBw{idx}.bytes);"
            );
            writeln!(out, "    }}");
        }
    }
}

pub(super) fn emit_dart_set_props(out: &mut CodeWriter, pascal: &str, _v: &HybridViewSpec) {
    writeln!(out, "  @override");
    writeln!(out, "  void setProps({pascal}Props props) {{");
    writeln!(
        out,
        "    final _nveSt = HybridPropsWriter.withEncodedProps<int>("
    );
    writeln!(out, "      props,");
    writeln!(
        out,
        "      (_nveEntries, _nveCount) => _setPropsDirect(_nveHandle, _nveEntries, _nveCount),"
    );
    writeln!(out, "    );");
    writeln!(out, "    if (_nveSt != HybridStatus.ok.code) {{");
    writeln!(
        out,
        "      throw HybridException(HybridStatus.fromCode(_nveSt) ?? HybridStatus.err, 'setProps failed on {pascal} (status $_nveSt)');"
    );
    writeln!(out, "    }}");
    writeln!(out, "  }}");
    writeln!(out);
}

fn method_needs_arena(m: &HybridMethodSpec, spec: &HybridFileSpec) -> bool {
    if matches!(m.return_type, HybridType::Result { .. }) {
        return true;
    }
    m.params.iter().any(|p| match &p.ty {
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
        | HybridType::Enum(_) => false,
        HybridType::Struct(_) if p.ty.is_pod(spec) => false,
        HybridType::Optional(inner) if inner.is_pod(spec) => false,
        _ => true,
    })
}

pub(super) fn emit_dart_client_method(
    out: &mut CodeWriter,
    pascal: &str,
    m: &HybridMethodSpec,
    spec: &HybridFileSpec,
) {
    let mp = to_pascal_case(&m.name);
    let mfield = dart_ident(&m.name);
    let needs_arena = method_needs_arena(m, spec);
    let ret_dart_ty = match &m.return_type {
        HybridType::Result { ok, .. } => dart_type(ok),
        other => dart_type(other),
    };
    let is_void_ret = ret_dart_ty == "void";

    writeln!(out, "  @override");
    writeln!(out, "  {} {{", dart_method_sig(m));
    writeln!(
        out,
        "    final _nveFn = _vtable.ref.{mfield}.asFunction<_{pascal}{mp}Dart>();"
    );
    let indent = if needs_arena {
        if is_void_ret {
            writeln!(out, "    HybridArena.using<void>((_nveArena) {{");
        } else {
            writeln!(
                out,
                "    return HybridArena.using<{ret_dart_ty}>((_nveArena) {{"
            );
        }
        "      "
    } else {
        "    "
    };

    let mut args = vec!["_nveHandle".to_string()];
    for (idx, p) in m.params.iter().enumerate() {
        let pname = dart_ident(&p.name);
        if let (HybridType::Optional(inner), Some(suffix)) = (&p.ty, p.ty.optional_pod_suffix(spec))
        {
            writeln!(
                out,
                "{indent}final _nveArg{idx} = ffi.Struct.create<NveHybridOpt{suffix}>();"
            );
            writeln!(out, "{indent}if ({pname} != null) {{");
            writeln!(out, "{indent}  _nveArg{idx}.isSome = true;");
            match inner.as_ref() {
                HybridType::Enum(_) => {
                    writeln!(out, "{indent}  _nveArg{idx}.value = {pname}.rawValue;")
                }
                HybridType::Struct(_) => {
                    writeln!(out, "{indent}  _nveArg{idx}.value = {pname}.toAbi();")
                }
                _ => writeln!(out, "{indent}  _nveArg{idx}.value = {pname};"),
            }
            writeln!(out, "{indent}}}");
            args.push(format!("_nveArg{idx}"));
            continue;
        }
        match &p.ty {
            HybridType::Bool
            | HybridType::U8
            | HybridType::U16
            | HybridType::Usize
            | HybridType::I32
            | HybridType::U32
            | HybridType::I64
            | HybridType::U64
            | HybridType::F32
            | HybridType::F64 => args.push(pname),
            HybridType::Enum(_) => args.push(format!("{pname}.rawValue")),
            HybridType::Struct(_) if p.ty.is_pod(spec) => args.push(format!("{pname}.toAbi()")),
            HybridType::String => {
                writeln!(
                    out,
                    "{indent}final _nveArg{idx} = _nveArena.strSlice({pname});"
                );
                args.push(format!("_nveArg{idx}"));
            }
            HybridType::Bytes => {
                writeln!(
                    out,
                    "{indent}final _nveArg{idx} = _nveArena.bufferSlice({pname});"
                );
                args.push(format!("_nveArg{idx}"));
            }
            HybridType::Value => {
                writeln!(
                    out,
                    "{indent}final _nveArg{idx} = NveValueTree.fromDynamic(_nveArena, {pname}).root;"
                );
                args.push(format!("_nveArg{idx}"));
            }
            other => {
                writeln!(out, "{indent}final _nveW{idx} = BinWriter();");
                emit_dart_bin_write(out, indent, &format!("_nveW{idx}"), &pname, other);
                writeln!(
                    out,
                    "{indent}final _nveArg{idx} = _nveArena.bufferSlice(_nveW{idx}.bytes);"
                );
                args.push(format!("_nveArg{idx}"));
            }
        }
    }

    match &m.return_type {
        HybridType::Void => {
            writeln!(out, "{indent}_nveFn({});", args.join(", "));
        }
        HybridType::Result { ok, .. } => {
            let has_ok = !matches!(ok.as_ref(), HybridType::Void);
            if has_ok {
                let ok_native = dart_native_value_type(ok, spec);
                writeln!(
                    out,
                    "{indent}final _nveOkPtr = _nveArena.allocBytes(ffi.sizeOf<{ok_native}>()).cast<{ok_native}>();"
                );
                args.push("_nveOkPtr".to_string());
            }
            writeln!(
                out,
                "{indent}final _nveErrPtr = _nveArena.allocStruct<HybridOwnedBytes>(ffi.sizeOf<HybridOwnedBytes>());"
            );
            args.push("_nveErrPtr".to_string());

            writeln!(out, "{indent}final _nveStatus = _nveFn({});", args.join(", "));
            writeln!(out, "{indent}if (_nveStatus != HybridStatus.ok.code) {{");
            writeln!(
                out,
                "{indent}  final _nveMsg = hybridTakeString(_nveErrPtr.ref, _freeBytes);"
            );
            writeln!(
                out,
                "{indent}  throw HybridException(HybridStatus.fromCode(_nveStatus) ?? HybridStatus.err, _nveMsg.isEmpty ? 'hybrid call failed' : _nveMsg);"
            );
            writeln!(out, "{indent}}}");
            writeln!(out, "{indent}if (_nveErrPtr.ref.ptr != ffi.nullptr) {{");
            writeln!(out, "{indent}  _freeBytes(_nveErrPtr.ref);");
            writeln!(out, "{indent}}}");
            if has_ok {
                let read_expr = dart_read_ptr_expr("_nveOkPtr", ok, spec);
                writeln!(out, "{indent}final _nveRawOk = {read_expr};");
                emit_dart_return_decode(out, indent, "_nveRawOk", ok, spec);
            }
        }
        other => {
            writeln!(out, "{indent}final _nveRawRet = _nveFn({});", args.join(", "));
            emit_dart_return_decode(out, indent, "_nveRawRet", other, spec);
        }
    }

    if needs_arena {
        writeln!(out, "    }});");
    }
    writeln!(out, "  }}");
    writeln!(out);
}

pub(super) fn dart_read_ptr_expr(ptr: &str, ty: &HybridType, spec: &HybridFileSpec) -> String {
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
        | HybridType::Enum(_) => format!("{ptr}.value"),
        HybridType::Struct(_) if ty.is_pod(spec) => format!("{ptr}.ref"),
        HybridType::Optional(inner) if inner.is_pod(spec) => format!("{ptr}.ref"),
        _ => format!("{ptr}.ref"),
    }
}

pub(super) fn emit_dart_return_decode(
    out: &mut CodeWriter,
    indent: &str,
    var: &str,
    ty: &HybridType,
    spec: &HybridFileSpec,
) {
    match ty {
        HybridType::Void => {}
        HybridType::Bool
        | HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::F32
        | HybridType::F64 => writeln!(out, "{indent}return {var};"),
        HybridType::Enum(e) => writeln!(out, "{indent}return {e}.fromRaw({var});"),
        HybridType::Struct(s) if ty.is_pod(spec) => {
            writeln!(out, "{indent}return {s}.fromAbi({var});")
        }
        HybridType::String => writeln!(out, "{indent}return hybridTakeString({var}, _freeBytes);"),
        HybridType::Bytes => writeln!(out, "{indent}return hybridTakeBytes({var}, _freeBytes);"),
        HybridType::Value => {
            writeln!(
                out,
                "{indent}return hybridTakeDynamicValue({var}, _freeBytes);"
            )
        }
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            writeln!(out, "{indent}if (!{var}.isSome) {{");
            writeln!(out, "{indent}  return null;");
            writeln!(out, "{indent}}}");
            match inner.as_ref() {
                HybridType::Enum(e) => {
                    writeln!(out, "{indent}return {e}.fromRaw({var}.value);")
                }
                HybridType::Struct(s) => {
                    writeln!(out, "{indent}return {s}.fromAbi({var}.value);")
                }
                _ => writeln!(out, "{indent}return {var}.value;"),
            }
        }
        other => {
            let ret_ty = dart_type(other);
            writeln!(
                out,
                "{indent}return hybridDecodeOwned<{ret_ty}>({var}, _freeBytes, (_nveR) {{"
            );
            writeln!(out, "{indent}  late final {ret_ty} _nveOut;");
            emit_dart_bin_read(out, &format!("{indent}  "), "_nveR", "_nveOut", other);
            writeln!(out, "{indent}  return _nveOut;");
            writeln!(out, "{indent}}});");
        }
    }
}
