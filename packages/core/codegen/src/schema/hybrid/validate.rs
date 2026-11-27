mod idents;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use idents::{validate_ident, validate_top_level_ident, validate_vtable_method_ident};

use super::*;

impl HybridFileSpec {
    pub fn validate(&self) -> Result<(), CodegenError> {
        if self.consts.is_empty()
            && self.aliases.is_empty()
            && self.enums.is_empty()
            && self.structs.is_empty()
            && self.modules.is_empty()
            && self.views.is_empty()
            && self.c_functions.is_empty()
        {
            return Err(CodegenError::validation(
                "hybrid spec must declare at least one enum, struct, module, or view",
            ));
        }

        if let Some(ns) = &self.namespace {
            validate_ident("namespace", ns)?;
        }

        let mut type_names = BTreeSet::new();
        let mut const_names = BTreeSet::new();
        let mut alias_names = BTreeSet::new();
        let mut enum_map: BTreeMap<&str, &HybridEnumSpec> = BTreeMap::new();
        let mut struct_map: BTreeMap<&str, &HybridStructSpec> = BTreeMap::new();

        for c in &self.consts {
            validate_top_level_ident("const", &c.name)?;
            if !const_names.insert(c.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate const name '{}'",
                    c.name
                )));
            }
        }

        for a in &self.aliases {
            validate_top_level_ident("type alias", &a.name)?;
            if !type_names.insert(a.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate type name '{}'",
                    a.name
                )));
            }
            alias_names.insert(a.name.as_str());
        }

        for e in &self.enums {
            validate_top_level_ident("enum", &e.name)?;
            if !type_names.insert(e.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate type name '{}'",
                    e.name
                )));
            }
            if e.variants.is_empty() {
                return Err(CodegenError::validation(format!(
                    "enum '{}' must declare at least one variant",
                    e.name
                )));
            }
            let mut variant_names = BTreeSet::new();
            let mut variant_values = BTreeSet::new();
            for v in &e.variants {
                validate_ident("enum variant", &v.name)?;
                if !variant_names.insert(v.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate variant '{}' in enum '{}'",
                        v.name, e.name
                    )));
                }
                if !variant_values.insert(v.value) {
                    return Err(CodegenError::validation(format!(
                        "duplicate discriminant value {} in enum '{}'",
                        v.value, e.name
                    )));
                }
            }
            enum_map.insert(e.name.as_str(), e);
        }

        for s in &self.structs {
            validate_top_level_ident("struct", &s.name)?;
            if !type_names.insert(s.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate type name '{}'",
                    s.name
                )));
            }
            if let Some(align) = s.align
                && (align == 0 || !align.is_power_of_two())
            {
                return Err(CodegenError::validation(format!(
                    "struct '{}' align {} must be a power of two",
                    s.name, align
                )));
            }
            if s.fields.is_empty() {
                return Err(CodegenError::validation(format!(
                    "struct '{}' must declare at least one field",
                    s.name
                )));
            }
            let mut field_names = BTreeSet::new();
            for f in &s.fields {
                validate_ident("struct field", &f.name)?;
                if !field_names.insert(f.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate field '{}' in struct '{}'",
                        f.name, s.name
                    )));
                }
            }
            struct_map.insert(s.name.as_str(), s);
        }

        for a in &self.aliases {
            validate_type(
                &a.target,
                true,
                &enum_map,
                &struct_map,
                &alias_names,
                &format!("type alias '{}'", a.name),
            )?;
        }

        // Validate all struct field types and check for infinite-size recursive cycles.
        for s in &self.structs {
            for f in &s.fields {
                validate_type(
                    &f.ty,
                    false,
                    &enum_map,
                    &struct_map,
                    &alias_names,
                    &format!("struct '{}' field '{}'", s.name, f.name),
                )?;
            }
        }
        detect_recursive_value_structs(&struct_map)?;

        let mut item_names = BTreeSet::new();
        let mut iface_ids: BTreeMap<u32, String> = BTreeMap::new();

        for m in &self.modules {
            validate_top_level_ident("module", &m.name)?;
            validate_ident("module trait", &m.trait_name)?;
            if !item_names.insert(m.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate hybrid module/view name '{}'",
                    m.name
                )));
            }
            if type_names.contains(m.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "hybrid module '{}' collides with a struct or enum of the same name",
                    m.name
                )));
            }
            let iface_id = m.interface_id();
            if let Some(explicit) = m.explicit_interface_id {
                if explicit == 0 {
                    return Err(CodegenError::validation(format!(
                        "module '{}' explicit interface_id cannot be 0",
                        m.name
                    )));
                }
            } else {
                assert!(iface_id >= NVE_HYBRID_INTERFACE_ID_BASE);
            }
            if let Some(prev) = iface_ids.insert(iface_id, format!("module '{}'", m.name)) {
                return Err(CodegenError::validation(format!(
                    "hybrid interface_id collision 0x{iface_id:08X} between {prev} and module '{}'",
                    m.name
                )));
            }
            for method in &m.methods {
                validate_vtable_method_ident(&format!("module '{}'", m.name), &method.name)?;
            }
            validate_methods(
                &m.methods,
                &enum_map,
                &struct_map,
                &alias_names,
                &format!("module '{}'", m.name),
            )?;
        }

        for v in &self.views {
            validate_top_level_ident("view", &v.name)?;
            validate_ident("view trait", &v.trait_name)?;
            if !item_names.insert(v.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate hybrid module/view name '{}'",
                    v.name
                )));
            }
            if type_names.contains(v.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "hybrid view '{}' collides with a struct or enum of the same name",
                    v.name
                )));
            }
            let iface_id = v.interface_id();
            if let Some(explicit) = v.explicit_interface_id {
                if explicit == 0 {
                    return Err(CodegenError::validation(format!(
                        "view '{}' explicit interface_id cannot be 0",
                        v.name
                    )));
                }
            } else {
                assert!(iface_id >= NVE_HYBRID_INTERFACE_ID_BASE);
            }
            if let Some(prev) = iface_ids.insert(iface_id, format!("view '{}'", v.name)) {
                return Err(CodegenError::validation(format!(
                    "hybrid interface_id collision 0x{iface_id:08X} between {prev} and view '{}'",
                    v.name
                )));
            }

            let mut member_names = BTreeSet::new();
            for p in &v.props {
                validate_ident("prop", &p.name)?;
                if !member_names.insert(p.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate member '{}' in view '{}'",
                        p.name, v.name
                    )));
                }
                validate_type(
                    &p.ty,
                    false,
                    &enum_map,
                    &struct_map,
                    &alias_names,
                    &format!("view '{}' prop '{}'", v.name, p.name),
                )?;
                if let Some(def) = &p.default_value {
                    validate_default_value(def, &p.ty, &enum_map, &v.name, &p.name)?;
                }
            }
            for ev in &v.events {
                validate_ident("event", &ev.name)?;
                if !member_names.insert(ev.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate member '{}' in view '{}'",
                        ev.name, v.name
                    )));
                }
                let mut param_names = BTreeSet::new();
                for param in &ev.params {
                    validate_ident("event parameter", &param.name)?;
                    if !param_names.insert(param.name.as_str()) {
                        return Err(CodegenError::validation(format!(
                            "duplicate parameter '{}' in view '{}' event '{}'",
                            param.name, v.name, ev.name
                        )));
                    }
                    validate_type(
                        &param.ty,
                        false,
                        &enum_map,
                        &struct_map,
                        &alias_names,
                        &format!(
                            "view '{}' event '{}' param '{}'",
                            v.name, ev.name, param.name
                        ),
                    )?;
                }
            }
            for m in &v.methods {
                validate_vtable_method_ident(&format!("view '{}'", v.name), &m.name)?;
                if !member_names.insert(m.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate member '{}' in view '{}'",
                        m.name, v.name
                    )));
                }
            }
            validate_methods(
                &v.methods,
                &enum_map,
                &struct_map,
                &alias_names,
                &format!("view '{}'", v.name),
            )?;
        }

        if !self.c_functions.is_empty() {
            for f in &self.c_functions {
                validate_top_level_ident("c_function", &f.name)?;
            }
            validate_methods(
                &self.c_functions,
                &enum_map,
                &struct_map,
                &alias_names,
                "c_functions",
            )?;
        }

        Ok(())
    }
}

fn validate_methods(
    methods: &[HybridMethodSpec],
    enum_map: &BTreeMap<&str, &HybridEnumSpec>,
    struct_map: &BTreeMap<&str, &HybridStructSpec>,
    alias_names: &BTreeSet<&str>,
    owner: &str,
) -> Result<(), CodegenError> {
    let mut names = BTreeSet::new();
    for m in methods {
        validate_ident("method", &m.name)?;
        if !names.insert(m.name.as_str()) {
            return Err(CodegenError::validation(format!(
                "duplicate method '{}' in {owner}",
                m.name
            )));
        }
        let mut param_names = BTreeSet::new();
        for p in &m.params {
            validate_ident("parameter", &p.name)?;
            if !param_names.insert(p.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate parameter '{}' in {owner} method '{}'",
                    p.name, m.name
                )));
            }
            validate_type(
                &p.ty,
                false,
                enum_map,
                struct_map,
                alias_names,
                &format!("{owner} method '{}' param '{}'", m.name, p.name),
            )?;
            if p.ty.is_result() {
                return Err(CodegenError::validation(format!(
                    "{owner} method '{}' param '{}' cannot use Result<T, E> as input parameter",
                    m.name, p.name
                )));
            }
        }
        validate_type(
            &m.return_type,
            true,
            enum_map,
            struct_map,
            alias_names,
            &format!("{owner} method '{}' return type", m.name),
        )?;
    }
    Ok(())
}

fn validate_type(
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
                    "{ctx}: fixed-size array length must be > 0"
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

fn validate_default_value(
    def: &HybridDefaultValue,
    ty: &HybridType,
    enum_map: &BTreeMap<&str, &HybridEnumSpec>,
    view_name: &str,
    prop_name: &str,
) -> Result<(), CodegenError> {
    let ok = match (def, ty) {
        (HybridDefaultValue::None, HybridType::Optional(_)) => true,
        (other, HybridType::Optional(inner)) => {
            return validate_default_value(other, inner, enum_map, view_name, prop_name);
        }
        (HybridDefaultValue::Bool(_), HybridType::Bool) => true,
        // Integers must fit the target type; floats must stay finite at its precision.
        (HybridDefaultValue::Int(v), HybridType::I32) => i32::try_from(*v).is_ok(),
        (HybridDefaultValue::Int(_), HybridType::I64 | HybridType::F32 | HybridType::F64) => true,
        (HybridDefaultValue::Int(v), HybridType::U32) => u32::try_from(*v).is_ok(),
        (HybridDefaultValue::Int(v), HybridType::U64) => *v >= 0,
        (HybridDefaultValue::Int(v), HybridType::U8) => u8::try_from(*v).is_ok(),
        (HybridDefaultValue::Int(v), HybridType::U16) => u16::try_from(*v).is_ok(),
        // `usize` is 32-bit on wasm32 / ILP32 hosts, so a default must fit `u32` everywhere.
        (HybridDefaultValue::Int(v), HybridType::Usize) => u32::try_from(*v).is_ok(),
        (HybridDefaultValue::UInt(v), HybridType::Usize) => u32::try_from(*v).is_ok(),
        (HybridDefaultValue::UInt(v), HybridType::U8) => u8::try_from(*v).is_ok(),
        (HybridDefaultValue::UInt(v), HybridType::U16) => u16::try_from(*v).is_ok(),
        (HybridDefaultValue::UInt(v), HybridType::U32) => u32::try_from(*v).is_ok(),
        (HybridDefaultValue::UInt(v), HybridType::I32) => i32::try_from(*v).is_ok(),
        (HybridDefaultValue::UInt(v), HybridType::I64) => i64::try_from(*v).is_ok(),
        (HybridDefaultValue::UInt(_), HybridType::U64 | HybridType::F32 | HybridType::F64) => true,
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the narrowing is what is being checked"
        )]
        (HybridDefaultValue::Float(v), HybridType::F32) => (*v as f32).is_finite(),
        (HybridDefaultValue::Float(v), HybridType::F64) => v.is_finite(),
        (HybridDefaultValue::String(_), HybridType::String) => true,
        (
            HybridDefaultValue::EnumVariant { enum_name, variant },
            HybridType::Enum(expected_enum),
        ) => {
            enum_name == expected_enum
                && enum_map
                    .get(expected_enum.as_str())
                    .is_some_and(|e| e.variants.iter().any(|v| v.name == *variant))
        }
        _ => false,
    };
    if !ok {
        return Err(CodegenError::validation(format!(
            "view '{view_name}' prop '{prop_name}': default value {:?} does not match type '{}'",
            def,
            ty.canonical_signature()
        )));
    }
    Ok(())
}

/// Rejects cyclic value-embedded structs (`struct A { b: B }`, `struct B { a: A }`, or through
/// `Option<B>`), which have infinite size in Rust and C11/C++20.
fn detect_recursive_value_structs(
    struct_map: &BTreeMap<&str, &HybridStructSpec>,
) -> Result<(), CodegenError> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Visited,
    }

    fn dfs<'a>(
        name: &'a str,
        struct_map: &BTreeMap<&'a str, &'a HybridStructSpec>,
        marks: &mut BTreeMap<&'a str, Mark>,
        stack: &mut Vec<&'a str>,
    ) -> Result<(), CodegenError> {
        if let Some(mark) = marks.get(name) {
            if *mark == Mark::Visiting {
                stack.push(name);
                return Err(CodegenError::validation(format!(
                    "recursive infinite-size value struct cycle detected: {}",
                    stack.join(" -> ")
                )));
            }
            return Ok(());
        }
        marks.insert(name, Mark::Visiting);
        stack.push(name);
        if let Some(spec) = struct_map.get(name) {
            for field in &spec.fields {
                collect_direct_struct_edges(&field.ty, &mut |target| {
                    dfs(target, struct_map, marks, stack)
                })?;
            }
        }
        stack.pop();
        marks.insert(name, Mark::Visited);
        Ok(())
    }

    fn collect_direct_struct_edges<'a, F>(ty: &'a HybridType, f: &mut F) -> Result<(), CodegenError>
    where
        F: FnMut(&'a str) -> Result<(), CodegenError>,
    {
        match ty {
            HybridType::Struct(name) => f(name.as_str()),
            HybridType::Optional(inner) | HybridType::FixedArray { elem: inner, .. } => {
                collect_direct_struct_edges(inner, f)
            }
            // `Vec<T>` introduces heap indirection and breaks direct value-size cycles.
            _ => Ok(()),
        }
    }

    let mut marks = BTreeMap::new();
    let mut stack = Vec::new();
    for &name in struct_map.keys() {
        dfs(name, struct_map, &mut marks, &mut stack)?;
    }
    Ok(())
}
