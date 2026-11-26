use crate::writer::CodeWriter;

use super::*;

pub(super) fn emit_ts_enum(out: &mut CodeWriter, e: &HybridEnumSpec) {
    writeln!(out, "export enum {} {{", e.name);
    for v in &e.variants {
        writeln!(out, "  {} = {},", v.name, v.value);
    }
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_ts_struct(out: &mut CodeWriter, s: &HybridStructSpec) {
    writeln!(out, "export interface {} {{", s.name);
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        writeln!(out, "  {fname}: {};", ts_type(&f.ty));
    }
    writeln!(out, "}}");
    writeln!(out);

    writeln!(out, "export function encode{}(w: NveBinWriter, value: {}): void {{", s.name, s.name);
    writeln!(out, "  w.writeMap({}, (mw) => {{", s.fields.len());
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        let key_lit = escape_ts_single_quoted(&f.name);
        writeln!(out, "    mw.writeMapKey('{key_lit}');");
        emit_ts_bin_write(out, "    ", "mw", &format!("value.{fname}"), &f.ty);
    }
    writeln!(out, "  }});");
    writeln!(out, "}}");
    writeln!(out);

    writeln!(out, "export function decode{}(r: NveBinReader): {} {{", s.name, s.name);
    writeln!(out, "  const out: Partial<{}> = {{}};", s.name);
    writeln!(out, "  const count = r.beginMap();");
    writeln!(out, "  for (let i = 0; i < count; i++) {{");
    writeln!(out, "    switch (r.readMapKey()) {{");
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        let key_lit = escape_ts_single_quoted(&f.name);
        writeln!(out, "      case '{key_lit}':");
        emit_ts_bin_read(out, "        ", "r", &format!("out.{fname}"), &f.ty);
        writeln!(out, "        break;");
    }
    writeln!(out, "      default:");
    writeln!(out, "        r.skipValue();");
    writeln!(out, "    }}");
    writeln!(out, "  }}");
    writeln!(out, "  return out as {};", s.name);
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_ts_module(out: &mut CodeWriter, m: &HybridModuleSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&m.name);
    let upper = to_snake_case(&m.name).to_ascii_uppercase();
    let snake = to_snake_case(&m.name);

    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_INTERFACE_ID = 0x{:08X} as const;",
        m.interface_id()
    );
    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_SCHEMA_HASH = 0x{:016X}n;",
        m.schema_hash(spec)
    );
    writeln!(out);

    writeln!(out, "export interface Hybrid{pascal}Spec {{");
    for method in &m.methods {
        writeln!(out, "  {};", ts_method_sig(method));
    }
    writeln!(out, "}}");
    writeln!(out);

    emit_ts_wasm_client(out, &pascal, &snake, &upper, &m.methods, None);
}

pub(super) fn emit_ts_view(out: &mut CodeWriter, v: &HybridViewSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&v.name);
    let upper = to_snake_case(&v.name).to_ascii_uppercase();
    let snake = to_snake_case(&v.name);

    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_INTERFACE_ID = 0x{:08X} as const;",
        v.interface_id()
    );
    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_SCHEMA_HASH = 0x{:016X}n;",
        v.schema_hash(spec)
    );
    writeln!(out);

    writeln!(out, "export interface {pascal}Props {{");
    for p in &v.props {
        let pname = to_camel_case(&p.name);
        let opt = if p.default_value.is_some() || matches!(p.ty, HybridType::Optional(_)) {
            "?"
        } else {
            ""
        };
        writeln!(out, "  {pname}{opt}: {};", ts_type(&p.ty));
    }
    writeln!(out, "}}");
    writeln!(out);

    for ev in &v.events {
        let ev_pascal = to_pascal_case(&ev.name);
        writeln!(out, "export interface {pascal}{ev_pascal}Event {{");
        for param in &ev.params {
            let pname = to_camel_case(&param.name);
            writeln!(out, "  {pname}: {};", ts_type(&param.ty));
        }
        writeln!(out, "}}");
        writeln!(out);
    }

    writeln!(out, "export interface Hybrid{pascal}Spec {{");
    writeln!(out, "  setProps(props: {pascal}Props): void;");
    for method in &v.methods {
        writeln!(out, "  {};", ts_method_sig(method));
    }
    writeln!(out, "}}");
    writeln!(out);

    emit_ts_wasm_client(out, &pascal, &snake, &upper, &v.methods, Some(v));
}

fn emit_ts_wasm_client(
    out: &mut CodeWriter,
    pascal: &str,
    snake: &str,
    upper: &str,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
) {
    writeln!(
        out,
        "export interface {pascal}WasmExports extends NveHybridWasmBaseExports {{"
    );
    writeln!(out, "  nve_hybrid_{snake}_schema_hash(): bigint;");
    writeln!(out, "  nve_hybrid_{snake}_create(): number;");
    writeln!(out, "  nve_hybrid_{snake}_destroy(handle: number): void;");
    if view.is_some() {
        writeln!(
            out,
            "  nve_hybrid_{snake}_set_props_bin(handle: number, ptr: number, len: number): number;"
        );
    }
    for m in methods {
        let msnake = to_snake_case(&m.name);
        writeln!(
            out,
            "  nve_hybrid_{snake}_{msnake}_bin(handle: number, reqPtr: number, reqLen: number, outLenPtr: number): number;"
        );
    }
    writeln!(out, "}}");
    writeln!(out);

    writeln!(
        out,
        "export class {pascal}HybridClient implements Hybrid{pascal}Spec {{"
    );
    writeln!(out, "  static readonly INTERFACE_ID = NVE_HYBRID_{upper}_INTERFACE_ID;");
    writeln!(out, "  static readonly SCHEMA_HASH = NVE_HYBRID_{upper}_SCHEMA_HASH;");
    writeln!(out, "  private handle: number;");
    writeln!(out);
    writeln!(out, "  constructor(private readonly wasm: {pascal}WasmExports) {{");
    writeln!(out, "    const actualHash = BigInt.asUintN(64, wasm.nve_hybrid_{snake}_schema_hash());");
    writeln!(out, "    if (actualHash !== {pascal}HybridClient.SCHEMA_HASH) {{");
    writeln!(out, "      throw new Error(`Hybrid schema hash mismatch for {pascal}: expected 0x${{{pascal}HybridClient.SCHEMA_HASH.toString(16)}}, got 0x${{actualHash.toString(16)}}`);");
    writeln!(out, "    }}");
    writeln!(out, "    this.handle = wasm.nve_hybrid_{snake}_create();");
    writeln!(out, "    if (this.handle === 0) throw new Error('failed to create {pascal} hybrid instance');");
    writeln!(out, "  }}");
    writeln!(out);
    writeln!(out, "  dispose(): void {{");
    writeln!(out, "    if (this.handle !== 0) {{");
    writeln!(out, "      this.wasm.nve_hybrid_{snake}_destroy(this.handle);");
    writeln!(out, "      this.handle = 0;");
    writeln!(out, "    }}");
    writeln!(out, "  }}");
    writeln!(out);

    if let Some(v) = view {
        writeln!(out, "  setProps(props: {pascal}Props): void {{");
        writeln!(out, "    const w = new NveBinWriter();");
        writeln!(out, "    const entries: Array<[string, () => void]> = [];");
        for p in &v.props {
            let pname = to_camel_case(&p.name);
            let key_lit = escape_ts_single_quoted(&p.name);
            writeln!(out, "    if (props.{pname} !== undefined) {{");
            writeln!(out, "      const _val = props.{pname};");
            writeln!(out, "      entries.push(['{key_lit}', () => {{");
            emit_ts_bin_write(out, "        ", "w", "_val", &p.ty);
            writeln!(out, "      }}]);");
            writeln!(out, "    }}");
        }
        writeln!(out, "    w.writeMap(entries.length, (mw) => {{");
        writeln!(out, "      for (const [k, emit] of entries) {{ mw.writeMapKey(k); emit(); }}");
        writeln!(out, "    }});");
        writeln!(out, "    const payload = w.toUint8Array();");
        writeln!(out, "    const ptr = this.copyIn(payload);");
        writeln!(out, "    try {{");
        writeln!(out, "      const st = this.wasm.nve_hybrid_{snake}_set_props_bin(this.handle, ptr, payload.byteLength);");
        writeln!(out, "      if (st !== 0) throw new Error('setProps failed on {pascal}');");
        writeln!(out, "    }} finally {{");
        writeln!(out, "      if (ptr !== 0) this.wasm.nve_hybrid_free(ptr, payload.byteLength);");
        writeln!(out, "    }}");
        writeln!(out, "  }}");
        writeln!(out);
    }

    for m in methods {
        let msnake = to_snake_case(&m.name);
        writeln!(out, "  {} {{", ts_method_sig(m));
        writeln!(out, "    const w = new NveBinWriter();");
        writeln!(out, "    w.writeArray({}, (aw) => {{", m.params.len());
        for p in &m.params {
            let pname = to_camel_case(&p.name);
            emit_ts_bin_write(out, "      ", "aw", &pname, &p.ty);
        }
        writeln!(out, "    }});");
        writeln!(out, "    const req = w.toUint8Array();");
        writeln!(out, "    const reqPtr = this.copyIn(req);");
        writeln!(out, "    const outLenPtr = this.wasm.nve_hybrid_alloc(4);");
        writeln!(out, "    try {{");
        writeln!(
            out,
            "      const respPtr = this.wasm.nve_hybrid_{snake}_{msnake}_bin(this.handle, reqPtr, req.byteLength, outLenPtr);"
        );
        writeln!(out, "      const dv = new DataView(this.wasm.memory.buffer);");
        writeln!(out, "      const respLen = dv.getUint32(outLenPtr, true);");
        writeln!(out, "      const respBytes = respPtr !== 0 && respLen > 0 ? new Uint8Array(this.wasm.memory.buffer, respPtr, respLen).slice() : new Uint8Array(0);");
        writeln!(out, "      if (respPtr !== 0 && respLen > 0) this.wasm.nve_hybrid_free(respPtr, respLen);");

        match &m.return_type {
            HybridType::Void => {}
            HybridType::Result { ok, .. } => {
                writeln!(out, "      const r = new NveBinReader(respBytes);");
                writeln!(out, "      const isOk = r.readBool();");
                writeln!(out, "      if (!isOk) throw new Error(r.readString());");
                if !matches!(ok.as_ref(), HybridType::Void) {
                    writeln!(out, "      let outVal!: {};", ts_type(ok));
                    emit_ts_bin_read(out, "      ", "r", "outVal", ok);
                    writeln!(out, "      return outVal;");
                }
            }
            other => {
                writeln!(out, "      const r = new NveBinReader(respBytes);");
                writeln!(out, "      let outVal!: {};", ts_type(other));
                emit_ts_bin_read(out, "      ", "r", "outVal", other);
                writeln!(out, "      return outVal;");
            }
        }

        writeln!(out, "    }} finally {{");
        writeln!(out, "      if (reqPtr !== 0) this.wasm.nve_hybrid_free(reqPtr, req.byteLength);");
        writeln!(out, "      if (outLenPtr !== 0) this.wasm.nve_hybrid_free(outLenPtr, 4);");
        writeln!(out, "    }}");
        writeln!(out, "  }}");
        writeln!(out);
    }

    writeln!(out, "  private copyIn(bytes: Uint8Array): number {{");
    writeln!(out, "    if (bytes.byteLength === 0) return 0;");
    writeln!(out, "    const ptr = this.wasm.nve_hybrid_alloc(bytes.byteLength);");
    writeln!(out, "    new Uint8Array(this.wasm.memory.buffer, ptr, bytes.byteLength).set(bytes);");
    writeln!(out, "    return ptr;");
    writeln!(out, "  }}");
    writeln!(out, "}}");
    writeln!(out);
}

fn ts_method_sig(m: &HybridMethodSpec) -> String {
    let mname = to_camel_case(&m.name);
    let params: Vec<String> = m
        .params
        .iter()
        .map(|p| format!("{}: {}", to_camel_case(&p.name), ts_type(&p.ty)))
        .collect();
    let ret = match &m.return_type {
        HybridType::Result { ok, .. } => ts_type(ok),
        other => ts_type(other),
    };
    format!("{mname}({}): {ret}", params.join(", "))
}

fn ts_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "void".to_string(),
        HybridType::Bool => "boolean".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::F32
        | HybridType::F64
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => "number".to_string(),
        HybridType::I64 | HybridType::U64 => "bigint".to_string(),
        HybridType::String => "string".to_string(),
        HybridType::Bytes => "Uint8Array".to_string(),
        HybridType::Optional(inner) => format!("{} | null", ts_type(inner)),
        HybridType::Vec(inner) | HybridType::FixedArray { elem: inner, .. } => {
            format!("Array<{}>", ts_type(inner))
        }
        HybridType::Result { ok, .. } => ts_type(ok),
        HybridType::Struct(name) | HybridType::Enum(name) => name.clone(),
        HybridType::Value => "NveHybridDynamicValue".to_string(),
    }
}

fn emit_ts_bin_write(out: &mut CodeWriter, indent: &str, w: &str, expr: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr});"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::Enum(_) => {
            writeln!(out, "{indent}{w}.writeI64(BigInt({expr}));")
        }
        HybridType::I64 | HybridType::U64 => writeln!(out, "{indent}{w}.writeI64({expr});"),
        HybridType::F32 | HybridType::F64 => writeln!(out, "{indent}{w}.writeF64({expr});"),
        HybridType::String => writeln!(out, "{indent}{w}.writeString({expr});"),
        HybridType::Bytes => writeln!(out, "{indent}{w}.writeBytes({expr});"),
        HybridType::Struct(s) => writeln!(out, "{indent}encode{s}({w}, {expr});"),
        HybridType::Value => writeln!(out, "{indent}{w}.writeRaw({expr}.binBytes);"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({expr} !== null && {expr} !== undefined) {{");
            emit_ts_bin_write(out, &format!("{indent}  "), w, expr, inner);
            writeln!(out, "{indent}}} else {{ {w}.writeNull(); }}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{w}.writeArray({expr}.length, (aw) => {{");
            writeln!(out, "{indent}  for (const _el of {expr}) {{");
            emit_ts_bin_write(out, &format!("{indent}    "), "aw", "_el", inner);
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

fn emit_ts_bin_read(out: &mut CodeWriter, indent: &str, r: &str, dst: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.readBool();"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32 => {
            writeln!(out, "{indent}{dst} = Number({r}.readI64());")
        }
        HybridType::I64 | HybridType::U64 => writeln!(out, "{indent}{dst} = {r}.readI64();"),
        HybridType::F32 | HybridType::F64 => writeln!(out, "{indent}{dst} = {r}.readF64();"),
        HybridType::Enum(e) => {
            writeln!(out, "{indent}{dst} = Number({r}.readI64()) as {e};")
        }
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.readString();"),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = {r}.readBytes();"),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = decode{s}({r});"),
        HybridType::Value => {
            writeln!(out, "{indent}{dst} = {{ binBytes: {r}.readRawValue() }};")
        }
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({r}.readNullIfPresent()) {{");
            writeln!(out, "{indent}  {dst} = null;");
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  let _ov!: {};", ts_type(inner));
            emit_ts_bin_read(out, &format!("{indent}  "), r, "_ov", inner);
            writeln!(out, "{indent}  {dst} = _ov;");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  const _n = {r}.beginArray();");
            writeln!(out, "{indent}  const _arr: Array<{}> = [];", ts_type(inner));
            writeln!(out, "{indent}  for (let _i = 0; _i < _n; _i++) {{");
            writeln!(out, "{indent}    let _el!: {};", ts_type(inner));
            emit_ts_bin_read(out, &format!("{indent}    "), r, "_el", inner);
            writeln!(out, "{indent}    _arr.push(_el);");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  {dst} = _arr;");
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
