use std::fmt::Write as _;

use super::*;

pub(super) fn emit_swift_client_method(out: &mut String, method: &HybridMethodSpec, spec: &HybridFileSpec) {
    let cname = c_field_ident(&method.name);
    writeln!(out, "    public {} {{", swift_method_sig(method)).unwrap();
    writeln!(
        out,
        "        guard let h = handle, let fnPtr = vtable.pointee.{cname} else {{ throw NveHybridError.nullHandle }}"
    )
    .unwrap();

    // Prepare any binary-encoded or copied UTF-8 buffers before entering nested withUnsafeBufferPointer scopes
    let mut pinned_scopes: Vec<(String, String, bool)> = Vec::new(); // (buffer_var, slice_var, is_str)
    let mut call_args = vec!["h".to_string()];

    for (idx, p) in method.params.iter().enumerate() {
        let pname = swift_ident(&p.name);
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
            | HybridType::F64 => call_args.push(pname),
            HybridType::Enum(_) => call_args.push(format!("{pname}.rawValue")),
            HybridType::Struct(_) if p.ty.is_pod(spec) => {
                call_args.push(format!("{pname}.toAbi()"));
            }
            HybridType::String => {
                let bvar = format!("_strBuf{idx}");
                let svar = format!("_arg{idx}");
                writeln!(out, "        let {bvar} = Array({pname}.utf8)").unwrap();
                pinned_scopes.push((bvar, svar.clone(), true));
                call_args.push(svar);
            }
            HybridType::Bytes => {
                let svar = format!("_arg{idx}");
                pinned_scopes.push((pname, svar.clone(), false));
                call_args.push(svar);
            }
            HybridType::Value => call_args.push(pname),
            HybridType::Optional(inner) if inner.is_pod(spec) => {
                let suffix = inner.pod_opt_suffix().unwrap();
                let svar = format!("_arg{idx}");
                let zero = swift_c_zero_value(inner, spec);
                let unwrap_expr = match inner.as_ref() {
                    HybridType::Enum(_) => "v.rawValue".to_string(),
                    HybridType::Struct(_) => "v.toAbi()".to_string(),
                    _ => "v".to_string(),
                };
                writeln!(
                    out,
                    "        let {svar}: NveHybridOpt{suffix} = {pname}.map {{ v in NveHybridOpt{suffix}(is_some: true, value: {unwrap_expr}) }} ?? NveHybridOpt{suffix}(is_some: false, value: {zero})"
                )
                .unwrap();
                call_args.push(svar);
            }
            other => {
                let wvar = format!("_w{idx}");
                let bvar = format!("_binBuf{idx}");
                let svar = format!("_arg{idx}");
                writeln!(out, "        var {wvar} = NveHybridBinWriter()").unwrap();
                emit_swift_bin_write(out, "        ", &wvar, &pname, other);
                writeln!(out, "        let {bvar} = {wvar}.bytes").unwrap();
                pinned_scopes.push((bvar, svar.clone(), false));
                call_args.push(svar);
            }
        }
    }

    let mut indent = "        ".to_string();
    let has_ret = !matches!(method.return_type, HybridType::Void)
        && !matches!(&method.return_type, HybridType::Result { ok, .. } if matches!(ok.as_ref(), HybridType::Void));

    for (idx, (bvar, svar, is_str)) in pinned_scopes.iter().enumerate() {
        let ret_prefix = if idx == 0 && has_ret { "return " } else { "" };
        writeln!(
            out,
            "{indent}{ret_prefix}try {bvar}.withUnsafeBufferPointer {{ _p{idx} in"
        )
        .unwrap();
        indent.push_str("    ");
        if *is_str {
            writeln!(
                out,
                "{indent}let {svar} = NveStrSlice(ptr: _p{idx}.baseAddress, len: _p{idx}.count)"
            )
            .unwrap();
        } else {
            writeln!(
                out,
                "{indent}let {svar} = NveBufferSlice(ptr: _p{idx}.baseAddress, len: _p{idx}.count)"
            )
            .unwrap();
        }
    }

    match &method.return_type {
        HybridType::Void => {
            writeln!(out, "{indent}fnPtr({})", call_args.join(", ")).unwrap();
        }
        HybridType::Result { ok, .. } => {
            if !matches!(ok.as_ref(), HybridType::Void) {
                writeln!(
                    out,
                    "{indent}var _rawOk = {}",
                    swift_c_return_zero_value(ok, spec)
                )
                .unwrap();
                call_args.push("&_rawOk".to_string());
            }
            writeln!(
                out,
                "{indent}var _rawErr = NveHybridOwnedBytes(ptr: nil, len: 0, cap: 0)"
            )
            .unwrap();
            call_args.push("&_rawErr".to_string());
            writeln!(
                out,
                "{indent}let _status = fnPtr({})",
                call_args.join(", ")
            )
            .unwrap();
            writeln!(
                out,
                "{indent}if _status != Int32(NVE_HYBRID_OK.rawValue) {{"
            )
            .unwrap();
            writeln!(out, "{indent}    let _msg = self.takeString(_rawErr)").unwrap();
            writeln!(
                out,
                "{indent}    throw NveHybridError.remoteError(_msg.isEmpty ? \"hybrid call failed\" : _msg)"
            )
            .unwrap();
            writeln!(out, "{indent}}}").unwrap();
            if !matches!(ok.as_ref(), HybridType::Void) {
                emit_swift_return_decode(out, &indent, "_rawOk", ok, spec);
            }
        }
        other => {
            writeln!(
                out,
                "{indent}let _rawRet = fnPtr({})",
                call_args.join(", ")
            )
            .unwrap();
            emit_swift_return_decode(out, &indent, "_rawRet", other, spec);
        }
    }

    for _ in 0..pinned_scopes.len() {
        indent.truncate(indent.len() - 4);
        writeln!(out, "{indent}}}").unwrap();
    }

    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_swift_return_decode(
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
        | HybridType::F64 => {
            writeln!(out, "{indent}return {var}").unwrap();
        }
        HybridType::Enum(e) => {
            writeln!(
                out,
                "{indent}return try {e}(rawValue: {var}).unwrapOrThrow(\"{e}\", {var})"
            )
            .unwrap();
        }
        HybridType::Struct(s) if ty.is_pod(spec) => {
            writeln!(out, "{indent}return try {s}.fromAbi({var})").unwrap();
        }
        HybridType::String => {
            writeln!(out, "{indent}return self.takeString({var})").unwrap();
        }
        HybridType::Bytes => {
            writeln!(out, "{indent}return self.takeBytes({var})").unwrap();
        }
        HybridType::Value => {
            writeln!(
                out,
                "{indent}return NveHybridDynamicValue(binBytes: self.takeBytes({var}))"
            )
            .unwrap();
        }
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            writeln!(out, "{indent}guard {var}.is_some else {{ return nil }}").unwrap();
            match inner.as_ref() {
                HybridType::Enum(e) => {
                    writeln!(
                        out,
                        "{indent}return try {e}(rawValue: {var}.value).unwrapOrThrow(\"{e}\", {var}.value)"
                    )
                    .unwrap();
                }
                HybridType::Struct(s) => {
                    writeln!(out, "{indent}return try {s}.fromAbi({var}.value)").unwrap();
                }
                _ => {
                    writeln!(out, "{indent}return {var}.value").unwrap();
                }
            }
        }
        other => {
            writeln!(out, "{indent}var _r = NveHybridBinReader(bytes: self.takeBytes({var}))").unwrap();
            writeln!(
                out,
                "{indent}var _out: {} = {}",
                swift_type(other),
                swift_default_value(other, None, spec)
            )
            .unwrap();
            emit_swift_bin_read(out, indent, "_r", "_out", other);
            writeln!(out, "{indent}return _out").unwrap();
        }
    }
}

pub(super) fn swift_method_sig(method: &HybridMethodSpec) -> String {
    let mname = swift_ident(&to_camel_case(&method.name));
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| format!("{}: {}", swift_ident(&p.name), swift_param_type(&p.ty)))
        .collect();
    let ret = match &method.return_type {
        HybridType::Void => String::new(),
        HybridType::Result { ok, .. } if matches!(ok.as_ref(), HybridType::Void) => String::new(),
        HybridType::Result { ok, .. } => format!(" -> {}", swift_type(ok)),
        other => format!(" -> {}", swift_type(other)),
    };
    format!("func {mname}({}) throws{ret}", params.join(", "))
}

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
        HybridType::Value => "NveHybridDynamicValue".to_string(),
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

pub(super) fn swift_param_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Value => "NveValue".to_string(),
        other => swift_type(other),
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
        return match (d, ty) {
            (HybridDefaultValue::Bool(b), _) => b.to_string(),
            (HybridDefaultValue::Int(i), HybridType::F32 | HybridType::F64) => format!("{i}.0"),
            (HybridDefaultValue::Int(i), _) => i.to_string(),
            (HybridDefaultValue::UInt(u), HybridType::F32 | HybridType::F64) => format!("{u}.0"),
            (HybridDefaultValue::UInt(u), _) => u.to_string(),
            (HybridDefaultValue::Float(f), _) => format_float_literal(*f),
            (HybridDefaultValue::String(s), _) => format!("\"{}\"", escape_c_like_string(s)),
            (HybridDefaultValue::EnumVariant { enum_name, variant }, _) => {
                format!("{enum_name}.{}", swift_ident(variant))
            }
            (HybridDefaultValue::None, _) => "nil".to_string(),
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
        HybridType::Enum(name) => {
            if let Some(e) = spec.find_enum(name) {
                format!("{name}.{}", swift_ident(&e.variants[0].name))
            } else {
                format!("{name}(rawValue: 0)!")
            }
        }
        HybridType::Value => "NveHybridDynamicValue()".to_string(),
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
    match ty {
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            let suffix = inner.pod_opt_suffix().unwrap();
            format!(
                "NveHybridOpt{suffix}(is_some: false, value: {})",
                swift_c_zero_value(inner, spec)
            )
        }
        other => swift_c_zero_value(other, spec),
    }
}

pub(super) fn emit_swift_bin_write(out: &mut String, indent: &str, w: &str, expr: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{w}.writeBool({expr})").unwrap(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64 => {
            writeln!(out, "{indent}{w}.writeInt64(Int64({expr}))").unwrap();
        }
        HybridType::U64 => {
            writeln!(out, "{indent}{w}.writeInt64(Int64(bitPattern: {expr}))").unwrap();
        }
        HybridType::F32 | HybridType::F64 => {
            writeln!(out, "{indent}{w}.writeDouble(Double({expr}))").unwrap();
        }
        HybridType::Enum(_) => {
            writeln!(out, "{indent}{w}.writeInt64(Int64({expr}.rawValue))").unwrap();
        }
        HybridType::String => writeln!(out, "{indent}{w}.writeString({expr})").unwrap(),
        HybridType::Bytes => writeln!(out, "{indent}{w}.writeBytes({expr})").unwrap(),
        HybridType::Struct(_) => writeln!(out, "{indent}{expr}.encodeBin(into: &{w})").unwrap(),
        HybridType::Value => writeln!(out, "{indent}{w}.writeRaw({expr}.binBytes)").unwrap(),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if let _optV = {expr} {{").unwrap();
            emit_swift_bin_write(out, &format!("{indent}    "), w, "_optV", inner);
            writeln!(out, "{indent}}} else {{").unwrap();
            writeln!(out, "{indent}    {w}.writeNull()").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}{w}.writeArray(count: {expr}.count) {{ _aw in").unwrap();
            writeln!(out, "{indent}    for _elem in {expr} {{").unwrap();
            emit_swift_bin_write(out, &format!("{indent}        "), "_aw", "_elem", inner);
            writeln!(out, "{indent}    }}").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::FixedArray { .. }
        | HybridType::Handle(_) => {
            writeln!(out, "{indent}{w}.writeNull()").unwrap();
        }
    }
}

pub(super) fn emit_swift_bin_read(out: &mut String, indent: &str, r: &str, dst: &str, ty: &HybridType) {
    match ty {
        HybridType::Bool => writeln!(out, "{indent}{dst} = try {r}.readBool()").unwrap(),
        HybridType::U8 => writeln!(out, "{indent}{dst} = UInt8(truncatingIfNeeded: try {r}.readInt64())").unwrap(),
        HybridType::U16 => writeln!(out, "{indent}{dst} = UInt16(truncatingIfNeeded: try {r}.readInt64())").unwrap(),
        HybridType::Usize => writeln!(out, "{indent}{dst} = Int(truncatingIfNeeded: try {r}.readInt64())").unwrap(),
        HybridType::I32 => writeln!(out, "{indent}{dst} = Int32(truncatingIfNeeded: try {r}.readInt64())").unwrap(),
        HybridType::U32 => writeln!(out, "{indent}{dst} = UInt32(truncatingIfNeeded: try {r}.readInt64())").unwrap(),
        HybridType::I64 => writeln!(out, "{indent}{dst} = try {r}.readInt64()").unwrap(),
        HybridType::U64 => writeln!(out, "{indent}{dst} = UInt64(bitPattern: try {r}.readInt64())").unwrap(),
        HybridType::F32 => writeln!(out, "{indent}{dst} = Float(try {r}.readDouble())").unwrap(),
        HybridType::F64 => writeln!(out, "{indent}{dst} = try {r}.readDouble()").unwrap(),
        HybridType::Enum(e) => {
            writeln!(out, "{indent}do {{").unwrap();
            writeln!(out, "{indent}    let _raw = UInt32(truncatingIfNeeded: try {r}.readInt64())").unwrap();
            writeln!(out, "{indent}    {dst} = try {e}(rawValue: _raw).unwrapOrThrow(\"{e}\", _raw)").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::String => writeln!(out, "{indent}{dst} = try {r}.readString()").unwrap(),
        HybridType::Bytes => writeln!(out, "{indent}{dst} = try {r}.readBytes()").unwrap(),
        HybridType::Struct(s) => writeln!(out, "{indent}{dst} = try {s}.decodeBin(from: &{r})").unwrap(),
        HybridType::Value => writeln!(out, "{indent}{dst} = NveHybridDynamicValue(binBytes: try {r}.readRawValue())").unwrap(),
        HybridType::Optional(inner) => {
            writeln!(out, "{indent}if try {r}.readNullIfPresent() {{").unwrap();
            writeln!(out, "{indent}    {dst} = nil").unwrap();
            writeln!(out, "{indent}}} else {{").unwrap();
            writeln!(out, "{indent}    let _optVal: {}", swift_type(inner)).unwrap();
            emit_swift_bin_read(out, &format!("{indent}    "), r, "_optVal", inner);
            writeln!(out, "{indent}    {dst} = _optVal").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        HybridType::Vec(inner) => {
            writeln!(out, "{indent}do {{").unwrap();
            writeln!(out, "{indent}    let _n = try {r}.beginArray()").unwrap();
            writeln!(out, "{indent}    var _arr: [{}] = []", swift_type(inner)).unwrap();
            writeln!(out, "{indent}    _arr.reserveCapacity(_n)").unwrap();
            writeln!(out, "{indent}    for _ in 0 ..< _n {{").unwrap();
            writeln!(out, "{indent}        let _elem: {}", swift_type(inner)).unwrap();
            emit_swift_bin_read(out, &format!("{indent}        "), r, "_elem", inner);
            writeln!(out, "{indent}        _arr.append(_elem)").unwrap();
            writeln!(out, "{indent}    }}").unwrap();
            writeln!(out, "{indent}    {dst} = _arr").unwrap();
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

// Note: append helper extension for Optional enum rawValue unwrapping in generated Swift.
pub(crate) fn swift_optional_unwrap_extension() -> &'static str {
    r#"private extension Optional {
    func unwrapOrThrow(_ enumName: String, _ raw: UInt32) throws -> Wrapped {
        guard let value = self else {
            throw NveHybridError.invalidEnum(enumName, raw)
        }
        return value
    }
}
"#
}
