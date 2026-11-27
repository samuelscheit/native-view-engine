//! Rust and C-ABI spellings of hybrid types, and which types cross as `NveBinValue`.

use std::collections::BTreeSet;

use crate::error::CodegenError;
use crate::naming::{escape_c_like_string, format_float_literal};
use crate::schema::hybrid::{HybridDefaultValue, HybridFileSpec, HybridMethodSpec, HybridType};

pub(super) fn rust_owned_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "()".to_owned(),
        HybridType::Bool => "bool".to_owned(),
        HybridType::U8 => "u8".to_owned(),
        HybridType::U16 => "u16".to_owned(),
        HybridType::Usize => "usize".to_owned(),
        HybridType::I32 => "i32".to_owned(),
        HybridType::U32 => "u32".to_owned(),
        HybridType::I64 => "i64".to_owned(),
        HybridType::U64 => "u64".to_owned(),
        HybridType::F32 => "f32".to_owned(),
        HybridType::F64 => "f64".to_owned(),
        HybridType::String => "String".to_owned(),
        HybridType::Bytes => "Vec<u8>".to_owned(),
        HybridType::Value => "NveHybridValue".to_owned(),
        HybridType::Optional(inner) => format!("Option<{}>", rust_owned_type(inner)),
        HybridType::Vec(inner) => format!("Vec<{}>", rust_owned_type(inner)),
        HybridType::Result { ok, err } => {
            format!("Result<{}, {}>", rust_owned_type(ok), rust_owned_type(err))
        }
        HybridType::Struct(name) | HybridType::Enum(name) | HybridType::Handle(name) => {
            name.clone()
        }
        HybridType::ConstPtr(inner) => format!("*const {}", pointee(inner)),
        HybridType::MutPtr(inner) => format!("*mut {}", pointee(inner)),
        HybridType::FixedArray { elem, len } => format!("[{}; {len}]", rust_owned_type(elem)),
    }
}

fn pointee(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "std::ffi::c_void".to_owned(),
        other => rust_owned_type(other),
    }
}

/// Trait-method parameter type: strings and bytes are borrowed from the caller's buffer.
pub(super) fn rust_param_type(ty: &HybridType) -> String {
    match ty {
        HybridType::String => "&str".to_owned(),
        HybridType::Bytes => "&[u8]".to_owned(),
        other => rust_owned_type(other),
    }
}

/// C-ABI spelling of a POD type: enums cross as their `u32` discriminant.
pub(super) fn pod_c_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Enum(_) => "u32".to_owned(),
        HybridType::FixedArray { elem, len } => format!("[{}; {len}]", pod_c_type(elem)),
        other => rust_owned_type(other),
    }
}

/// Crosses a vtable signature by value (scalars, enums, POD structs). A top-level fixed-size array
/// does not (C passes arrays by pointer), so it crosses as `NveBinValue` like other aggregates.
pub(super) fn is_by_value(ty: &HybridType, spec: &HybridFileSpec) -> bool {
    ty.is_pod(spec) && !matches!(ty, HybridType::FixedArray { .. })
}

pub(super) fn c_param_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if let Some(suffix) = ty.optional_pod_suffix(spec) {
        return format!("NveHybridOpt{suffix}");
    }
    match ty {
        _ if is_by_value(ty, spec) => pod_c_type(ty),
        HybridType::String => "NveStrSlice".to_owned(),
        HybridType::Value => "NveValue".to_owned(),
        _ => "NveBufferSlice".to_owned(),
    }
}

pub(super) fn c_return_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if let Some(suffix) = ty.optional_pod_suffix(spec) {
        return format!("NveHybridOpt{suffix}");
    }
    match ty {
        HybridType::Void => "()".to_owned(),
        _ if is_by_value(ty, spec) => pod_c_type(ty),
        _ => "NveHybridOwnedBytes".to_owned(),
    }
}

/// Value a trampoline returns when it cannot call the implementation (null handle, bad argument).
pub(super) fn c_zero_value(ty: &HybridType, spec: &HybridFileSpec) -> &'static str {
    if c_return_type(ty, spec) == "NveHybridOwnedBytes" {
        "NveHybridOwnedBytes::EMPTY"
    } else {
        "Default::default()"
    }
}

/// `NveValue` of an event parameter that needs no encoding (scalars, enums, strings, bytes).
pub(super) fn direct_nve_value_expr(expr: &str, ty: &HybridType) -> Option<String> {
    Some(match ty {
        HybridType::Bool => format!("NveValue::bool({expr})"),
        HybridType::U8 | HybridType::U16 | HybridType::I32 | HybridType::U32 => {
            format!("NveValue::i64(i64::from({expr}))")
        }
        HybridType::I64 => format!("NveValue::i64({expr})"),
        // Bit-cast, matching the `NveBinValue` codec of `u64` / `usize`.
        HybridType::U64 | HybridType::Usize => format!("NveValue::i64({expr} as i64)"),
        HybridType::F32 => format!("NveValue::f64(f64::from({expr}))"),
        HybridType::F64 => format!("NveValue::f64({expr})"),
        HybridType::Enum(_) => format!("NveValue::i64(i64::from({expr}.to_u32()))"),
        HybridType::String => format!("NveValue::from_str({expr}.as_str())"),
        HybridType::Bytes => format!("NveValue::from_bytes({expr}.as_slice())"),
        _ => return None,
    })
}

pub(super) fn rust_prop_default_expr(ty: &HybridType, def: Option<&HybridDefaultValue>) -> String {
    let Some(def) = def else {
        return "Default::default()".to_owned();
    };
    if let HybridType::Optional(inner) = ty {
        return match def {
            HybridDefaultValue::None => "None".to_owned(),
            other => format!("Some({})", rust_prop_default_expr(inner, Some(other))),
        };
    }
    match (def, ty) {
        (HybridDefaultValue::Bool(b), _) => b.to_string(),
        (HybridDefaultValue::Int(i), HybridType::F32 | HybridType::F64) => format!("{i}.0"),
        (HybridDefaultValue::Int(i), _) => i.to_string(),
        (HybridDefaultValue::UInt(u), HybridType::F32 | HybridType::F64) => format!("{u}.0"),
        (HybridDefaultValue::UInt(u), _) => u.to_string(),
        (HybridDefaultValue::Float(f), _) => format_float_literal(*f),
        (HybridDefaultValue::String(s), _) => format!("\"{}\".to_owned()", escape_c_like_string(s)),
        (HybridDefaultValue::EnumVariant { enum_name, variant }, _) => {
            format!("{enum_name}::{variant}")
        }
        (HybridDefaultValue::None, _) => "Default::default()".to_owned(),
    }
}

/// Which declared types the `nve_abi::hybrid` codec can encode: everything except unions, raw
/// pointers, and handles (C-only), and structs that contain one transitively.
pub(super) struct CodecSupport<'a> {
    blocked: BTreeSet<&'a str>,
}

impl<'a> CodecSupport<'a> {
    pub(super) fn new(spec: &'a HybridFileSpec) -> Self {
        let mut blocked: BTreeSet<&str> = spec
            .structs
            .iter()
            .filter(|s| s.is_union)
            .map(|s| s.name.as_str())
            .collect();
        loop {
            let newly: Vec<&str> = spec
                .structs
                .iter()
                .filter(|s| !blocked.contains(s.name.as_str()))
                .filter(|s| s.fields.iter().any(|f| !encodable(&blocked, &f.ty)))
                .map(|s| s.name.as_str())
                .collect();
            if newly.is_empty() {
                return Self { blocked };
            }
            blocked.extend(newly);
        }
    }

    pub(super) fn encodes_struct(&self, name: &str) -> bool {
        !self.blocked.contains(name)
    }

    /// Every prop, event parameter, and method signature of a module or view must cross a Rust
    /// vtable by value or as `NveBinValue`.
    pub(super) fn check_members(&self, spec: &HybridFileSpec) -> Result<(), CodegenError> {
        for m in &spec.modules {
            self.check_methods(&m.methods, &format!("module '{}'", m.name))?;
        }
        for v in &spec.views {
            let owner = format!("view '{}'", v.name);
            for p in &v.props {
                self.check(&p.ty, &format!("{owner} prop '{}'", p.name))?;
            }
            for ev in &v.events {
                for p in &ev.params {
                    self.check(
                        &p.ty,
                        &format!("{owner} event '{}' param '{}'", ev.name, p.name),
                    )?;
                }
            }
            self.check_methods(&v.methods, &owner)?;
        }
        Ok(())
    }

    fn check_methods(&self, methods: &[HybridMethodSpec], owner: &str) -> Result<(), CodegenError> {
        for m in methods {
            for p in &m.params {
                self.check(
                    &p.ty,
                    &format!("{owner} method '{}' param '{}'", m.name, p.name),
                )?;
            }
            let ret = match &m.return_type {
                HybridType::Result { ok, err } => {
                    self.check(err, &format!("{owner} method '{}' error", m.name))?;
                    ok.as_ref()
                }
                other => other,
            };
            if *ret != HybridType::Void {
                self.check(ret, &format!("{owner} method '{}' return", m.name))?;
            }
        }
        Ok(())
    }

    fn check(&self, ty: &HybridType, ctx: &str) -> Result<(), CodegenError> {
        if encodable(&self.blocked, ty) {
            return Ok(());
        }
        Err(CodegenError::validation(format!(
            "Rust hybrid target: {ctx} has type '{}', which holds a union, raw pointer, or handle \
             and cannot cross a Rust hybrid vtable",
            ty.canonical_signature()
        )))
    }
}

fn encodable(blocked: &BTreeSet<&str>, ty: &HybridType) -> bool {
    match ty {
        HybridType::Void
        | HybridType::Result { .. }
        | HybridType::ConstPtr(_)
        | HybridType::MutPtr(_)
        | HybridType::Handle(_) => false,
        HybridType::Struct(name) => !blocked.contains(name.as_str()),
        HybridType::Optional(inner)
        | HybridType::Vec(inner)
        | HybridType::FixedArray { elem: inner, .. } => encodable(blocked, inner),
        _ => true,
    }
}
