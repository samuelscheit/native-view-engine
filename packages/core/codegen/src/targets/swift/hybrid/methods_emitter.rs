//! Swift hybrid client methods: argument marshalling into C slices / `NveValue` trees, the vtable
//! call, `Result` status handling, and return decoding. Locals carry an `_nve` prefix and members
//! are reached through `self.`, so no user parameter name can shadow them.

use crate::writer::CodeWriter;

use super::codec_emitter::{
    emit_swift_bin_read, emit_swift_bin_write, swift_c_return_zero_value, swift_c_zero_value,
    swift_type,
};
use super::*;

/// A closure scope the call runs inside: the buffers it pins stay valid until it closes.
struct Scope {
    open: String,
    slice: Option<String>,
}

pub(super) fn emit_swift_client_method(
    out: &mut CodeWriter,
    method: &HybridMethodSpec,
    spec: &HybridFileSpec,
) {
    let cname = c_field_ident(&method.name);
    let entry = swift_member(&cname);
    writeln!(out, "    public {} {{", swift_method_sig(method));
    writeln!(
        out,
        "        guard let _nveFn = self.vtable.pointee.{entry} else {{ throw HybridError.nullFunction(\"{cname}\") }}"
    );

    let mut scopes: Vec<Scope> = Vec::new();
    let mut call_args = vec!["self.handle".to_string()];
    let mut has_tree = false;

    for (idx, p) in method.params.iter().enumerate() {
        let pname = swift_ident(&p.name);
        let arg = format!("_nveArg{idx}");
        let pin = format!("_nvePin{idx}");
        if let (HybridType::Optional(inner), Some(suffix)) = (&p.ty, p.ty.optional_pod_suffix(spec))
        {
            let zero = swift_c_zero_value(inner, spec);
            let value = match inner.as_ref() {
                HybridType::Enum(_) => "_nveSome.rawValue",
                HybridType::Struct(_) => "_nveSome.toAbi()",
                _ => "_nveSome",
            };
            writeln!(
                out,
                "        let {arg} = {pname}.map {{ _nveSome in NveHybridOpt{suffix}(is_some: true, value: {value}) }} ?? NveHybridOpt{suffix}(is_some: false, value: {zero})"
            );
            call_args.push(arg);
            continue;
        }
        let buffer_slice = Some(format!(
            "let {arg} = NveBufferSlice(ptr: {pin}.baseAddress, len: {pin}.count)"
        ));
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
            | HybridType::F64 => {
                call_args.push(pname);
                continue;
            }
            HybridType::Enum(_) => {
                call_args.push(format!("{pname}.rawValue"));
                continue;
            }
            HybridType::Struct(_) if p.ty.is_pod(spec) => {
                call_args.push(format!("{pname}.toAbi()"));
                continue;
            }
            HybridType::String => {
                // Native strings are contiguous UTF-8, so `withUTF8` pins them without a copy.
                writeln!(out, "        var _nveStr{idx} = {pname}");
                scopes.push(Scope {
                    open: format!("_nveStr{idx}.withUTF8 {{ {pin} in"),
                    slice: Some(format!(
                        "let {arg} = NveStrSlice(ptr: {pin}.baseAddress, len: {pin}.count)"
                    )),
                });
            }
            HybridType::Bytes => scopes.push(Scope {
                open: format!("{pname}.withUnsafeBufferPointer {{ {pin} in"),
                slice: buffer_slice,
            }),
            HybridType::Value => {
                if !has_tree {
                    has_tree = true;
                    writeln!(out, "        let _nveTree = NveValueTree()");
                    scopes.push(Scope {
                        open: "withExtendedLifetime(_nveTree) {".to_string(),
                        slice: None,
                    });
                }
                writeln!(out, "        let {arg} = try _nveTree.value({pname})");
            }
            other => {
                let w = format!("_nveW{idx}");
                writeln!(out, "        var {w} = BinWriter()");
                emit_swift_bin_write(out, "        ", &w, &pname, other, 0);
                scopes.push(Scope {
                    open: format!("{w}.bytes.withUnsafeBufferPointer {{ {pin} in"),
                    slice: buffer_slice,
                });
            }
        }
        call_args.push(arg);
    }

    let has_ret = !matches!(method.return_type, HybridType::Void)
        && !matches!(&method.return_type, HybridType::Result { ok, .. } if matches!(ok.as_ref(), HybridType::Void));
    // Pinning closures rethrow, so they need `try` only when the call's own result handling throws;
    // every nested closure returns its inner closure's result.
    let try_ = if return_throws(&method.return_type, spec) { "try " } else { "" };
    let ret = if has_ret { "return " } else { "" };
    let mut indent = "        ".to_string();
    for scope in &scopes {
        writeln!(out, "{indent}{ret}{try_}{}", scope.open);
        indent.push_str("    ");
        if let Some(slice) = &scope.slice {
            writeln!(out, "{indent}{slice}");
        }
    }

    match &method.return_type {
        HybridType::Void => writeln!(out, "{indent}_nveFn({})", call_args.join(", ")),
        HybridType::Result { ok, .. } => {
            let has_ok = !matches!(ok.as_ref(), HybridType::Void);
            if has_ok {
                writeln!(out, "{indent}var _nveRawOk = {}", swift_c_return_zero_value(ok, spec));
                call_args.push("&_nveRawOk".to_string());
            }
            writeln!(out, "{indent}var _nveRawErr = NveHybridOwnedBytes(ptr: nil, len: 0, cap: 0)");
            call_args.push("&_nveRawErr".to_string());
            writeln!(out, "{indent}let _nveStatus = _nveFn({})", call_args.join(", "));
            writeln!(out, "{indent}if _nveStatus != HybridStatus.ok.rawValue {{");
            writeln!(out, "{indent}    let _nveMessage = self.takeString(_nveRawErr)");
            writeln!(
                out,
                "{indent}    if !_nveMessage.isEmpty {{ throw HybridError.remote(_nveMessage) }}"
            );
            writeln!(out, "{indent}    try HybridStatus.check(_nveStatus)");
            writeln!(out, "{indent}}}");
            if has_ok {
                emit_swift_return_decode(out, &indent, "_nveRawOk", ok, spec);
            }
        }
        other => {
            writeln!(out, "{indent}let _nveRet = _nveFn({})", call_args.join(", "));
            emit_swift_return_decode(out, &indent, "_nveRet", other, spec);
        }
    }

    for _ in &scopes {
        indent.truncate(indent.len() - 4);
        writeln!(out, "{indent}}}");
    }
    writeln!(out, "    }}");
    writeln!(out);
}

/// Whether handling a call's result can throw (a `Result` status or a validating decode).
fn return_throws(ty: &HybridType, spec: &HybridFileSpec) -> bool {
    match ty {
        HybridType::Void
        | HybridType::Bool
        | HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::F32
        | HybridType::F64
        | HybridType::String
        | HybridType::Bytes => false,
        HybridType::Optional(inner) if ty.optional_pod_suffix(spec).is_some() => {
            matches!(inner.as_ref(), HybridType::Enum(_) | HybridType::Struct(_))
        }
        _ => true,
    }
}

fn emit_swift_return_decode(
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
        | HybridType::F64 => writeln!(out, "{indent}return {var}"),
        HybridType::Enum(e) => writeln!(out, "{indent}return try {e}.decode({var})"),
        HybridType::Struct(s) if ty.is_pod(spec) => {
            writeln!(out, "{indent}return try {s}.fromAbi({var})");
        }
        HybridType::String => writeln!(out, "{indent}return self.takeString({var})"),
        HybridType::Bytes => writeln!(out, "{indent}return self.takeBytes({var})"),
        HybridType::Value => {
            writeln!(out, "{indent}return try DynamicValue(bytes: self.takeBytes({var}))");
        }
        HybridType::Optional(inner) if ty.optional_pod_suffix(spec).is_some() => {
            writeln!(out, "{indent}guard {var}.is_some else {{ return nil }}");
            match inner.as_ref() {
                HybridType::Enum(e) => writeln!(out, "{indent}return try {e}.decode({var}.value)"),
                HybridType::Struct(s) => {
                    writeln!(out, "{indent}return try {s}.fromAbi({var}.value)");
                }
                _ => writeln!(out, "{indent}return {var}.value"),
            }
        }
        other => {
            // Validated up front (container bounds, depth, UTF-8, exactly one value; allocation-free),
            // so the typed decode below only reads a well-formed reply.
            writeln!(out, "{indent}let _nveBytes = self.takeBytes({var})");
            writeln!(out, "{indent}try BinReader.validate(_nveBytes)");
            writeln!(out, "{indent}var _nveR = BinReader(_nveBytes)");
            writeln!(out, "{indent}let _nveOut: {}", swift_type(other));
            emit_swift_bin_read(out, indent, "_nveR", "_nveOut", other, 0);
            writeln!(out, "{indent}return _nveOut");
        }
    }
}

pub(super) fn swift_method_sig(method: &HybridMethodSpec) -> String {
    let mname = swift_ident(&to_camel_case(&method.name));
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| format!("{}: {}", swift_ident(&p.name), swift_type(&p.ty)))
        .collect();
    let ret = match &method.return_type {
        HybridType::Void => String::new(),
        HybridType::Result { ok, .. } if matches!(ok.as_ref(), HybridType::Void) => String::new(),
        HybridType::Result { ok, .. } => format!(" -> {}", swift_type(ok)),
        other => format!(" -> {}", swift_type(other)),
    };
    format!("func {mname}({}) throws{ret}", params.join(", "))
}
