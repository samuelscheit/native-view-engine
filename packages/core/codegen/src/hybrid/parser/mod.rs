//! Zero-dependency pure-Rust tokenizer and recursive-descent parser for declarative
//! NVE Hybrid FFI `.rs` specification files.
//!
//! Recognizes idiomatic Rust declarations annotated with:
//! - `#[nve_hybrid_enum]` on `#[repr(u32)] pub enum ...`
//! - `#[nve_hybrid_struct]` on `pub struct ... { ... }`
//! - `#[nve_hybrid_module(name = "...")]` on `pub trait ... { ... }`
//! - `#[nve_hybrid_view(name = "...")]` on `pub trait ... { ... }`
//!   with member attributes `#[prop]`, `#[prop(default = ...)]`, `#[event]`, and `#[method]`.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::CodegenError;
use crate::hybrid::ir::{
    HybridAliasSpec, HybridConstSpec, HybridDefaultValue, HybridEnumSpec, HybridEnumVariant,
    HybridEventSpec, HybridFieldSpec, HybridFileSpec, HybridMethodSpec, HybridModuleSpec,
    HybridParamSpec, HybridPropSpec, HybridStructSpec, HybridType, HybridViewSpec,
};


mod items;
mod lexer;
mod traits;

use lexer::{tokenize, Tok, TokKind};

#[derive(Debug, Clone)]
struct ParsedAttr {
    name: String,
    args: Vec<(String, AttrValue)>,
    line: usize,
    col: usize,
}

#[derive(Debug, Clone)]
enum AttrValue {
    Flag,
    Ident(String),
    Path(String, String),
    Str(String),
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
}

const MAX_PARSE_DEPTH: usize = 128;

struct Parser {
    tokens: Vec<Tok>,
    eof: Tok,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn new(tokens: Vec<Tok>) -> Self {
        let eof = tokens
            .last()
            .filter(|t| matches!(t.kind, TokKind::Eof))
            .cloned()
            .unwrap_or(Tok {
                kind: TokKind::Eof,
                line: 1,
                col: 1,
            });
        Self {
            tokens,
            eof,
            pos: 0,
            depth: 0,
        }
    }

    fn enter_depth(&mut self) -> Result<(), CodegenError> {
        if self.depth >= MAX_PARSE_DEPTH {
            let tok = self.peek();
            return Err(CodegenError::parse(
                tok.line,
                tok.col,
                "maximum type nesting depth exceeded",
            ));
        }
        self.depth += 1;
        Ok(())
    }

    fn exit_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn peek(&self) -> &Tok {
        self.tokens.get(self.pos).unwrap_or(&self.eof)
    }

    fn peek_next(&self) -> &Tok {
        self.tokens.get(self.pos + 1).unwrap_or(&self.eof)
    }

    fn bump(&mut self) -> Tok {
        let tok = self.peek().clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, expected: &TokKind) -> bool {
        if &self.peek().kind == expected {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: &TokKind, desc: &str) -> Result<Tok, CodegenError> {
        let tok = self.peek().clone();
        if &tok.kind == expected {
            Ok(self.bump())
        } else {
            Err(CodegenError::parse(
                tok.line,
                tok.col,
                format!("expected {desc}, found {:?}", tok.kind),
            ))
        }
    }

    fn expect_ident(&mut self, desc: &str) -> Result<String, CodegenError> {
        let tok = self.peek().clone();
        match tok.kind {
            TokKind::Ident(name) => {
                self.bump();
                Ok(name)
            }
            _ => Err(CodegenError::parse(
                tok.line,
                tok.col,
                format!("expected {desc} identifier, found {:?}", tok.kind),
            )),
        }
    }

    fn is_ident(&self, kw: &str) -> bool {
        matches!(&self.peek().kind, TokKind::Ident(s) if s == kw)
    }

    fn eat_ident(&mut self, kw: &str) -> bool {
        if self.is_ident(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

}

fn attr_matches(attr_name: &str, short_name: &str) -> bool {
    attr_name == short_name || attr_name.ends_with(&format!("::{short_name}"))
}

fn attr_string_arg(attr: &ParsedAttr, key: &str) -> Option<String> {
    attr.args.iter().find_map(|(k, v)| {
        if k == key
            && let AttrValue::Str(s) = v
        {
            return Some(s.clone());
        }
        None
    })
}

fn attr_u32_arg(attr: &ParsedAttr, key: &str) -> Result<Option<u32>, CodegenError> {
    for (k, v) in &attr.args {
        if k == key {
            return match v {
                AttrValue::UInt(u) => u32::try_from(*u).map(Some).map_err(|_| {
                    CodegenError::parse(attr.line, attr.col, format!("attribute '{key}' value {u} exceeds u32::MAX"))
                }),
                other => Err(CodegenError::parse(
                    attr.line,
                    attr.col,
                    format!("expected unsigned integer for attribute '{key}', found {other:?}"),
                )),
            };
        }
    }
    Ok(None)
}

fn attr_usize_arg(attr: &ParsedAttr, key: &str) -> Result<Option<usize>, CodegenError> {
    for (k, v) in &attr.args {
        if k == key {
            return match v {
                AttrValue::UInt(u) => usize::try_from(*u).map(Some).map_err(|_| {
                    CodegenError::parse(attr.line, attr.col, format!("attribute '{key}' value {u} exceeds usize::MAX"))
                }),
                other => Err(CodegenError::parse(
                    attr.line,
                    attr.col,
                    format!("expected unsigned integer for attribute '{key}', found {other:?}"),
                )),
            };
        }
    }
    Ok(None)
}

fn apply_struct_layout_attrs(
    spec: &mut HybridStructSpec,
    attr: &ParsedAttr,
    repr_attr: Option<&ParsedAttr>,
) -> Result<(), CodegenError> {
    spec.expected_size = attr_usize_arg(attr, "size")?;
    spec.expected_size_64 = attr_usize_arg(attr, "size_64")?;
    spec.align = attr_usize_arg(attr, "align")?;
    if spec.align.is_none()
        && let Some(ra) = repr_attr
    {
        spec.align = attr_usize_arg(ra, "align")?;
    }
    Ok(())
}

fn parse_prop_default(attr: &ParsedAttr) -> Result<Option<HybridDefaultValue>, CodegenError> {
    for (k, v) in &attr.args {
        if k == "default" {
            let dv = match v {
                AttrValue::Bool(b) => HybridDefaultValue::Bool(*b),
                AttrValue::Int(i) => HybridDefaultValue::Int(*i),
                AttrValue::UInt(u) => HybridDefaultValue::UInt(*u),
                AttrValue::Float(f) => HybridDefaultValue::Float(*f),
                AttrValue::Str(s) => HybridDefaultValue::String(s.clone()),
                AttrValue::Ident(id) if id == "None" => HybridDefaultValue::None,
                AttrValue::Path(enum_name, variant) => HybridDefaultValue::EnumVariant {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                },
                _ => {
                    return Err(CodegenError::parse(
                        attr.line,
                        attr.col,
                        format!("unsupported #[prop(default = ...)] value: {v:?}"),
                    ));
                }
            };
            return Ok(Some(dv));
        }
    }
    Ok(None)
}

fn strip_spec_suffix(trait_name: &str) -> String {
    trait_name
        .strip_suffix("Spec")
        .filter(|s| !s.is_empty())
        .unwrap_or(trait_name)
        .to_string()
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
    for v in &mut file.views {
        for p in &mut v.props {
            rewrite(&mut p.ty);
        }
        for ev in &mut v.events {
            for p in &mut ev.params {
                rewrite(&mut p.ty);
            }
        }
        for method in &mut v.methods {
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

/// Parses a declarative Rust `.rs` hybrid specification source string into a validated
/// [`HybridFileSpec`].
pub fn parse_hybrid_rust_spec(source: &str) -> Result<HybridFileSpec, CodegenError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser::new(tokens);
    parser.parse_file()
}

/// Reads and parses a `.rs` hybrid specification file on disk.
pub fn parse_hybrid_rust_spec_file(path: &Path) -> Result<HybridFileSpec, CodegenError> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| CodegenError::io(path.display().to_string(), e))?;
    parse_hybrid_rust_spec(&source)
}
