//! TypeScript `NveBinValue` encoders, decoders, and type helpers over `@nve/host`'s
//! `BinWriter` / `BinReader` / `DynamicValue`.

use crate::writer::CodeWriter;

use super::*;

pub(super) fn ts_param_ident(raw: &str) -> String {
    let ident = to_camel_case(raw);
    match ident.as_str() {
        "await" | "break" | "case" | "catch" | "class" | "const" | "continue" | "debugger"
        | "default" | "delete" | "do" | "else" | "enum" | "export" | "extends" | "false"
        | "finally" | "for" | "function" | "if" | "implements" | "import" | "in"
        | "instanceof" | "interface" | "let" | "new" | "null" | "package" | "private"
        | "protected" | "public" | "return" | "static" | "super" | "switch" | "this" | "throw"
        | "true" | "try" | "typeof" | "var" | "void" | "while" | "with" | "yield" => {
            format!("{ident}_")
        }
        _ => ident,
    }
}

pub(super) fn ts_method_sig(m: &HybridMethodSpec, imports: &mut TsImports) -> String {
    let mname = to_camel_case(&m.name);
    let params: Vec<String> = m
        .params
        .iter()
        .map(|p| format!("{}: {}", ts_param_ident(&p.name), ts_type(&p.ty, imports)))
        .collect();
    let ret = match &m.return_type {
        HybridType::Result { ok, .. } => ts_type(ok, imports),
        other => ts_type(other, imports),
    };
    format!("{mname}({}): {ret}", params.join(", "))
}

pub(super) fn ts_type(ty: &HybridType, imports: &mut TsImports) -> String {
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
        HybridType::Optional(inner) => format!("{} | null", ts_type(inner, imports)),
        HybridType::Vec(inner) | HybridType::FixedArray { elem: inner, .. } => {
            format!("Array<{}>", ts_type(inner, imports))
        }
        HybridType::Result { ok, .. } => ts_type(ok, imports),
        HybridType::Struct(name) | HybridType::Enum(name) => name.clone(),
        HybridType::Value => {
            imports.ty("DynamicValue");
            "DynamicValue".to_string()
        }
    }
}

pub(super) fn emit_ts_bin_write(
    out: &mut CodeWriter,
    imports: &mut TsImports,
    indent: &str,
    w: &str,
    expr: &str,
    ty: &HybridType,
) {
    let d = indent.len();
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr});"),
        HybridType::U8 => {
            imports.value("writeIntInRange");
            writeln!(out, "{indent}writeIntInRange({w}, {expr}, 0, 255);");
        }
        HybridType::U16 => {
            imports.value("writeIntInRange");
            writeln!(out, "{indent}writeIntInRange({w}, {expr}, 0, 65535);");
        }
        HybridType::I32 => {
            imports.value("writeIntInRange");
            writeln!(out, "{indent}writeIntInRange({w}, {expr}, -2147483648, 2147483647);");
        }
        HybridType::U32
        | HybridType::Usize
        | HybridType::Enum(_)
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => {
            imports.value("writeIntInRange");
            writeln!(out, "{indent}writeIntInRange({w}, {expr}, 0, 0xffff_ffff);");
        }
        HybridType::I64 => {
            imports.value("writeI64Checked");
            writeln!(out, "{indent}writeI64Checked({w}, {expr});");
        }
        HybridType::U64 => {
            imports.value("writeU64Checked");
            writeln!(out, "{indent}writeU64Checked({w}, {expr});");
        }
        HybridType::F32 | HybridType::F64 => writeln!(out, "{indent}{w}.writeF64({expr});"),
        HybridType::String => writeln!(out, "{indent}{w}.writeStr({expr});"),
        HybridType::Bytes => writeln!(out, "{indent}{w}.writeBytes({expr});"),
        HybridType::Struct(s) => writeln!(out, "{indent}encode{s}({w}, {expr});"),
        // Preserves the true `NveValue` tag directly (never double-wrapped in `Bytes`).
        HybridType::Value => writeln!(out, "{indent}{expr}.writeTo({w});"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({expr} !== null && {expr} !== undefined) {{");
            emit_ts_bin_write(out, imports, &format!("{indent}  "), w, expr, inner);
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  {w}.writeNull();");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}const _mark{d} = {w}.beginArray({expr}.length);");
            writeln!(out, "{indent}for (const _el{d} of {expr}) {{");
            emit_ts_bin_write(
                out,
                imports,
                &format!("{indent}  "),
                w,
                &format!("_el{d}"),
                inner,
            );
            writeln!(out, "{indent}}}");
            writeln!(out, "{indent}{w}.end(_mark{d});");
        }
        HybridType::FixedArray { elem: inner, len } => {
            writeln!(
                out,
                "{indent}if ({expr}.length !== {len}) throw new RangeError(`expected fixed array of length {len}, got ${{{expr}.length}}`);"
            );
            writeln!(out, "{indent}const _mark{d} = {w}.beginArray({len});");
            writeln!(out, "{indent}for (const _el{d} of {expr}) {{");
            emit_ts_bin_write(
                out,
                imports,
                &format!("{indent}  "),
                w,
                &format!("_el{d}"),
                inner,
            );
            writeln!(out, "{indent}}}");
            writeln!(out, "{indent}{w}.end(_mark{d});");
        }
        HybridType::Void | HybridType::Result { .. } => {
            writeln!(out, "{indent}{w}.writeNull();");
        }
    }
}

pub(super) fn emit_ts_bin_read(
    out: &mut CodeWriter,
    imports: &mut TsImports,
    indent: &str,
    r: &str,
    dst: &str,
    ty: &HybridType,
) {
    let d = indent.len();
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.readBool();"),
        HybridType::U8 => {
            imports.value("readIntInRange");
            writeln!(out, "{indent}{dst} = readIntInRange({r}, 0, 255);");
        }
        HybridType::U16 => {
            imports.value("readIntInRange");
            writeln!(out, "{indent}{dst} = readIntInRange({r}, 0, 65535);");
        }
        HybridType::I32 => {
            imports.value("readIntInRange");
            writeln!(out, "{indent}{dst} = readIntInRange({r}, -2147483648, 2147483647);");
        }
        HybridType::U32
        | HybridType::Usize
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => {
            imports.value("readIntInRange");
            writeln!(out, "{indent}{dst} = readIntInRange({r}, 0, 0xffff_ffff);");
        }
        HybridType::I64 => writeln!(out, "{indent}{dst} = {r}.readI64();"),
        HybridType::U64 => writeln!(out, "{indent}{dst} = {r}.readI64Bits().toBigInt();"),
        HybridType::F32 | HybridType::F64 => writeln!(out, "{indent}{dst} = {r}.readF64();"),
        HybridType::Enum(e) => writeln!(out, "{indent}{dst} = decode{e}({r});"),
        HybridType::String => writeln!(out, "{indent}{dst} = {r}.readStr();"),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = {r}.readBytes().slice();"),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = decode{s}({r});"),
        // Validates and preserves the true `NveValue` tag directly (never unwrapping `Bytes`).
        HybridType::Value => {
            imports.value("DynamicValue");
            writeln!(out, "{indent}{dst} = DynamicValue.read({r});");
        }
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({r}.readNull()) {{");
            writeln!(out, "{indent}  {dst} = null;");
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  let _ov{d}!: {};", ts_type(inner, imports));
            emit_ts_bin_read(
                out,
                imports,
                &format!("{indent}  "),
                r,
                &format!("_ov{d}"),
                inner,
            );
            writeln!(out, "{indent}  {dst} = _ov{d};");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            let elem_ty = ts_type(inner, imports);
            writeln!(out, "{indent}{dst} = {r}.readArray((_n{d}) => {{");
            writeln!(out, "{indent}  const _arr{d}: Array<{elem_ty}> = [];");
            writeln!(out, "{indent}  for (let _i{d} = 0; _i{d} < _n{d}; _i{d} += 1) {{");
            writeln!(out, "{indent}    let _el{d}!: {elem_ty};");
            emit_ts_bin_read(
                out,
                imports,
                &format!("{indent}    "),
                r,
                &format!("_el{d}"),
                inner,
            );
            writeln!(out, "{indent}    _arr{d}.push(_el{d});");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  return _arr{d};");
            writeln!(out, "{indent}}});");
        }
        HybridType::FixedArray { elem: inner, len } => {
            imports.value("BinCodecError");
            let elem_ty = ts_type(inner, imports);
            writeln!(out, "{indent}{dst} = {r}.readArray((_n{d}) => {{");
            writeln!(
                out,
                "{indent}  if (_n{d} !== {len}) throw new BinCodecError('truncated', `expected fixed array of length {len}, got ${{_n{d}}}`);"
            );
            writeln!(out, "{indent}  const _arr{d}: Array<{elem_ty}> = [];");
            writeln!(out, "{indent}  for (let _i{d} = 0; _i{d} < _n{d}; _i{d} += 1) {{");
            writeln!(out, "{indent}    let _el{d}!: {elem_ty};");
            emit_ts_bin_read(
                out,
                imports,
                &format!("{indent}    "),
                r,
                &format!("_el{d}"),
                inner,
            );
            writeln!(out, "{indent}    _arr{d}.push(_el{d});");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  return _arr{d};");
            writeln!(out, "{indent}}});");
        }
        HybridType::Void | HybridType::Result { .. } => {
            writeln!(out, "{indent}{r}.skipValue();");
        }
    }
}
