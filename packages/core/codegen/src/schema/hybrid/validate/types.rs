//! Structural rules for one [`HybridType`] occurrence: known names, supported shapes, and sizes.

use std::collections::{BTreeMap, BTreeSet};

use super::super::*;

/// Most elements one `[T; N]` holds, nested dimensions multiplied (`[[T; M]; N]` holds `N * M`).
/// Swift's Clang importer drops C struct array fields over 4,096 elements, and the cap keeps
/// eager defaults and C arrays small on every host.
pub(crate) const MAX_FIXED_ARRAY_ELEMENTS: usize = 4_096;

/// The element count of `[T; N]` with every nested dimension multiplied, `None` on overflow.
fn fixed_array_elements(ty: &HybridType) -> Option<usize> {
    ty.fixed_array_dims().0.into_iter().try_fold(1usize, usize::checked_mul)
}

pub(super) fn validate_type(
    ty: &HybridType,
    allow_void: bool,
    enum_map: &BTreeMap<&str, &HybridEnumSpec>,
    struct_map: &BTreeMap<&str, &HybridStructSpec>,
    alias_names: &BTreeSet<&str>,
    ctx: &str,
) -> Result<(), CodegenError> {
    match ty {
        HybridType::Void => {
            if !allow_void {
                return Err(CodegenError::validation(format!(
                    "{ctx}: void/'()' is only permitted as a method return or Result<(), String> ok type"
                )));
            }
            Ok(())
        }
        HybridType::Bool
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
        | HybridType::Bytes
        | HybridType::Value => Ok(()),
        HybridType::ConstPtr(inner) | HybridType::MutPtr(inner) => {
            validate_type(inner, true, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::FixedArray { elem, len } => {
            if *len == 0 {
                return Err(CodegenError::validation(format!(
                    "{ctx}: fixed-size array `{}` must hold at least one element",
                    ty.canonical_signature()
                )));
            }
            if fixed_array_elements(ty).is_none_or(|n| n > MAX_FIXED_ARRAY_ELEMENTS) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: fixed-size array `{}` holds more than {MAX_FIXED_ARRAY_ELEMENTS} elements (all dimensions multiplied)",
                    ty.canonical_signature()
                )));
            }
            validate_type(elem, false, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::Handle(name) => {
            if !alias_names.contains(name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: unknown handle alias '{name}'"
                )));
            }
            Ok(())
        }
        HybridType::Optional(inner) => {
            if matches!(inner.as_ref(), HybridType::Result { .. }) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Option<Result<...>> is not supported across FFI"
                )));
            }
            // `None` is encoded as `Null`, which an inner `Option` / dynamic value can hold too:
            // `Some(None)` / `Some(null)` would decode as `None`.
            if matches!(inner.as_ref(), HybridType::Optional(_) | HybridType::Value) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Option<{}> is ambiguous on the wire (its `None` and `Some(null)` both encode as null)",
                    inner.canonical_signature()
                )));
            }
            validate_type(inner, false, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::Vec(inner) => {
            if matches!(inner.as_ref(), HybridType::Result { .. }) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Vec<Result<...>> is not supported across FFI"
                )));
            }
            // One spelling per wire form: a byte vector crosses as one `Bytes` run, not an `Array`.
            if matches!(inner.as_ref(), HybridType::U8) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Vec<u8> must be declared as Bytes (one `Bytes` run on the wire)"
                )));
            }
            validate_type(inner, false, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::Result { ok, err } => {
            if !allow_void {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Result<T, String> is only supported as a top-level method return type"
                )));
            }
            if matches!(ok.as_ref(), HybridType::Result { .. }) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: nested Result<Result<...>> is not supported"
                )));
            }
            if !matches!(err.as_ref(), HybridType::String) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Result error type must be String (found '{}')",
                    err.canonical_signature()
                )));
            }
            validate_type(ok, true, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::Struct(name) => {
            if !struct_map.contains_key(name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: unknown struct or type '{name}'"
                )));
            }
            Ok(())
        }
        HybridType::Enum(name) => {
            if !enum_map.contains_key(name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: unknown enum '{name}'"
                )));
            }
            Ok(())
        }
    }
}
