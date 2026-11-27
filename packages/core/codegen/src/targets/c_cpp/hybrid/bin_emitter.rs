//! C++20 `NveBinValue` encoders / decoders over the static `nve::host` `BinWriter` / `BinReader`.
//!
//! Decoders are sticky-error and never throw: every read after a failure is a no-op, containers
//! are closed with `end(mark)` (so a crafted entry cannot read its parent's bytes), and the ABI
//! boundary checks the reader once (`require_done`). Writers record bad input in `BinWriter`,
//! checked once by `checked_bytes` / `to_slice`. Locals carry the nesting depth (the indent width),
//! so nested sequences and options never shadow their parent's.

use crate::writer::CodeWriter;

use super::type_helpers::cpp_owned_type;
use super::*;

/// Largest up-front `reserve` for a decoded sequence: a peer-claimed count only grows it after
/// the elements actually decode.
const MAX_DECODE_RESERVE: u32 = 1024;

pub(crate) fn emit_cpp_bin_write(
    out: &mut CodeWriter,
    indent: &str,
    w: &str,
    expr: &str,
    ty: &HybridType,
) {
    let d = indent.len();
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.write_bool({expr});"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => {
            writeln!(out, "{indent}{w}.write_i64(static_cast<int64_t>({expr}));");
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{w}.write_f64(static_cast<double>({expr}));");
        }
        HybridType::Enum(_) => {
            writeln!(
                out,
                "{indent}{w}.write_i64(static_cast<int64_t>(static_cast<uint32_t>({expr})));"
            );
        }
        HybridType::String => writeln!(out, "{indent}{w}.write_str({expr});"),
        HybridType::Bytes => writeln!(out, "{indent}{w}.write_bytes({expr});"),
        HybridType::Struct(_) => writeln!(out, "{indent}{expr}.encode_bin({w});"),
        HybridType::Value => writeln!(out, "{indent}{w}.raw({expr}.bytes());"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({expr}.has_value()) {{");
            emit_cpp_bin_write(out, &format!("{indent}  "), w, &format!("(*{expr})"), inner);
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}  {w}.write_null();");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{{");
            writeln!(
                out,
                "{indent}  if ({expr}.size() > std::numeric_limits<uint32_t>::max()) throw std::length_error(\"sequence longer than u32::MAX\");"
            );
            writeln!(
                out,
                "{indent}  const auto _mark{d} = {w}.begin_array(static_cast<uint32_t>({expr}.size()));"
            );
            writeln!(out, "{indent}  for (const auto& _elem{d} : {expr}) {{");
            emit_cpp_bin_write(
                out,
                &format!("{indent}    "),
                w,
                &format!("_elem{d}"),
                inner,
            );
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  {w}.end(_mark{d});");
            writeln!(out, "{indent}}}");
        }
        // `std::array<T, N>` carries its length in the type: an `Array` of exactly `len` values.
        HybridType::FixedArray { elem, len } => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  const auto _mark{d} = {w}.begin_array({len}u);");
            writeln!(out, "{indent}  for (const auto& _elem{d} : {expr}) {{");
            emit_cpp_bin_write(out, &format!("{indent}    "), w, &format!("_elem{d}"), elem);
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  {w}.end(_mark{d});");
            writeln!(out, "{indent}}}");
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => writeln!(out, "{indent}{w}.write_null();"),
    }
}

pub(crate) fn emit_cpp_bin_read(
    out: &mut CodeWriter,
    indent: &str,
    r: &str,
    dst: &str,
    ty: &HybridType,
) {
    let d = indent.len();
    let cast = |out: &mut CodeWriter, ty: &str, read: &str| {
        writeln!(out, "{indent}{dst} = static_cast<{ty}>({r}.{read}());");
    };
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = {r}.read_bool();"),
        HybridType::U8 => cast(out, "uint8_t", "read_i64"),
        HybridType::U16 => cast(out, "uint16_t", "read_i64"),
        HybridType::Usize => cast(out, "size_t", "read_i64"),
        HybridType::I32 => cast(out, "int32_t", "read_i64"),
        HybridType::U32 => cast(out, "uint32_t", "read_i64"),
        HybridType::I64 => writeln!(out, "{indent}{dst} = {r}.read_i64();"),
        HybridType::U64 => cast(out, "uint64_t", "read_i64"),
        HybridType::F32 => cast(out, "float", "read_f64"),
        HybridType::F64 => writeln!(out, "{indent}{dst} = {r}.read_f64();"),
        HybridType::Enum(e) => {
            writeln!(
                out,
                "{indent}{dst} = static_cast<{e}>(static_cast<uint32_t>({r}.read_i64()));"
            );
        }
        HybridType::String => writeln!(out, "{indent}{dst} = std::string({r}.read_str());"),
        HybridType::Bytes => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  const auto _span{d} = {r}.read_bytes();");
            writeln!(
                out,
                "{indent}  {dst}.assign(_span{d}.begin(), _span{d}.end());"
            );
            writeln!(out, "{indent}}}");
        }
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = {s}::decode_bin({r});"),
        HybridType::Value => writeln!(out, "{indent}{dst} = DynamicValue::read({r});"),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if ({r}.read_null()) {{");
            writeln!(out, "{indent}  {dst} = std::nullopt;");
            writeln!(out, "{indent}}} else if ({r}.ok()) {{");
            writeln!(out, "{indent}  {} _opt{d}{{}};", cpp_owned_type(inner));
            emit_cpp_bin_read(out, &format!("{indent}  "), r, &format!("_opt{d}"), inner);
            writeln!(out, "{indent}  {dst} = std::move(_opt{d});");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  const auto _seq{d} = {r}.begin_array();");
            writeln!(out, "{indent}  {dst}.clear();");
            writeln!(
                out,
                "{indent}  {dst}.reserve(std::min<uint32_t>(_seq{d}.count, {MAX_DECODE_RESERVE}u));"
            );
            writeln!(
                out,
                "{indent}  for (uint32_t _i{d} = 0; _i{d} < _seq{d}.count && {r}.ok(); ++_i{d}) {{"
            );
            writeln!(out, "{indent}    {} _elem{d}{{}};", cpp_owned_type(inner));
            emit_cpp_bin_read(
                out,
                &format!("{indent}    "),
                r,
                &format!("_elem{d}"),
                inner,
            );
            writeln!(out, "{indent}    {dst}.push_back(std::move(_elem{d}));");
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  {r}.end(_seq{d});");
            writeln!(out, "{indent}}}");
        }
        // Any count other than `len` fails the reader (`begin_array_exact`, count 0).
        HybridType::FixedArray { elem, len } => {
            writeln!(out, "{indent}{{");
            writeln!(out, "{indent}  const auto _seq{d} = {r}.begin_array_exact({len}u);");
            writeln!(
                out,
                "{indent}  for (uint32_t _i{d} = 0; _i{d} < _seq{d}.count && {r}.ok(); ++_i{d}) {{"
            );
            emit_cpp_bin_read(out, &format!("{indent}    "), r, &format!("{dst}[_i{d}]"), elem);
            writeln!(out, "{indent}  }}");
            writeln!(out, "{indent}  {r}.end(_seq{d});");
            writeln!(out, "{indent}}}");
        }
        // Not carried by the binary codec (they encode as `Null`): consume the value so the next
        // map key stays aligned.
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => writeln!(out, "{indent}{r}.skip_value();"),
    }
}

/// `encode_bin` / `decode_bin` of a struct: a map keyed by field name. Fields are accessed through
/// `this->` / `_out.`, so field names never collide with the generated locals.
pub(super) fn emit_cpp_struct_codec(out: &mut CodeWriter, s: &HybridStructSpec) {
    writeln!(out, "  void encode_bin(BinWriter& w) const {{");
    writeln!(
        out,
        "    const auto _mark = w.begin_map({}u);",
        s.fields.len()
    );
    for f in &s.fields {
        let fname = cpp_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "    w.write_map_key(\"{key_lit}\");");
        emit_cpp_bin_write(out, "    ", "w", &format!("this->{fname}"), &f.ty);
    }
    writeln!(out, "    w.end(_mark);");
    writeln!(out, "  }}");
    writeln!(out);

    writeln!(
        out,
        "  /// Sticky-error decode: on malformed input `r` fails and the result is partial."
    );
    writeln!(out, "  static {} decode_bin(BinReader& r) {{", s.name);
    writeln!(out, "    {} _out{{}};", s.name);
    writeln!(out, "    const auto _map = r.begin_map();");
    writeln!(
        out,
        "    for (uint32_t _i = 0; _i < _map.count && r.ok(); ++_i) {{"
    );
    writeln!(out, "      const auto _key = r.read_map_key();");
    writeln!(out, "      if (!r.ok()) break;");
    for (idx, f) in s.fields.iter().enumerate() {
        let fname = cpp_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        let cond = if idx == 0 { "if" } else { "} else if" };
        writeln!(out, "      {cond} (_key == \"{key_lit}\") {{");
        emit_cpp_bin_read(out, "        ", "r", &format!("_out.{fname}"), &f.ty);
    }
    if s.fields.is_empty() {
        writeln!(out, "      r.skip_value();");
    } else {
        writeln!(out, "      }} else {{");
        writeln!(out, "        r.skip_value();");
        writeln!(out, "      }}");
    }
    writeln!(out, "    }}");
    writeln!(out, "    r.end(_map);");
    writeln!(out, "    return _out;");
    writeln!(out, "  }}");
}
