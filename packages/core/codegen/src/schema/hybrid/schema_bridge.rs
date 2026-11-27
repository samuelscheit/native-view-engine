//! Converts colocated compile-time [`nve_abi::schema::AbiFileDescriptor`] metadata
//! (populated by `#[derive(NveAbi)]` on real compiled Rust types) into a validated
//! [`HybridFileSpec`] with zero intermediate spec parsing.

use std::collections::BTreeSet;

use nve_abi::schema::{
    AbiEnumDescriptor, AbiFieldDescriptor, AbiFileDescriptor, AbiMethodDescriptor,
    AbiModuleDescriptor, AbiParamDescriptor, AbiRecordDescriptor,
};

use super::{
    HybridAliasSpec, HybridConstSpec, HybridEnumSpec, HybridEnumVariant, HybridFieldSpec,
    HybridFileSpec, HybridMethodSpec, HybridModuleSpec, HybridParamSpec, HybridStructSpec,
    HybridType,
};

/// Converts a colocated [`AbiFileDescriptor`] directly into a validated [`HybridFileSpec`].
pub fn from_abi_descriptor(desc: &AbiFileDescriptor) -> HybridFileSpec {
    let consts = desc
        .consts
        .iter()
        .map(|c| HybridConstSpec {
            name: c.name.to_owned(),
            ty: parse_abi_type(c.ty),
            value: c.value,
            hex: c.hex,
            doc: c.doc.map(str::to_owned),
        })
        .collect();

    let aliases = desc
        .aliases
        .iter()
        .map(|a| HybridAliasSpec {
            name: a.name.to_owned(),
            target: parse_abi_type(a.target),
            doc: a.doc.map(str::to_owned),
        })
        .collect();

    let enums = desc.enums.iter().map(convert_enum).collect();
    let structs = desc.records.iter().map(convert_record).collect();
    let modules = desc.modules.iter().map(convert_module).collect();
    let c_functions = desc
        .c_functions
        .iter()
        .map(|f| HybridMethodSpec {
            name: f.name.to_owned(),
            params: f.params.iter().map(convert_param).collect(),
            return_type: parse_abi_type(f.return_type),
            doc: f.doc.map(str::to_owned),
            has_self: false,
            optional: false,
        })
        .collect();

    let mut spec = HybridFileSpec {
        namespace: desc.namespace.map(str::to_owned),
        consts,
        aliases,
        enums,
        structs,
        modules,
        views: Vec::new(),
        c_functions,
    };
    resolve_named_types(&mut spec);
    spec.validate()
        .expect("colocated AbiFileDescriptor must produce a valid HybridFileSpec");
    spec
}

fn convert_enum(e: &AbiEnumDescriptor) -> HybridEnumSpec {
    let variants = e
        .variants
        .iter()
        .map(|v| HybridEnumVariant {
            name: v.name.to_owned(),
            value: u32::try_from(v.value)
                .expect("colocated AbiEnumDescriptor variant discriminant must fit in u32"),
            doc: v.doc.map(str::to_owned),
        })
        .collect();
    HybridEnumSpec {
        name: e.name.to_owned(),
        variants,
        doc: e.doc.map(str::to_owned),
        c_prefix: e.c_prefix.map(str::to_owned),
        repr: Some(e.repr.to_owned()),
        dart_fallback: e.dart_fallback.map(str::to_owned),
    }
}

fn convert_record(rec: &AbiRecordDescriptor) -> HybridStructSpec {
    match rec {
        AbiRecordDescriptor::Struct(s) => HybridStructSpec {
            name: s.name.to_owned(),
            fields: s.fields.iter().map(convert_field).collect(),
            doc: s.doc.map(str::to_owned),
            is_union: false,
            align: s.explicit_align,
            expected_size: s.expected_size,
            expected_size_64: s.expected_size_64,
        },
        AbiRecordDescriptor::Union(u) => HybridStructSpec {
            name: u.name.to_owned(),
            fields: u.fields.iter().map(convert_field).collect(),
            doc: u.doc.map(str::to_owned),
            is_union: true,
            align: u.explicit_align,
            expected_size: u.expected_size,
            expected_size_64: u.expected_size_64,
        },
    }
}

fn convert_field(f: &AbiFieldDescriptor) -> HybridFieldSpec {
    HybridFieldSpec {
        name: f.name.to_owned(),
        ty: parse_abi_type(f.abi_type()),
        doc: f.doc.map(str::to_owned),
        offset: f.expected_offset,
        offset_64: f.expected_offset_64,
    }
}

fn convert_module(m: &AbiModuleDescriptor) -> HybridModuleSpec {
    HybridModuleSpec {
        name: m.name.to_owned(),
        trait_name: m.trait_name.to_owned(),
        methods: m.methods.iter().map(convert_method).collect(),
        doc: m.doc.map(str::to_owned),
        explicit_interface_id: Some(m.interface_id),
        c_name: Some(m.c_name.to_owned()),
    }
}

fn convert_method(m: &AbiMethodDescriptor) -> HybridMethodSpec {
    HybridMethodSpec {
        name: m.name.to_owned(),
        params: m.params.iter().map(convert_param).collect(),
        return_type: parse_abi_type(m.return_type),
        doc: m.doc.map(str::to_owned),
        has_self: m.has_self,
        optional: m.optional,
    }
}

fn convert_param(p: &AbiParamDescriptor) -> HybridParamSpec {
    HybridParamSpec {
        name: p.name.to_owned(),
        ty: parse_abi_type(p.ty),
    }
}

fn parse_abi_type(raw: &str) -> HybridType {
    let s = raw.trim();
    if s == "()" || s == "void" || s == "c_void" {
        return HybridType::Void;
    }
    if let Some(rest) = s.strip_prefix("*const ") {
        return HybridType::ConstPtr(Box::new(parse_abi_type(rest)));
    }
    if let Some(rest) = s.strip_prefix("*mut ") {
        return HybridType::MutPtr(Box::new(parse_abi_type(rest)));
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']'))
        && let Some((elem, len_str)) = inner.rsplit_once(';')
    {
        let len = len_str
            .trim()
            .parse::<usize>()
            .expect("colocated ABI fixed array length must be a valid usize");
        return HybridType::FixedArray {
            elem: Box::new(parse_abi_type(elem)),
            len,
        };
    }
    if let Some(inner) = s.strip_prefix("Option<").and_then(|r| r.strip_suffix('>')) {
        return HybridType::Optional(Box::new(parse_abi_type(inner)));
    }
    if let Some(inner) = s.strip_prefix("Vec<").and_then(|r| r.strip_suffix('>')) {
        let elem = parse_abi_type(inner);
        return match elem {
            HybridType::U8 => HybridType::Bytes,
            other => HybridType::Vec(Box::new(other)),
        };
    }
    let base = s.rsplit("::").next().unwrap_or(s);
    match base {
        "bool" => HybridType::Bool,
        "u8" | "PrimarySlot" => HybridType::U8,
        "u16" | "ShortLen" | "PropFieldId" => HybridType::U16,
        "usize" => HybridType::Usize,
        "i32" | "ZIndex" => HybridType::I32,
        "u32" | "NodeIndex" => HybridType::U32,
        "i64" => HybridType::I64,
        "u64" => HybridType::U64,
        "f32" => HybridType::F32,
        "f64" => HybridType::F64,
        "String" | "str" => HybridType::String,
        "NveValue" => HybridType::Value,
        "NodeId" => HybridType::Struct("NveNodeId".to_owned()),
        "SurfaceId" => HybridType::Struct("NveSurfaceId".to_owned()),
        "FrameId" => HybridType::Struct("NveFrameId".to_owned()),
        "TimestampUs" => HybridType::Struct("NveTimestampUs".to_owned()),
        "DurationUs" => HybridType::Struct("NveDurationUs".to_owned()),
        "EventId" => HybridType::Struct("NveEventId".to_owned()),
        "EventFlags" => HybridType::Struct("NveEventFlags".to_owned()),
        "ArenaOffset" => HybridType::Struct("NveArenaOffset".to_owned()),
        "ByteLen" => HybridType::Struct("NveByteLen".to_owned()),
        "InterfaceId" => HybridType::Struct("NveInterfaceId".to_owned()),
        ident => HybridType::Struct(ident.to_owned()),
    }
}

fn resolve_named_types(file: &mut HybridFileSpec) {
    let enum_names: BTreeSet<String> = file.enums.iter().map(|e| e.name.clone()).collect();
    let alias_names: BTreeSet<String> = file.aliases.iter().map(|a| a.name.clone()).collect();
    let rewrite = |ty: &mut HybridType| rewrite_named_type(ty, &enum_names, &alias_names);

    for c in &mut file.consts {
        rewrite(&mut c.ty);
    }
    for a in &mut file.aliases {
        rewrite(&mut a.target);
    }
    for s in &mut file.structs {
        for f in &mut s.fields {
            rewrite(&mut f.ty);
        }
    }
    for m in &mut file.modules {
        for method in &mut m.methods {
            for p in &mut method.params {
                rewrite(&mut p.ty);
            }
            rewrite(&mut method.return_type);
        }
    }
    for f in &mut file.c_functions {
        for p in &mut f.params {
            rewrite(&mut p.ty);
        }
        rewrite(&mut f.return_type);
    }
}

fn rewrite_named_type(
    ty: &mut HybridType,
    enum_names: &BTreeSet<String>,
    alias_names: &BTreeSet<String>,
) {
    match ty {
        HybridType::Struct(name) => {
            if enum_names.contains(name) {
                *ty = HybridType::Enum(name.clone());
            } else if alias_names.contains(name) {
                *ty = HybridType::Handle(name.clone());
            }
        }
        HybridType::Optional(inner)
        | HybridType::Vec(inner)
        | HybridType::ConstPtr(inner)
        | HybridType::MutPtr(inner)
        | HybridType::FixedArray { elem: inner, .. } => {
            rewrite_named_type(inner, enum_names, alias_names);
        }
        HybridType::Result { ok, err } => {
            rewrite_named_type(ok, enum_names, alias_names);
            rewrite_named_type(err, enum_names, alias_names);
        }
        _ => {}
    }
}
