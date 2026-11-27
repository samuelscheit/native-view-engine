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
        "  static const int schemaHash = {};",
        dart_u64_const(schema_hash)
    );
    writeln!(out);
    writeln!(out, "  final ffi.Pointer<NveHybrid{pascal}VTable> _vtable;");
    writeln!(out, "  ffi.Pointer<ffi.Void> _handle;");
    writeln!(out, "  final _NveDestroyDart _destroy;");
    writeln!(out, "  final _NveAllocBytesDart _allocBytes;");
    writeln!(out, "  final _NveFreeBytesDart _freeBytes;");
    if view.is_some() {
        writeln!(out, "  final _NveSetPropsDirectDart _setPropsDirect;");
    }
    writeln!(out);

    writeln!(
        out,
        "  static {pascal}HybridClient resolve(ffi.DynamicLibrary library) {{"
    );
    writeln!(
        out,
        "    final getIface = library.lookupFunction<_NveGetInterfaceNative, _NveGetInterfaceDart>('nve_get_interface');"
    );
    writeln!(out, "    final raw = getIface(interfaceId);");
    writeln!(out, "    if (raw == ffi.nullptr) {{");
    writeln!(
        out,
        "      throw StateError('NVE hybrid interface {pascal} (0x{interface_id:08X}) not found');"
    );
    writeln!(out, "    }}");
    writeln!(
        out,
        "    return {pascal}HybridClient._(raw.cast<NveHybrid{pascal}VTable>());"
    );
    writeln!(out, "  }}");
    writeln!(out);

    writeln!(out, "  {pascal}HybridClient._(this._vtable)");
    writeln!(out, "      : _handle = ffi.nullptr,");
    writeln!(
        out,
        "        _destroy = _vtable.ref.destroy.asFunction<_NveDestroyDart>(),"
    );
    writeln!(
        out,
        "        _allocBytes = _vtable.ref.allocBytes.asFunction<_NveAllocBytesDart>(),"
    );
    writeln!(
        out,
        "        _freeBytes = _vtable.ref.freeBytes.asFunction<_NveFreeBytesDart>()"
    );
    if view.is_some() {
        writeln!(
            out,
            "        , _setPropsDirect = _vtable.ref.setPropsDirect.asFunction<_NveSetPropsDirectDart>()"
        );
    }
    writeln!(out, "  {{");
    writeln!(
        out,
        "    if (_vtable.ref.abiVersion != nveHybridAbiVersion || _vtable.ref.schemaHash != schemaHash) {{"
    );
    writeln!(
        out,
        "      throw StateError('NVE hybrid schema hash mismatch for {pascal}');"
    );
    writeln!(out, "    }}");
    writeln!(
        out,
        "    final createFn = _vtable.ref.create.asFunction<_NveCreateDart>();"
    );
    writeln!(out, "    _handle = createFn();");
    writeln!(out, "    if (_handle == ffi.nullptr) {{");
    writeln!(
        out,
        "      throw StateError('failed to create {pascal} hybrid instance');"
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

    let mut methods_body = CodeWriter::default();
    if let Some(v) = view {
        emit_dart_set_props(&mut methods_body, pascal, v);
    }

    for m in methods {
        emit_dart_client_method(&mut methods_body, pascal, m, spec);
    }

    out.push_str(&methods_body);

    if methods_body.contains("_copyToNative(") {
        writeln!(
            out,
            "  NveHybridOwnedBytes _copyToNative(List<int> bytes) {{"
        );
        writeln!(
            out,
            "    if (bytes.isEmpty) return ffi.Struct.create<NveHybridOwnedBytes>();"
        );
        writeln!(out, "    final buf = _allocBytes(bytes.length);");
        writeln!(
            out,
            "    buf.ptr.asTypedList(bytes.length).setAll(0, bytes);"
        );
        writeln!(out, "    return buf;");
        writeln!(out, "  }}");
        writeln!(out);
    }

    if methods_body.contains("_takeString(") {
        writeln!(out, "  String _takeString(NveHybridOwnedBytes raw) {{");
        writeln!(out, "    try {{");
        writeln!(
            out,
            "      if (raw.ptr == ffi.nullptr || raw.len == 0) return '';"
        );
        writeln!(
            out,
            "      return utf8.decode(raw.ptr.asTypedList(raw.len));"
        );
        writeln!(out, "    }} finally {{");
        writeln!(out, "      if (raw.ptr != ffi.nullptr) _freeBytes(raw);");
        writeln!(out, "    }}");
        writeln!(out, "  }}");
        writeln!(out);
    }

    if methods_body.contains("_takeBytes(") {
        writeln!(out, "  Uint8List _takeBytes(NveHybridOwnedBytes raw) {{");
        writeln!(out, "    try {{");
        writeln!(
            out,
            "      if (raw.ptr == ffi.nullptr || raw.len == 0) return Uint8List(0);"
        );
        writeln!(
            out,
            "      return Uint8List.fromList(raw.ptr.asTypedList(raw.len));"
        );
        writeln!(out, "    }} finally {{");
        writeln!(out, "      if (raw.ptr != ffi.nullptr) _freeBytes(raw);");
        writeln!(out, "    }}");
        writeln!(out, "  }}");
    }
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_dart_direct_prop_write(
    out: &mut CodeWriter,
    key_lit: &str,
    expr: &str,
    ty: &HybridType,
) {
    match ty {
        HybridType::Bool => writeln!(out, "    builder.writeBool('{key_lit}', {expr});"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => writeln!(out, "    builder.writeInt64('{key_lit}', {expr});"),
        HybridType::Enum(_) => {
            writeln!(out, "    builder.writeInt64('{key_lit}', {expr}.rawValue);")
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "    builder.writeDouble('{key_lit}', {expr});")
        }
        HybridType::String => {
            writeln!(out, "    builder.writeString('{key_lit}', {expr});")
        }
        HybridType::Bytes => writeln!(out, "    builder.writeBytes('{key_lit}', {expr});"),
        HybridType::Optional(inner) => match inner.as_ref() {
            HybridType::Bool => {
                writeln!(out, "    builder.writeBool('{key_lit}', {expr});")
            }
            HybridType::U8
            | HybridType::U16
            | HybridType::Usize
            | HybridType::I32
            | HybridType::U32
            | HybridType::I64
            | HybridType::U64 => {
                writeln!(out, "    builder.writeInt64('{key_lit}', {expr});")
            }
            HybridType::Enum(_) => {
                writeln!(
                    out,
                    "    builder.writeInt64('{key_lit}', {expr}?.rawValue);"
                )
            }
            HybridType::F32 | HybridType::F64 => {
                writeln!(out, "    builder.writeDouble('{key_lit}', {expr});")
            }
            HybridType::String => {
                writeln!(out, "    builder.writeString('{key_lit}', {expr});")
            }
            HybridType::Bytes => {
                writeln!(out, "    builder.writeBytes('{key_lit}', {expr});")
            }
            other => {
                writeln!(out, "    final _opt_{expr} = {expr};");
                writeln!(out, "    if (_opt_{expr} == null) {{");
                writeln!(out, "      builder.writeNull('{key_lit}');");
                writeln!(out, "    }} else {{");
                writeln!(out, "      final _bw = NveBinWriter();");
                emit_dart_bin_write(out, "      ", "_bw", &format!("_opt_{expr}"), other);
                writeln!(
                    out,
                    "      builder.writeBytes('{key_lit}', _bw.takeBytes());"
                );
                writeln!(out, "    }}");
            }
        },
        other => {
            writeln!(out, "    {{");
            writeln!(out, "      final _bw = NveBinWriter();");
            emit_dart_bin_write(out, "      ", "_bw", expr, other);
            writeln!(
                out,
                "      builder.writeBytes('{key_lit}', _bw.takeBytes());"
            );
            writeln!(out, "    }}");
        }
    }
}

pub(super) fn emit_dart_set_props(out: &mut CodeWriter, pascal: &str, _v: &HybridViewSpec) {
    writeln!(out, "  @override");
    writeln!(out, "  void setProps({pascal}Props props) {{");
    writeln!(out, "    final st = NveDirectPropsWriter.withEncodedProps(");
    writeln!(out, "      props,");
    writeln!(
        out,
        "      (entriesPtr, count) => _setPropsDirect(_handle, entriesPtr, count),"
    );
    writeln!(out, "    );");
    writeln!(
        out,
        "    if (st != nveHybridOk) throw StateError('setProps failed on {pascal}');"
    );
    writeln!(out, "  }}");
    writeln!(out);
}

pub(super) fn emit_dart_client_method(
    out: &mut CodeWriter,
    pascal: &str,
    m: &HybridMethodSpec,
    spec: &HybridFileSpec,
) {
    let mp = to_pascal_case(&m.name);
    let mfield = dart_ident(&m.name);

    writeln!(out, "  @override");
    writeln!(out, "  {} {{", dart_method_sig(m));
    writeln!(
        out,
        "    final fn = _vtable.ref.{mfield}.asFunction<_{pascal}{mp}Dart>();"
    );
    writeln!(out, "    final tempBufs = <NveHybridOwnedBytes>[];");
    writeln!(out, "    try {{");

    let mut args = vec!["_handle".to_string()];
    for (idx, p) in m.params.iter().enumerate() {
        let pname = dart_ident(&p.name);
        if let (HybridType::Optional(inner), Some(suffix)) = (&p.ty, p.ty.optional_pod_suffix(spec))
        {
            writeln!(
                out,
                "      final _o{idx} = ffi.Struct.create<NveHybridOpt{suffix}>();"
            );
            writeln!(out, "      if ({pname} != null) {{");
            writeln!(out, "        _o{idx}.isSome = true;");
            match inner.as_ref() {
                HybridType::Enum(_) => writeln!(out, "        _o{idx}.value = {pname}.rawValue;"),
                HybridType::Struct(_) => writeln!(out, "        _o{idx}.value = {pname}.toAbi();"),
                _ => writeln!(out, "        _o{idx}.value = {pname};"),
            }
            writeln!(out, "      }}");
            args.push(format!("_o{idx}"));
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
                    "      final _b{idx} = _copyToNative(utf8.encode({pname}));"
                );
                writeln!(out, "      tempBufs.add(_b{idx});");
                writeln!(
                    out,
                    "      final _s{idx} = ffi.Struct.create<NveStrSlice>()..ptr = _b{idx}.ptr..len = _b{idx}.len;"
                );
                args.push(format!("_s{idx}"));
            }
            HybridType::Bytes => {
                writeln!(out, "      final _b{idx} = _copyToNative({pname});");
                writeln!(out, "      tempBufs.add(_b{idx});");
                writeln!(
                    out,
                    "      final _s{idx} = ffi.Struct.create<NveBufferSlice>()..ptr = _b{idx}.ptr..len = _b{idx}.len;"
                );
                args.push(format!("_s{idx}"));
            }
            HybridType::Value => {
                writeln!(
                    out,
                    "      final _b{idx} = _copyToNative({pname}.binBytes);"
                );
                writeln!(out, "      tempBufs.add(_b{idx});");
                writeln!(out, "      final _v{idx} = ffi.Struct.create<NveValue>();");
                writeln!(out, "      _v{idx}.tag = 5;");
                writeln!(out, "      _v{idx}.data.bytesVal.ptr = _b{idx}.ptr;");
                writeln!(out, "      _v{idx}.data.bytesVal.len = _b{idx}.len;");
                args.push(format!("_v{idx}"));
            }
            other => {
                writeln!(out, "      final _w{idx} = NveBinWriter();");
                emit_dart_bin_write(out, "      ", &format!("_w{idx}"), &pname, other);
                writeln!(
                    out,
                    "      final _b{idx} = _copyToNative(_w{idx}.takeBytes());"
                );
                writeln!(out, "      tempBufs.add(_b{idx});");
                writeln!(
                    out,
                    "      final _s{idx} = ffi.Struct.create<NveBufferSlice>()..ptr = _b{idx}.ptr..len = _b{idx}.len;"
                );
                args.push(format!("_s{idx}"));
            }
        }
    }

    match &m.return_type {
        HybridType::Void => {
            writeln!(out, "      fn({});", args.join(", "));
        }
        HybridType::Result { ok, .. } => {
            let has_ok = !matches!(ok.as_ref(), HybridType::Void);
            if has_ok {
                let ok_native = dart_native_value_type(ok, spec);
                writeln!(
                    out,
                    "      final _okSlot = _allocBytes(ffi.sizeOf<{ok_native}>());"
                );
                writeln!(out, "      tempBufs.add(_okSlot);");
                writeln!(out, "      final _okPtr = _okSlot.ptr.cast<{ok_native}>();");
                args.push("_okPtr".to_string());
            }
            writeln!(
                out,
                "      final _errSlot = _allocBytes(ffi.sizeOf<NveHybridOwnedBytes>());"
            );
            writeln!(out, "      tempBufs.add(_errSlot);");
            writeln!(
                out,
                "      final _errPtr = _errSlot.ptr.cast<NveHybridOwnedBytes>();"
            );
            args.push("_errPtr".to_string());

            writeln!(out, "      final _status = fn({});", args.join(", "));
            writeln!(out, "      if (_status != nveHybridOk) {{");
            writeln!(out, "        final _msg = _takeString(_errPtr.ref);");
            writeln!(
                out,
                "        throw StateError(_msg.isEmpty ? 'hybrid call failed' : _msg);"
            );
            writeln!(out, "      }}");
            if has_ok {
                let read_expr = dart_read_ptr_expr("_okPtr", ok, spec);
                writeln!(out, "      final _rawOk = {read_expr};");
                emit_dart_return_decode(out, "      ", "_rawOk", ok, spec);
            }
        }
        other => {
            writeln!(out, "      final _rawRet = fn({});", args.join(", "));
            emit_dart_return_decode(out, "      ", "_rawRet", other, spec);
        }
    }

    writeln!(out, "    }} finally {{");
    writeln!(
        out,
        "      for (final b in tempBufs) {{ if (b.ptr != ffi.nullptr) _freeBytes(b); }}"
    );
    writeln!(out, "    }}");
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
        HybridType::String => writeln!(out, "{indent}return _takeString({var});"),
        HybridType::Bytes => writeln!(out, "{indent}return _takeBytes({var});"),
        HybridType::Value => {
            writeln!(
                out,
                "{indent}return NveHybridDynamicValue(_takeBytes({var}));"
            )
        }
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            writeln!(out, "{indent}if (!{var}.isSome) return null;");
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
            writeln!(out, "{indent}final _r = NveBinReader(_takeBytes({var}));");
            writeln!(out, "{indent}late final {} _out;", dart_type(other));
            emit_dart_bin_read(out, indent, "_r", "_out", other);
            writeln!(out, "{indent}return _out;");
        }
    }
}
