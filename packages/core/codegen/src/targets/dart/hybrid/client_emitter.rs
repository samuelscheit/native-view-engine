use std::fmt::Write as _;

use super::*;
use super::type_helpers::*;

pub(super) fn emit_dart_client(
    out: &mut String,
    pascal: &str,
    interface_id: u32,
    schema_hash: u64,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
    spec: &HybridFileSpec,
) {
    writeln!(out, "class {pascal}HybridClient implements Hybrid{pascal}Spec {{").unwrap();
    writeln!(out, "  static const int interfaceId = 0x{interface_id:08X};").unwrap();
    writeln!(
        out,
        "  static const int schemaHash = {};",
        dart_u64_const(schema_hash)
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  final ffi.Pointer<NveHybrid{pascal}VTable> _vtable;").unwrap();
    writeln!(out, "  ffi.Pointer<ffi.Void> _handle;").unwrap();
    writeln!(out, "  final _NveDestroyDart _destroy;").unwrap();
    writeln!(out, "  final _NveAllocBytesDart _allocBytes;").unwrap();
    writeln!(out, "  final _NveFreeBytesDart _freeBytes;").unwrap();
    if view.is_some() {
        writeln!(out, "  final _NveSetPropsDirectDart _setPropsDirect;").unwrap();
    }
    writeln!(out).unwrap();

    writeln!(
        out,
        "  static {pascal}HybridClient resolve(ffi.DynamicLibrary library) {{"
    )
    .unwrap();
    writeln!(
        out,
        "    final getIface = library.lookupFunction<_NveGetInterfaceNative, _NveGetInterfaceDart>('nve_get_interface');"
    )
    .unwrap();
    writeln!(out, "    final raw = getIface(interfaceId);").unwrap();
    writeln!(out, "    if (raw == ffi.nullptr) {{").unwrap();
    writeln!(
        out,
        "      throw StateError('NVE hybrid interface {pascal} (0x{interface_id:08X}) not found');"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(
        out,
        "    return {pascal}HybridClient._(raw.cast<NveHybrid{pascal}VTable>());"
    )
    .unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "  {pascal}HybridClient._(this._vtable)").unwrap();
    writeln!(out, "      : _handle = ffi.nullptr,").unwrap();
    writeln!(out, "        _destroy = _vtable.ref.destroy.asFunction<_NveDestroyDart>(),")
        .unwrap();
    writeln!(
        out,
        "        _allocBytes = _vtable.ref.allocBytes.asFunction<_NveAllocBytesDart>(),"
    )
    .unwrap();
    writeln!(
        out,
        "        _freeBytes = _vtable.ref.freeBytes.asFunction<_NveFreeBytesDart>()"
    )
    .unwrap();
    if view.is_some() {
        writeln!(
            out,
            "        , _setPropsDirect = _vtable.ref.setPropsDirect.asFunction<_NveSetPropsDirectDart>()"
        )
        .unwrap();
    }
    writeln!(out, "  {{").unwrap();
    writeln!(
        out,
        "    if (_vtable.ref.abiVersion != nveHybridAbiVersion || _vtable.ref.schemaHash != schemaHash) {{"
    )
    .unwrap();
    writeln!(
        out,
        "      throw StateError('NVE hybrid schema hash mismatch for {pascal}');"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    final createFn = _vtable.ref.create.asFunction<_NveCreateDart>();").unwrap();
    writeln!(out, "    _handle = createFn();").unwrap();
    writeln!(out, "    if (_handle == ffi.nullptr) {{").unwrap();
    writeln!(out, "      throw StateError('failed to create {pascal} hybrid instance');").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "  void dispose() {{").unwrap();
    writeln!(out, "    if (_handle != ffi.nullptr) {{").unwrap();
    writeln!(out, "      _destroy(_handle);").unwrap();
    writeln!(out, "      _handle = ffi.nullptr;").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();

    let mut methods_body = String::new();
    if let Some(v) = view {
        emit_dart_set_props(&mut methods_body, pascal, v);
    }

    for m in methods {
        emit_dart_client_method(&mut methods_body, pascal, m, spec);
    }

    out.push_str(&methods_body);

    if methods_body.contains("_copyToNative(") {
        writeln!(out, "  NveHybridOwnedBytes _copyToNative(List<int> bytes) {{").unwrap();
        writeln!(out, "    if (bytes.isEmpty) return ffi.Struct.create<NveHybridOwnedBytes>();").unwrap();
        writeln!(out, "    final buf = _allocBytes(bytes.length);").unwrap();
        writeln!(out, "    buf.ptr.asTypedList(bytes.length).setAll(0, bytes);").unwrap();
        writeln!(out, "    return buf;").unwrap();
        writeln!(out, "  }}").unwrap();
        writeln!(out).unwrap();
    }

    if methods_body.contains("_takeString(") {
        writeln!(out, "  String _takeString(NveHybridOwnedBytes raw) {{").unwrap();
        writeln!(out, "    try {{").unwrap();
        writeln!(out, "      if (raw.ptr == ffi.nullptr || raw.len == 0) return '';").unwrap();
        writeln!(out, "      return utf8.decode(raw.ptr.asTypedList(raw.len));").unwrap();
        writeln!(out, "    }} finally {{").unwrap();
        writeln!(out, "      if (raw.ptr != ffi.nullptr) _freeBytes(raw);").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "  }}").unwrap();
        writeln!(out).unwrap();
    }

    if methods_body.contains("_takeBytes(") {
        writeln!(out, "  Uint8List _takeBytes(NveHybridOwnedBytes raw) {{").unwrap();
        writeln!(out, "    try {{").unwrap();
        writeln!(out, "      if (raw.ptr == ffi.nullptr || raw.len == 0) return Uint8List(0);").unwrap();
        writeln!(out, "      return Uint8List.fromList(raw.ptr.asTypedList(raw.len));").unwrap();
        writeln!(out, "    }} finally {{").unwrap();
        writeln!(out, "      if (raw.ptr != ffi.nullptr) _freeBytes(raw);").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "  }}").unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_dart_direct_prop_write(out: &mut String, key_lit: &str, expr: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "    builder.writeBool('{key_lit}', {expr});").unwrap(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => writeln!(out, "    builder.writeInt64('{key_lit}', {expr});").unwrap(),
        HybridType::Enum(_) => {
            writeln!(out, "    builder.writeInt64('{key_lit}', {expr}.rawValue);").unwrap()
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "    builder.writeDouble('{key_lit}', {expr});").unwrap()
        }
        HybridType::String => {
            writeln!(out, "    builder.writeString('{key_lit}', {expr});").unwrap()
        }
        HybridType::Bytes => writeln!(out, "    builder.writeBytes('{key_lit}', {expr});").unwrap(),
        HybridType::Optional(inner) => match inner.as_ref() {
            HybridType::Bool => {
                writeln!(out, "    builder.writeBool('{key_lit}', {expr});").unwrap()
            }
            HybridType::U8
            | HybridType::U16
            | HybridType::Usize
            | HybridType::I32
            | HybridType::U32
            | HybridType::I64
            | HybridType::U64 => {
                writeln!(out, "    builder.writeInt64('{key_lit}', {expr});").unwrap()
            }
            HybridType::Enum(_) => {
                writeln!(out, "    builder.writeInt64('{key_lit}', {expr}?.rawValue);").unwrap()
            }
            HybridType::F32 | HybridType::F64 => {
                writeln!(out, "    builder.writeDouble('{key_lit}', {expr});").unwrap()
            }
            HybridType::String => {
                writeln!(out, "    builder.writeString('{key_lit}', {expr});").unwrap()
            }
            HybridType::Bytes => {
                writeln!(out, "    builder.writeBytes('{key_lit}', {expr});").unwrap()
            }
            other => {
                writeln!(out, "    final _opt_{expr} = {expr};").unwrap();
                writeln!(out, "    if (_opt_{expr} == null) {{").unwrap();
                writeln!(out, "      builder.writeNull('{key_lit}');").unwrap();
                writeln!(out, "    }} else {{").unwrap();
                writeln!(out, "      final _bw = NveBinWriter();").unwrap();
                emit_dart_bin_write(out, "      ", "_bw", &format!("_opt_{expr}"), other);
                writeln!(out, "      builder.writeBytes('{key_lit}', _bw.takeBytes());").unwrap();
                writeln!(out, "    }}").unwrap();
            }
        },
        other => {
            writeln!(out, "    {{").unwrap();
            writeln!(out, "      final _bw = NveBinWriter();").unwrap();
            emit_dart_bin_write(out, "      ", "_bw", expr, other);
            writeln!(out, "      builder.writeBytes('{key_lit}', _bw.takeBytes());").unwrap();
            writeln!(out, "    }}").unwrap();
        }
    }
}

pub(super) fn emit_dart_set_props(out: &mut String, pascal: &str, _v: &HybridViewSpec) {
    writeln!(out, "  @override").unwrap();
    writeln!(out, "  void setProps({pascal}Props props) {{").unwrap();
    writeln!(out, "    final st = NveDirectPropsWriter.withEncodedProps(").unwrap();
    writeln!(out, "      props,").unwrap();
    writeln!(out, "      (entriesPtr, count) => _setPropsDirect(_handle, entriesPtr, count),").unwrap();
    writeln!(out, "    );").unwrap();
    writeln!(out, "    if (st != nveHybridOk) throw StateError('setProps failed on {pascal}');").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_dart_client_method(
    out: &mut String,
    pascal: &str,
    m: &HybridMethodSpec,
    spec: &HybridFileSpec,
) {
    let mp = to_pascal_case(&m.name);
    let mfield = dart_ident(&m.name);

    writeln!(out, "  @override").unwrap();
    writeln!(out, "  {} {{", dart_method_sig(m)).unwrap();
    writeln!(
        out,
        "    final fn = _vtable.ref.{mfield}.asFunction<_{pascal}{mp}Dart>();"
    )
    .unwrap();
    writeln!(out, "    final tempBufs = <NveHybridOwnedBytes>[];").unwrap();
    writeln!(out, "    try {{").unwrap();

    let mut args = vec!["_handle".to_string()];
    for (idx, p) in m.params.iter().enumerate() {
        let pname = dart_ident(&p.name);
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
                writeln!(out, "      final _b{idx} = _copyToNative(utf8.encode({pname}));").unwrap();
                writeln!(out, "      tempBufs.add(_b{idx});").unwrap();
                writeln!(out, "      final _s{idx} = ffi.Struct.create<NveStrSlice>()..ptr = _b{idx}.ptr..len = _b{idx}.len;").unwrap();
                args.push(format!("_s{idx}"));
            }
            HybridType::Bytes => {
                writeln!(out, "      final _b{idx} = _copyToNative({pname});").unwrap();
                writeln!(out, "      tempBufs.add(_b{idx});").unwrap();
                writeln!(out, "      final _s{idx} = ffi.Struct.create<NveBufferSlice>()..ptr = _b{idx}.ptr..len = _b{idx}.len;").unwrap();
                args.push(format!("_s{idx}"));
            }
            HybridType::Value => {
                writeln!(out, "      final _b{idx} = _copyToNative({pname}.binBytes);").unwrap();
                writeln!(out, "      tempBufs.add(_b{idx});").unwrap();
                writeln!(out, "      final _v{idx} = ffi.Struct.create<NveValue>();").unwrap();
                writeln!(out, "      _v{idx}.tag = 5;").unwrap();
                writeln!(out, "      _v{idx}.data.bytesVal.ptr = _b{idx}.ptr;").unwrap();
                writeln!(out, "      _v{idx}.data.bytesVal.len = _b{idx}.len;").unwrap();
                args.push(format!("_v{idx}"));
            }
            HybridType::Optional(inner) if inner.is_pod(spec) => {
                let suffix = inner.pod_opt_suffix().unwrap();
                writeln!(out, "      final _o{idx} = ffi.Struct.create<NveHybridOpt{suffix}>();").unwrap();
                writeln!(out, "      if ({pname} != null) {{").unwrap();
                writeln!(out, "        _o{idx}.isSome = true;").unwrap();
                match inner.as_ref() {
                    HybridType::Enum(_) => writeln!(out, "        _o{idx}.value = {pname}.rawValue;").unwrap(),
                    HybridType::Struct(_) => writeln!(out, "        _o{idx}.value = {pname}.toAbi();").unwrap(),
                    _ => writeln!(out, "        _o{idx}.value = {pname};").unwrap(),
                }
                writeln!(out, "      }}").unwrap();
                args.push(format!("_o{idx}"));
            }
            other => {
                writeln!(out, "      final _w{idx} = NveBinWriter();").unwrap();
                emit_dart_bin_write(out, "      ", &format!("_w{idx}"), &pname, other);
                writeln!(out, "      final _b{idx} = _copyToNative(_w{idx}.takeBytes());").unwrap();
                writeln!(out, "      tempBufs.add(_b{idx});").unwrap();
                writeln!(out, "      final _s{idx} = ffi.Struct.create<NveBufferSlice>()..ptr = _b{idx}.ptr..len = _b{idx}.len;").unwrap();
                args.push(format!("_s{idx}"));
            }
        }
    }

    match &m.return_type {
        HybridType::Void => {
            writeln!(out, "      fn({});", args.join(", ")).unwrap();
        }
        HybridType::Result { ok, .. } => {
            let has_ok = !matches!(ok.as_ref(), HybridType::Void);
            if has_ok {
                let ok_native = dart_native_value_type(ok, spec);
                writeln!(out, "      final _okSlot = _allocBytes(ffi.sizeOf<{ok_native}>());").unwrap();
                writeln!(out, "      tempBufs.add(_okSlot);").unwrap();
                writeln!(out, "      final _okPtr = _okSlot.ptr.cast<{ok_native}>();").unwrap();
                args.push("_okPtr".to_string());
            }
            writeln!(out, "      final _errSlot = _allocBytes(ffi.sizeOf<NveHybridOwnedBytes>());").unwrap();
            writeln!(out, "      tempBufs.add(_errSlot);").unwrap();
            writeln!(out, "      final _errPtr = _errSlot.ptr.cast<NveHybridOwnedBytes>();").unwrap();
            args.push("_errPtr".to_string());

            writeln!(out, "      final _status = fn({});", args.join(", ")).unwrap();
            writeln!(out, "      if (_status != nveHybridOk) {{").unwrap();
            writeln!(out, "        final _msg = _takeString(_errPtr.ref);").unwrap();
            writeln!(out, "        throw StateError(_msg.isEmpty ? 'hybrid call failed' : _msg);").unwrap();
            writeln!(out, "      }}").unwrap();
            if has_ok {
                let read_expr = dart_read_ptr_expr("_okPtr", ok, spec);
                writeln!(out, "      final _rawOk = {read_expr};").unwrap();
                emit_dart_return_decode(out, "      ", "_rawOk", ok, spec);
            }
        }
        other => {
            writeln!(out, "      final _rawRet = fn({});", args.join(", ")).unwrap();
            emit_dart_return_decode(out, "      ", "_rawRet", other, spec);
        }
    }

    writeln!(out, "    }} finally {{").unwrap();
    writeln!(out, "      for (final b in tempBufs) {{ if (b.ptr != ffi.nullptr) _freeBytes(b); }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();
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
    out: &mut String,
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
        | HybridType::F64 => writeln!(out, "{indent}return {var};").unwrap(),
        HybridType::Enum(e) => writeln!(out, "{indent}return {e}.fromRaw({var});").unwrap(),
        HybridType::Struct(s) if ty.is_pod(spec) => {
            writeln!(out, "{indent}return {s}.fromAbi({var});").unwrap()
        }
        HybridType::String => writeln!(out, "{indent}return _takeString({var});").unwrap(),
        HybridType::Bytes => writeln!(out, "{indent}return _takeBytes({var});").unwrap(),
        HybridType::Value => {
            writeln!(out, "{indent}return NveHybridDynamicValue(_takeBytes({var}));").unwrap()
        }
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            writeln!(out, "{indent}if (!{var}.isSome) return null;").unwrap();
            match inner.as_ref() {
                HybridType::Enum(e) => {
                    writeln!(out, "{indent}return {e}.fromRaw({var}.value);").unwrap()
                }
                HybridType::Struct(s) => {
                    writeln!(out, "{indent}return {s}.fromAbi({var}.value);").unwrap()
                }
                _ => writeln!(out, "{indent}return {var}.value;").unwrap(),
            }
        }
        other => {
            writeln!(out, "{indent}final _r = NveBinReader(_takeBytes({var}));").unwrap();
            writeln!(out, "{indent}late final {} _out;", dart_type(other)).unwrap();
            emit_dart_bin_read(out, indent, "_r", "_out", other);
            writeln!(out, "{indent}return _out;").unwrap();
        }
    }
}

