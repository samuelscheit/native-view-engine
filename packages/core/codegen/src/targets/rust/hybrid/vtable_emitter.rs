use std::fmt::Write as _;

use super::*;
use super::codec_helpers::*;

pub(super) fn emit_rust_trait_method(out: &mut String, method: &HybridMethodSpec) {
    let mname = rust_ident(&method.name);
    write!(out, "    fn {mname}(&mut self").unwrap();
    for p in &method.params {
        let pname = rust_ident(&p.name);
        write!(out, ", {pname}: {}", rust_param_type(&p.ty)).unwrap();
    }
    if method.return_type == HybridType::Void {
        writeln!(out, ");").unwrap();
    } else {
        writeln!(out, ") -> {};", rust_return_type(&method.return_type)).unwrap();
    }
}

pub(super) fn emit_rust_vtable_struct(
    out: &mut String,
    pascal: &str,
    methods: &[HybridMethodSpec],
    is_view: bool,
    spec: &HybridFileSpec,
) {
    writeln!(out, "#[repr(C)]").unwrap();
    writeln!(out, "pub struct NveHybrid{pascal}VTable {{").unwrap();
    writeln!(out, "    pub abi_version: u32,").unwrap();
    writeln!(out, "    pub interface_id: u32,").unwrap();
    writeln!(out, "    pub schema_hash: u64,").unwrap();
    writeln!(out, "    pub create: unsafe extern \"C\" fn() -> NveAppHandle,").unwrap();
    writeln!(out, "    pub destroy: unsafe extern \"C\" fn(handle: NveAppHandle),").unwrap();
    writeln!(out, "    pub alloc_bytes: unsafe extern \"C\" fn(len: usize) -> NveHybridOwnedBytes,").unwrap();
    writeln!(out, "    pub free_bytes: unsafe extern \"C\" fn(buf: NveHybridOwnedBytes),").unwrap();
    if is_view {
        writeln!(
            out,
            "    pub set_props_direct: unsafe extern \"C\" fn(handle: NveAppHandle, entries: *const NveMapEntry, count: u32) -> i32,"
        )
        .unwrap();
    }
    for method in methods {
        let mname = rust_ident(&method.name);
        write!(out, "    pub {mname}: unsafe extern \"C\" fn(handle: NveAppHandle").unwrap();
        for p in &method.params {
            let pname = rust_ident(&p.name);
            write!(out, ", {pname}: {}", rust_c_abi_param_type(&p.ty, spec)).unwrap();
        }
        match &method.return_type {
            HybridType::Result { ok, .. } => {
                if !matches!(ok.as_ref(), HybridType::Void) {
                    write!(out, ", out_ok: *mut {}", rust_c_abi_return_type(ok, spec)).unwrap();
                }
                writeln!(out, ", out_err: *mut NveHybridOwnedBytes) -> i32,").unwrap();
            }
            HybridType::Void => writeln!(out, "),").unwrap(),
            other => writeln!(out, ") -> {},", rust_c_abi_return_type(other, spec)).unwrap(),
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_rust_vtable_builder(
    out: &mut String,
    pascal: &str,
    snake: &str,
    upper: &str,
    methods: &[HybridMethodSpec],
    is_view: bool,
    spec: &HybridFileSpec,
) {
    writeln!(
        out,
        "/// Builds a static C-ABI vtable for any concrete `T: {pascal}Hybrid + Default + 'static`."
    )
    .unwrap();
    writeln!(
        out,
        "pub const fn build_{snake}_vtable<T: {pascal}Hybrid + Default + 'static>() -> NveHybrid{pascal}VTable {{"
    )
    .unwrap();
    writeln!(out, "    unsafe extern \"C\" fn _create<T: {pascal}Hybrid + Default + 'static>() -> NveAppHandle {{").unwrap();
    writeln!(out, "        Box::into_raw(Box::new(T::default())) as NveAppHandle").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    unsafe extern \"C\" fn _destroy<T: {pascal}Hybrid + Default + 'static>(handle: NveAppHandle) {{").unwrap();
    writeln!(out, "        if !handle.is_null() {{").unwrap();
    writeln!(out, "            let _ = unsafe {{ Box::from_raw(handle as *mut T) }};").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();

    if is_view {
        writeln!(
            out,
            "    unsafe extern \"C\" fn _set_props_direct<T: {pascal}Hybrid + Default + 'static>(handle: NveAppHandle, entries: *const NveMapEntry, count: u32) -> i32 {{"
        )
        .unwrap();
        writeln!(out, "        if handle.is_null() {{").unwrap();
        writeln!(out, "            return NveHybridStatus::NullHandle as i32;").unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(out, "        let slice = if entries.is_null() || count == 0 {{").unwrap();
        writeln!(out, "            &[]").unwrap();
        writeln!(out, "        }} else {{").unwrap();
        writeln!(out, "            unsafe {{ std::slice::from_raw_parts(entries, count as usize) }}").unwrap();
        writeln!(out, "        }};").unwrap();
        writeln!(out, "        match unsafe {{ {pascal}Props::decode_props_direct(slice) }} {{").unwrap();
        writeln!(out, "            Ok(props) => {{").unwrap();
        writeln!(out, "                let instance = unsafe {{ &mut *(handle as *mut T) }};").unwrap();
        writeln!(out, "                instance.apply_props(props);").unwrap();
        writeln!(out, "                NveHybridStatus::Ok as i32").unwrap();
        writeln!(out, "            }}").unwrap();
        writeln!(out, "            Err(_) => NveHybridStatus::InvalidArg as i32,").unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(out, "    }}").unwrap();
    }

    for method in methods {
        emit_rust_trampoline(out, pascal, method, spec);
    }

    writeln!(out, "    NveHybrid{pascal}VTable {{").unwrap();
    writeln!(out, "        abi_version: NVE_HYBRID_ABI_VERSION,").unwrap();
    writeln!(out, "        interface_id: NVE_HYBRID_{upper}_INTERFACE_ID,").unwrap();
    writeln!(out, "        schema_hash: NVE_HYBRID_{upper}_SCHEMA_HASH,").unwrap();
    writeln!(out, "        create: _create::<T>,").unwrap();
    writeln!(out, "        destroy: _destroy::<T>,").unwrap();
    writeln!(out, "        alloc_bytes: nve_hybrid_alloc_bytes,").unwrap();
    writeln!(out, "        free_bytes: nve_hybrid_free_bytes,").unwrap();
    if is_view {
        writeln!(out, "        set_props_direct: _set_props_direct::<T>,").unwrap();
    }
    for method in methods {
        let mname = rust_ident(&method.name);
        let raw_snake = to_snake_case(&method.name);
        writeln!(out, "        {mname}: _trampoline_{raw_snake}::<T>,").unwrap();
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_rust_trampoline(
    out: &mut String,
    pascal: &str,
    method: &HybridMethodSpec,
    spec: &HybridFileSpec,
) {
    let mname = rust_ident(&method.name);
    let raw_snake = to_snake_case(&method.name);
    let is_result = method.return_type.is_result();

    write!(
        out,
        "    unsafe extern \"C\" fn _trampoline_{raw_snake}<T: {pascal}Hybrid + Default + 'static>(handle: NveAppHandle"
    )
    .unwrap();
    for p in &method.params {
        let pname = rust_ident(&p.name);
        write!(out, ", {pname}: {}", rust_c_abi_param_type(&p.ty, spec)).unwrap();
    }
    match &method.return_type {
        HybridType::Result { ok, .. } => {
            if !matches!(ok.as_ref(), HybridType::Void) {
                write!(out, ", out_ok: *mut {}", rust_c_abi_return_type(ok, spec)).unwrap();
            }
            writeln!(out, ", out_err: *mut NveHybridOwnedBytes) -> i32 {{").unwrap();
        }
        HybridType::Void => writeln!(out, ") {{").unwrap(),
        other => writeln!(out, ") -> {} {{", rust_c_abi_return_type(other, spec)).unwrap(),
    }

    // Handle null check
    writeln!(out, "        if handle.is_null() {{").unwrap();
    if is_result {
        writeln!(out, "            if !out_err.is_null() {{").unwrap();
        writeln!(
            out,
            "                unsafe {{ *out_err = NveHybridOwnedBytes::from_string(\"null hybrid handle\".to_string()); }}"
        )
        .unwrap();
        writeln!(out, "            }}").unwrap();
        writeln!(out, "            return NveHybridStatus::NullHandle as i32;").unwrap();
    } else if method.return_type == HybridType::Void {
        writeln!(out, "            return;").unwrap();
    } else {
        writeln!(
            out,
            "            return {};",
            rust_c_abi_zero_value(&method.return_type, spec)
        )
        .unwrap();
    }
    writeln!(out, "        }}").unwrap();

    // Decode input params
    let mut call_args = Vec::new();
    for p in &method.params {
        let pname = rust_ident(&p.name);
        let decoded = format!("_arg_{}", to_snake_case(&p.name));
        emit_rust_param_decode(out, &pname, &decoded, &p.ty, &method.return_type, spec);
        call_args.push(decoded);
    }

    writeln!(out, "        let instance = unsafe {{ &mut *(handle as *mut T) }};").unwrap();
    let args_joined = call_args.join(", ");

    match &method.return_type {
        HybridType::Void => {
            writeln!(out, "        instance.{mname}({args_joined});").unwrap();
        }
        HybridType::Result { ok, .. } => {
            writeln!(out, "        match instance.{mname}({args_joined}) {{").unwrap();
            if matches!(ok.as_ref(), HybridType::Void) {
                writeln!(out, "            Ok(()) => {{").unwrap();
                writeln!(out, "                if !out_err.is_null() {{").unwrap();
                writeln!(out, "                    unsafe {{ *out_err = NveHybridOwnedBytes::EMPTY; }}").unwrap();
                writeln!(out, "                }}").unwrap();
                writeln!(out, "                NveHybridStatus::Ok as i32").unwrap();
                writeln!(out, "            }}").unwrap();
            } else {
                writeln!(out, "            Ok(val) => {{").unwrap();
                writeln!(out, "                if !out_ok.is_null() {{").unwrap();
                let encoded = rust_return_encode_expr("val", ok, spec);
                writeln!(out, "                    unsafe {{ *out_ok = {encoded}; }}").unwrap();
                writeln!(out, "                }}").unwrap();
                writeln!(out, "                if !out_err.is_null() {{").unwrap();
                writeln!(out, "                    unsafe {{ *out_err = NveHybridOwnedBytes::EMPTY; }}").unwrap();
                writeln!(out, "                }}").unwrap();
                writeln!(out, "                NveHybridStatus::Ok as i32").unwrap();
                writeln!(out, "            }}").unwrap();
            }
            writeln!(out, "            Err(msg) => {{").unwrap();
            writeln!(out, "                if !out_err.is_null() {{").unwrap();
            writeln!(
                out,
                "                    unsafe {{ *out_err = NveHybridOwnedBytes::from_string(msg); }}"
            )
            .unwrap();
            writeln!(out, "                }}").unwrap();
            writeln!(out, "                NveHybridStatus::Err as i32").unwrap();
            writeln!(out, "            }}").unwrap();
            writeln!(out, "        }}").unwrap();
        }
        other => {
            writeln!(out, "        let _ret = instance.{mname}({args_joined});").unwrap();
            let encoded = rust_return_encode_expr("_ret", other, spec);
            writeln!(out, "        {encoded}").unwrap();
        }
    }

    writeln!(out, "    }}").unwrap();
}

pub(super) fn emit_rust_param_decode(
    out: &mut String,
    raw_name: &str,
    dst_name: &str,
    ty: &HybridType,
    method_ret: &HybridType,
    spec: &HybridFileSpec,
) {
    let invalid_return = if method_ret.is_result() {
        format!(
            "{{ if !out_err.is_null() {{ unsafe {{ *out_err = NveHybridOwnedBytes::from_string(\"invalid hybrid argument '{raw_name}'\".to_string()); }} }} return NveHybridStatus::InvalidArg as i32; }}"
        )
    } else if *method_ret == HybridType::Void {
        "{ return; }".to_string()
    } else {
        format!("{{ return {}; }}", rust_c_abi_zero_value(method_ret, spec))
    };

    match ty {
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
            writeln!(out, "        let {dst_name} = {raw_name};").unwrap();
        }
        HybridType::Enum(e) => {
            writeln!(
                out,
                "        let {dst_name} = match {e}::from_u32({raw_name}) {{ Some(v) => v, None => {invalid_return} }};"
            )
            .unwrap();
        }
        HybridType::Struct(_) if ty.is_pod(spec) => {
            writeln!(out, "        let {dst_name} = {raw_name};").unwrap();
        }
        HybridType::String => {
            writeln!(
                out,
                "        let {dst_name} = match unsafe {{ {raw_name}.as_str() }} {{ Ok(s) => s, Err(_) => {invalid_return} }};"
            )
            .unwrap();
        }
        HybridType::Bytes => {
            writeln!(
                out,
                "        let {dst_name} = unsafe {{ {raw_name}.as_slice() }};"
            )
            .unwrap();
        }
        HybridType::Value => {
            writeln!(
                out,
                "        let {dst_name} = match NveHybridValue::from_ffi_ref(unsafe {{ {raw_name}.as_ref() }}) {{ Ok(v) => v, Err(_) => {invalid_return} }};"
            )
            .unwrap();
        }
        HybridType::Optional(inner) if inner.is_pod(spec) => match inner.as_ref() {
            HybridType::Enum(e) => {
                writeln!(
                    out,
                    "        let {dst_name} = if {raw_name}.is_some {{ match {e}::from_u32({raw_name}.value) {{ Some(v) => Some(v), None => {invalid_return} }} }} else {{ None }};"
                )
                .unwrap();
            }
            _ => {
                writeln!(
                    out,
                    "        let {dst_name} = if {raw_name}.is_some {{ Some({raw_name}.value) }} else {{ None }};"
                )
                .unwrap();
            }
        },
        // Non-POD Struct, Vec<T>, Optional<non-POD>: compact binary NveBinValue slice
        other => {
            writeln!(
                out,
                "        let _{dst_name}_view = match nve_bin_codec::parse_view(unsafe {{ {raw_name}.as_slice() }}) {{ Ok(v) => v, Err(_) => {invalid_return} }};"
            )
            .unwrap();
            let decode_expr = bin_view_decode_expr(&format!("_{dst_name}_view"), other);
            writeln!(
                out,
                "        let {dst_name} = match (|| -> Result<_, DecodeError> {{ Ok({decode_expr}) }})() {{ Ok(v) => v, Err(_) => {invalid_return} }};"
            )
            .unwrap();
        }
    }
}

pub(super) fn rust_return_encode_expr(expr: &str, ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Bool
        | HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::F32
        | HybridType::F64 => expr.to_string(),
        HybridType::Enum(_) => format!("{expr}.to_u32()"),
        HybridType::Struct(_) if ty.is_pod(spec) => expr.to_string(),
        HybridType::String => format!("NveHybridOwnedBytes::from_string({expr})"),
        HybridType::Bytes => format!("NveHybridOwnedBytes::from_vec({expr})"),
        HybridType::Value => format!("NveHybridOwnedBytes::from_vec({expr}.bin_bytes)"),
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            let suffix = inner.pod_opt_suffix().unwrap();
            match inner.as_ref() {
                HybridType::Enum(_) => format!(
                    "match {expr} {{ Some(v) => NveHybridOpt{suffix} {{ is_some: true, value: v.to_u32() }}, None => NveHybridOpt{suffix} {{ is_some: false, value: 0 }} }}"
                ),
                _ => format!(
                    "match {expr} {{ Some(v) => NveHybridOpt{suffix} {{ is_some: true, value: v }}, None => NveHybridOpt{suffix} {{ is_some: false, value: Default::default() }} }}"
                ),
            }
        }
        other => {
            let mut body = String::new();
            writeln!(body, "{{").unwrap();
            writeln!(body, "            let mut _bin = Vec::with_capacity(64);").unwrap();
            emit_bin_write_expr(&mut body, "            ", "&mut _bin", &format!("&{expr}"), other);
            writeln!(body, "            NveHybridOwnedBytes::from_vec(_bin)").unwrap();
            write!(body, "        }}").unwrap();
            body
        }
    }
}

