//! POD struct conversions between a Dart class and its `ffi.Struct` (`NveHybrid<Name>`), including
//! (nested) `[T; N]` fields as `ffi.Array` (`@ffi.Array.multi([N, M])` when nested).

use crate::writer::CodeWriter;

use super::*;

/// The `@ffi.Array` annotation of a (nested) `[T; N]` C array field.
pub(super) fn dart_ffi_array_annotation(ty: &HybridType) -> String {
    match ty.fixed_array_dims().0.as_slice() {
        [len] => format!("@ffi.Array({len})"),
        dims => format!(
            "@ffi.Array.multi([{}])",
            dims.iter().map(usize::to_string).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// `_writeAbi` / `toAbi` / `fromAbi` of POD struct `s`.
pub(super) fn emit_dart_pod_abi_conversions(out: &mut CodeWriter, s: &HybridStructSpec) {
    writeln!(out);
    writeln!(out, "  void _writeAbi(NveHybrid{} _nveDst) {{", s.name);
    for f in &s.fields {
        let fname = dart_ident(&f.name);
        let field = escape_dart_single_quoted_string(&f.name);
        emit_abi_write(out, "    ", &format!("_nveDst.{fname}"), &fname, &field, &f.ty, 0);
    }
    writeln!(out, "  }}");
    writeln!(out);
    writeln!(out, "  NveHybrid{} toAbi() {{", s.name);
    writeln!(out, "    final _nveRaw = ffi.Struct.create<NveHybrid{}>();", s.name);
    writeln!(out, "    _writeAbi(_nveRaw);");
    writeln!(out, "    return _nveRaw;");
    writeln!(out, "  }}");
    writeln!(out);
    let name = &s.name;
    writeln!(out, "  factory {name}.fromAbi(NveHybrid{name} _nveRaw) => {name}(");
    for f in &s.fields {
        let fname = dart_ident(&f.name);
        writeln!(out, "    {fname}: {},", abi_read(&format!("_nveRaw.{fname}"), &f.ty, 0));
    }
    writeln!(out, "  );");
}

/// Writes `src` (a Dart value of `ty`) into the C memory `dst`. Every array level checks its length
/// before writing its elements; a wrong inner row throws after the rows before it were written, so
/// `dst` must be scratch memory the caller drops on a throw (`toAbi`, a call's arena).
fn emit_abi_write(
    out: &mut CodeWriter,
    indent: &str,
    dst: &str,
    src: &str,
    field: &str,
    ty: &HybridType,
    depth: usize,
) {
    match ty {
        HybridType::Enum(_) => writeln!(out, "{indent}{dst} = {src}.rawValue;"),
        HybridType::Struct(_) => writeln!(out, "{indent}{src}._writeAbi({dst});"),
        HybridType::FixedArray { elem, len } => {
            writeln!(out, "{indent}if ({src}.length != {len}) {{");
            writeln!(
                out,
                "{indent}  throw RangeError.value({src}.length, '{field}', 'expected fixed length {len}');"
            );
            writeln!(out, "{indent}}}");
            let i = format!("_nveI{depth}");
            writeln!(out, "{indent}for (var {i} = 0; {i} < {len}; {i}++) {{");
            let (dst, src) = (format!("{dst}[{i}]"), format!("{src}[{i}]"));
            emit_abi_write(out, &format!("{indent}  "), &dst, &src, field, elem, depth + 1);
            writeln!(out, "{indent}}}");
        }
        _ => writeln!(out, "{indent}{dst} = {src};"),
    }
}

/// The Dart value of `ty` read from the C memory `src`.
fn abi_read(src: &str, ty: &HybridType, depth: usize) -> String {
    match ty {
        HybridType::Enum(e) => format!("{e}.fromRaw({src})"),
        HybridType::Struct(s) => format!("{s}.fromAbi({src})"),
        HybridType::FixedArray { elem, len } => {
            let i = format!("_nveI{depth}");
            let elem_expr = abi_read(&format!("{src}[{i}]"), elem, depth + 1);
            format!("List<{}>.generate({len}, ({i}) => {elem_expr})", dart_type(elem))
        }
        _ => src.to_string(),
    }
}
