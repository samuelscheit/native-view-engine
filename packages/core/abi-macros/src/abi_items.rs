//! Function-like zero-dependency proc-macros for colocated ABI constants, type aliases, and
//! callback typedefs: `abi_consts!`, `abi_aliases!`, and `abi_callbacks!`.
//!
//! Each macro emits the exact Rust items verbatim (preserving `///` docs and other attributes
//! while stripping `#[nve_abi(...)]` helper attributes) alongside a `const` descriptor slice.

use std::fmt::Write as _;

use proc_macro::TokenStream;

use crate::emit::{opt_str, quote_str};
use crate::fn_sig::{format_type, parse_fn_ptr_sig, strip_raw};
use crate::parse::parse_string_literal;
use crate::tok::{
    attr_name, check_no_cfg_attr, expect_ident, from_stream, is_punct, paren_inner,
    render_expr_tokens, render_tokens, skip_vis, Delim, Tok,
};

#[cfg(test)]
#[path = "abi_items_tests.rs"]
mod tests;

const DEFAULT_CRATE_PATH: &str = "::nve_abi::schema";

struct ItemHeader {
    vis: String,
    const_name: String,
    impl_ty: Option<String>,
    prefix: Option<String>,
    crate_path: String,
}

#[derive(Default)]
struct HelperAttrs {
    pass_attrs: Vec<String>,
    doc_lines: Vec<String>,
    doc_override: Option<String>,
    name_override: Option<String>,
    target_override: Option<String>,
    hex: bool,
    skip: bool,
}

impl HelperAttrs {
    fn doc(self) -> Option<String> {
        self.doc_override
            .or_else(|| (!self.doc_lines.is_empty()).then(|| self.doc_lines.join("\n")))
    }
}

pub(crate) fn expand_consts(input: TokenStream) -> Result<String, String> {
    expand_consts_tokens(&from_stream(input))
}

pub(crate) fn expand_aliases(input: TokenStream) -> Result<String, String> {
    expand_aliases_tokens(&from_stream(input))
}

pub(crate) fn expand_callbacks(input: TokenStream) -> Result<String, String> {
    expand_callbacks_tokens(&from_stream(input))
}

fn parse_vis(tokens: &[Tok], pos: &mut usize) -> String {
    let start = *pos;
    skip_vis(tokens, pos);
    render_tokens(&tokens[start..*pos])
}

fn parse_header(tokens: &[Tok], pos: &mut usize, macro_name: &str) -> Result<ItemHeader, String> {
    let vis = parse_vis(tokens, pos);
    let kw = expect_ident(tokens, pos, "`const`")?;
    if kw != "const" {
        return Err(format!("{macro_name}! header expects `[vis] const NAME;`"));
    }
    let const_name = expect_ident(tokens, pos, "descriptor slice constant name")?;
    let mut impl_ty = None;
    let mut prefix = None;
    let mut crate_path = DEFAULT_CRATE_PATH.to_owned();

    if let Some(Tok::Group(Delim::Paren, opts)) = tokens.get(*pos) {
        *pos += 1;
        let mut i = 0;
        while i < opts.len() {
            if crate::tok::is_ident(opts, i, "impl") {
                i += 1;
                let start = i;
                while i < opts.len() && !is_punct(opts, i, ',') {
                    i += 1;
                }
                if i == start {
                    return Err(format!("{macro_name}!: expected type after `impl`"));
                }
                impl_ty = Some(render_tokens(&opts[start..i]));
            } else {
                let k = expect_ident(opts, &mut i, "header option")?;
                if !is_punct(opts, i, '=') {
                    return Err(format!("{macro_name}!: expected `=` after `{k}`"));
                }
                i += 1;
                let Some(Tok::Literal(lit)) = opts.get(i) else {
                    return Err(format!(
                        "{macro_name}!: expected string literal for `{k}`"
                    ));
                };
                i += 1;
                let val = parse_string_literal(lit)?;
                match k.as_str() {
                    "prefix" => prefix = Some(val),
                    "crate_path" => crate_path = val,
                    other => return Err(format!("{macro_name}!: unknown header option `{other}`")),
                }
            }
            if is_punct(opts, i, ',') {
                i += 1;
            }
        }
    }

    if !is_punct(tokens, *pos, ';') {
        return Err(format!(
            "{macro_name}!: expected `;` after header `{const_name}`"
        ));
    }
    *pos += 1;
    Ok(ItemHeader {
        vis,
        const_name,
        impl_ty,
        prefix,
        crate_path,
    })
}

fn collect_helper_attrs(tokens: &[Tok], pos: &mut usize) -> Result<HelperAttrs, String> {
    let mut out = HelperAttrs::default();
    while is_punct(tokens, *pos, '#') {
        let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(*pos + 1) else {
            return Err("expected `[...]` after `#`".to_owned());
        };
        *pos += 2;
        let (name, rest) = attr_name(inner);
        check_no_cfg_attr(
            name,
            rest,
            true,
            &["nve_abi", "cfg"],
            "`abi_{consts,aliases,callbacks}!`",
        )?;
        match name {
            "nve_abi" => {
                let args = paren_inner(rest).ok_or("expected `#[nve_abi(...)]`")?;
                let mut k = 0;
                while k < args.len() {
                    let key = expect_ident(args, &mut k, "nve_abi option")?;
                    if is_punct(args, k, '=') {
                        k += 1;
                        let Some(Tok::Literal(lit)) = args.get(k) else {
                            return Err(format!("expected string literal for `#[nve_abi({key} = ...)]`"));
                        };
                        k += 1;
                        let val = parse_string_literal(lit)?;
                        match key.as_str() {
                            "name" => out.name_override = Some(val),
                            "target" => out.target_override = Some(val),
                            "doc" => out.doc_override = Some(val),
                            other => return Err(format!("unknown #[nve_abi] key `{other}`")),
                        }
                    } else {
                        match key.as_str() {
                            "hex" => out.hex = true,
                            "skip" => out.skip = true,
                            other => return Err(format!("unknown #[nve_abi] flag `{other}`")),
                        }
                    }
                    if is_punct(args, k, ',') {
                        k += 1;
                    }
                }
            }
            "doc" if is_punct(rest, 0, '=') => {
                if let Some(Tok::Literal(lit)) = rest.get(1) {
                    let s = parse_string_literal(lit)?;
                    out.doc_lines
                        .push(s.strip_prefix(' ').unwrap_or(&s).to_owned());
                }
                out.pass_attrs.push(format!("#[{}]", render_tokens(inner)));
            }
            _ => out.pass_attrs.push(format!("#[{}]", render_tokens(inner))),
        }
    }
    Ok(out)
}

fn const_abi_mapping(
    ty_str: &str,
    qual_name: &str,
    impl_ty: Option<&str>,
) -> Result<(&'static str, String, Option<String>), String> {
    let resolved = if ty_str == "Self" {
        impl_ty.ok_or("`Self` const type requires `(impl Type)` header")?
    } else {
        ty_str
    };
    let short = resolved.rsplit("::").next().unwrap_or(resolved).trim();
    match short {
        "u8" | "u16" | "u32" => Ok(("u32", format!("({qual_name}) as u64"), None)),
        "usize" => Ok((
            "u32",
            format!("({qual_name}) as u64"),
            Some(format!(
                "const _: () = ::core::assert!(({qual_name} as u64) <= (::core::primitive::u32::MAX as u64));"
            )),
        )),
        "u64" => Ok(("u64", qual_name.to_owned(), None)),
        "NodeId" | "SurfaceId" | "InterfaceId" | "EventId" | "EventFlags" | "ArenaOffset"
        | "ByteLen" | "DurationMs" | "NodeIndex" | "HostServiceId" => {
            Ok(("u32", format!("({qual_name}).raw() as u64"), None))
        }
        "FrameId" | "TimestampUs" | "DurationUs" | "TimestampMs" | "HostRequestId" => {
            Ok(("u64", format!("({qual_name}).raw()"), None))
        }
        "TableKey" | "Key64" => Ok(("u64", format!("({qual_name}).0"), None)),
        other => Err(format!(
            "abi_consts!: unsupported const type `{other}` on `{qual_name}`"
        )),
    }
}

pub(crate) fn expand_consts_tokens(tokens: &[Tok]) -> Result<String, String> {
    let mut pos = 0usize;
    let hdr = parse_header(tokens, &mut pos, "abi_consts")?;
    let mut items_out = String::with_capacity(1024);
    let mut descs_out = String::with_capacity(1024);
    let mut asserts_out = String::new();

    if let Some(ref impl_ty) = hdr.impl_ty {
        let _ = writeln!(items_out, "impl {impl_ty} {{");
    }

    while pos < tokens.len() {
        let attrs = collect_helper_attrs(tokens, &mut pos)?;
        if pos >= tokens.len() {
            break;
        }
        if attrs.target_override.is_some() {
            return Err("abi_consts!: `target` is only valid in `abi_aliases!`".to_owned());
        }
        let item_vis = parse_vis(tokens, &mut pos);
        let kw = expect_ident(tokens, &mut pos, "`const`")?;
        if kw != "const" {
            return Err(format!("abi_consts! expects `const`, found `{kw}`"));
        }
        let rust_ident = expect_ident(tokens, &mut pos, "const name")?;
        if !is_punct(tokens, pos, ':') {
            return Err(format!("abi_consts!: expected `:` after `{rust_ident}`"));
        }
        pos += 1;
        let ty_start = pos;
        while pos < tokens.len() && !is_punct(tokens, pos, '=') {
            pos += 1;
        }
        if pos == ty_start || !is_punct(tokens, pos, '=') {
            return Err(format!("abi_consts!: expected `= expr;` on `{rust_ident}`"));
        }
        let ty_toks = &tokens[ty_start..pos];
        pos += 1;
        let expr_start = pos;
        while pos < tokens.len() && !is_punct(tokens, pos, ';') {
            pos += 1;
        }
        if pos == expr_start || !is_punct(tokens, pos, ';') {
            return Err(format!("abi_consts!: expected `;` after `{rust_ident}`"));
        }
        let expr_toks = &tokens[expr_start..pos];
        pos += 1;

        for a in &attrs.pass_attrs {
            let _ = writeln!(items_out, "{a}");
        }
        let _ = writeln!(
            items_out,
            "{item_vis} const {rust_ident}: {} = {};",
            render_tokens(ty_toks),
            render_expr_tokens(expr_toks)
        );

        if !attrs.skip {
            let qual_name = match &hdr.impl_ty {
                Some(ty) => format!("{ty}::{rust_ident}"),
                None => rust_ident.clone(),
            };
            let abi_name = if let Some(ref ov) = attrs.name_override {
                ov.clone()
            } else if let Some(ref pref) = hdr.prefix {
                format!("{pref}{}", strip_raw(&rust_ident))
            } else {
                strip_raw(&rust_ident)
            };
            let ty_short = format_type(ty_toks)?;
            let (abi_ty, val_expr, maybe_assert) =
                const_abi_mapping(&ty_short, &qual_name, hdr.impl_ty.as_deref())?;
            if let Some(a) = maybe_assert {
                let _ = writeln!(asserts_out, "{a}");
            }
            let hex = attrs.hex;
            let doc = attrs.doc();
            let _ = writeln!(
                descs_out,
                "    {}::AbiConstDescriptor {{ name: {}, ty: {}, value: {val_expr}, hex: {hex}, doc: {} }},",
                hdr.crate_path,
                quote_str(&abi_name),
                quote_str(abi_ty),
                opt_str(doc.as_deref()),
            );
        }
    }

    if hdr.impl_ty.is_some() {
        let _ = writeln!(items_out, "}}");
    }

    let _ = writeln!(
        items_out,
        "{} const {}: &[{}::AbiConstDescriptor] = &[\n{descs_out}];\n{asserts_out}",
        hdr.vis, hdr.const_name, hdr.crate_path
    );
    Ok(items_out)
}

pub(crate) fn expand_aliases_tokens(tokens: &[Tok]) -> Result<String, String> {
    let mut pos = 0usize;
    let hdr = parse_header(tokens, &mut pos, "abi_aliases")?;
    if hdr.impl_ty.is_some() || hdr.prefix.is_some() {
        return Err("abi_aliases!: `impl` and `prefix` options are only valid in `abi_consts!`".to_owned());
    }
    let mut items_out = String::with_capacity(512);
    let mut descs_out = String::with_capacity(512);

    while pos < tokens.len() {
        let attrs = collect_helper_attrs(tokens, &mut pos)?;
        if pos >= tokens.len() {
            break;
        }
        if attrs.hex {
            return Err("abi_aliases!: `hex` is only valid in `abi_consts!`".to_owned());
        }
        let item_vis = parse_vis(tokens, &mut pos);
        let kw = expect_ident(tokens, &mut pos, "`type`")?;
        if kw != "type" {
            return Err(format!("abi_aliases! expects `type`, found `{kw}`"));
        }
        let rust_ident = expect_ident(tokens, &mut pos, "alias name")?;
        if !is_punct(tokens, pos, '=') {
            return Err(format!("abi_aliases!: expected `=` after `{rust_ident}`"));
        }
        pos += 1;
        let rhs_start = pos;
        while pos < tokens.len() && !is_punct(tokens, pos, ';') {
            pos += 1;
        }
        if pos == rhs_start || !is_punct(tokens, pos, ';') {
            return Err(format!("abi_aliases!: expected `;` after `{rust_ident}`"));
        }
        let rhs_toks = &tokens[rhs_start..pos];
        pos += 1;

        for a in &attrs.pass_attrs {
            let _ = writeln!(items_out, "{a}");
        }
        let _ = writeln!(
            items_out,
            "{item_vis} type {rust_ident} = {};",
            render_tokens(rhs_toks)
        );

        if !attrs.skip {
            let stripped = strip_raw(&rust_ident);
            let abi_name = attrs.name_override.clone().unwrap_or_else(|| stripped.clone());
            let rust_name = (stripped != abi_name).then_some(stripped);
            let target = match &attrs.target_override {
                Some(ov) => ov.clone(),
                None => format_type(rhs_toks)?,
            };
            let doc = attrs.doc();
            let _ = writeln!(
                descs_out,
                "    {}::AbiAliasDescriptor {{ name: {}, rust_name: {}, target: {}, doc: {} }},",
                hdr.crate_path,
                quote_str(&abi_name),
                opt_str(rust_name.as_deref()),
                quote_str(&target),
                opt_str(doc.as_deref()),
            );
        }
    }

    let _ = writeln!(
        items_out,
        "{} const {}: &[{}::AbiAliasDescriptor] = &[\n{descs_out}];",
        hdr.vis, hdr.const_name, hdr.crate_path
    );
    Ok(items_out)
}

pub(crate) fn expand_callbacks_tokens(tokens: &[Tok]) -> Result<String, String> {
    let mut pos = 0usize;
    let hdr = parse_header(tokens, &mut pos, "abi_callbacks")?;
    if hdr.impl_ty.is_some() || hdr.prefix.is_some() {
        return Err("abi_callbacks!: `impl` and `prefix` options are only valid in `abi_consts!`".to_owned());
    }
    let mut items_out = String::with_capacity(1024);
    let mut descs_out = String::with_capacity(1024);

    while pos < tokens.len() {
        let attrs = collect_helper_attrs(tokens, &mut pos)?;
        if pos >= tokens.len() {
            break;
        }
        if attrs.hex || attrs.target_override.is_some() {
            return Err("abi_callbacks!: unsupported `hex`/`target` option".to_owned());
        }
        let item_vis = parse_vis(tokens, &mut pos);
        let kw = expect_ident(tokens, &mut pos, "`type`")?;
        if kw != "type" {
            return Err(format!("abi_callbacks! expects `type`, found `{kw}`"));
        }
        let rust_ident = expect_ident(tokens, &mut pos, "callback typedef name")?;
        if !is_punct(tokens, pos, '=') {
            return Err(format!("abi_callbacks!: expected `=` after `{rust_ident}`"));
        }
        pos += 1;
        let rhs_start = pos;
        while pos < tokens.len() && !is_punct(tokens, pos, ';') {
            pos += 1;
        }
        if pos == rhs_start || !is_punct(tokens, pos, ';') {
            return Err(format!("abi_callbacks!: expected `;` after `{rust_ident}`"));
        }
        let rhs_toks = &tokens[rhs_start..pos];
        pos += 1;

        for a in &attrs.pass_attrs {
            let _ = writeln!(items_out, "{a}");
        }
        let _ = writeln!(
            items_out,
            "{item_vis} type {rust_ident} = {};",
            render_tokens(rhs_toks)
        );

        if !attrs.skip {
            let abi_name = attrs
                .name_override
                .clone()
                .unwrap_or_else(|| strip_raw(&rust_ident));
            let sig = parse_fn_ptr_sig(rhs_toks, false, &abi_name)?;
            let doc = attrs.doc();
            let _ = write!(
                descs_out,
                "    {}::AbiCallbackDescriptor {{ name: {}, params: &[",
                hdr.crate_path,
                quote_str(&abi_name),
            );
            for p in &sig.params {
                let _ = write!(
                    descs_out,
                    "{}::AbiParamDescriptor {{ name: {}, ty: {} }}, ",
                    hdr.crate_path,
                    quote_str(&p.abi_name),
                    quote_str(&p.ty),
                );
            }
            let _ = writeln!(
                descs_out,
                "], return_type: {}, doc: {} }},",
                quote_str(&sig.return_type),
                opt_str(doc.as_deref()),
            );
        }
    }

    let _ = writeln!(
        items_out,
        "{} const {}: &[{}::AbiCallbackDescriptor] = &[\n{descs_out}];",
        hdr.vis, hdr.const_name, hdr.crate_path
    );
    Ok(items_out)
}
