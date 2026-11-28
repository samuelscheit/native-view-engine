//! Shared zero-dependency `Tok`-based parser for C ABI type spellings and `extern "C" fn`
//! signatures used by `#[derive(NveAbi)]`, `abi_consts!`, `abi_aliases!`, and `abi_callbacks!`.

use crate::parse::parse_string_literal;
use crate::tok::{is_colon2, is_ident, is_punct, Delim, Tok};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedParam {
    pub rust_ident: String,
    pub abi_name: String,
    pub ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedFnSig {
    pub params: Vec<ParsedParam>,
    pub return_type: String,
    pub optional: bool,
}

pub(crate) fn strip_raw(s: &str) -> String {
    s.strip_prefix("r#").unwrap_or(s).to_owned()
}

/// Unwraps transparent macro-metavar `Delim::None` wrappers around a token slice.
pub(crate) fn unwrap_none(mut tokens: &[Tok]) -> &[Tok] {
    while let [Tok::Group(Delim::None, inner)] = tokens {
        tokens = inner.as_slice();
    }
    tokens
}

/// Formats a Rust ABI type token slice into its canonical short spelling (`*const NveMapEntry`,
/// `[u32; 4]`, `Option<NveRuntimeWakeFn>`, `c_void`, `()`), stripping module paths on identifiers.
pub(crate) fn format_type(tokens: &[Tok]) -> String {
    let tokens = unwrap_none(tokens);
    let mut out = String::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Tok::Group(Delim::None, inner) => {
                out.push_str(&format_type(inner));
                i += 1;
            }
            Tok::Punct('*', _) => {
                out.push('*');
                i += 1;
                if let Some(Tok::Ident(kw)) = tokens.get(i) {
                    out.push_str(kw);
                    out.push(' ');
                    i += 1;
                }
            }
            Tok::Group(Delim::Bracket, inner) => {
                out.push('[');
                if let Some(semi) = find_top_level_punct(inner, ';') {
                    out.push_str(&format_type(&inner[..semi]));
                    out.push_str("; ");
                    out.push_str(&format_type(&inner[semi + 1..]));
                } else {
                    out.push_str(&format_type(inner));
                }
                out.push(']');
                i += 1;
            }
            Tok::Group(Delim::Paren, inner) => {
                out.push('(');
                out.push_str(&format_type(inner));
                out.push(')');
                i += 1;
            }
            Tok::Punct(':', true) if is_colon2(tokens, i) => {
                i += 2;
            }
            Tok::Ident(id) => {
                let mut last = id.clone();
                i += 1;
                while is_colon2(tokens, i) {
                    i += 2;
                    if let Some(Tok::Ident(seg)) = tokens.get(i) {
                        last = seg.clone();
                        i += 1;
                    }
                }
                out.push_str(&last);
            }
            Tok::Punct(ch, _) => {
                out.push(*ch);
                if *ch == ',' {
                    out.push(' ');
                }
                i += 1;
            }
            Tok::Literal(lit) => {
                out.push_str(lit);
                i += 1;
            }
            Tok::Group(Delim::Brace, inner) => {
                out.push('{');
                out.push_str(&format_type(inner));
                out.push('}');
                i += 1;
            }
        }
    }
    out.trim().to_owned()
}

fn find_top_level_punct(tokens: &[Tok], ch: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Tok::Punct('<', _) => depth += 1,
            Tok::Punct('>', _) => depth = depth.saturating_sub(1),
            Tok::Punct(c, _) if *c == ch && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// If `tokens` is `[::]...[Option]<INNER>`, returns `Some(INNER)`.
pub(crate) fn unwrap_option_type(tokens: &[Tok]) -> Option<&[Tok]> {
    let tokens = unwrap_none(tokens);
    let mut pos = 0usize;
    if is_colon2(tokens, pos) {
        pos += 2;
    }
    let mut last: Option<&str> = None;
    while let Some(Tok::Ident(seg)) = tokens.get(pos) {
        last = Some(seg.as_str());
        pos += 1;
        if is_colon2(tokens, pos) {
            pos += 2;
        } else {
            break;
        }
    }
    if last != Some("Option") || !is_punct(tokens, pos, '<') {
        return None;
    }
    let start = pos + 1;
    let mut depth = 1usize;
    let mut i = start;
    while i < tokens.len() && depth > 0 {
        match tokens[i] {
            Tok::Punct('<', _) => depth += 1,
            Tok::Punct('>', _) => depth -= 1,
            _ => {}
        }
        if depth > 0 {
            i += 1;
        }
    }
    if depth == 0 && i + 1 == tokens.len() {
        Some(&tokens[start..i])
    } else {
        None
    }
}

/// Returns `true` when `tokens` is an inline function-pointer type (optionally wrapped in
/// `Option<...>`), including invalid forms such as `fn(...)`, `extern "Rust" fn(...)`, or
/// nested `Option<Option<extern "C" fn()>>` so `parse_fn_ptr_sig` reports a targeted error.
pub(crate) fn is_fn_ptr_type(tokens: &[Tok]) -> bool {
    let tokens = unwrap_none(tokens);
    if is_ident(tokens, 0, "unsafe")
        || is_ident(tokens, 0, "extern")
        || is_ident(tokens, 0, "fn")
    {
        return true;
    }
    if let Some(inner) = unwrap_option_type(tokens) {
        return is_fn_ptr_type(inner);
    }
    false
}

/// Parses `[unsafe] extern "C" fn(name: Ty, ...) [-> Ret]` (and optionally one outer
/// `Option<...>` when `allow_option` is true).
pub(crate) fn parse_fn_ptr_sig(
    tokens: &[Tok],
    allow_option: bool,
    ctx: &str,
) -> Result<ParsedFnSig, String> {
    let tokens = unwrap_none(tokens);
    if let Some(inner) = unwrap_option_type(tokens) {
        if !allow_option {
            return Err(format!(
                "`{ctx}` does not allow `Option<...>` wrapper around function pointer"
            ));
        }
        if unwrap_option_type(inner).is_some() {
            return Err(format!(
                "`{ctx}`: nested `Option<Option<...>>` vtable slots are unsupported"
            ));
        }
        let mut sig = parse_fn_ptr_sig(inner, false, ctx)?;
        sig.optional = true;
        return Ok(sig);
    }

    let mut pos = 0usize;
    if is_ident(tokens, pos, "unsafe") {
        pos += 1;
    }
    if is_ident(tokens, pos, "extern") {
        pos += 1;
        let Some(Tok::Literal(abi_lit)) = tokens.get(pos) else {
            return Err(format!(
                "`{ctx}` requires explicit `extern \"C\"` calling convention"
            ));
        };
        let abi = parse_string_literal(abi_lit)?;
        if abi != "C" {
            return Err(format!(
                "`{ctx}` requires `extern \"C\"` calling convention, found `extern \"{abi}\"`"
            ));
        }
        pos += 1;
    } else if is_ident(tokens, pos, "fn") {
        return Err(format!(
            "`{ctx}` requires `extern \"C\"` calling convention"
        ));
    } else {
        return Err(format!(
            "`{ctx}` expected `[unsafe] extern \"C\" fn(...)`"
        ));
    }

    if !is_ident(tokens, pos, "fn") {
        return Err(format!("`{ctx}` expected `fn` after `extern \"C\"`"));
    }
    pos += 1;

    let Some(Tok::Group(Delim::Paren, param_toks)) = tokens.get(pos) else {
        return Err(format!("`{ctx}` expected parameter list `(...)` after `fn`"));
    };
    pos += 1;

    let params = parse_fn_params(param_toks, ctx)?;

    let return_type = if pos == tokens.len() {
        "()".to_owned()
    } else if is_punct(tokens, pos, '-') && is_punct(tokens, pos + 1, '>') {
        pos += 2;
        if pos >= tokens.len() {
            return Err(format!("`{ctx}` missing return type after `->`"));
        }
        format_type(&tokens[pos..])
    } else {
        return Err(format!(
            "`{ctx}` unexpected trailing tokens in function pointer signature"
        ));
    };

    Ok(ParsedFnSig {
        params,
        return_type,
        optional: false,
    })
}

fn is_variadic_dots(tokens: &[Tok], pos: usize) -> bool {
    is_punct(tokens, pos, '.') && is_punct(tokens, pos + 1, '.') && is_punct(tokens, pos + 2, '.')
}

fn parse_fn_params(tokens: &[Tok], ctx: &str) -> Result<Vec<ParsedParam>, String> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    while pos < tokens.len() {
        if is_variadic_dots(tokens, pos) {
            return Err(format!("`{ctx}`: variadic functions are unsupported"));
        }
        let rust_ident = match (tokens.get(pos), is_punct(tokens, pos + 1, ':')) {
            (Some(Tok::Ident(id)), true) => {
                pos += 2;
                id.clone()
            }
            _ => {
                return Err(format!(
                    "`{ctx}`: parameter must be named (`name: Type`)"
                ));
            }
        };
        let start = pos;
        let mut depth = 0usize;
        while pos < tokens.len() {
            if depth == 0 && is_variadic_dots(tokens, pos) {
                return Err(format!("`{ctx}`: variadic functions are unsupported"));
            }
            match &tokens[pos] {
                Tok::Punct('<', _) => depth += 1,
                Tok::Punct('>', _) => depth = depth.saturating_sub(1),
                Tok::Punct(',', _) if depth == 0 => break,
                _ => {}
            }
            pos += 1;
        }
        if pos == start {
            return Err(format!("`{ctx}`: missing type for parameter `{rust_ident}`"));
        }
        let ty = format_type(&tokens[start..pos]);
        if is_punct(tokens, pos, ',') {
            pos += 1;
        }
        out.push(ParsedParam {
            abi_name: strip_raw(&rust_ident),
            rust_ident,
            ty,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tok::lex;

    #[test]
    fn parses_fn_sigs_optional_raw_idents_and_rejects_invalid() {
        let s = parse_fn_ptr_sig(
            &lex("unsafe extern \"C\" fn(r#type: crate::id::NodeId, cb: Option<crate::WakeFn>) -> i32"),
            true,
            "t",
        )
        .unwrap();
        assert!(!s.optional);
        assert_eq!(s.return_type, "i32");
        assert_eq!(s.params.len(), 2);
        assert_eq!(s.params[0].rust_ident, "r#type");
        assert_eq!(s.params[0].abi_name, "type");
        assert_eq!(s.params[0].ty, "NodeId");
        assert_eq!(s.params[1].ty, "Option<WakeFn>");

        let opt = parse_fn_ptr_sig(&lex("Option<unsafe extern \"C\" fn()>"), true, "opt").unwrap();
        assert!(opt.optional);
        assert_eq!(opt.return_type, "()");
        assert!(opt.params.is_empty());

        for (src, needle) in [
            ("extern \"C\" fn(u32)", "parameter must be named"),
            ("fn(x: u32)", "extern \"C\""),
            ("extern \"Rust\" fn(x: u32)", "extern \"C\""),
            ("extern \"C\" fn(x: u32, ...)", "variadic"),
            ("Option<Option<extern \"C\" fn()>>", "nested"),
        ] {
            let err = parse_fn_ptr_sig(&lex(src), true, "bad").unwrap_err();
            assert!(err.contains(needle), "for `{src}`, got `{err}`");
        }
    }
}
