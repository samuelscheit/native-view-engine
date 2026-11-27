//! `#[repr(C)]` vtables, their `build_<name>_vtable` builders, and `extern "C"` trampolines.
//!
//! Every name a trampoline binds itself starts with `_nve_` (reserved by spec validation), and its
//! implementation type parameter is `NveHybridImpl` (the `NveHybrid` prefix is reserved too), so
//! spec parameter names can never shadow or collide with them.

use crate::naming::{escape_c_like_string, rust_ident, to_pascal_case, to_snake_case};
use crate::schema::hybrid::{HybridFileSpec, HybridMethodSpec, HybridType};
use crate::writer::CodeWriter;

use super::type_map::{c_param_type, c_return_type, c_zero_value, is_by_value};

pub(super) struct VtableOwner {
    pub(super) pascal: String,
    pub(super) snake: String,
    pub(super) upper: String,
    pub(super) is_view: bool,
}

impl VtableOwner {
    pub(super) fn new(name: &str, is_view: bool) -> Self {
        let snake = to_snake_case(name);
        Self {
            pascal: to_pascal_case(name),
            upper: snake.to_ascii_uppercase(),
            snake,
            is_view,
        }
    }
}

pub(super) fn emit_rust_vtable(
    out: &mut CodeWriter,
    owner: &VtableOwner,
    methods: &[HybridMethodSpec],
    spec: &HybridFileSpec,
) {
    emit_vtable_struct(out, owner, methods, spec);
    emit_vtable_builder(out, owner, methods, spec);
}

/// `extern "C"` signature after the handle: parameters, then `Result` out-pointers.
fn c_signature_tail(method: &HybridMethodSpec, spec: &HybridFileSpec) -> String {
    let mut sig = String::new();
    for p in &method.params {
        sig.push_str(&format!(
            ", {}: {}",
            rust_ident(&p.name),
            c_param_type(&p.ty, spec)
        ));
    }
    match &method.return_type {
        HybridType::Result { ok, .. } => {
            if **ok != HybridType::Void {
                sig.push_str(&format!(", _nve_out_ok: *mut {}", c_return_type(ok, spec)));
            }
            sig.push_str(", _nve_out_err: *mut NveHybridOwnedBytes) -> i32");
        }
        HybridType::Void => sig.push(')'),
        ret => sig.push_str(&format!(") -> {}", c_return_type(ret, spec))),
    }
    sig
}

fn emit_vtable_struct(
    out: &mut CodeWriter,
    owner: &VtableOwner,
    methods: &[HybridMethodSpec],
    spec: &HybridFileSpec,
) {
    writeln!(out, "#[repr(C)]");
    writeln!(out, "pub struct NveHybrid{}VTable {{", owner.pascal);
    writeln!(out, "    pub abi_version: u32,");
    writeln!(out, "    pub interface_id: u32,");
    writeln!(out, "    pub schema_hash: u64,");
    writeln!(
        out,
        "    pub create: unsafe extern \"C\" fn() -> NveAppHandle,"
    );
    writeln!(
        out,
        "    pub destroy: unsafe extern \"C\" fn(handle: NveAppHandle),"
    );
    writeln!(
        out,
        "    pub alloc_bytes: unsafe extern \"C\" fn(len: usize) -> NveHybridOwnedBytes,"
    );
    writeln!(
        out,
        "    pub free_bytes: unsafe extern \"C\" fn(buf: NveHybridOwnedBytes),"
    );
    if owner.is_view {
        writeln!(
            out,
            "    pub set_props_direct: unsafe extern \"C\" fn(handle: NveAppHandle, entries: *const NveMapEntry, count: u32) -> i32,"
        );
    }
    for method in methods {
        writeln!(
            out,
            "    pub {}: unsafe extern \"C\" fn(_nve_handle: NveAppHandle{},",
            rust_ident(&method.name),
            c_signature_tail(method, spec)
        );
    }
    writeln!(out, "}}");
    writeln!(out);
}

fn emit_vtable_builder(
    out: &mut CodeWriter,
    owner: &VtableOwner,
    methods: &[HybridMethodSpec],
    spec: &HybridFileSpec,
) {
    let VtableOwner {
        pascal,
        snake,
        upper,
        is_view,
    } = owner;
    let bound = format!("NveHybridImpl: {pascal}Hybrid + Default + 'static");
    writeln!(
        out,
        "/// The C-ABI vtable of `NveHybridImpl`; every instance it creates is a boxed `NveHybridImpl`."
    );
    writeln!(
        out,
        "pub const fn build_{snake}_vtable<{bound}>() -> NveHybrid{pascal}VTable {{"
    );
    writeln!(
        out,
        "    unsafe extern \"C\" fn _nve_create<{bound}>() -> NveAppHandle {{"
    );
    writeln!(
        out,
        "        Box::into_raw(Box::new(NveHybridImpl::default())).cast()"
    );
    writeln!(out, "    }}");
    writeln!(
        out,
        "    unsafe extern \"C\" fn _nve_destroy<{bound}>(_nve_handle: NveAppHandle) {{"
    );
    writeln!(out, "        if !_nve_handle.is_null() {{");
    writeln!(
        out,
        "            // SAFETY: `_nve_handle` came from `_nve_create` and is destroyed once."
    );
    writeln!(
        out,
        "            drop(unsafe {{ Box::from_raw(_nve_handle.cast::<NveHybridImpl>()) }});"
    );
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    if *is_view {
        writeln!(
            out,
            "    unsafe extern \"C\" fn _nve_set_props_direct<{bound}>(_nve_handle: NveAppHandle, _nve_entries: *const NveMapEntry, _nve_count: u32) -> i32 {{"
        );
        writeln!(out, "        if _nve_handle.is_null() {{");
        writeln!(
            out,
            "            return NveHybridStatus::NullHandle as i32;"
        );
        writeln!(out, "        }}");
        writeln!(
            out,
            "        // SAFETY: The caller passes `_nve_count` valid entries, or null and 0."
        );
        writeln!(
            out,
            "        let Some(_nve_map) = (unsafe {{ NveMapRef::try_from_raw_parts(_nve_entries, _nve_count) }}) else {{"
        );
        writeln!(
            out,
            "            return NveHybridStatus::InvalidArg as i32;"
        );
        writeln!(out, "        }};");
        writeln!(
            out,
            "        match {pascal}Props::decode_props(_nve_map) {{"
        );
        writeln!(out, "            Ok(_nve_props) => {{");
        writeln!(
            out,
            "                // SAFETY: A non-null handle came from `_nve_create`."
        );
        writeln!(
            out,
            "                unsafe {{ &mut *_nve_handle.cast::<NveHybridImpl>() }}.apply_props(_nve_props);"
        );
        writeln!(out, "                NveHybridStatus::Ok as i32");
        writeln!(out, "            }}");
        writeln!(
            out,
            "            Err(_) => NveHybridStatus::InvalidArg as i32,"
        );
        writeln!(out, "        }}");
        writeln!(out, "    }}");
    }
    for method in methods {
        emit_trampoline(out, &bound, method, spec);
    }

    writeln!(out, "    NveHybrid{pascal}VTable {{");
    writeln!(out, "        abi_version: NVE_HYBRID_ABI_VERSION,");
    writeln!(
        out,
        "        interface_id: NVE_HYBRID_{upper}_INTERFACE_ID,"
    );
    writeln!(out, "        schema_hash: NVE_HYBRID_{upper}_SCHEMA_HASH,");
    writeln!(out, "        create: _nve_create::<NveHybridImpl>,");
    writeln!(out, "        destroy: _nve_destroy::<NveHybridImpl>,");
    writeln!(
        out,
        "        alloc_bytes: nve_abi::hybrid::nve_hybrid_alloc_bytes,"
    );
    writeln!(
        out,
        "        free_bytes: nve_abi::hybrid::nve_hybrid_free_bytes,"
    );
    if *is_view {
        writeln!(
            out,
            "        set_props_direct: _nve_set_props_direct::<NveHybridImpl>,"
        );
    }
    for method in methods {
        writeln!(
            out,
            "        {}: _nve_call_{}::<NveHybridImpl>,",
            rust_ident(&method.name),
            to_snake_case(&method.name)
        );
    }
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

/// How a trampoline turns one C argument into the trait method's argument.
enum ArgDecode {
    /// Passed through unchanged (scalars and POD structs by value).
    Direct,
    /// An infallible expression.
    Infallible(String),
    /// An expression of type `Result<_, DecodeError>`.
    Fallible(String),
}

fn arg_decode(p: &str, ty: &HybridType, spec: &HybridFileSpec) -> ArgDecode {
    if let (HybridType::Optional(inner), Some(_)) = (ty, ty.optional_pod_suffix(spec)) {
        return match inner.as_ref() {
            HybridType::Enum(e) => ArgDecode::Fallible(format!(
                "if {p}.is_some {{ nve_abi::hybrid::decode_enum(i64::from({p}.value), \"{e}\", {e}::from_u32).map(Some) }} else {{ Ok(None) }}"
            )),
            _ => ArgDecode::Infallible(format!("{p}.is_some.then_some({p}.value)")),
        };
    }
    match ty {
        HybridType::Enum(e) => ArgDecode::Fallible(format!(
            "nve_abi::hybrid::decode_enum(i64::from({p}), \"{e}\", {e}::from_u32)"
        )),
        _ if is_by_value(ty, spec) => ArgDecode::Direct,
        HybridType::String => ArgDecode::Fallible(format!("unsafe {{ {p}.as_str() }}")),
        HybridType::Bytes => ArgDecode::Infallible(format!("unsafe {{ {p}.as_slice() }}")),
        HybridType::Value => ArgDecode::Fallible(format!(
            "NveHybridValue::from_ffi_ref(unsafe {{ {p}.as_ref() }})"
        )),
        _ => ArgDecode::Fallible(format!(
            "nve_abi::hybrid::decode_bin_arg(unsafe {{ {p}.as_slice() }})"
        )),
    }
}

/// How a trampoline returns a trait method's value across the C ABI.
enum RetEncode {
    /// An infallible expression of the C return type.
    Value(String),
    /// An expression of type `Result<NveHybridOwnedBytes, DecodeError>`.
    Encoded(String),
}

fn ret_encode(v: &str, ty: &HybridType, spec: &HybridFileSpec) -> RetEncode {
    if let (HybridType::Optional(inner), Some(suffix)) = (ty, ty.optional_pod_suffix(spec)) {
        let value = if matches!(inner.as_ref(), HybridType::Enum(_)) {
            "_nve_v.to_u32()"
        } else {
            "_nve_v"
        };
        return RetEncode::Value(format!(
            "match {v} {{ Some(_nve_v) => NveHybridOpt{suffix} {{ is_some: true, value: {value} }}, None => Default::default() }}"
        ));
    }
    RetEncode::Value(match ty {
        HybridType::Enum(_) => format!("{v}.to_u32()"),
        _ if is_by_value(ty, spec) => v.to_owned(),
        HybridType::String => format!("NveHybridOwnedBytes::from_string({v})"),
        HybridType::Bytes => format!("NveHybridOwnedBytes::from_vec({v})"),
        HybridType::Value => format!("NveHybridOwnedBytes::from_vec({v}.into_bytes())"),
        _ => return RetEncode::Encoded(format!("NveHybridOwnedBytes::encode(&{v})")),
    })
}

fn emit_trampoline(
    out: &mut CodeWriter,
    bound: &str,
    method: &HybridMethodSpec,
    spec: &HybridFileSpec,
) {
    let is_result = method.return_type.is_result();
    // What a trampoline returns when it cannot call the implementation.
    let bail = |status: &str, message: &str| -> String {
        if is_result {
            format!(
                "return unsafe {{ nve_abi::hybrid::fail(_nve_out_err, NveHybridStatus::{status}, {message}) }};"
            )
        } else if method.return_type == HybridType::Void {
            "return;".to_owned()
        } else {
            format!("return {};", c_zero_value(&method.return_type, spec))
        }
    };

    writeln!(
        out,
        "    unsafe extern \"C\" fn _nve_call_{}<{bound}>(_nve_handle: NveAppHandle{} {{",
        to_snake_case(&method.name),
        c_signature_tail(method, spec)
    );
    writeln!(out, "        if _nve_handle.is_null() {{");
    writeln!(
        out,
        "            {}",
        bail("NullHandle", "String::from(\"null hybrid handle\")")
    );
    writeln!(out, "        }}");

    let mut args = Vec::with_capacity(method.params.len());
    for (idx, p) in method.params.iter().enumerate() {
        let pname = rust_ident(&p.name);
        match arg_decode(&pname, &p.ty, spec) {
            ArgDecode::Direct => {
                args.push(pname);
                continue;
            }
            ArgDecode::Infallible(expr) => writeln!(out, "        let _nve_arg_{idx} = {expr};"),
            ArgDecode::Fallible(expr) => {
                let message = format!(
                    "format!(\"invalid hybrid argument '{}': {{_nve_error}}\")",
                    escape_c_like_string(&p.name)
                );
                let on_error = if is_result {
                    "Err(_nve_error)"
                } else {
                    "Err(_)"
                };
                writeln!(out, "        let _nve_arg_{idx} = match {expr} {{");
                writeln!(out, "            Ok(_nve_v) => _nve_v,");
                writeln!(
                    out,
                    "            {on_error} => {{ {} }}",
                    bail("InvalidArg", &message)
                );
                writeln!(out, "        }};");
            }
        }
        args.push(format!("_nve_arg_{idx}"));
    }

    writeln!(
        out,
        "        // SAFETY: A non-null handle came from `_nve_create`."
    );
    writeln!(
        out,
        "        let _nve_instance = unsafe {{ &mut *_nve_handle.cast::<NveHybridImpl>() }};"
    );
    let call = format!(
        "_nve_instance.{}({})",
        rust_ident(&method.name),
        args.join(", ")
    );
    match &method.return_type {
        HybridType::Void => writeln!(out, "        {call};"),
        HybridType::Result { ok, .. } => {
            writeln!(out, "        match {call} {{");
            match ret_encode("_nve_ok", ok, spec) {
                _ if **ok == HybridType::Void => {
                    writeln!(
                        out,
                        "            Ok(()) => unsafe {{ nve_abi::hybrid::succeed(_nve_out_err) }},"
                    );
                }
                RetEncode::Value(expr) => {
                    writeln!(out, "            Ok(_nve_ok) => unsafe {{");
                    writeln!(
                        out,
                        "                nve_abi::hybrid::store(_nve_out_ok, {expr});"
                    );
                    writeln!(
                        out,
                        "                nve_abi::hybrid::succeed(_nve_out_err)"
                    );
                    writeln!(out, "            }},");
                }
                RetEncode::Encoded(expr) => {
                    writeln!(out, "            Ok(_nve_ok) => match {expr} {{");
                    writeln!(out, "                Ok(_nve_bytes) => unsafe {{");
                    writeln!(
                        out,
                        "                    nve_abi::hybrid::store(_nve_out_ok, _nve_bytes);"
                    );
                    writeln!(
                        out,
                        "                    nve_abi::hybrid::succeed(_nve_out_err)"
                    );
                    writeln!(out, "                }},");
                    writeln!(
                        out,
                        "                Err(_nve_error) => unsafe {{ nve_abi::hybrid::fail(_nve_out_err, NveHybridStatus::Err, format!(\"failed to encode hybrid result: {{_nve_error}}\")) }},"
                    );
                    writeln!(out, "            }},");
                }
            }
            writeln!(
                out,
                "            Err(_nve_message) => unsafe {{ nve_abi::hybrid::fail(_nve_out_err, NveHybridStatus::Err, _nve_message) }},"
            );
            writeln!(out, "        }}");
        }
        ret => {
            writeln!(out, "        let _nve_ret = {call};");
            match ret_encode("_nve_ret", ret, spec) {
                RetEncode::Value(expr) => writeln!(out, "        {expr}"),
                // Encoding fails only past the wire format's limits; the empty buffer then fails
                // the caller's decode instead of crossing a partial value.
                RetEncode::Encoded(expr) => {
                    writeln!(out, "        {expr}.unwrap_or(NveHybridOwnedBytes::EMPTY)")
                }
            }
        }
    }
    writeln!(out, "    }}");
}
