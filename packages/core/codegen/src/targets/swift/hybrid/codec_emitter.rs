//! Swift type names, defaults, and `NveBinValue` / `NveValue` encoders over the static
//! `NveHost` (`BinReader`, `BinWriter`, `DynamicValue`) and `NveHostAbi` (`NveValueTree`) runtime.

use crate::writer::CodeWriter;

use super::*;

pub(super) fn swift_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "Void".to_string(),
        HybridType::Bool => "Bool".to_string(),
        HybridType::U8 => "UInt8".to_string(),
        HybridType::U16 => "UInt16".to_string(),
        HybridType::Usize => "Int".to_string(),
        HybridType::I32 => "Int32".to_string(),
        HybridType::U32 => "UInt32".to_string(),
        HybridType::I64 => "Int64".to_string(),
        HybridType::U64 => "UInt64".to_string(),
        HybridType::F32 => "Float".to_string(),
        HybridType::F64 => "Double".to_string(),
        HybridType::String => "String".to_string(),
        HybridType::Bytes => "[UInt8]".to_string(),
        HybridType::Optional(inner) => format!("{}?", swift_type(inner)),
        HybridType::Vec(inner) | HybridType::FixedArray { elem: inner, .. } => {
            format!("[{}]", swift_type(inner))
        }
        HybridType::Result { ok, .. } => swift_type(ok),
        HybridType::Struct(name) | HybridType::Enum(name) | HybridType::Handle(name) => {
            name.clone()
        }
        HybridType::Value => "DynamicValue".to_string(),
        HybridType::ConstPtr(inner) => {
            if matches!(inner.as_ref(), HybridType::Void) {
                "UnsafeRawPointer?".to_string()
            } else {
                format!("UnsafePointer<{}>?", swift_type(inner))
            }
        }
        HybridType::MutPtr(inner) => {
            if matches!(inner.as_ref(), HybridType::Void) {
                "UnsafeMutableRawPointer?".to_string()
            } else {
                format!("UnsafeMutablePointer<{}>?", swift_type(inner))
            }
        }
    }
}

pub(super) fn swift_default_value(
    ty: &HybridType,
    def: Option<&HybridDefaultValue>,
    spec: &HybridFileSpec,
) -> String {
    if let Some(d) = def {
        if let HybridType::Optional(inner) = ty {
            return match d {
                HybridDefaultValue::None => "nil".to_string(),
                other => swift_default_value(inner, Some(other), spec),
            };
        }
        if let Some(lit) = d.format_scalar_literal(ty) {
            return lit;
        }
        return match d {
            HybridDefaultValue::String(s) => format!("\"{}\"", escape_c_like_string(s)),
            HybridDefaultValue::EnumVariant { enum_name, variant } => {
                format!("{enum_name}.{}", swift_ident(variant))
            }
            HybridDefaultValue::None => "nil".to_string(),
            _ => "nil".to_string(),
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
        HybridType::String => "\"\"".to_string(),
        HybridType::Bytes | HybridType::Vec(_) | HybridType::FixedArray { .. } => "[]".to_string(),
        HybridType::Optional(_)
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => "nil".to_string(),
        HybridType::Struct(name) => format!("{name}()"),
        // `validate()` guarantees the enum exists and has at least one variant.
        HybridType::Enum(name) => spec
            .find_enum(name)
            .and_then(|e| e.variants.first())
            .map_or_else(
                || format!("{name}()"),
                |v| format!("{name}.{}", swift_ident(&v.name)),
            ),
        HybridType::Value => "DynamicValue()".to_string(),
        HybridType::Void | HybridType::Result { .. } => "()".to_string(),
    }
}

pub(super) fn swift_c_zero_value(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Bool => "false".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::Enum(_) => "0".to_string(),
        HybridType::F32 | HybridType::F64 => "0.0".to_string(),
        HybridType::Struct(name) if ty.is_pod(spec) => format!("{name}().toAbi()"),
        _ => "NveHybridOwnedBytes(ptr: nil, len: 0, cap: 0)".to_string(),
    }
}

pub(super) fn swift_c_return_zero_value(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if let (HybridType::Optional(inner), Some(suffix)) = (ty, ty.optional_pod_suffix(spec)) {
        return format!(
            "NveHybridOpt{suffix}(is_some: false, value: {})",
            swift_c_zero_value(inner, spec)
        );
    }
    swift_c_zero_value(ty, spec)
}

/// Writes `expr` as one `NveBinValue` into the `BinWriter` named `w`. `depth` suffixes the locals,
/// so nested sequences and options never shadow each other or user field names.
pub(super) fn emit_swift_bin_write(
    out: &mut CodeWriter,
    indent: &str,
    w: &str,
    expr: &str,
    ty: &HybridType,
    depth: usize,
) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr})"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32 => writeln!(out, "{indent}{w}.writeI64(Int64({expr}))"),
        HybridType::I64 => writeln!(out, "{indent}{w}.writeI64({expr})"),
        HybridType::U64 => writeln!(out, "{indent}{w}.writeI64(Int64(bitPattern: {expr}))"),
        HybridType::F32 | HybridType::F64 => writeln!(out, "{indent}{w}.writeF64(Double({expr}))"),
        HybridType::Enum(_) => writeln!(out, "{indent}{w}.writeI64(Int64({expr}.rawValue))"),
        HybridType::String => writeln!(out, "{indent}try {w}.writeStr({expr})"),
        HybridType::Bytes => writeln!(out, "{indent}try {w}.writeBytes({expr})"),
        HybridType::Struct(_) => writeln!(out, "{indent}try {expr}.encodeBin(into: &{w})"),
        HybridType::Value => writeln!(out, "{indent}{w}.raw({expr}.bytes)"),
        HybridType::Optional(inner) => {
            let some = format!("_nveSome{depth}");
            writeln!(out, "{indent}if let {some} = {expr} {{");
            emit_swift_bin_write(out, &format!("{indent}    "), w, &some, inner, depth + 1);
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}    {w}.writeNull()");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            // Its own scope, so sibling sequences (two `Vec` fields or params) reuse the names.
            let mark = format!("_nveArray{depth}");
            let elem = format!("_nveElem{depth}");
            writeln!(out, "{indent}do {{");
            writeln!(out, "{indent}    let {mark} = try {w}.beginArray(count: {expr}.count)");
            writeln!(out, "{indent}    for {elem} in {expr} {{");
            emit_swift_bin_write(out, &format!("{indent}        "), w, &elem, inner, depth + 1);
            writeln!(out, "{indent}    }}");
            writeln!(out, "{indent}    try {w}.end({mark})");
            writeln!(out, "{indent}}}");
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => writeln!(out, "{indent}{w}.writeNull()"),
    }
}

/// Reads one `NveBinValue` from the `BinReader` named `r` into the assignable `dst`.
pub(super) fn emit_swift_bin_read(
    out: &mut CodeWriter,
    indent: &str,
    r: &str,
    dst: &str,
    ty: &HybridType,
    depth: usize,
) {
    let int = |swift: &str| format!("{indent}{dst} = {swift}(truncatingIfNeeded: try {r}.readI64())");
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = try {r}.readBool()"),
        HybridType::U8 => writeln!(out, "{}", int("UInt8")),
        HybridType::U16 => writeln!(out, "{}", int("UInt16")),
        HybridType::Usize => writeln!(out, "{}", int("Int")),
        HybridType::I32 => writeln!(out, "{}", int("Int32")),
        HybridType::U32 => writeln!(out, "{}", int("UInt32")),
        HybridType::I64 => writeln!(out, "{indent}{dst} = try {r}.readI64()"),
        HybridType::U64 => writeln!(out, "{indent}{dst} = UInt64(bitPattern: try {r}.readI64())"),
        HybridType::F32 => writeln!(out, "{indent}{dst} = Float(try {r}.readF64())"),
        HybridType::F64 => writeln!(out, "{indent}{dst} = try {r}.readF64()"),
        HybridType::Enum(e) => writeln!(
            out,
            "{indent}{dst} = try {e}.decode(UInt32(truncatingIfNeeded: try {r}.readI64()))"
        ),
        HybridType::String => writeln!(out, "{indent}{dst} = try {r}.readStr()"),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = Array(try {r}.readBytes())"),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = try {s}.decodeBin(from: &{r})"),
        HybridType::Value => writeln!(
            out,
            "{indent}{dst} = try DynamicValue(bytes: Array(try {r}.readRawValue()))"
        ),
        HybridType::Optional(inner) => {
            let some = format!("_nveSome{depth}");
            writeln!(out, "{indent}if try {r}.readNull() {{");
            writeln!(out, "{indent}    {dst} = nil");
            writeln!(out, "{indent}}} else {{");
            writeln!(out, "{indent}    let {some}: {}", swift_type(inner));
            emit_swift_bin_read(out, &format!("{indent}    "), r, &some, inner, depth + 1);
            writeln!(out, "{indent}    {dst} = {some}");
            writeln!(out, "{indent}}}");
        }
        HybridType::Vec(inner) => {
            let (count, list, elem) = (
                format!("_nveCount{depth}"),
                format!("_nveList{depth}"),
                format!("_nveElem{depth}"),
            );
            let elem_ty = swift_type(inner);
            writeln!(out, "{indent}do {{");
            writeln!(out, "{indent}    let {count} = try {r}.beginArray()");
            writeln!(out, "{indent}    var {list}: [{elem_ty}] = []");
            // `beginArray` bounds the count by the payload; the cap also bounds the up-front reserve.
            writeln!(out, "{indent}    {list}.reserveCapacity(Swift.min({count}, 1024))");
            writeln!(out, "{indent}    for _ in 0..<{count} {{");
            writeln!(out, "{indent}        let {elem}: {elem_ty}");
            emit_swift_bin_read(out, &format!("{indent}        "), r, &elem, inner, depth + 1);
            writeln!(out, "{indent}        {list}.append({elem})");
            writeln!(out, "{indent}    }}");
            writeln!(out, "{indent}    {dst} = {list}");
            writeln!(out, "{indent}}}");
        }
        // Never encoded (written as `null`): consume the value so the stream stays in sync, and
        // give `dst` its empty value so a fresh `let` is definitely initialized.
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {
            writeln!(out, "{indent}try {r}.skipValue()");
            match ty {
                HybridType::FixedArray { .. } => writeln!(out, "{indent}{dst} = []"),
                HybridType::ConstPtr(_) | HybridType::MutPtr(_) | HybridType::Handle(_) => {
                    writeln!(out, "{indent}{dst} = nil");
                }
                HybridType::Void => writeln!(out, "{indent}{dst} = ()"),
                // `validate()` only admits `Result` as a top-level return type.
                _ => {}
            }
        }
    }
}

/// `expr` as a C `NveValue` built in the `NveValueTree` named `tree`, for types with their own
/// `NveValue` tag (`HybridType::Value` keeps its true tags); `None` for `Bytes`-encoded types.
fn swift_direct_nve_value(tree: &str, expr: &str, ty: &HybridType) -> Option<String> {
    Some(match ty {
        HybridType::Bool => format!("{tree}.bool({expr})"),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32 => format!("{tree}.int(Int64({expr}))"),
        HybridType::I64 => format!("{tree}.int({expr})"),
        HybridType::U64 => format!("{tree}.int(Int64(bitPattern: {expr}))"),
        HybridType::F32 | HybridType::F64 => format!("{tree}.float(Double({expr}))"),
        HybridType::Enum(_) => format!("{tree}.int(Int64({expr}.rawValue))"),
        HybridType::String => format!("{tree}.string({expr})"),
        HybridType::Bytes => format!("{tree}.bytes({expr})"),
        HybridType::Value => format!("try {tree}.value({expr})"),
        _ => return None,
    })
}

/// `let {dst} = <expr as a C NveValue in tree>`: scalars, strings, bytes, and dynamic values keep
/// their own tags; `Struct` / `Vec` cross as one `NveBinValue` in `Bytes`; an absent option is
/// `Null` itself (never an encoded null inside `Bytes`).
pub(super) fn emit_swift_nve_value(
    out: &mut CodeWriter,
    indent: &str,
    tree: &str,
    dst: &str,
    expr: &str,
    ty: &HybridType,
) {
    if let Some(value) = swift_direct_nve_value(tree, expr, ty) {
        writeln!(out, "{indent}let {dst} = {value}");
        return;
    }
    let nested = format!("{indent}    ");
    match ty {
        HybridType::Optional(inner) => {
            if let Some(value) = swift_direct_nve_value(tree, "_nveSome", inner) {
                let t = if value.starts_with("try ") { "try " } else { "" };
                writeln!(
                    out,
                    "{indent}let {dst} = {t}{expr}.map {{ _nveSome in {value} }} ?? {tree}.null()"
                );
                return;
            }
            if !matches!(inner.as_ref(), HybridType::Struct(_) | HybridType::Vec(_)) {
                writeln!(out, "{indent}let {dst} = {tree}.null()"); // no wire form, as below
                return;
            }
            writeln!(out, "{indent}let {dst} = try {expr}.map {{ _nveSome in");
            emit_swift_encoded(out, &nested, tree, "_nveSome", inner);
            writeln!(out, "{indent}}} ?? {tree}.null()");
        }
        HybridType::Struct(_) | HybridType::Vec(_) => {
            writeln!(out, "{indent}let {dst} =");
            emit_swift_encoded(out, &nested, tree, expr, ty);
        }
        // Types with no wire form (pointers, handles, fixed arrays) cross as `null`, like their codec.
        _ => writeln!(out, "{indent}let {dst} = {tree}.null()"),
    }
}

/// `try tree.encoded { ... }`: `expr` as one `NveBinValue` in a `Bytes` value.
fn emit_swift_encoded(out: &mut CodeWriter, indent: &str, tree: &str, expr: &str, ty: &HybridType) {
    writeln!(out, "{indent}try {tree}.encoded {{ _nveW in");
    emit_swift_bin_write(out, &format!("{indent}    "), "_nveW", expr, ty, 0);
    writeln!(out, "{indent}}}");
}
