//! Zero-dependency `Tok`-based parser for `#[derive(NveAbi)]` supporting `#[repr(C)]` structs,
//! unions, vtables (`#[nve_abi(interface = ...)]` / `#[nve_abi(vtable)]`), `#[repr(transparent)]`
//! handle newtypes, and `#[repr(u8|u16|u32|i32)]` enums.

use proc_macro::TokenStream;

use crate::fn_sig::{format_type, is_fn_ptr_type, parse_fn_ptr_sig, strip_raw, unwrap_option_type};
pub(crate) use crate::lit::{parse_i128_literal, parse_string_literal, parse_usize_literal};
use crate::parse_attrs::collect_attrs;
use crate::tok::{
    expect_ident, from_stream, is_arrow, is_ident, is_punct, render_tokens, skip_vis, Delim, Tok,
};
use crate::{
    ParsedAlias, ParsedEnum, ParsedField, ParsedItem, ParsedRecord, ParsedVariant, ParsedVtable,
    ParsedVtableMethod,
};

const DEFAULT_CRATE_PATH: &str = "::nve_abi::schema";

pub(crate) fn parse_item(input: TokenStream) -> Result<ParsedItem, String> {
    parse_item_tokens(&from_stream(input))
}

pub(crate) fn parse_item_tokens(tokens: &[Tok]) -> Result<ParsedItem, String> {
    let mut pos = 0usize;
    let mut a = collect_attrs(tokens, &mut pos, true)?;
    skip_vis(tokens, &mut pos);

    let kw = expect_ident(tokens, &mut pos, "struct, union, or enum")?;
    let rust_ident = expect_ident(tokens, &mut pos, "type name")?;
    if is_punct(tokens, pos, '<') {
        return Err(format!(
            "#[derive(NveAbi)] does not support generics on `{rust_ident}`"
        ));
    }
    if is_ident(tokens, pos, "where") {
        return Err(format!(
            "#[derive(NveAbi)] does not support `where` clauses on `{rust_ident}`"
        ));
    }

    let (
        mut name,
        mut cp,
        mut size,
        mut size_64,
        mut align,
        mut c_prefix,
        mut dart_fb,
        mut iface_id,
        mut module_name,
        mut is_vtable,
        mut slots_offset,
    ) = (None, None, None, None, None, None, None, None, None, false, None);

    for (k, v) in core::mem::take(&mut a.kvs) {
        match k.as_str() {
            "name" => name = Some(v.str(&k)?),
            "doc" => a.doc_override = Some(v.str(&k)?),
            "crate_path" => cp = Some(v.str(&k)?),
            "c_prefix" => c_prefix = Some(v.str(&k)?),
            "dart_fallback" => dart_fb = Some(v.str(&k)?),
            "size" => size = Some(v.usize(&k)?),
            "size_64" => size_64 = Some(v.usize(&k)?),
            "align" => align = Some(v.usize(&k)?),
            "interface" => iface_id = Some(v.path(&k)?),
            "module_name" => module_name = Some(v.str(&k)?),
            "vtable" => {
                v.flag(&k)?;
                is_vtable = true;
            }
            "slots_offset" => slots_offset = Some(v.usize(&k)?),
            other => return Err(format!("unknown #[nve_abi] item key `{other}`")),
        }
    }

    let abi_name = name.unwrap_or_else(|| strip_raw(&rust_ident));
    let crate_path = cp.unwrap_or_else(|| DEFAULT_CRATE_PATH.to_owned());
    let (repr_c, repr_transparent, repr_align, repr_int) = (
        a.repr_c,
        a.repr_transparent,
        a.repr_align,
        a.repr_int.take(),
    );
    let doc = a.doc();

    if kw == "struct" && repr_transparent {
        if repr_c
            || repr_align.is_some()
            || repr_int.is_some()
            || size.is_some()
            || size_64.is_some()
            || align.is_some()
            || c_prefix.is_some()
            || dart_fb.is_some()
            || iface_id.is_some()
            || is_vtable
            || module_name.is_some()
            || slots_offset.is_some()
        {
            return Err(format!(
                "#[derive(NveAbi)] on `#[repr(transparent)]` `{rust_ident}` only supports `name`/`doc`/`crate_path`"
            ));
        }
        let Some(Tok::Group(Delim::Paren, inner)) = tokens.get(pos) else {
            return Err(format!(
                "#[derive(NveAbi)] on `#[repr(transparent)]` `{rust_ident}` requires a single-field tuple struct"
            ));
        };
        let (rust_target, abi_target) = parse_single_tuple_field(inner, &rust_ident)?;
        return Ok(ParsedItem::Alias(ParsedAlias {
            rust_ident,
            abi_name,
            rust_target,
            abi_target,
            doc,
            crate_path,
        }));
    }

    let Some(Tok::Group(Delim::Brace, body)) = tokens.get(pos) else {
        return Err(format!(
            "#[derive(NveAbi)] requires a braced body on `{rust_ident}`"
        ));
    };

    if kw == "struct" && (iface_id.is_some() || is_vtable) {
        if !repr_c || repr_align.is_some() || repr_int.is_some() {
            return Err(format!(
                "#[derive(NveAbi)] vtable `{rust_ident}` requires plain `#[repr(C)]` (no `align`/`packed`/int repr)"
            ));
        }
        if size.is_some()
            || size_64.is_some()
            || align.is_some()
            || c_prefix.is_some()
            || dart_fb.is_some()
        {
            return Err(format!(
                "vtable `{rust_ident}` does not support `size`/`size_64`/`align`/`c_prefix`/`dart_fallback`"
            ));
        }
        let resolved_module = if iface_id.is_some() {
            if slots_offset.is_some() {
                return Err(format!(
                    "`slots_offset` is not valid on `#[nve_abi(interface = ...)]` vtable `{rust_ident}`"
                ));
            }
            let inferred = match module_name {
                Some(m) => m,
                None => abi_name
                    .strip_prefix("Nve")
                    .and_then(|s| s.strip_suffix("Api"))
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        format!(
                            "#[derive(NveAbi)] on `{rust_ident}` cannot infer `module_name` from `{abi_name}` (expected `Nve<Name>Api` or explicit `module_name = \"...\"`)"
                        )
                    })?,
            };
            Some(inferred)
        } else {
            module_name
        };
        let (header_fields, methods) =
            parse_vtable_body(body, &rust_ident, iface_id.is_some())?;
        return Ok(ParsedItem::Vtable(ParsedVtable {
            rust_ident,
            abi_name,
            module_name: resolved_module,
            interface_id: iface_id,
            slots_offset,
            doc,
            crate_path,
            header_fields,
            methods,
        }));
    }

    if iface_id.is_some() || is_vtable || module_name.is_some() || slots_offset.is_some() {
        return Err(format!(
            "`interface`/`vtable`/`module_name`/`slots_offset` are struct-only (`{rust_ident}`)"
        ));
    }

    match kw.as_str() {
        "struct" | "union" => {
            if !repr_c || repr_int.is_some() {
                return Err(format!(
                    "#[derive(NveAbi)] on `{rust_ident}` requires #[repr(C)]"
                ));
            }
            if c_prefix.is_some() || dart_fb.is_some() {
                return Err(format!(
                    "`c_prefix`/`dart_fallback` are enum-only (`{rust_ident}`)"
                ));
            }
            let rec = ParsedRecord {
                rust_ident,
                abi_name,
                doc,
                crate_path,
                explicit_align: align.or(repr_align),
                expected_size: size,
                expected_size_64: size_64,
                fields: parse_fields(body)?,
            };
            Ok(if kw == "union" {
                ParsedItem::Union(rec)
            } else {
                ParsedItem::Struct(rec)
            })
        }
        "enum" => {
            if repr_c || repr_transparent || repr_align.is_some() {
                return Err(format!(
                    "#[derive(NveAbi)] enum `{rust_ident}` requires plain `#[repr(u8|u16|u32|i32)]` (no `C`/`transparent`/`align`/`packed`)"
                ));
            }
            let repr = repr_int.ok_or_else(|| {
                format!("#[derive(NveAbi)] on `{rust_ident}` requires #[repr(u8|u16|u32|i32)]")
            })?;
            if size.is_some() || size_64.is_some() || align.is_some() {
                return Err(format!(
                    "`size`/`size_64`/`align` are struct/union-only (`{rust_ident}`)"
                ));
            }
            Ok(ParsedItem::Enum(ParsedEnum {
                variants: parse_variants(body, &rust_ident)?,
                rust_ident,
                abi_name,
                repr,
                doc,
                crate_path,
                c_prefix,
                dart_fallback: dart_fb,
            }))
        }
        other => Err(format!(
            "#[derive(NveAbi)] expects struct/union/enum; found `{other}`"
        )),
    }
}

struct RawField<'a> {
    rust_ident: String,
    ty_toks: &'a [Tok],
    doc: Option<String>,
    c_type: Option<String>,
    expected_offset: Option<usize>,
    expected_offset_64: Option<usize>,
}

fn parse_raw_fields(tokens: &[Tok]) -> Result<Vec<RawField<'_>>, String> {
    let (mut pos, mut out) = (0usize, Vec::new());
    while pos < tokens.len() {
        let mut a = collect_attrs(tokens, &mut pos, false)?;
        if pos >= tokens.len() {
            break;
        }
        let (mut off, mut off64, mut c_type) = (None, None, None);
        for (k, v) in core::mem::take(&mut a.kvs) {
            match k.as_str() {
                "offset" => off = Some(v.usize(&k)?),
                "offset64" => off64 = Some(v.usize(&k)?),
                "c_type" => c_type = Some(v.str(&k)?),
                "doc" => a.doc_override = Some(v.str(&k)?),
                other => return Err(format!("unknown #[nve_abi] field key `{other}`")),
            }
        }
        skip_vis(tokens, &mut pos);
        let rust_ident = expect_ident(tokens, &mut pos, "field name")?;
        if !is_punct(tokens, pos, ':') {
            return Err(format!("expected `:` after `{rust_ident}`"));
        }
        pos += 1;
        let (start, mut depth) = (pos, 0usize);
        while pos < tokens.len() {
            if is_arrow(tokens, pos) {
                pos += 2;
                continue;
            }
            match tokens[pos] {
                Tok::Punct('<', _) => depth += 1,
                Tok::Punct('>', _) => depth = depth.saturating_sub(1),
                Tok::Punct(',', _) if depth == 0 => break,
                _ => {}
            }
            pos += 1;
        }
        if pos == start {
            return Err(format!("missing type for `{rust_ident}`"));
        }
        let ty_toks = &tokens[start..pos];
        if is_punct(tokens, pos, ',') {
            pos += 1;
        }
        out.push(RawField {
            rust_ident,
            ty_toks,
            doc: a.doc(),
            c_type,
            expected_offset: off,
            expected_offset_64: off64,
        });
    }
    Ok(out)
}

fn parse_fields(tokens: &[Tok]) -> Result<Vec<ParsedField>, String> {
    parse_raw_fields(tokens)?
        .into_iter()
        .map(|f| {
            Ok(ParsedField {
                abi_name: strip_raw(&f.rust_ident),
                rust_ident: f.rust_ident,
                ty: format_type(f.ty_toks)?,
                c_type: f.c_type,
                doc: f.doc,
                expected_offset: f.expected_offset,
                expected_offset_64: f.expected_offset_64,
            })
        })
        .collect()
}

fn parse_vtable_body(
    body: &[Tok],
    vtable_name: &str,
    is_interface: bool,
) -> Result<(Vec<ParsedField>, Vec<ParsedVtableMethod>), String> {
    let mut header_fields = Vec::new();
    let mut methods = Vec::new();
    let mut seen_fn = false;

    for f in parse_raw_fields(body)? {
        if is_fn_ptr_type(f.ty_toks) {
            if f.expected_offset.is_some() || f.expected_offset_64.is_some() || f.c_type.is_some() {
                return Err(format!(
                    "vtable slot `{vtable_name}::{}` does not accept `offset`/`offset64`/`c_type`",
                    f.rust_ident
                ));
            }
            seen_fn = true;
            let sig =
                parse_fn_ptr_sig(f.ty_toks, true, &format!("{vtable_name}::{}", f.rust_ident))?;
            methods.push(ParsedVtableMethod {
                abi_name: strip_raw(&f.rust_ident),
                rust_ident: f.rust_ident,
                doc: f.doc,
                sig,
            });
        } else {
            if is_interface {
                return Err(format!(
                    "#[nve_abi(interface = ...)] vtable `{vtable_name}` cannot contain data header field `{}`",
                    f.rust_ident
                ));
            }
            if seen_fn {
                return Err(format!(
                    "#[nve_abi(vtable)] data field `{}` cannot appear after function-pointer slots on `{vtable_name}`",
                    f.rust_ident
                ));
            }
            if unwrap_option_type(f.ty_toks).is_some() {
                return Err(format!(
                    "vtable `{vtable_name}` header field `{}` cannot use `Option<...>`; vtable slots must use inline `[unsafe] extern \"C\" fn(...)` signatures",
                    f.rust_ident
                ));
            }
            header_fields.push(ParsedField {
                abi_name: strip_raw(&f.rust_ident),
                rust_ident: f.rust_ident,
                ty: format_type(f.ty_toks)?,
                c_type: f.c_type,
                doc: f.doc,
                expected_offset: f.expected_offset,
                expected_offset_64: f.expected_offset_64,
            });
        }
    }

    if methods.is_empty() {
        return Err(format!(
            "#[derive(NveAbi)] vtable `{vtable_name}` must declare at least one function-pointer slot"
        ));
    }
    Ok((header_fields, methods))
}

fn parse_single_tuple_field(inner: &[Tok], struct_name: &str) -> Result<(String, String), String> {
    let mut pos = 0usize;
    let mut fields = Vec::new();
    while pos < inner.len() {
        let _ = collect_attrs(inner, &mut pos, false)?;
        if pos >= inner.len() {
            break;
        }
        skip_vis(inner, &mut pos);
        let start = pos;
        let mut depth = 0usize;
        while pos < inner.len() {
            if is_arrow(inner, pos) {
                pos += 2;
                continue;
            }
            match inner[pos] {
                Tok::Punct('<', _) => depth += 1,
                Tok::Punct('>', _) => depth = depth.saturating_sub(1),
                Tok::Punct(',', _) if depth == 0 => break,
                _ => {}
            }
            pos += 1;
        }
        if pos == start {
            return Err(format!(
                "missing inner type on `#[repr(transparent)]` `{struct_name}`"
            ));
        }
        fields.push((
            render_tokens(&inner[start..pos]),
            format_type(&inner[start..pos])?,
        ));
        if is_punct(inner, pos, ',') {
            pos += 1;
        }
    }
    if fields.len() != 1 {
        return Err(format!(
            "#[derive(NveAbi)] on `#[repr(transparent)]` `{struct_name}` requires exactly 1 field, found {}",
            fields.len()
        ));
    }
    Ok(fields.remove(0))
}

fn parse_variants(tokens: &[Tok], enum_name: &str) -> Result<Vec<ParsedVariant>, String> {
    let (mut pos, mut out) = (0usize, Vec::new());
    while pos < tokens.len() {
        let mut a = collect_attrs(tokens, &mut pos, false)?;
        if pos >= tokens.len() {
            break;
        }
        for (k, v) in core::mem::take(&mut a.kvs) {
            if k == "doc" {
                a.doc_override = Some(v.str(&k)?);
            } else {
                return Err(format!("unknown #[nve_abi] variant key `{k}`"));
            }
        }
        let rust_ident = expect_ident(tokens, &mut pos, "enum variant")?;
        if matches!(
            tokens.get(pos),
            Some(Tok::Group(Delim::Paren | Delim::Brace, _))
        ) {
            return Err(format!(
                "#[derive(NveAbi)] enum `{enum_name}::{rust_ident}` must be unit-only"
            ));
        }
        if is_punct(tokens, pos, '=') {
            pos += 1;
            while pos < tokens.len() && !is_punct(tokens, pos, ',') {
                pos += 1;
            }
        }
        if is_punct(tokens, pos, ',') {
            pos += 1;
        }
        out.push(ParsedVariant {
            abi_name: strip_raw(&rust_ident),
            rust_ident,
            doc: a.doc(),
        });
    }
    Ok(out)
}
