//! Semantic validation and direct by-value cycle detection for [`RelocSchema`].

use std::collections::{BTreeMap, BTreeSet};

use crate::error::CodegenError;
use super::ir::{visit_direct_struct_deps, RelocEnum, RelocSchema, RelocStruct, RelocType};

impl RelocSchema {
    /// Validates all identifiers, enum discriminants, field types, alignments, and cycles.
    pub fn validate(&self) -> Result<(), CodegenError> {
        validate_ident("schema", &self.name)?;
        if self.magic == [0u8; 8] {
            return Err(CodegenError::validation(
                "reloc schema magic must not be all-zero",
            ));
        }
        validate_ident("root struct", &self.root)?;

        let mut type_names = BTreeSet::new();
        let mut enum_map: BTreeMap<&str, &RelocEnum> = BTreeMap::new();
        let mut struct_map: BTreeMap<&str, &RelocStruct> = BTreeMap::new();

        for e in &self.enums {
            validate_ident("enum", &e.name)?;
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
            let mut vnames = BTreeSet::new();
            let mut vvals = BTreeSet::new();
            for v in &e.variants {
                validate_ident("enum variant", &v.name)?;
                if !vnames.insert(v.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate variant '{}' in enum '{}'",
                        v.name, e.name
                    )));
                }
                if !vvals.insert(v.value) {
                    return Err(CodegenError::validation(format!(
                        "duplicate discriminant {} in enum '{}'",
                        v.value, e.name
                    )));
                }
                if v.value > e.repr.max_value() {
                    return Err(CodegenError::validation(format!(
                        "variant '{}::{}' discriminant {} exceeds {}::MAX ({})",
                        e.name,
                        v.name,
                        v.value,
                        e.repr.rust_type(),
                        e.repr.max_value()
                    )));
                }
            }
            enum_map.insert(e.name.as_str(), e);
        }

        for s in &self.structs {
            validate_ident("struct", &s.name)?;
            if !type_names.insert(s.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate type name '{}'",
                    s.name
                )));
            }
            if let Some(a) = s.align
                && (a == 0 || !a.is_power_of_two())
            {
                return Err(CodegenError::validation(format!(
                    "struct '{}' align {a} must be a non-zero power of two",
                    s.name
                )));
            }
            if s.fields.is_empty() {
                return Err(CodegenError::validation(format!(
                    "struct '{}' must declare at least one field",
                    s.name
                )));
            }
            let mut fnames = BTreeSet::new();
            for f in &s.fields {
                validate_ident("struct field", &f.name)?;
                if !fnames.insert(f.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate field '{}' in struct '{}'",
                        f.name, s.name
                    )));
                }
            }
            struct_map.insert(s.name.as_str(), s);
        }

        if !struct_map.contains_key(self.root.as_str()) {
            return Err(CodegenError::validation(format!(
                "root struct '{}' is not declared in schema '{}'",
                self.root, self.name
            )));
        }

        for s in &self.structs {
            for f in &s.fields {
                validate_reloc_type(
                    &f.ty,
                    &enum_map,
                    &struct_map,
                    &format!("struct '{}' field '{}'", s.name, f.name),
                )?;
            }
        }

        detect_direct_value_cycles(&struct_map)?;
        Ok(())
    }
}

fn validate_ident(kind: &str, name: &str) -> Result<(), CodegenError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(CodegenError::validation(format!(
            "{kind} name cannot be empty"
        )));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(CodegenError::validation(format!(
            "invalid {kind} identifier '{name}'"
        )));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(CodegenError::validation(format!(
            "invalid {kind} identifier '{name}'"
        )));
    }
    Ok(())
}

fn validate_reloc_type(
    ty: &RelocType,
    enums: &BTreeMap<&str, &RelocEnum>,
    structs: &BTreeMap<&str, &RelocStruct>,
    ctx: &str,
) -> Result<(), CodegenError> {
    match ty {
        RelocType::Bool
        | RelocType::U8
        | RelocType::I8
        | RelocType::U16
        | RelocType::I16
        | RelocType::U32
        | RelocType::I32
        | RelocType::F32
        | RelocType::U64
        | RelocType::I64
        | RelocType::F64
        | RelocType::RelocStr => Ok(()),
        RelocType::FixedArray { elem, len } => {
            if *len == 0 {
                return Err(CodegenError::validation(format!(
                    "{ctx}: fixed array length must be > 0"
                )));
            }
            validate_reloc_type(elem, enums, structs, ctx)
        }
        RelocType::RelocSlice(elem) => validate_reloc_type(elem, enums, structs, ctx),
        RelocType::Enum(name) => {
            if !enums.contains_key(name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: unknown enum '{name}'"
                )));
            }
            Ok(())
        }
        RelocType::Struct(name) => {
            if !structs.contains_key(name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: unknown struct '{name}'"
                )));
            }
            Ok(())
        }
    }
}

fn detect_direct_value_cycles(
    struct_map: &BTreeMap<&str, &RelocStruct>,
) -> Result<(), CodegenError> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Visited,
    }

    fn dfs<'a>(
        name: &'a str,
        struct_map: &BTreeMap<&'a str, &'a RelocStruct>,
        marks: &mut BTreeMap<&'a str, Mark>,
        stack: &mut Vec<&'a str>,
    ) -> Result<(), CodegenError> {
        if let Some(mark) = marks.get(name) {
            if *mark == Mark::Visiting {
                stack.push(name);
                return Err(CodegenError::validation(format!(
                    "recursive infinite-size reloc struct cycle: {}",
                    stack.join(" -> ")
                )));
            }
            return Ok(());
        }
        marks.insert(name, Mark::Visiting);
        stack.push(name);
        if let Some(spec) = struct_map.get(name) {
            let mut deps = Vec::new();
            for field in &spec.fields {
                visit_direct_struct_deps(&field.ty, &mut |d| deps.push(d));
            }
            for dep in deps {
                dfs(dep, struct_map, marks, stack)?;
            }
        }
        stack.pop();
        marks.insert(name, Mark::Visited);
        Ok(())
    }

    let mut marks = BTreeMap::new();
    let mut stack = Vec::new();
    for &name in struct_map.keys() {
        dfs(name, struct_map, &mut marks, &mut stack)?;
    }
    Ok(())
}
