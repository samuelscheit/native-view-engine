//! Identifier rules shared by every hybrid target.

use crate::error::CodegenError;
use crate::naming::to_snake_case;

/// Prefix of the names generated code binds itself (locals, parameters, type parameters) in every
/// target language, so a spec identifier can never shadow or collide with one.
pub(super) const RESERVED_BINDING_PREFIX: &str = "_nve_";

/// Prefix of generated top-level items (`NveHybrid<Name>VTable`, `NveHybridOpt<T>`, ...).
const RESERVED_TYPE_PREFIX: &str = "NveHybrid";

/// Top-level symbols the generated bindings import from the hybrid runtimes: C++
/// `nve::hybrid` / `nve::host` and Rust `nve_abi` / `nve_abi::hybrid`.
const RESERVED_RUNTIME_NAMES: &[&str] = &[
    "BinReader",
    "BinWriter",
    "DynamicValue",
    "NveValueTree",
    "OwnedBytes",
    "checked_bytes",
    "to_slice",
    "require_done",
    "BinMapWriter",
    "BinValueView",
    "DecodeError",
    "HybridCodec",
    "HybridElem",
    "HybridFields",
    "HybridStruct",
    "NveAppHandle",
    "NveBufferSlice",
    "NveDirectEvent",
    "NveMapEntry",
    "NveMapRef",
    "NveStrSlice",
    "NveValue",
    "NveValueRef",
];

/// Slots every module/view vtable carries ahead of its methods (C header, all bindings), and the
/// props hook of the Rust view trait (`apply_props`); compared in snake case.
const RESERVED_VTABLE_MEMBERS: &[&str] = &[
    "abi_version",
    "interface_id",
    "schema_hash",
    "create",
    "destroy",
    "alloc_bytes",
    "free_bytes",
    "set_props_direct",
    "apply_props",
];

pub(super) fn validate_top_level_ident(kind: &str, name: &str) -> Result<(), CodegenError> {
    validate_ident(kind, name)?;
    if RESERVED_RUNTIME_NAMES.contains(&name) || name.starts_with(RESERVED_TYPE_PREFIX) {
        return Err(CodegenError::validation(format!(
            "{kind} name '{name}' is reserved by the hybrid runtime"
        )));
    }
    Ok(())
}

/// A module/view method: an identifier that is not a vtable slot.
pub(super) fn validate_vtable_method_ident(owner: &str, name: &str) -> Result<(), CodegenError> {
    validate_ident("method", name)?;
    if RESERVED_VTABLE_MEMBERS.contains(&to_snake_case(name).as_str()) {
        return Err(CodegenError::validation(format!(
            "{owner} method '{name}' collides with a reserved hybrid vtable member"
        )));
    }
    Ok(())
}

pub(super) fn validate_ident(kind: &str, name: &str) -> Result<(), CodegenError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(CodegenError::validation(format!(
            "{kind} name cannot be empty"
        )));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(CodegenError::validation(format!(
            "invalid {kind} identifier '{name}': must start with ASCII letter or '_'"
        )));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(CodegenError::validation(format!(
            "invalid {kind} identifier '{name}': must contain only ASCII alphanumeric or '_'"
        )));
    }
    let prefix = name.get(..RESERVED_BINDING_PREFIX.len());
    if prefix.is_some_and(|p| p.eq_ignore_ascii_case(RESERVED_BINDING_PREFIX)) {
        return Err(CodegenError::validation(format!(
            "{kind} identifier '{name}' uses the '{RESERVED_BINDING_PREFIX}' prefix reserved for generated bindings"
        )));
    }
    Ok(())
}
