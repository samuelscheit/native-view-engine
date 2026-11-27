//! Rust and C-ABI spellings of hybrid types, and which types cross as `NveBinValue`.

use std::collections::BTreeSet;

use crate::error::CodegenError;
use crate::naming::{escape_c_like_string, format_float_literal};
use crate::schema::hybrid::{HybridDefaultValue, HybridFileSpec, HybridMethodSpec, HybridType};

/// Prelude items generated code names by path, so a spec type of the same name never shadows them.
pub(super) const RESULT: &str = "::core::result::Result";
pub(super) const DEFAULT: &str = "::core::default::Default";
pub(super) const FROM: &str = "::core::convert::From";

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
        HybridType::String => "::std::string::String".to_owned(),
        HybridType::Bytes => "::std::vec::Vec<u8>".to_owned(),
        HybridType::Value => "NveHybridValue".to_owned(),
        HybridType::Optional(inner) => {
            format!("::core::option::Option<{}>", rust_owned_type(inner))
        }
        HybridType::Vec(inner) => format!("::std::vec::Vec<{}>", rust_owned_type(inner)),
        HybridType::Result { ok, err } => {
            format!(
                "{RESULT}<{}, {}>",
                rust_owned_type(ok),
                rust_owned_type(err)
            )
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

/// Whether a by-value POD type has bit patterns a C caller may pass that are undefined behavior
/// for its Rust type (`bool`, enums, and aggregates holding one), so it crosses as a raw carrier.
pub(super) fn needs_raw(ty: &HybridType, spec: &HybridFileSpec) -> bool {
    match ty {
        HybridType::Bool | HybridType::Enum(_) => true,
        HybridType::Struct(name) => spec
            .find_struct(name)
            .is_some_and(|s| s.fields.iter().any(|f| needs_raw(&f.ty, spec))),
        HybridType::FixedArray { elem, .. } => needs_raw(elem, spec),
        _ => false,
    }
}

/// C-ABI carrier of a POD type: `bool` as `u8`, enums as their `u32` discriminant, structs that
/// hold either as their generated `NveHybridRaw<Name>`.
pub(super) fn raw_c_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    match ty {
        HybridType::Bool => "u8".to_owned(),
        HybridType::Enum(_) => "u32".to_owned(),
        HybridType::Struct(name) if needs_raw(ty, spec) => format!("NveHybridRaw{name}"),
        HybridType::FixedArray { elem, len } => format!("[{}; {len}]", raw_c_type(elem, spec)),
        other => rust_owned_type(other),
    }
}

/// `Result<T, DecodeError>` expression validating carrier `expr` into its Rust type, or `None`
/// when the carrier already is the Rust type.
pub(super) fn raw_decode_expr(
    expr: &str,
    ty: &HybridType,
    spec: &HybridFileSpec,
) -> Option<String> {
    if !needs_raw(ty, spec) {
        return None;
    }
    Some(match ty {
        HybridType::Bool => format!("nve_abi::hybrid::decode_bool({expr})"),
        HybridType::Enum(e) => format!(
            "nve_abi::hybrid::decode_enum(<i64 as ::core::convert::From<u32>>::from({expr}), \"{e}\", {e}::from_u32)"
        ),
        HybridType::FixedArray { elem, .. } => format!(
            "nve_abi::hybrid::decode_array({expr}, |_nve_e| {})",
            raw_decode_expr("_nve_e", elem, spec)?
        ),
        _ => format!("{expr}.decode()"),
    })
}

/// Carrier of Rust value `expr` (infallible: Rust values are always valid).
pub(super) fn raw_encode_expr(expr: &str, ty: &HybridType, spec: &HybridFileSpec) -> String {
    if !needs_raw(ty, spec) {
        return expr.to_owned();
    }
    match ty {
        HybridType::Bool => format!("<u8 as {FROM}<bool>>::from({expr})"),
        HybridType::Enum(_) => format!("{expr}.to_u32()"),
        HybridType::FixedArray { elem, .. } => format!(
            "{expr}.map(|_nve_e| {})",
            raw_encode_expr("_nve_e", elem, spec)
        ),
        _ => format!(
            "<{} as {FROM}<{}>>::from({expr})",
            raw_c_type(ty, spec),
            rust_owned_type(ty)
        ),
    }
}

/// Whether `#[derive(Default)]` cannot cover a field of type `ty`: std implements `Default` for
/// `[T; N]` only up to `N = 32`.
pub(super) fn needs_manual_default(ty: &HybridType) -> bool {
    match ty {
        HybridType::FixedArray { elem, len } => *len > 32 || needs_manual_default(elem),
        _ => false,
    }
}

/// Default value expression of a field, valid for fixed-size arrays of any length.
pub(super) fn default_expr(ty: &HybridType) -> String {
    match ty {
        HybridType::FixedArray { elem, .. } => {
            format!("::core::array::from_fn(|_| {})", default_expr(elem))
        }
        _ => format!("{DEFAULT}::default()"),
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
        _ if is_by_value(ty, spec) => raw_c_type(ty, spec),
        HybridType::String => "nve_abi::NveStrSlice".to_owned(),
        HybridType::Value => "nve_abi::NveValue".to_owned(),
        _ => "nve_abi::NveBufferSlice".to_owned(),
    }
}

/// Whether `ty` returns through an owned `NveHybridOwnedBytes` buffer.
pub(super) fn returns_owned_bytes(ty: &HybridType, spec: &HybridFileSpec) -> bool {
    ty.optional_pod_suffix(spec).is_none() && *ty != HybridType::Void && !is_by_value(ty, spec)
}

pub(super) fn c_return_type(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if let Some(suffix) = ty.optional_pod_suffix(spec) {
        return format!("NveHybridOpt{suffix}");
    }
    match ty {
        HybridType::Void => "()".to_owned(),
        _ if is_by_value(ty, spec) => raw_c_type(ty, spec),
        _ => "NveHybridOwnedBytes".to_owned(),
    }
}

/// Value a trampoline returns when it cannot call the implementation (null handle, bad argument).
pub(super) fn c_zero_value(ty: &HybridType, spec: &HybridFileSpec) -> String {
    if returns_owned_bytes(ty, spec) {
        "NveHybridOwnedBytes::EMPTY".to_owned()
    } else {
        default_expr(ty)
    }
}

/// `NveValue` of an event parameter that needs no encoding (scalars, enums, strings, bytes).
///
/// Mirrors the `HybridCodec` of each type: `u64` bit-casts, `usize` past `i64::MAX` is an error
/// (the enclosing function returns `Result<_, DecodeError>`).
pub(super) fn direct_nve_value_expr(expr: &str, ty: &HybridType) -> Option<String> {
    const V: &str = "nve_abi::NveValue";
    Some(match ty {
        HybridType::Bool => format!("{V}::bool({expr})"),
        HybridType::U8 | HybridType::U16 | HybridType::I32 | HybridType::U32 => {
            format!("{V}::i64(<i64 as {FROM}<_>>::from({expr}))")
        }
        HybridType::I64 => format!("{V}::i64({expr})"),
        HybridType::U64 => format!("{V}::i64(i64::from_ne_bytes({expr}.to_ne_bytes()))"),
        HybridType::Usize => format!(
            "{V}::i64(i64::try_from({expr}).map_err(|_| nve_abi::DecodeError::OutOfRange(\"usize\"))?)"
        ),
        HybridType::F32 => format!("{V}::f64(<f64 as {FROM}<f32>>::from({expr}))"),
        HybridType::F64 => format!("{V}::f64({expr})"),
        HybridType::Enum(_) => format!("{V}::i64(<i64 as {FROM}<u32>>::from({expr}.to_u32()))"),
        HybridType::String => format!("{V}::from_str({expr}.as_str())"),
        HybridType::Bytes => format!("{V}::from_bytes({expr}.as_slice())"),
        _ => return None,
    })
}

pub(super) fn rust_prop_default_expr(ty: &HybridType, def: Option<&HybridDefaultValue>) -> String {
    let Some(def) = def else {
        return default_expr(ty);
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
        (HybridDefaultValue::String(s), _) => format!(
            "<::std::string::String as {FROM}<&str>>::from(\"{}\")",
            escape_c_like_string(s)
        ),
        (HybridDefaultValue::EnumVariant { enum_name, variant }, _) => {
            format!("{enum_name}::{variant}")
        }
        (HybridDefaultValue::None, _) => default_expr(ty),
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
