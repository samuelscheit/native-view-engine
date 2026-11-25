//! Pure-Rust code emitter for `#[derive(NveAbi)]`.

use std::fmt::Write as _;

use crate::{ParsedEnum, ParsedItem, ParsedRecord};

pub(crate) fn emit_item(item: &ParsedItem) -> String {
    match item {
        ParsedItem::Struct(rec) => emit_record(rec, false),
        ParsedItem::Union(rec) => emit_record(rec, true),
        ParsedItem::Enum(en) => emit_enum(en),
    }
}

fn emit_record(rec: &ParsedRecord, is_union: bool) -> String {
    let mut out = String::with_capacity(1024);
    let (cp, ty) = (&rec.crate_path, &rec.rust_ident);
    let (tr, desc, var) = if is_union { ("NveUnionSchema", "AbiUnionDescriptor", "Union") } else { ("NveStructSchema", "AbiStructDescriptor", "Struct") };

    let _ = writeln!(
        out,
        "impl {cp}::{tr} for {ty} {{\n    const SCHEMA: {cp}::{desc} = {cp}::{desc} {{\n        name: {}, doc: {},\n        size: ::core::mem::size_of::<Self>(), align: ::core::mem::align_of::<Self>(),\n        explicit_align: {}, expected_size: {}, expected_size_64: {},\n        fields: &[",
        quote_str(&rec.abi_name), opt_str(rec.doc.as_deref()),
        opt_usize(rec.explicit_align), opt_usize(rec.expected_size), opt_usize(rec.expected_size_64)
    );
    for f in &rec.fields {
        let _ = writeln!(
            out,
            "            {cp}::AbiFieldDescriptor {{ name: {}, ty: {}, c_type: {}, doc: {}, offset: ::core::mem::offset_of!(Self, {}), expected_offset: {}, expected_offset_64: {} }},",
            quote_str(&f.abi_name), quote_str(&f.ty), opt_str(f.c_type.as_deref()),
            opt_str(f.doc.as_deref()), f.rust_ident, opt_usize(f.expected_offset), opt_usize(f.expected_offset_64),
        );
    }
    let _ = writeln!(
        out,
        "        ],\n    }};\n}}\nimpl {cp}::NveSchema for {ty} {{ const ITEM: {cp}::AbiItemDescriptor = {cp}::AbiItemDescriptor::{var}(<Self as {cp}::{tr}>::SCHEMA); }}"
    );

    if rec.expected_size.is_some() || rec.explicit_align.is_some() || rec.fields.iter().any(|f| f.expected_offset.is_some()) {
        let _ = writeln!(out, "const _: () = {{");
        if let Some(sz) = rec.expected_size { let _ = writeln!(out, "    ::core::assert!(::core::mem::size_of::<{ty}>() == {sz}usize);"); }
        if let Some(al) = rec.explicit_align { let _ = writeln!(out, "    ::core::assert!(::core::mem::align_of::<{ty}>() == {al}usize);"); }
        for f in &rec.fields {
            if let Some(off) = f.expected_offset { let _ = writeln!(out, "    ::core::assert!(::core::mem::offset_of!({ty}, {}) == {off}usize);", f.rust_ident); }
        }
        let _ = writeln!(out, "}};");
    }
    if rec.expected_size_64.is_some() || rec.fields.iter().any(|f| f.expected_offset_64.is_some()) {
        let _ = writeln!(out, "#[cfg(target_pointer_width = \"64\")]\nconst _: () = {{");
        if let Some(sz64) = rec.expected_size_64 { let _ = writeln!(out, "    ::core::assert!(::core::mem::size_of::<{ty}>() == {sz64}usize);"); }
        for f in &rec.fields {
            if let Some(off64) = f.expected_offset_64 { let _ = writeln!(out, "    ::core::assert!(::core::mem::offset_of!({ty}, {}) == {off64}usize);", f.rust_ident); }
        }
        let _ = writeln!(out, "}};");
    }
    out
}

fn emit_enum(en: &ParsedEnum) -> String {
    let mut out = String::with_capacity(512);
    let (cp, ty, repr) = (&en.crate_path, &en.rust_ident, &en.repr);
    let _ = writeln!(
        out,
        "impl {cp}::NveEnumSchema for {ty} {{\n    const SCHEMA: {cp}::AbiEnumDescriptor = {cp}::AbiEnumDescriptor {{\n        name: {}, repr: {}, doc: {}, c_prefix: {}, dart_fallback: {},\n        variants: &[",
        quote_str(&en.abi_name), quote_str(repr), opt_str(en.doc.as_deref()),
        opt_str(en.c_prefix.as_deref()), opt_str(en.dart_fallback.as_deref())
    );
    for v in &en.variants {
        let _ = writeln!(
            out,
            "            {cp}::AbiEnumVariantDescriptor {{ name: {}, value: (Self::{} as {repr}) as i64, doc: {} }},",
            quote_str(&v.abi_name), v.rust_ident, opt_str(v.doc.as_deref())
        );
    }
    let _ = writeln!(
        out,
        "        ],\n    }};\n}}\nimpl {cp}::NveSchema for {ty} {{ const ITEM: {cp}::AbiItemDescriptor = {cp}::AbiItemDescriptor::Enum(<Self as {cp}::NveEnumSchema>::SCHEMA); }}"
    );
    out
}

pub(crate) fn quote_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"), '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"), '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"), '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn opt_str(s: Option<&str>) -> String {
    match s { Some(v) => format!("::core::option::Option::Some({})", quote_str(v)), None => "::core::option::Option::None".to_owned() }
}

fn opt_usize(v: Option<usize>) -> String {
    match v { Some(n) => format!("::core::option::Option::Some({n}usize)"), None => "::core::option::Option::None".to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse_string_literal, parse_usize_literal};
    use crate::{ParsedField, ParsedVariant};

    #[test]
    fn literal_parsers_and_code_emitter() {
        assert_eq!(parse_usize_literal("0xFFFF").unwrap(), 65535);
        assert_eq!(parse_usize_literal("1_024usize").unwrap(), 1024);
        assert_eq!(parse_usize_literal("0b1010").unwrap(), 10);
        assert_eq!(parse_usize_literal("0o77").unwrap(), 63);
        assert!(parse_usize_literal("bad").is_err());
        assert_eq!(parse_string_literal(r##"r#"a "b" \n"#"##).unwrap(), r#"a "b" \n"#);
        assert_eq!(parse_string_literal(r#""\x41\u{2014}\n\t""#).unwrap(), "A—\n\t");
        let s = emit_item(&ParsedItem::Struct(ParsedRecord {
            rust_ident: "Evt".into(), abi_name: "Evt".into(), doc: Some("Doc".into()),
            crate_path: "::nve_abi::schema".into(), explicit_align: Some(8),
            expected_size: None, expected_size_64: Some(16),
            fields: vec![ParsedField {
                rust_ident: "cb".into(), abi_name: "cb".into(), ty: "Option<Fn>".into(),
                c_type: Some("Fn".into()), doc: None, expected_offset: Some(0), expected_offset_64: Some(8),
            }],
        }));
        assert!(s.contains("offset_of!(Self, cb)") && s.contains("align_of::<Evt>() == 8usize") && s.contains("#[cfg(target_pointer_width = \"64\")]"));
        let e = emit_item(&ParsedItem::Enum(ParsedEnum {
            rust_ident: "St".into(), abi_name: "NveSt".into(), repr: "i32".into(), doc: None,
            crate_path: "::nve_abi::schema".into(), c_prefix: Some("S_".into()), dart_fallback: Some("Err".into()),
            variants: vec![ParsedVariant { rust_ident: "Err".into(), abi_name: "Err".into(), doc: None }],
        }));
        assert!(e.contains("value: (Self::Err as i32) as i64"));
    }
}
