//! Identifier rules shared by every hybrid target.

use std::collections::BTreeSet;

use crate::error::CodegenError;
use crate::naming::{to_pascal_case, to_snake_case};
use crate::schema::hybrid::HybridFileSpec;

/// Prefix of the names generated code binds itself (locals, parameters, type parameters) in every
/// target language, so a spec identifier can never shadow or collide with one.
pub(super) const RESERVED_BINDING_PREFIX: &str = "_nve_";

/// Snake-case prefix of generated top-level items in every spelling: types
/// (`NveHybrid<Name>VTable`, `NveHybridOpt<T>`, `NveHybridRaw<T>`), constants
/// (`NVE_HYBRID_ABI_VERSION`, `NVE_HYBRID_<NAME>_INTERFACE_ID`), and C functions
/// (`nve_hybrid_alloc_bytes`).
const RESERVED_ITEM_PREFIX: &str = "nve_hybrid";

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

/// Slots every module/view vtable carries ahead of its methods (C header, all bindings); compared
/// in snake case.
const RESERVED_VTABLE_MEMBERS: &[&str] = &[
    "abi_version",
    "interface_id",
    "schema_hash",
    "create",
    "destroy",
    "alloc_bytes",
    "free_bytes",
];

/// What only a view vtable adds: its direct-props slot and the props hook of the view trait.
const RESERVED_VIEW_MEMBERS: &[&str] = &["set_props_direct", "apply_props"];

/// The key two names of one scope must differ in: generated code spells identifiers in snake
/// case too (`_nve_call_<snake>`, `emit_<view>_<snake>`, `NVE_HYBRID_<SNAKE>_*`), so `getValue`,
/// `get_value`, and `_get_value` are the same name.
pub(super) fn ident_key(name: &str) -> String {
    to_snake_case(name)
}

pub(super) fn validate_top_level_ident(kind: &str, name: &str) -> Result<(), CodegenError> {
    validate_ident(kind, name)?;
    if RESERVED_RUNTIME_NAMES.contains(&name) || ident_key(name).starts_with(RESERVED_ITEM_PREFIX) {
        return Err(CodegenError::validation(format!(
            "{kind} name '{name}' is reserved by the hybrid runtime"
        )));
    }
    Ok(())
}

/// Every enum / struct `Name` is the C type `NveHybrid<Name>` (C, C++, Swift, Dart, Rust), so it
/// must not spell a runtime C type (`NveHybridStatus`, `NveHybridOwnedBytes`) or another generated
/// one (`NveHybrid<Module>VTable`, `NveHybridOpt<T>`, the Rust carrier `NveHybridRaw<Struct>`).
pub(super) fn validate_c_type_names(spec: &HybridFileSpec) -> Result<(), CodegenError> {
    let mut taken: BTreeSet<String> = ["Status", "OwnedBytes"].map(String::from).into();
    let interfaces = spec.modules.iter().map(|m| &m.name).chain(spec.views.iter().map(|v| &v.name));
    taken.extend(interfaces.map(|name| format!("{}VTable", to_pascal_case(name))));
    taken.extend(spec.structs.iter().map(|s| format!("Raw{}", s.name)));
    taken.extend(spec.collect_pod_option_types().into_iter().map(|(suffix, _)| format!("Opt{suffix}")));
    let types = spec.enums.iter().map(|e| ("enum", &e.name));
    for (kind, name) in types.chain(spec.structs.iter().map(|s| ("struct", &s.name))) {
        if taken.contains(name.as_str()) {
            return Err(CodegenError::validation(format!(
                "{kind} name '{name}' collides with the generated C type 'NveHybrid{name}'"
            )));
        }
    }
    Ok(())
}

/// A module/view method: an identifier that is not a slot of its vtable.
pub(super) fn validate_vtable_method_ident(
    owner: &str,
    name: &str,
    is_view: bool,
) -> Result<(), CodegenError> {
    validate_ident("method", name)?;
    let key = ident_key(name);
    if RESERVED_VTABLE_MEMBERS.contains(&key.as_str())
        || (is_view && RESERVED_VIEW_MEMBERS.contains(&key.as_str()))
    {
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
