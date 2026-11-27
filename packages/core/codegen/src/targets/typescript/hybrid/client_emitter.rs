use crate::writer::CodeWriter;

use super::bin_emitter::{
    emit_ts_bin_read, emit_ts_bin_write, ts_method_sig, ts_param_ident, ts_type,
};
use super::*;

pub(super) fn emit_ts_enum(out: &mut CodeWriter, imports: &mut TsImports, e: &HybridEnumSpec) {
    imports.ty("BinReader");
    imports.value("HybridError");
    imports.value("HybridStatus");
    imports.value("readIntInRange");

    writeln!(out, "export const {} = {{", e.name);
    for v in &e.variants {
        writeln!(out, "  {}: {},", v.name, v.value);
    }
    writeln!(out, "}} as const;");
    writeln!(
        out,
        "export type {} = (typeof {})[keyof typeof {}];",
        e.name, e.name, e.name
    );
    writeln!(out);

    writeln!(out, "export function decode{}(r: BinReader): {} {{", e.name, e.name);
    writeln!(out, "  const raw = readIntInRange(r, 0, 0xffff_ffff);");
    writeln!(out, "  switch (raw) {{");
    for v in &e.variants {
        writeln!(out, "    case {}:", v.value);
    }
    writeln!(out, "      return raw;");
    writeln!(out, "    default:");
    writeln!(
        out,
        "      throw new HybridError(HybridStatus.InvalidArg, `invalid {} discriminant ${{raw}}`);",
        e.name
    );
    writeln!(out, "  }}");
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_ts_struct(out: &mut CodeWriter, imports: &mut TsImports, s: &HybridStructSpec) {
    imports.ty("BinWriter");
    imports.ty("BinReader");

    writeln!(out, "export interface {} {{", s.name);
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        writeln!(out, "  {fname}: {};", ts_type(&f.ty, imports));
    }
    writeln!(out, "}}");
    writeln!(out);

    writeln!(
        out,
        "export function encode{}(w: BinWriter, value: {}): void {{",
        s.name, s.name
    );
    writeln!(out, "  const _mark = w.beginMap({});", s.fields.len());
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        let key_lit = escape_ts_single_quoted(&f.name);
        writeln!(out, "  w.writeMapKey('{key_lit}');");
        emit_ts_bin_write(out, imports, "  ", "w", &format!("value.{fname}"), &f.ty);
    }
    writeln!(out, "  w.end(_mark);");
    writeln!(out, "}}");
    writeln!(out);

    writeln!(out, "export function decode{}(r: BinReader): {} {{", s.name, s.name);
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        let ty = ts_type(&f.ty, imports);
        if matches!(f.ty, HybridType::Optional(_)) {
            writeln!(out, "  let _f_{fname}: {ty} = null;");
        } else {
            imports.value("BinCodecError");
            writeln!(out, "  let _f_{fname}: {ty} | undefined;");
        }
    }
    writeln!(out, "  r.readMap((_count) => {{");
    writeln!(out, "    for (let _i = 0; _i < _count; _i += 1) {{");
    writeln!(out, "      switch (r.readMapKey()) {{");
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        let key_lit = escape_ts_single_quoted(&f.name);
        writeln!(out, "        case '{key_lit}':");
        emit_ts_bin_read(out, imports, "          ", "r", &format!("_f_{fname}"), &f.ty);
        writeln!(out, "          break;");
    }
    writeln!(out, "        default:");
    writeln!(out, "          r.skipValue();");
    writeln!(out, "      }}");
    writeln!(out, "    }}");
    writeln!(out, "  }});");
    for f in &s.fields {
        if !matches!(f.ty, HybridType::Optional(_)) {
            let fname = to_camel_case(&f.name);
            let key_lit = escape_ts_single_quoted(&f.name);
            writeln!(
                out,
                "  if (_f_{fname} === undefined) throw new BinCodecError('truncated', 'missing {}.{key_lit}');",
                s.name
            );
        }
    }
    writeln!(out, "  return {{");
    for f in &s.fields {
        let fname = to_camel_case(&f.name);
        writeln!(out, "    {fname}: _f_{fname},");
    }
    writeln!(out, "  }};");
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_ts_module(
    out: &mut CodeWriter,
    imports: &mut TsImports,
    m: &HybridModuleSpec,
    spec: &HybridFileSpec,
) {
    let pascal = to_pascal_case(&m.name);
    let upper = to_snake_case(&m.name).to_ascii_uppercase();
    let snake = to_snake_case(&m.name);

    imports.value("Key64");
    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_INTERFACE_ID = 0x{:08X} as const;",
        m.interface_id()
    );
    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_SCHEMA_HASH = {};",
        format_key64(m.schema_hash(spec))
    );
    writeln!(out);

    writeln!(out, "export interface Hybrid{pascal}Spec {{");
    for method in &m.methods {
        writeln!(out, "  {};", ts_method_sig(method, imports));
    }
    writeln!(out, "}}");
    writeln!(out);

    emit_ts_wasm_client(out, imports, &pascal, &snake, &upper, &m.methods, None);
}

pub(super) fn emit_ts_view(
    out: &mut CodeWriter,
    imports: &mut TsImports,
    v: &HybridViewSpec,
    spec: &HybridFileSpec,
) {
    let pascal = to_pascal_case(&v.name);
    let upper = to_snake_case(&v.name).to_ascii_uppercase();
    let snake = to_snake_case(&v.name);

    imports.value("Key64");
    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_INTERFACE_ID = 0x{:08X} as const;",
        v.interface_id()
    );
    writeln!(
        out,
        "export const NVE_HYBRID_{upper}_SCHEMA_HASH = {};",
        format_key64(v.schema_hash(spec))
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
        writeln!(out, "  {pname}{opt}: {};", ts_type(&p.ty, imports));
    }
    writeln!(out, "}}");
    writeln!(out);

    for ev in &v.events {
        let ev_pascal = to_pascal_case(&ev.name);
        writeln!(out, "export interface {pascal}{ev_pascal}Event {{");
        for param in &ev.params {
            let pname = to_camel_case(&param.name);
            writeln!(out, "  {pname}: {};", ts_type(&param.ty, imports));
        }
        writeln!(out, "}}");
        writeln!(out);
    }

    writeln!(out, "export interface Hybrid{pascal}Spec {{");
    writeln!(out, "  setProps(props: {pascal}Props): void;");
    for method in &v.methods {
        writeln!(out, "  {};", ts_method_sig(method, imports));
    }
    writeln!(out, "}}");
    writeln!(out);

    emit_ts_wasm_client(out, imports, &pascal, &snake, &upper, &v.methods, Some(v));
}

fn emit_ts_wasm_client(
    out: &mut CodeWriter,
    imports: &mut TsImports,
    pascal: &str,
    snake: &str,
    upper: &str,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
) {
    imports.ty("HybridWasmExports");
    imports.value("wasmCreateHandle");

    writeln!(
        out,
        "export interface {pascal}WasmExports extends HybridWasmExports {{"
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
    writeln!(
        out,
        "  static readonly INTERFACE_ID = NVE_HYBRID_{upper}_INTERFACE_ID;"
    );
    writeln!(
        out,
        "  static readonly SCHEMA_HASH = NVE_HYBRID_{upper}_SCHEMA_HASH;"
    );
    writeln!(out, "  private readonly wasm: {pascal}WasmExports;");
    writeln!(out, "  private handle: number;");
    writeln!(out);
    writeln!(out, "  constructor(wasm: {pascal}WasmExports) {{");
    writeln!(out, "    this.wasm = wasm;");
    writeln!(out, "    this.handle = wasmCreateHandle(");
    writeln!(out, "      '{pascal}',");
    writeln!(out, "      {pascal}HybridClient.SCHEMA_HASH,");
    writeln!(out, "      wasm.nve_hybrid_{snake}_schema_hash(),");
    writeln!(out, "      () => wasm.nve_hybrid_{snake}_create(),");
    writeln!(out, "    );");
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
        imports.value("BinWriter");
        imports.value("wasmSetPropsBin");
        if v.props.is_empty() {
            writeln!(out, "  setProps(_props: {pascal}Props): void {{");
            writeln!(out, "    const _nve_w = new BinWriter();");
            writeln!(out, "    const _nve_mark = _nve_w.beginMap(0);");
            writeln!(out, "    _nve_w.end(_nve_mark);");
        } else {
            writeln!(out, "  setProps(props: {pascal}Props): void {{");
            writeln!(out, "    let _nve_count = 0;");
            for p in &v.props {
                let pname = to_camel_case(&p.name);
                writeln!(out, "    if (props.{pname} !== undefined) _nve_count += 1;");
            }
            writeln!(out, "    const _nve_w = new BinWriter();");
            writeln!(out, "    const _nve_mark = _nve_w.beginMap(_nve_count);");
            for p in &v.props {
                let pname = to_camel_case(&p.name);
                let key_lit = escape_ts_single_quoted(&p.name);
                writeln!(out, "    if (props.{pname} !== undefined) {{");
                writeln!(out, "      _nve_w.writeMapKey('{key_lit}');");
                emit_ts_bin_write(
                    out,
                    imports,
                    "      ",
                    "_nve_w",
                    &format!("props.{pname}"),
                    &p.ty,
                );
                writeln!(out, "    }}");
            }
            writeln!(out, "    _nve_w.end(_nve_mark);");
        }
        writeln!(out, "    wasmSetPropsBin(");
        writeln!(out, "      this.wasm,");
        writeln!(out, "      '{pascal}',");
        writeln!(out, "      this.handle,");
        writeln!(out, "      _nve_w.bytes,");
        writeln!(
            out,
            "      (h, p, l) => this.wasm.nve_hybrid_{snake}_set_props_bin(h, p, l),"
        );
        writeln!(out, "    );");
        writeln!(out, "  }}");
        writeln!(out);
    }

    for m in methods {
        imports.value("BinWriter");
        imports.value("wasmCallBin");
        let msnake = to_snake_case(&m.name);
        writeln!(out, "  {} {{", ts_method_sig(m, imports));
        writeln!(out, "    const _nve_w = new BinWriter();");
        writeln!(out, "    const _nve_mark = _nve_w.beginArray({});", m.params.len());
        for p in &m.params {
            let pname = ts_param_ident(&p.name);
            emit_ts_bin_write(out, imports, "    ", "_nve_w", &pname, &p.ty);
        }
        writeln!(out, "    _nve_w.end(_nve_mark);");

        let has_resp = !matches!(m.return_type, HybridType::Void);
        if has_resp {
            writeln!(out, "    const _nve_resp = wasmCallBin(");
        } else {
            writeln!(out, "    wasmCallBin(");
        }
        writeln!(out, "      this.wasm,");
        writeln!(out, "      '{pascal}',");
        writeln!(out, "      this.handle,");
        writeln!(out, "      _nve_w.bytes,");
        writeln!(
            out,
            "      (h, rp, rl, olp) => this.wasm.nve_hybrid_{snake}_{msnake}_bin(h, rp, rl, olp),"
        );
        writeln!(out, "    );");

        match &m.return_type {
            HybridType::Void => {}
            HybridType::Result { ok, .. } => {
                imports.value("BinReader");
                imports.value("readResultOk");
                writeln!(out, "    const _nve_r = new BinReader(_nve_resp);");
                writeln!(out, "    readResultOk(_nve_r);");
                if matches!(ok.as_ref(), HybridType::Void) {
                    writeln!(out, "    _nve_r.requireEnd();");
                } else {
                    writeln!(out, "    let _nve_out!: {};", ts_type(ok, imports));
                    emit_ts_bin_read(out, imports, "    ", "_nve_r", "_nve_out", ok);
                    writeln!(out, "    _nve_r.requireEnd();");
                    writeln!(out, "    return _nve_out;");
                }
            }
            other => {
                imports.value("BinReader");
                writeln!(out, "    const _nve_r = new BinReader(_nve_resp);");
                writeln!(out, "    let _nve_out!: {};", ts_type(other, imports));
                emit_ts_bin_read(out, imports, "    ", "_nve_r", "_nve_out", other);
                writeln!(out, "    _nve_r.requireEnd();");
                writeln!(out, "    return _nve_out;");
            }
        }
        writeln!(out, "  }}");
        writeln!(out);
    }

    writeln!(out, "}}");
    writeln!(out);
}
