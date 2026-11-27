//! Typed binary `NveBinValue` helpers for `import_rn` Rust emitter.

use std::fmt::Write as _;

use crate::schema::export::{DefaultValue, EventSpec, ObjectField, ParamSpec, PropType};
use crate::naming::{
    escape_c_like_string, int_enum_variant_pascal, rust_ident, strip_on_prefix, to_pascal_case,
};

pub(super) fn emit_rust_bin_map_payload(
    out: &mut String,
    params: &[ParamSpec],
    indent: &str,
    respect_optional: bool,
) {
    let _ = writeln!(out, "{indent}let mut entries: Vec<NveMapEntry> = Vec::with_capacity({});", params.len());
    for p in params {
        let pname = rust_ident(&p.name);
        let key = escape_c_like_string(&p.name);
        if respect_optional && p.optional {
            let val_expr = rust_nve_value_expr("v", &p.param_type, true);
            let _ = writeln!(out, "{indent}if let Some(ref v) = {pname} {{");
            let _ = writeln!(out, "{indent}    entries.push(NveMapEntry::new(\"{key}\", {val_expr}));");
            let _ = writeln!(out, "{indent}}}");
        } else {
            let val_expr = rust_nve_value_expr(&pname, &p.param_type, false);
            let _ = writeln!(out, "{indent}entries.push(NveMapEntry::new(\"{key}\", {val_expr}));");
        }
    }
    let _ = writeln!(out, "{indent}let map_val = NveValue::map(&entries);");
    let _ = writeln!(
        out,
        "{indent}let payload = encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&map_val) }})?;"
    );
}

pub(super) fn rust_nve_value_expr(var: &str, ty: &PropType, is_ref: bool) -> String {
    let deref = if is_ref { format!("*{var}") } else { var.to_owned() };
    let as_str = format!("{var}.as_str()");
    match ty {
        PropType::Bool => format!("NveValue::bool({deref})"),
        PropType::Int32 | PropType::IntEnum { .. } => format!("NveValue::i64({deref} as i64)"),
        PropType::Float => format!("NveValue::f64({deref} as f64)"),
        PropType::Double => format!("NveValue::f64({deref})"),
        PropType::Color => format!("NveValue::i64({deref}.to_u32() as i64)"),
        PropType::String
        | PropType::ImageSource
        | PropType::Dimension
        | PropType::StringEnum { .. }
        | PropType::Mixed
        | PropType::Callback { .. } => format!("NveValue::from_str({as_str})"),
        PropType::Object { .. } => format!("NveValue::from_bytes({var}.as_slice())"),
        _ => "NveValue::null()".to_owned(),
    }
}

pub(super) fn emit_rust_aux_types(
    out: &mut String,
    comp_pascal: &str,
    prop_name: &str,
    prop_type: &PropType,
    default_val: Option<&DefaultValue>,
) {
    match prop_type {
        PropType::StringEnum { variants } => {
            let enum_name = format!("{comp_pascal}{}", to_pascal_case(prop_name));
            let _ = writeln!(out, "    #[derive(Debug, Clone, Copy, PartialEq, Eq)]");
            let _ = writeln!(out, "    pub enum {enum_name} {{");
            for v in variants {
                let _ = writeln!(out, "        {},", to_pascal_case(v));
            }
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out);
            let _ = writeln!(out, "    impl {enum_name} {{");
            let _ = writeln!(out, "        pub const fn as_str(self) -> &'static str {{");
            let _ = writeln!(out, "            match self {{");
            for v in variants {
                let _ = writeln!(
                    out,
                    "                Self::{} => \"{}\",",
                    to_pascal_case(v),
                    escape_c_like_string(v)
                );
            }
            let _ = writeln!(out, "            }}");
            let _ = writeln!(out, "        }}");
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out);
            if let Some(first) = variants.first() {
                let def_variant = match default_val {
                    Some(DefaultValue::String(s)) if variants.contains(s) => to_pascal_case(s),
                    _ => to_pascal_case(first),
                };
                let _ = writeln!(out, "    impl Default for {enum_name} {{");
                let _ = writeln!(out, "        fn default() -> Self {{ Self::{def_variant} }}");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out);
            }
            let _ = writeln!(out, "    impl From<{enum_name}> for NveValue {{");
            let _ = writeln!(
                out,
                "        fn from(v: {enum_name}) -> Self {{ NveValue::from_str(v.as_str()) }}"
            );
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out);
        }
        PropType::IntEnum { variants } => {
            let enum_name = format!("{comp_pascal}{}", to_pascal_case(prop_name));
            let _ = writeln!(out, "    #[derive(Debug, Clone, Copy, PartialEq, Eq)]");
            let _ = writeln!(out, "    pub enum {enum_name} {{");
            for v in variants {
                let _ = writeln!(out, "        {} = {v},", int_enum_variant_pascal(*v));
            }
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out);
            if let Some(first) = variants.first() {
                let def_v = match default_val {
                    Some(DefaultValue::Int(i)) if variants.contains(&(*i as i32)) => {
                        int_enum_variant_pascal(*i as i32)
                    }
                    _ => int_enum_variant_pascal(*first),
                };
                let _ = writeln!(out, "    impl Default for {enum_name} {{");
                let _ = writeln!(out, "        fn default() -> Self {{ Self::{def_v} }}");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out);
            }
            let _ = writeln!(out, "    impl From<{enum_name}> for NveValue {{");
            let _ = writeln!(
                out,
                "        fn from(v: {enum_name}) -> Self {{ NveValue::i64(v as i64) }}"
            );
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out);
        }
        PropType::Array { item } => {
            emit_rust_aux_types(
                out,
                comp_pascal,
                &format!("{prop_name}Item"),
                item,
                default_val,
            );
        }
        PropType::Object { fields } => {
            let struct_name = format!("{comp_pascal}{}", to_pascal_case(prop_name));
            emit_rust_object_struct(out, &struct_name, fields);
        }
        _ => {}
    }
}

pub(super) fn emit_rust_event_payload(out: &mut String, comp_pascal: &str, ev: &EventSpec) {
    if ev.payload.is_empty() {
        return;
    }
    let variant = to_pascal_case(&strip_on_prefix(&ev.name));
    let struct_name = format!("{comp_pascal}{variant}Event");
    emit_rust_object_struct(out, &struct_name, &ev.payload);
}

pub(super) fn emit_rust_object_struct(out: &mut String, struct_name: &str, fields: &[ObjectField]) {
    let _ = writeln!(out, "    #[derive(Debug, Clone, PartialEq, Default)]");
    let _ = writeln!(out, "    pub struct {struct_name} {{");
    for f in fields {
        let fname = rust_ident(&f.name);
        let fty = rust_simple_type(&f.field_type);
        if f.optional || f.nullable {
            let _ = writeln!(out, "        pub {fname}: Option<{fty}>,");
        } else {
            let _ = writeln!(out, "        pub {fname}: {fty},");
        }
    }
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);

    let _ = writeln!(out, "    impl {struct_name} {{");
    let _ = writeln!(out, "        pub fn to_bin_bytes(&self) -> Result<Vec<u8>, DecodeError> {{");
    let _ = writeln!(
        out,
        "            let mut entries: Vec<NveMapEntry> = Vec::with_capacity({});",
        fields.len()
    );
    for f in fields {
        let fname = rust_ident(&f.name);
        let key = escape_c_like_string(&f.name);
        if f.optional || f.nullable {
            let val_expr = rust_nve_value_expr("v", &f.field_type, true);
            let _ = writeln!(out, "            if let Some(ref v) = self.{fname} {{");
            let _ = writeln!(
                out,
                "                entries.push(NveMapEntry::new(\"{key}\", {val_expr}));"
            );
            let _ = writeln!(out, "            }}");
        } else {
            let val_expr = rust_nve_value_expr(&format!("self.{fname}"), &f.field_type, false);
            let _ = writeln!(
                out,
                "            entries.push(NveMapEntry::new(\"{key}\", {val_expr}));"
            );
        }
    }
    let _ = writeln!(out, "            let map_val = NveValue::map(&entries);");
    let _ = writeln!(
        out,
        "            encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&map_val) }})"
    );
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "        pub fn decode_bin(data: &[u8]) -> Option<Self> {{");
    let _ = writeln!(out, "            let view = BinValueView::parse(data).ok()?;");
    let _ = writeln!(out, "            Some(Self {{");
    for f in fields {
        let fname = rust_ident(&f.name);
        let key = escape_c_like_string(&f.name);
        let expr = rust_decode_bin_field_expr(&key, f);
        let _ = writeln!(out, "                {fname}: {expr},");
    }
    let _ = writeln!(out, "            }})");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);
}

pub(super) fn rust_decode_bin_field_expr(key: &str, f: &ObjectField) -> String {
    let opt = f.optional || f.nullable;
    let base = match &f.field_type {
        PropType::Bool => format!("view.map_get(\"{key}\").and_then(|v| v.as_bool())"),
        PropType::Int32 | PropType::IntEnum { .. } => {
            format!("view.map_get(\"{key}\").and_then(|v| v.as_i64()).map(|n| n as i32)")
        }
        PropType::Float => {
            format!("view.map_get(\"{key}\").and_then(|v| v.as_f64()).map(|n| n as f32)")
        }
        PropType::Double => format!("view.map_get(\"{key}\").and_then(|v| v.as_f64())"),
        PropType::Color => format!(
            "view.map_get(\"{key}\").and_then(|v| v.as_i64()).map(|n| ColorRgba::from_u32(n as u32))"
        ),
        PropType::Object { .. } => {
            format!("view.map_get(\"{key}\").and_then(|v| v.as_bytes()).map(|b| b.to_vec())")
        }
        _ => format!("view.map_get(\"{key}\").and_then(|v| v.as_str()).map(str::to_owned)"),
    };
    if opt {
        base
    } else {
        format!("{base}.unwrap_or_default()")
    }
}

pub(super) fn rust_prop_type(comp_pascal: &str, prop_name: &str, ty: &PropType) -> String {
    match ty {
        PropType::StringEnum { .. } | PropType::IntEnum { .. } | PropType::Object { .. } => {
            format!("{comp_pascal}{}", to_pascal_case(prop_name))
        }
        PropType::Array { item } => {
            let inner = rust_prop_type(comp_pascal, &format!("{prop_name}Item"), item);
            format!("Vec<{inner}>")
        }
        other => rust_simple_type(other),
    }
}

pub(super) fn rust_simple_type(ty: &PropType) -> String {
    match ty {
        PropType::Bool => "bool".to_owned(),
        PropType::Int32 => "i32".to_owned(),
        PropType::Float => "f32".to_owned(),
        PropType::Double => "f64".to_owned(),
        PropType::String => "String".to_owned(),
        PropType::Color => "ColorRgba".to_owned(),
        PropType::ImageSource | PropType::Dimension | PropType::Mixed => "String".to_owned(),
        PropType::Point => "(f32, f32)".to_owned(),
        PropType::EdgeInsets => "(f32, f32, f32, f32)".to_owned(),
        PropType::StringEnum { .. } => "String".to_owned(),
        PropType::IntEnum { .. } => "i32".to_owned(),
        PropType::Array { item } => format!("Vec<{}>", rust_simple_type(item)),
        PropType::Object { .. } => "Vec<u8>".to_owned(),
        PropType::Void => "()".to_owned(),
        PropType::Promise { inner } => rust_simple_type(inner),
        PropType::Callback { .. } => "String".to_owned(),
    }
}

pub(super) fn rust_prop_needs_preencode(ty: &PropType) -> bool {
    matches!(
        ty,
        PropType::Point | PropType::EdgeInsets | PropType::Object { .. } | PropType::Array { .. }
    )
}

pub(super) fn rust_prop_preencode_stmt(
    field: &str,
    ty: &PropType,
    is_option: bool,
) -> Option<String> {
    if !rust_prop_needs_preencode(ty) {
        return None;
    }
    let encode_var = |var: &str| match ty {
        PropType::Point => format!(
            "{{ let arr = [NveValue::f64({var}.0 as f64), NveValue::f64({var}.1 as f64)]; let v = NveValue::array(&arr); encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&v) }})? }}"
        ),
        PropType::EdgeInsets => format!(
            "{{ let arr = [NveValue::f64({var}.0 as f64), NveValue::f64({var}.1 as f64), NveValue::f64({var}.2 as f64), NveValue::f64({var}.3 as f64)]; let v = NveValue::array(&arr); encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&v) }})? }}"
        ),
        PropType::Object { .. } => format!("{var}.to_bin_bytes()?"),
        PropType::Array { item } => match item.as_ref() {
            PropType::Object { .. } => format!(
                "{{ let encoded_items: Vec<Vec<u8>> = {var}.iter().map(|item| item.to_bin_bytes()).collect::<Result<Vec<_>, _>>()?; let items: Vec<NveValue> = encoded_items.iter().map(|b| NveValue::from_bytes(b.as_slice())).collect(); let v = NveValue::array(&items); encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&v) }})? }}"
            ),
            PropType::Point => format!(
                "{{ let encoded_items: Vec<Vec<u8>> = {var}.iter().map(|item| {{ let arr = [NveValue::f64(item.0 as f64), NveValue::f64(item.1 as f64)]; let v = NveValue::array(&arr); encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&v) }}) }}).collect::<Result<Vec<_>, _>>()?; let items: Vec<NveValue> = encoded_items.iter().map(|b| NveValue::from_bytes(b.as_slice())).collect(); let v = NveValue::array(&items); encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&v) }})? }}"
            ),
            PropType::EdgeInsets => format!(
                "{{ let encoded_items: Vec<Vec<u8>> = {var}.iter().map(|item| {{ let arr = [NveValue::f64(item.0 as f64), NveValue::f64(item.1 as f64), NveValue::f64(item.2 as f64), NveValue::f64(item.3 as f64)]; let v = NveValue::array(&arr); encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&v) }}) }}).collect::<Result<Vec<_>, _>>()?; let items: Vec<NveValue> = encoded_items.iter().map(|b| NveValue::from_bytes(b.as_slice())).collect(); let v = NveValue::array(&items); encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&v) }})? }}"
            ),
            _ => {
                let elem_expr = rust_nve_value_expr("item", item, true);
                format!(
                    "{{ let items: Vec<NveValue> = {var}.iter().map(|item| {elem_expr}).collect(); let v = NveValue::array(&items); encode_bin_value_to_vec(unsafe {{ NveValueRef::from_raw(&v) }})? }}"
                )
            }
        },
        _ => unreachable!(),
    };
    if is_option {
        let body = encode_var("val");
        Some(format!(
            "let _bin_{field}: Option<Vec<u8>> = match self.{field}.as_ref() {{ Some(val) => Some({body}), None => None }};"
        ))
    } else {
        let body = encode_var(&format!("self.{field}"));
        Some(format!("let _bin_{field}: Vec<u8> = {body};"))
    }
}

pub(super) fn rust_prop_to_nve_value_expr(
    var: &str,
    _field: &str,
    ty: &PropType,
    is_ref: bool,
) -> String {
    let deref = if is_ref { format!("*{var}") } else { var.to_owned() };
    match ty {
        PropType::Bool => format!("NveValue::bool({deref})"),
        PropType::Int32 => format!("NveValue::i64({deref} as i64)"),
        PropType::Float => format!("NveValue::f64({deref} as f64)"),
        PropType::Double => format!("NveValue::f64({deref})"),
        PropType::String | PropType::ImageSource | PropType::Dimension | PropType::Mixed => {
            format!("NveValue::from_str({var}.as_str())")
        }
        PropType::Color => format!("NveValue::i64({deref}.to_u32() as i64)"),
        PropType::StringEnum { .. } | PropType::IntEnum { .. } => {
            format!("NveValue::from({deref})")
        }
        PropType::Void | PropType::Promise { .. } | PropType::Callback { .. } => {
            "NveValue::null()".to_owned()
        }
        PropType::Point | PropType::EdgeInsets | PropType::Object { .. } | PropType::Array { .. } => {
            unreachable!("handled via rust_prop_preencode_stmt")
        }
    }
}
