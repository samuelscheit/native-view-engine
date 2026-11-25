use std::fmt::Write as _;

use super::*;

pub(super) fn emit_ts_enum(out: &mut String, e: &HybridEnumSpec) {
    writeln!(out, "export enum {} {{", e.name).unwrap();
    for v in &e.variants {
        writeln!(out, "  {} = {},", v.name, v.value).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_ts_struct(out: &mut String, s: &HybridStructSpec) {
    writeln!(out, "export interface {} {{", s.name).unwrap();
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        writeln!(out, "  {fname}: {};", ts_type(&f.ty)).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "export function encode{}(w: NveBinWriter, value: {}): void {{", s.name, s.name).unwrap();
    writeln!(out, "  w.writeMap({}, (mw) => {{", s.fields.len()).unwrap();
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        let key_lit = escape_ts_single_quoted(&f.name);
        writeln!(out, "    mw.writeMapKey('{key_lit}');").unwrap();
        emit_ts_bin_write(out, "    ", "mw", &format!("value.{fname}"), &f.ty);
    }
    writeln!(out, "  }});").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "export function decode{}(r: NveBinReader): {} {{", s.name, s.name).unwrap();
    writeln!(out, "  const out: Partial<{}> = {{}};", s.name).unwrap();
    writeln!(out, "  const count = r.beginMap();").unwrap();
    writeln!(out, "  for (let i = 0; i < count; i++) {{").unwrap();
    writeln!(out, "    switch (r.readMapKey()) {{").unwrap();
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        let key_lit = escape_ts_single_quoted(&f.name);
        writeln!(out, "      case '{key_lit}':").unwrap();
        emit_ts_bin_read(out, "        ", "r", &format!("out.{fname}"), &f.ty);
        writeln!(out, "        break;").unwrap();
    }
    writeln!(out, "      default:").unwrap();
    writeln!(out, "        r.skipValue();").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out, "  return out as {};", s.name).unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_ts_module(out: &mut String, m: &HybridModuleSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&m.name);
    let upper = to_snake_case(&m.name).to_ascii_uppercase();
    let snake = to_snake_case(&m.name);

    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_INTERFACE_ID = 0x{:08X} as const;",
        m.interface_id()
    )
    .unwrap();
    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_SCHEMA_HASH = 0x{:016X}n;",
        m.schema_hash(spec)
    )
    .unwrap();
    writeln!(out).unwrap();

    writeln!(out, "export interface Hybrid{pascal}Spec {{").unwrap();
    for method in &m.methods {
        writeln!(out, "  {};", ts_method_sig(method)).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    emit_ts_wasm_client(out, &pascal, &snake, &upper, &m.methods, None);
}

pub(super) fn emit_ts_view(out: &mut String, v: &HybridViewSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&v.name);
    let upper = to_snake_case(&v.name).to_ascii_uppercase();
    let snake = to_snake_case(&v.name);

    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_INTERFACE_ID = 0x{:08X} as const;",
        v.interface_id()
    )
    .unwrap();
    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_SCHEMA_HASH = 0x{:016X}n;",
        v.schema_hash(spec)
    )
    .unwrap();
    writeln!(out).unwrap();

    writeln!(out, "export interface {pascal}Props {{").unwrap();
    for p in &v.props {
        let pname = to_camel_case(&p.name);
        let opt = if p.default_value.is_some() || matches!(p.ty, HybridType::Optional(_)) {
            "?"
        } else {
            ""
        };
        writeln!(out, "  {pname}{opt}: {};", ts_type(&p.ty)).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    for ev in &v.events {
        let ev_pascal = to_pascal_case(&ev.name);
        writeln!(out, "export interface {pascal}{ev_pascal}Event {{").unwrap();
        for param in &ev.params {
            let pname = to_camel_case(&param.name);
            writeln!(out, "  {pname}: {};", ts_type(&param.ty)).unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    writeln!(out, "export interface Hybrid{pascal}Spec {{").unwrap();
    writeln!(out, "  setProps(props: {pascal}Props): void;").unwrap();
    for method in &v.methods {
        writeln!(out, "  {};", ts_method_sig(method)).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    emit_ts_wasm_client(out, &pascal, &snake, &upper, &v.methods, Some(v));
}

fn emit_ts_wasm_client(
    out: &mut String,
    pascal: &str,
    snake: &str,
    upper: &str,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
) {
    writeln!(
        out,
        "export interface {pascal}WasmExports extends NveHybridWasmBaseExports {{"
    )
    .unwrap();
    writeln!(out, "  nve_hybrid_{snake}_schema_hash(): bigint;").unwrap();
    writeln!(out, "  nve_hybrid_{snake}_create(): number;").unwrap();
    writeln!(out, "  nve_hybrid_{snake}_destroy(handle: number): void;").unwrap();
    if view.is_some() {
        writeln!(
            out,
            "  nve_hybrid_{snake}_set_props_bin(handle: number, ptr: number, len: number): number;"
        )
        .unwrap();
    }
    for m in methods {
        let msnake = to_snake_case(&m.name);
        writeln!(
            out,
            "  nve_hybrid_{snake}_{msnake}_bin(handle: number, reqPtr: number, reqLen: number, outLenPtr: number): number;"
        )
        .unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    writeln!(
        out,
        "export class {pascal}HybridClient implements Hybrid{pascal}Spec {{"
    )
    .unwrap();
    writeln!(out, "  static readonly INTERFACE_ID = NVE_HYBRID_{upper}_INTERFACE_ID;").unwrap();
    writeln!(out, "  static readonly SCHEMA_HASH = NVE_HYBRID_{upper}_SCHEMA_HASH;").unwrap();
    writeln!(out, "  private handle: number;").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  constructor(private readonly wasm: {pascal}WasmExports) {{").unwrap();
    writeln!(out, "    const actualHash = BigInt.asUintN(64, wasm.nve_hybrid_{snake}_schema_hash());").unwrap();
    writeln!(out, "    if (actualHash !== {pascal}HybridClient.SCHEMA_HASH) {{").unwrap();
    writeln!(out, "      throw new Error(`Hybrid schema hash mismatch for {pascal}: expected 0x${{{pascal}HybridClient.SCHEMA_HASH.toString(16)}}, got 0x${{actualHash.toString(16)}}`);").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    this.handle = wasm.nve_hybrid_{snake}_create();").unwrap();
    writeln!(out, "    if (this.handle === 0) throw new Error('failed to create {pascal} hybrid instance');").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  dispose(): void {{").unwrap();
    writeln!(out, "    if (this.handle !== 0) {{").unwrap();
    writeln!(out, "      this.wasm.nve_hybrid_{snake}_destroy(this.handle);").unwrap();
    writeln!(out, "      this.handle = 0;").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();

    if let Some(v) = view {
        writeln!(out, "  setProps(props: {pascal}Props): void {{").unwrap();
        writeln!(out, "    const w = new NveBinWriter();").unwrap();
        writeln!(out, "    const entries: Array<[string, () => void]> = [];").unwrap();
        for p in &v.props {
            let pname = to_camel_case(&p.name);
            let key_lit = escape_ts_single_quoted(&p.name);
            writeln!(out, "    if (props.{pname} !== undefined) {{").unwrap();
            writeln!(out, "      const _val = props.{pname};").unwrap();
            writeln!(out, "      entries.push(['{key_lit}', () => {{").unwrap();
            emit_ts_bin_write(out, "        ", "w", "_val", &p.ty);
            writeln!(out, "      }}]);").unwrap();
            writeln!(out, "    }}").unwrap();
        }
        writeln!(out, "    w.writeMap(entries.length, (mw) => {{").unwrap();
        writeln!(out, "      for (const [k, emit] of entries) {{ mw.writeMapKey(k); emit(); }}").unwrap();
        writeln!(out, "    }});").unwrap();
        writeln!(out, "    const payload = w.toUint8Array();").unwrap();
        writeln!(out, "    const ptr = this.copyIn(payload);").unwrap();
        writeln!(out, "    try {{").unwrap();
        writeln!(out, "      const st = this.wasm.nve_hybrid_{snake}_set_props_bin(this.handle, ptr, payload.byteLength);").unwrap();
        writeln!(out, "      if (st !== 0) throw new Error('setProps failed on {pascal}');").unwrap();
        writeln!(out, "    }} finally {{").unwrap();
        writeln!(out, "      if (ptr !== 0) this.wasm.nve_hybrid_free(ptr, payload.byteLength);").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "  }}").unwrap();
        writeln!(out).unwrap();
    }

    for m in methods {
        let msnake = to_snake_case(&m.name);
        writeln!(out, "  {} {{", ts_method_sig(m)).unwrap();
        writeln!(out, "    const w = new NveBinWriter();").unwrap();
        writeln!(out, "    w.writeArray({}, (aw) => {{", m.params.len()).unwrap();
        for p in &m.params {
            let pname = to_camel_case(&p.name);
            emit_ts_bin_write(out, "      ", "aw", &pname, &p.ty);
        }
        writeln!(out, "    }});").unwrap();
        writeln!(out, "    const req = w.toUint8Array();").unwrap();
        writeln!(out, "    const reqPtr = this.copyIn(req);").unwrap();
        writeln!(out, "    const outLenPtr = this.wasm.nve_hybrid_alloc(4);").unwrap();
        writeln!(out, "    try {{").unwrap();
        writeln!(
            out,
            "      const respPtr = this.wasm.nve_hybrid_{snake}_{msnake}_bin(this.handle, reqPtr, req.byteLength, outLenPtr);"
        )
        .unwrap();
        writeln!(out, "      const dv = new DataView(this.wasm.memory.buffer);").unwrap();
        writeln!(out, "      const respLen = dv.getUint32(outLenPtr, true);").unwrap();
        writeln!(out, "      const respBytes = respPtr !== 0 && respLen > 0 ? new Uint8Array(this.wasm.memory.buffer, respPtr, respLen).slice() : new Uint8Array(0);").unwrap();
        writeln!(out, "      if (respPtr !== 0 && respLen > 0) this.wasm.nve_hybrid_free(respPtr, respLen);").unwrap();

        match &m.return_type {
            HybridType::Void => {}
            HybridType::Result { ok, .. } => {
                writeln!(out, "      const r = new NveBinReader(respBytes);").unwrap();
                writeln!(out, "      const isOk = r.readBool();").unwrap();
                writeln!(out, "      if (!isOk) throw new Error(r.readString());").unwrap();
                if !matches!(ok.as_ref(), HybridType::Void) {
                    writeln!(out, "      let outVal!: {};", ts_type(ok)).unwrap();
                    emit_ts_bin_read(out, "      ", "r", "outVal", ok);
                    writeln!(out, "      return outVal;").unwrap();
                }
            }
            other => {
                writeln!(out, "      const r = new NveBinReader(respBytes);").unwrap();
                writeln!(out, "      let outVal!: {};", ts_type(other)).unwrap();
                emit_ts_bin_read(out, "      ", "r", "outVal", other);
                writeln!(out, "      return outVal;").unwrap();
            }
        }

        writeln!(out, "    }} finally {{").unwrap();
        writeln!(out, "      if (reqPtr !== 0) this.wasm.nve_hybrid_free(reqPtr, req.byteLength);").unwrap();
        writeln!(out, "      if (outLenPtr !== 0) this.wasm.nve_hybrid_free(outLenPtr, 4);").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "  }}").unwrap();
        writeln!(out).unwrap();
    }

    writeln!(out, "  private copyIn(bytes: Uint8Array): number {{").unwrap();
    writeln!(out, "    if (bytes.byteLength === 0) return 0;").unwrap();
    writeln!(out, "    const ptr = this.wasm.nve_hybrid_alloc(bytes.byteLength);").unwrap();
    writeln!(out, "    new Uint8Array(this.wasm.memory.buffer, ptr, bytes.byteLength).set(bytes);").unwrap();
    writeln!(out, "    return ptr;").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
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

fn emit_ts_bin_write(out: &mut String, indent: &str, w: &str, expr: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr});").unwrap(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::Enum(_) => {
            writeln!(out, "{indent}{w}.writeI64(BigInt({expr}));").unwrap()
        }
        HybridType::I64 | HybridType::U64 => writeln!(out, "{indent}{w}.writeI64({expr});").unwrap(),
        HybridType::F32 | HybridType::F64 => writeln!(out, "{indent}{w}.writeF64({expr});").unwrap(),
        HybridType::String => writeln!(out, "{indent}{w}.writeString({expr});").unwrap(),
        HybridType::Bytes => writeln!(out, "{indent}{w}.writeBytes({expr});").unwrap(),
        HybridType::Struct(s) => writeln!(out, "{indent}encode{s}({w}, {expr});").unwrap(),
        HybridType::Value => writeln!(out, "{indent}{w}.writeRaw({expr}.binBytes);").unwrap(),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({expr} !== null && {expr} !== undefined) {{").unwrap();
            emit_ts_bin_write(out, &format!("{indent}  "), w, expr, inner);
            writeln!(out, "{indent}}} else {{ {w}.writeNull(); }}").unwrap();
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{w}.writeArray({expr}.length, (aw) => {{").unwrap();
            writeln!(out, "{indent}  for (const _el of {expr}) {{").unwrap();
            emit_ts_bin_write(out, &format!("{indent}    "), "aw", "_el", inner);
            writeln!(out, "{indent}  }}").unwrap();
            writeln!(out, "{indent}}});").unwrap();
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {}
    }
}

fn emit_ts_bin_read(out: &mut String, indent: &str, r: &str, dst: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.readBool();").unwrap(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32 => {
            writeln!(out, "{indent}{dst} = Number({r}.readI64());").unwrap()
        }
        HybridType::I64 | HybridType::U64 => writeln!(out, "{indent}{dst} = {r}.readI64();").unwrap(),
        HybridType::F32 | HybridType::F64 => writeln!(out, "{indent}{dst} = {r}.readF64();").unwrap(),
        HybridType::Enum(e) => {
            writeln!(out, "{indent}{dst} = Number({r}.readI64()) as {e};").unwrap()
        }
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.readString();").unwrap(),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = {r}.readBytes();").unwrap(),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = decode{s}({r});").unwrap(),
        HybridType::Value => {
            writeln!(out, "{indent}{dst} = {{ binBytes: {r}.readRawValue() }};").unwrap()
        }
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({r}.readNullIfPresent()) {{").unwrap();
            writeln!(out, "{indent}  {dst} = null;").unwrap();
            writeln!(out, "{indent}}} else {{").unwrap();
            writeln!(out, "{indent}  let _ov!: {};", ts_type(inner)).unwrap();
            emit_ts_bin_read(out, &format!("{indent}  "), r, "_ov", inner);
            writeln!(out, "{indent}  {dst} = _ov;").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{{").unwrap();
            writeln!(out, "{indent}  const _n = {r}.beginArray();").unwrap();
            writeln!(out, "{indent}  const _arr: Array<{}> = [];", ts_type(inner)).unwrap();
            writeln!(out, "{indent}  for (let _i = 0; _i < _n; _i++) {{").unwrap();
            writeln!(out, "{indent}    let _el!: {};", ts_type(inner)).unwrap();
            emit_ts_bin_read(out, &format!("{indent}    "), r, "_el", inner);
            writeln!(out, "{indent}    _arr.push(_el);").unwrap();
            writeln!(out, "{indent}  }}").unwrap();
            writeln!(out, "{indent}  {dst} = _arr;").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {}
    }
}
