//! Attribute, `#[repr(...)]`, and `#[nve_abi(...)]` key-value parsing for `#[derive(NveAbi)]`.

use crate::lit::{parse_string_literal, parse_usize_literal};
use crate::tok::{
    attr_name, check_no_cfg_attr, expect_ident, is_punct, paren_inner, render_tokens, Delim, Tok,
};

#[derive(Default)]
pub(crate) struct Attrs {
    pub doc_override: Option<String>,
    pub doc_lines: Vec<String>,
    pub repr_c: bool,
    pub repr_transparent: bool,
    pub repr_align: Option<usize>,
    pub repr_int: Option<String>,
    pub kvs: Vec<(String, Kv)>,
}

impl Attrs {
    pub(crate) fn doc(self) -> Option<String> {
        self.doc_override
            .or_else(|| (!self.doc_lines.is_empty()).then(|| self.doc_lines.join("\n")))
    }
}

pub(crate) enum Kv {
    Flag,
    Str(String),
    Usize(usize),
    Path(String),
}

impl Kv {
    pub(crate) fn str(self, k: &str) -> Result<String, String> {
        match self {
            Self::Str(s) => Ok(s),
            _ => Err(format!("expected string literal for `{k}`")),
        }
    }

    pub(crate) fn usize(self, k: &str) -> Result<usize, String> {
        match self {
            Self::Usize(u) => Ok(u),
            _ => Err(format!("expected integer literal for `{k}`")),
        }
    }

    pub(crate) fn path(self, k: &str) -> Result<String, String> {
        match self {
            Self::Path(p) => Ok(p),
            _ => Err(format!("expected path expression for `{k}`")),
        }
    }

    pub(crate) fn flag(self, k: &str) -> Result<(), String> {
        match self {
            Self::Flag => Ok(()),
            _ => Err(format!("`{k}` does not take a value")),
        }
    }
}

pub(crate) fn collect_attrs(
    tokens: &[Tok],
    pos: &mut usize,
    read_repr: bool,
) -> Result<Attrs, String> {
    let mut out = Attrs::default();
    while is_punct(tokens, *pos, '#') {
        let Some(Tok::Group(Delim::Bracket, inner)) = tokens.get(*pos + 1) else {
            return Err("expected `[...]` after `#`".to_owned());
        };
        *pos += 2;
        let (name, rest) = attr_name(inner);
        check_no_cfg_attr(
            name,
            rest,
            !read_repr,
            if read_repr {
                &["nve_abi", "repr", "cfg"]
            } else {
                &["nve_abi", "cfg"]
            },
            if read_repr {
                "#[derive(NveAbi)]"
            } else {
                "#[derive(NveAbi)] field/variant"
            },
        )?;
        match name {
            "doc" if is_punct(rest, 0, '=') => {
                if let Some(Tok::Literal(lit)) = rest.get(1) {
                    let s = parse_string_literal(lit)?;
                    out.doc_lines
                        .push(s.strip_prefix(' ').unwrap_or(&s).to_owned());
                }
            }
            "repr" if read_repr => {
                let args = paren_inner(rest).ok_or("expected `#[repr(...)]`")?;
                parse_repr(args, &mut out)?;
            }
            "nve_abi" => {
                let args = paren_inner(rest).ok_or("expected `#[nve_abi(...)]`")?;
                out.kvs.extend(parse_kvs(args)?);
            }
            _ => {}
        }
    }
    Ok(out)
}

fn parse_repr(inner: &[Tok], out: &mut Attrs) -> Result<(), String> {
    let mut i = 0;
    while i < inner.len() {
        if let Tok::Ident(s) = &inner[i] {
            i += 1;
            match s.as_str() {
                "C" => out.repr_c = true,
                "transparent" => out.repr_transparent = true,
                "u8" | "u16" | "u32" | "i32" => out.repr_int = Some(s.clone()),
                "packed" => {
                    return Err("#[derive(NveAbi)] does not support `#[repr(packed)]`".to_owned());
                }
                "align" => {
                    if let Some(Tok::Group(Delim::Paren, args)) = inner.get(i) {
                        i += 1;
                        let [Tok::Literal(lit)] = args.as_slice() else {
                            return Err("expected integer in `repr(align(...))`".to_owned());
                        };
                        let n = parse_usize_literal(lit)?;
                        out.repr_align = Some(out.repr_align.map_or(n, |prev| prev.max(n)));
                    }
                }
                other => {
                    return Err(format!(
                        "#[derive(NveAbi)] does not support `#[repr({other})]`"
                    ));
                }
            }
        } else {
            i += 1;
        }
        if is_punct(inner, i, ',') {
            i += 1;
        }
    }
    Ok(())
}

fn parse_kvs(tokens: &[Tok]) -> Result<Vec<(String, Kv)>, String> {
    let (mut pos, mut out) = (0usize, Vec::new());
    while pos < tokens.len() {
        let k = expect_ident(tokens, &mut pos, "attribute key")?;
        if !is_punct(tokens, pos, '=') {
            out.push((k, Kv::Flag));
            if is_punct(tokens, pos, ',') {
                pos += 1;
            }
            continue;
        }
        pos += 1;
        let v = match tokens.get(pos) {
            Some(Tok::Literal(raw)) => {
                pos += 1;
                if raw.starts_with('"') || raw.starts_with('r') {
                    Kv::Str(parse_string_literal(raw)?)
                } else {
                    Kv::Usize(parse_usize_literal(raw)?)
                }
            }
            Some(_) => {
                let start = pos;
                while pos < tokens.len() && !is_punct(tokens, pos, ',') {
                    pos += 1;
                }
                Kv::Path(render_tokens(&tokens[start..pos]))
            }
            None => return Err(format!("expected value for `#[nve_abi({k} = ...)]`")),
        };
        out.push((k, v));
        if is_punct(tokens, pos, ',') {
            pos += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::collect_attrs;
    use crate::tok::lex;

    #[test]
    fn duplicate_repr_align_uses_max_and_attr_name_skips_leading_colons() {
        let toks = lex("#[repr(C, align(16), align(4))] #[::nve_abi(size = 32)]");
        let mut pos = 0;
        let a = collect_attrs(&toks, &mut pos, true).expect("valid attrs");
        assert!(a.repr_c);
        assert_eq!(a.repr_align, Some(16));
        assert_eq!(a.kvs.len(), 1);
        assert_eq!(a.kvs[0].0, "size");

        let bad_cfg = lex("#[::core::cfg_attr(all(), repr(C))]");
        let mut pos2 = 0;
        let err = collect_attrs(&bad_cfg, &mut pos2, true)
            .err()
            .expect("leading :: cfg_attr must be rejected");
        assert!(err.contains("cfg_attr"), "{err}");
    }
}
