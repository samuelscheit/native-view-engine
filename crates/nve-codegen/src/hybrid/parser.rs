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

#[derive(Debug, Clone, PartialEq)]
enum TokKind {
    Ident(String),
    Int(u64, bool),
    Float(f64),
    Str(String),
    DocComment(String),
    Hash,
    Bang,
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LAngle,
    RAngle,
    Comma,
    Colon,
    ColonColon,
    Semi,
    Eq,
    Arrow,
    Amp,
    Minus,
    Star,
    Dot,
    Eof,
}

#[derive(Debug, Clone)]
struct Tok {
    kind: TokKind,
    line: usize,
    col: usize,
}

fn tokenize(src: &str) -> Result<Vec<Tok>, CodegenError> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;

    let advance = |i: &mut usize, line: &mut usize, col: &mut usize| -> Option<char> {
        let c = chars.get(*i).copied()?;
        *i += 1;
        if c == '\n' {
            *line += 1;
            *col = 1;
        } else {
            *col += 1;
        }
        Some(c)
    };

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let next2 = chars.get(i + 2).copied();

        if c.is_whitespace() {
            advance(&mut i, &mut line, &mut col);
            continue;
        }

        // Line comments & doc comments (`///` or `//!`)
        if c == '/' && next == Some('/') {
            let start_line = line;
            let start_col = col;
            let is_doc = next2 == Some('/') && chars.get(i + 3).copied() != Some('/');
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            if is_doc {
                advance(&mut i, &mut line, &mut col);
            }
            let mut text = String::new();
            while i < chars.len() && chars[i] != '\n' {
                if let Some(ch) = advance(&mut i, &mut line, &mut col) {
                    text.push(ch);
                }
            }
            if is_doc {
                tokens.push(Tok {
                    kind: TokKind::DocComment(text.trim().to_string()),
                    line: start_line,
                    col: start_col,
                });
            }
            continue;
        }

        // Block comments (supports nesting per Rust lexical rules)
        if c == '/' && next == Some('*') {
            let start_line = line;
            let start_col = col;
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            let mut depth = 1usize;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1).copied() == Some('*') {
                    depth += 1;
                    advance(&mut i, &mut line, &mut col);
                    advance(&mut i, &mut line, &mut col);
                } else if chars[i] == '*' && chars.get(i + 1).copied() == Some('/') {
                    depth -= 1;
                    advance(&mut i, &mut line, &mut col);
                    advance(&mut i, &mut line, &mut col);
                } else {
                    advance(&mut i, &mut line, &mut col);
                }
            }
            if depth > 0 {
                return Err(CodegenError::parse(
                    start_line,
                    start_col,
                    "unterminated block comment",
                ));
            }
            continue;
        }

        let start_line = line;
        let start_col = col;

        // String literal
        if c == '"' {
            advance(&mut i, &mut line, &mut col);
            let mut out = String::new();
            let mut closed = false;
            while let Some(ch) = advance(&mut i, &mut line, &mut col) {
                if ch == '"' {
                    closed = true;
                    break;
                }
                if ch == '\\' {
                    let esc = advance(&mut i, &mut line, &mut col).ok_or_else(|| {
                        CodegenError::parse(start_line, start_col, "unterminated string escape")
                    })?;
                    match esc {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        '\\' => out.push('\\'),
                        '"' => out.push('"'),
                        '0' => out.push('\0'),
                        other => {
                            return Err(CodegenError::parse(
                                line,
                                col,
                                format!("unsupported string escape '\\{other}'"),
                            ));
                        }
                    }
                } else {
                    out.push(ch);
                }
            }
            if !closed {
                return Err(CodegenError::parse(
                    start_line,
                    start_col,
                    "unterminated string literal",
                ));
            }
            tokens.push(Tok {
                kind: TokKind::Str(out),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Lifetimes (`'a`, `'static`) — skip `'ident` smoothly for references like `&'a str`
        if c == '\'' {
            advance(&mut i, &mut line, &mut col);
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                advance(&mut i, &mut line, &mut col);
            }
            continue;
        }

        // Numbers (decimal / hex / underscores / float)
        if c.is_ascii_digit() {
            let mut raw = String::new();
            if c == '0' && matches!(next, Some('x') | Some('X')) {
                advance(&mut i, &mut line, &mut col);
                advance(&mut i, &mut line, &mut col);
                while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
                    if let Some(ch) = advance(&mut i, &mut line, &mut col)
                        && ch != '_'
                    {
                        raw.push(ch);
                    }
                }
                let val = u64::from_str_radix(&raw, 16).map_err(|_| {
                    CodegenError::parse(
                        start_line,
                        start_col,
                        format!("invalid hex integer literal '0x{raw}'"),
                    )
                })?;
                tokens.push(Tok {
                    kind: TokKind::Int(val, true),
                    line: start_line,
                    col: start_col,
                });
                continue;
            }
            let mut is_float = false;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                if let Some(ch) = advance(&mut i, &mut line, &mut col)
                    && ch != '_'
                {
                    raw.push(ch);
                }
            }
            if i < chars.len()
                && chars[i] == '.'
                && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())
            {
                is_float = true;
                if let Some(dot) = advance(&mut i, &mut line, &mut col) {
                    raw.push(dot);
                }
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    if let Some(ch) = advance(&mut i, &mut line, &mut col)
                        && ch != '_'
                    {
                        raw.push(ch);
                    }
                }
            }
            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                is_float = true;
                if let Some(exp) = advance(&mut i, &mut line, &mut col) {
                    raw.push(exp);
                }
                if i < chars.len() && matches!(chars[i], '+' | '-')
                    && let Some(sign) = advance(&mut i, &mut line, &mut col)
                {
                    raw.push(sign);
                }
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    if let Some(ch) = advance(&mut i, &mut line, &mut col)
                        && ch != '_'
                    {
                        raw.push(ch);
                    }
                }
            }
            // Skip optional Rust numeric suffix (`u32`, `i64`, `f32`, `f64`, etc.)
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                advance(&mut i, &mut line, &mut col);
            }
            if is_float {
                let val = raw.parse::<f64>().map_err(|_| {
                    CodegenError::parse(
                        start_line,
                        start_col,
                        format!("invalid float literal '{raw}'"),
                    )
                })?;
                tokens.push(Tok {
                    kind: TokKind::Float(val),
                    line: start_line,
                    col: start_col,
                });
            } else {
                let val = raw.parse::<u64>().map_err(|_| {
                    CodegenError::parse(
                        start_line,
                        start_col,
                        format!("invalid integer literal '{raw}'"),
                    )
                })?;
                tokens.push(Tok {
                    kind: TokKind::Int(val, false),
                    line: start_line,
                    col: start_col,
                });
            }
            continue;
        }

        // Identifiers
        if c.is_ascii_alphabetic() || c == '_' {
            let mut ident = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                if let Some(ch) = advance(&mut i, &mut line, &mut col) {
                    ident.push(ch);
                }
            }
            tokens.push(Tok {
                kind: TokKind::Ident(ident),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Two-character punctuation
        if c == ':' && next == Some(':') {
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            tokens.push(Tok {
                kind: TokKind::ColonColon,
                line: start_line,
                col: start_col,
            });
            continue;
        }
        if c == '-' && next == Some('>') {
            advance(&mut i, &mut line, &mut col);
            advance(&mut i, &mut line, &mut col);
            tokens.push(Tok {
                kind: TokKind::Arrow,
                line: start_line,
                col: start_col,
            });
            continue;
        }

        // Single-character punctuation
        advance(&mut i, &mut line, &mut col);
        let kind = match c {
            '#' => TokKind::Hash,
            '!' => TokKind::Bang,
            '[' => TokKind::LBracket,
            ']' => TokKind::RBracket,
            '(' => TokKind::LParen,
            ')' => TokKind::RParen,
            '{' => TokKind::LBrace,
            '}' => TokKind::RBrace,
            '<' => TokKind::LAngle,
            '>' => TokKind::RAngle,
            ',' => TokKind::Comma,
            ':' => TokKind::Colon,
            ';' => TokKind::Semi,
            '=' => TokKind::Eq,
            '&' => TokKind::Amp,
            '-' => TokKind::Minus,
            '*' => TokKind::Star,
            '.' => TokKind::Dot,
            other => {
                return Err(CodegenError::parse(
                    start_line,
                    start_col,
                    format!("unexpected character '{other}' in hybrid spec"),
                ));
            }
        };
        tokens.push(Tok {
            kind,
            line: start_line,
            col: start_col,
        });
    }

    tokens.push(Tok {
        kind: TokKind::Eof,
        line,
        col,
    });
    Ok(tokens)
}

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

    /// Consumes doc comments and `#[...]` attributes before an item or member.
    fn parse_docs_and_attrs(&mut self) -> Result<(Option<String>, Vec<ParsedAttr>), CodegenError> {
        let mut doc_lines = Vec::new();
        let mut attrs = Vec::new();
        loop {
            match &self.peek().kind {
                TokKind::DocComment(line) => {
                    doc_lines.push(line.clone());
                    self.bump();
                }
                TokKind::Hash => {
                    let hash_tok = self.bump();
                    let is_inner = self.eat(&TokKind::Bang);
                    self.expect(&TokKind::LBracket, "'['")?;
                    let attr = self.parse_attr_body(hash_tok.line, hash_tok.col)?;
                    self.expect(&TokKind::RBracket, "']'")?;
                    if !is_inner {
                        attrs.push(attr);
                    }
                }
                _ => break,
            }
        }
        let doc = if doc_lines.is_empty() {
            None
        } else {
            Some(doc_lines.join("\n"))
        };
        Ok((doc, attrs))
    }

    fn parse_attr_body(&mut self, line: usize, col: usize) -> Result<ParsedAttr, CodegenError> {
        // Parse attribute path (e.g. `nve_hybrid_module`, `nve::hybrid_module`, `repr`, `prop`)
        let mut name = self.expect_ident("attribute")?;
        while self.eat(&TokKind::ColonColon) {
            let seg = self.expect_ident("attribute path segment")?;
            name.push_str("::");
            name.push_str(&seg);
        }

        let mut args = Vec::new();
        if self.eat(&TokKind::LParen) {
            while !matches!(self.peek().kind, TokKind::RParen | TokKind::Eof) {
                if let TokKind::Int(u, _) = self.peek().kind {
                    self.bump();
                    args.push(("value".to_string(), AttrValue::UInt(u)));
                } else {
                    let key = self.expect_ident("attribute argument")?;
                    if self.eat(&TokKind::Eq) {
                        let val = self.parse_attr_value()?;
                        args.push((key, val));
                    } else if self.eat(&TokKind::LParen) {
                        if let TokKind::Int(u, _) = self.peek().kind {
                            self.bump();
                            self.expect(&TokKind::RParen, "')'")?;
                            args.push((key, AttrValue::UInt(u)));
                        } else {
                            // Skip nested attribute parens like `cfg(not(test))`
                            let mut depth = 1usize;
                            while depth > 0 && self.peek().kind != TokKind::Eof {
                                match self.bump().kind {
                                    TokKind::LParen => depth += 1,
                                    TokKind::RParen => depth -= 1,
                                    _ => {}
                                }
                            }
                            args.push((key, AttrValue::Flag));
                        }
                    } else {
                        args.push((key, AttrValue::Flag));
                    }
                }
                if !self.eat(&TokKind::Comma) {
                    break;
                }
            }
            self.expect(&TokKind::RParen, "')'")?;
        } else if self.eat(&TokKind::Eq) {
            // Skip `#[doc = "..."]` or similar key=value attributes
            let val = self.parse_attr_value()?;
            args.push(("value".to_string(), val));
        }

        Ok(ParsedAttr {
            name,
            args,
            line,
            col,
        })
    }

    fn parse_attr_value(&mut self) -> Result<AttrValue, CodegenError> {
        let tok = self.peek().clone();
        match tok.kind {
            TokKind::Str(s) => {
                self.bump();
                Ok(AttrValue::Str(s))
            }
            TokKind::Minus => {
                self.bump();
                let next = self.bump();
                match next.kind {
                    TokKind::Int(u, _) => {
                        let neg = if u == (i64::MAX as u64) + 1 {
                            i64::MIN
                        } else {
                            let signed = i64::try_from(u).map_err(|_| {
                                CodegenError::parse(
                                    next.line,
                                    next.col,
                                    "integer literal out of i64 range",
                                )
                            })?;
                            signed.checked_neg().ok_or_else(|| {
                                CodegenError::parse(
                                    next.line,
                                    next.col,
                                    "integer literal out of i64 range",
                                )
                            })?
                        };
                        Ok(AttrValue::Int(neg))
                    }
                    TokKind::Float(f) => Ok(AttrValue::Float(-f)),
                    _ => Err(CodegenError::parse(
                        next.line,
                        next.col,
                        "expected numeric literal after '-'",
                    )),
                }
            }
            TokKind::Int(u, _) => {
                self.bump();
                Ok(AttrValue::UInt(u))
            }
            TokKind::Float(f) => {
                self.bump();
                Ok(AttrValue::Float(f))
            }
            TokKind::Ident(ident) => {
                self.bump();
                if ident == "true" {
                    Ok(AttrValue::Bool(true))
                } else if ident == "false" {
                    Ok(AttrValue::Bool(false))
                } else if self.eat(&TokKind::ColonColon) {
                    let variant = self.expect_ident("enum variant")?;
                    Ok(AttrValue::Path(ident, variant))
                } else {
                    Ok(AttrValue::Ident(ident))
                }
            }
            _ => Err(CodegenError::parse(
                tok.line,
                tok.col,
                format!("unexpected attribute value {:?}", tok.kind),
            )),
        }
    }

    fn skip_visibility(&mut self) -> Result<(), CodegenError> {
        if self.eat_ident("pub") && self.eat(&TokKind::LParen) {
            let mut depth = 1usize;
            while depth > 0 && self.peek().kind != TokKind::Eof {
                match self.bump().kind {
                    TokKind::LParen => depth += 1,
                    TokKind::RParen => depth -= 1,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn skip_use_stmt(&mut self) -> Result<(), CodegenError> {
        while !matches!(self.peek().kind, TokKind::Semi | TokKind::Eof) {
            self.bump();
        }
        self.expect(&TokKind::Semi, "';' after use statement")?;
        Ok(())
    }

    fn parse_file(&mut self) -> Result<HybridFileSpec, CodegenError> {
        let mut file = HybridFileSpec::default();

        loop {
            let (doc, attrs) = self.parse_docs_and_attrs()?;
            if self.peek().kind == TokKind::Eof {
                break;
            }

            // Optional file-level namespace via `#[nve_hybrid_namespace(name = "...")]` or module attr
            for attr in &attrs {
                if attr_matches(&attr.name, "nve_hybrid_namespace")
                    && let Some(ns) = attr_string_arg(attr, "name")
                {
                    file.namespace = Some(ns);
                }
            }

            self.skip_visibility()?;

            if self.eat_ident("use") {
                self.skip_use_stmt()?;
                continue;
            }

            if self.eat_ident("const") {
                let is_hybrid = attrs
                    .iter()
                    .any(|a| attr_matches(&a.name, "nve_hybrid_const"));
                let const_spec = self.parse_const(doc)?;
                if is_hybrid {
                    file.consts.push(const_spec);
                }
                continue;
            }

            if self.eat_ident("type") {
                let is_hybrid = attrs
                    .iter()
                    .any(|a| attr_matches(&a.name, "nve_hybrid_alias"));
                let alias_spec = self.parse_alias(doc)?;
                if is_hybrid {
                    file.aliases.push(alias_spec);
                }
                continue;
            }

            if self.eat_ident("extern") {
                let is_hybrid = attrs
                    .iter()
                    .any(|a| attr_matches(&a.name, "nve_hybrid_c_fn"));
                // Optional ABI string `"C"`
                if matches!(self.peek().kind, TokKind::Str(_)) {
                    self.bump();
                }
                let mut fn_spec = self.parse_fn_signature(doc)?;
                fn_spec.has_self = false;
                if is_hybrid {
                    file.c_functions.push(fn_spec);
                }
                continue;
            }

            if self.eat_ident("enum") {
                let enum_attr = attrs.iter().find(|a| attr_matches(&a.name, "nve_hybrid_enum"));
                let repr_attr = attrs.iter().find(|a| attr_matches(&a.name, "repr"));
                let mut enum_spec = self.parse_enum(doc)?;
                if let Some(attr) = enum_attr {
                    enum_spec.c_prefix = attr_string_arg(attr, "c_prefix");
                    enum_spec.dart_fallback = attr_string_arg(attr, "dart_fallback");
                    if let Some(ra) = repr_attr {
                        enum_spec.repr = ra.args.iter().find_map(|(k, v)| {
                            if matches!(v, AttrValue::Flag)
                                && matches!(k.as_str(), "u8" | "u16" | "u32" | "i32")
                            {
                                Some(k.clone())
                            } else {
                                None
                            }
                        });
                    }
                    file.enums.push(enum_spec);
                }
                continue;
            }

            if self.eat_ident("struct") {
                let struct_attr = attrs
                    .iter()
                    .find(|a| attr_matches(&a.name, "nve_hybrid_struct"));
                let repr_attr = attrs.iter().find(|a| attr_matches(&a.name, "repr"));
                let mut struct_spec = self.parse_struct(doc, false)?;
                if let Some(attr) = struct_attr {
                    apply_struct_layout_attrs(&mut struct_spec, attr, repr_attr)?;
                    file.structs.push(struct_spec);
                }
                continue;
            }

            if self.eat_ident("union") {
                let union_attr = attrs
                    .iter()
                    .find(|a| attr_matches(&a.name, "nve_hybrid_union"));
                let repr_attr = attrs.iter().find(|a| attr_matches(&a.name, "repr"));
                let mut union_spec = self.parse_struct(doc, true)?;
                if let Some(attr) = union_attr {
                    apply_struct_layout_attrs(&mut union_spec, attr, repr_attr)?;
                    file.structs.push(union_spec);
                }
                continue;
            }

            if self.eat_ident("trait") {
                let mod_attr = attrs
                    .iter()
                    .find(|a| attr_matches(&a.name, "nve_hybrid_module"));
                let view_attr = attrs
                    .iter()
                    .find(|a| attr_matches(&a.name, "nve_hybrid_view"));
                if let (Some(a), Some(_)) = (mod_attr, view_attr) {
                    return Err(CodegenError::parse(
                        a.line,
                        a.col,
                        "trait cannot carry both #[nve_hybrid_module] and #[nve_hybrid_view]",
                    ));
                }
                if let Some(attr) = mod_attr {
                    let explicit_name = attr_string_arg(attr, "name");
                    let explicit_interface_id = attr_u32_arg(attr, "interface_id")?;
                    let c_name = attr_string_arg(attr, "c_name");
                    file.modules.push(self.parse_module_trait(
                        explicit_name,
                        explicit_interface_id,
                        c_name,
                        doc,
                    )?);
                } else if let Some(attr) = view_attr {
                    let explicit_name = attr_string_arg(attr, "name");
                    let explicit_interface_id = attr_u32_arg(attr, "interface_id")?;
                    file.views.push(self.parse_view_trait(
                        explicit_name,
                        explicit_interface_id,
                        doc,
                    )?);
                } else {
                    let tok = self.peek().clone();
                    return Err(CodegenError::parse(
                        tok.line,
                        tok.col,
                        "trait in hybrid spec must be annotated with #[nve_hybrid_module] or #[nve_hybrid_view]",
                    ));
                }
                continue;
            }

            let tok = self.peek().clone();
            return Err(CodegenError::parse(
                tok.line,
                tok.col,
                format!(
                    "unexpected token {:?} at top level (expected enum, struct, union, trait, const, type, extern, or use)",
                    tok.kind
                ),
            ));
        }

        // Resolve named type references against declared enums and handle aliases.
        resolve_named_types(&mut file);
        file.validate()?;
        Ok(file)
    }

    fn parse_const(&mut self, doc: Option<String>) -> Result<HybridConstSpec, CodegenError> {
        let name = self.expect_ident("const")?;
        self.expect(&TokKind::Colon, "':'")?;
        let ty = self.parse_type()?;
        self.expect(&TokKind::Eq, "'='")?;
        let tok = self.bump();
        let (value, hex) = match tok.kind {
            TokKind::Int(v, hex) => (v, hex),
            _ => {
                return Err(CodegenError::parse(
                    tok.line,
                    tok.col,
                    "expected integer literal for const value",
                ));
            }
        };
        self.expect(&TokKind::Semi, "';'")?;
        Ok(HybridConstSpec {
            name,
            ty,
            value,
            hex,
            doc,
        })
    }

    fn parse_alias(&mut self, doc: Option<String>) -> Result<HybridAliasSpec, CodegenError> {
        let name = self.expect_ident("type alias")?;
        self.expect(&TokKind::Eq, "'='")?;
        let target = self.parse_type()?;
        self.expect(&TokKind::Semi, "';'")?;
        Ok(HybridAliasSpec { name, target, doc })
    }

    fn parse_enum(&mut self, doc: Option<String>) -> Result<HybridEnumSpec, CodegenError> {
        let name = self.expect_ident("enum")?;
        self.expect(&TokKind::LBrace, "'{'")?;
        let mut variants = Vec::new();
        let mut next_discriminant: u32 = 0;

        while !matches!(self.peek().kind, TokKind::RBrace | TokKind::Eof) {
            let (vdoc, _vattrs) = self.parse_docs_and_attrs()?;
            if self.peek().kind == TokKind::RBrace {
                break;
            }
            let vname = self.expect_ident("enum variant")?;
            let value = if self.eat(&TokKind::Eq) {
                let tok = self.bump();
                match tok.kind {
                    TokKind::Int(v, _) => u32::try_from(v).map_err(|_| {
                        CodegenError::parse(
                            tok.line,
                            tok.col,
                            format!("enum discriminant {v} exceeds u32::MAX"),
                        )
                    })?,
                    _ => {
                        return Err(CodegenError::parse(
                            tok.line,
                            tok.col,
                            "expected unsigned integer discriminant after '='",
                        ));
                    }
                }
            } else {
                next_discriminant
            };
            next_discriminant = value.checked_add(1).ok_or_else(|| {
                CodegenError::validation(format!("enum '{name}' discriminant overflow"))
            })?;
            variants.push(HybridEnumVariant {
                name: vname,
                value,
                doc: vdoc,
            });
            if !self.eat(&TokKind::Comma) {
                break;
            }
        }
        self.expect(&TokKind::RBrace, "'}'")?;
        Ok(HybridEnumSpec {
            name,
            variants,
            doc,
            c_prefix: None,
            repr: None,
            dart_fallback: None,
        })
    }

    fn parse_struct(
        &mut self,
        doc: Option<String>,
        is_union: bool,
    ) -> Result<HybridStructSpec, CodegenError> {
        let name = self.expect_ident(if is_union { "union" } else { "struct" })?;
        self.expect(&TokKind::LBrace, "'{'")?;
        let mut fields = Vec::new();

        while !matches!(self.peek().kind, TokKind::RBrace | TokKind::Eof) {
            let (fdoc, fattrs) = self.parse_docs_and_attrs()?;
            if self.peek().kind == TokKind::RBrace {
                break;
            }
            self.skip_visibility()?;
            let fname = self.expect_ident("struct field")?;
            self.expect(&TokKind::Colon, "':'")?;
            let ty = self.parse_type()?;
            let mut offset = None;
            let mut offset_64 = None;
            for a in &fattrs {
                if attr_matches(&a.name, "offset") {
                    offset = attr_usize_arg(a, "value")?;
                } else if attr_matches(&a.name, "offset64") {
                    offset_64 = attr_usize_arg(a, "value")?;
                }
            }
            fields.push(HybridFieldSpec {
                name: fname,
                ty,
                doc: fdoc,
                offset,
                offset_64,
            });
            if !self.eat(&TokKind::Comma) {
                break;
            }
        }
        self.expect(&TokKind::RBrace, "'}'")?;
        Ok(HybridStructSpec {
            name,
            fields,
            doc,
            is_union,
            align: None,
            expected_size: None,
            expected_size_64: None,
        })
    }

    fn parse_module_trait(
        &mut self,
        explicit_name: Option<String>,
        explicit_interface_id: Option<u32>,
        c_name: Option<String>,
        doc: Option<String>,
    ) -> Result<HybridModuleSpec, CodegenError> {
        let trait_name = self.expect_ident("trait")?;
        let name = explicit_name.unwrap_or_else(|| strip_spec_suffix(&trait_name));
        self.expect(&TokKind::LBrace, "'{'")?;
        let mut methods = Vec::new();

        while !matches!(self.peek().kind, TokKind::RBrace | TokKind::Eof) {
            let (mdoc, mattrs) = self.parse_docs_and_attrs()?;
            if self.peek().kind == TokKind::RBrace {
                break;
            }
            let mut optional = false;
            for attr in &mattrs {
                if attr_matches(&attr.name, "prop") || attr_matches(&attr.name, "event") {
                    return Err(CodegenError::parse(
                        attr.line,
                        attr.col,
                        format!(
                            "#[{}] is only valid inside #[nve_hybrid_view], not #[nve_hybrid_module]",
                            attr.name
                        ),
                    ));
                }
                if attr_matches(&attr.name, "optional") {
                    optional = true;
                }
            }
            let mut sig = self.parse_fn_signature(mdoc)?;
            sig.optional = optional;
            methods.push(sig);
        }
        self.expect(&TokKind::RBrace, "'}'")?;

        Ok(HybridModuleSpec {
            name,
            trait_name,
            methods,
            doc,
            explicit_interface_id,
            c_name,
        })
    }

    fn parse_view_trait(
        &mut self,
        explicit_name: Option<String>,
        explicit_interface_id: Option<u32>,
        doc: Option<String>,
    ) -> Result<HybridViewSpec, CodegenError> {
        let trait_name = self.expect_ident("trait")?;
        let name = explicit_name.unwrap_or_else(|| strip_spec_suffix(&trait_name));
        self.expect(&TokKind::LBrace, "'{'")?;

        let mut props = Vec::new();
        let mut events = Vec::new();
        let mut methods = Vec::new();

        while !matches!(self.peek().kind, TokKind::RBrace | TokKind::Eof) {
            let (mdoc, mattrs) = self.parse_docs_and_attrs()?;
            if self.peek().kind == TokKind::RBrace {
                break;
            }
            let prop_attr = mattrs.iter().find(|a| attr_matches(&a.name, "prop"));
            let event_attr = mattrs.iter().find(|a| attr_matches(&a.name, "event"));
            let method_attr = mattrs.iter().find(|a| attr_matches(&a.name, "method"));

            let role_count = usize::from(prop_attr.is_some())
                + usize::from(event_attr.is_some())
                + usize::from(method_attr.is_some());
            if role_count > 1 {
                let first = &mattrs[0];
                return Err(CodegenError::parse(
                    first.line,
                    first.col,
                    "view member cannot combine #[prop], #[event], and #[method]",
                ));
            }

            let sig = self.parse_fn_signature(mdoc)?;
            if let Some(attr) = prop_attr {
                if !sig.params.is_empty() {
                    return Err(CodegenError::parse(
                        attr.line,
                        attr.col,
                        format!(
                            "#[prop] '{}' must take only &self (no parameters)",
                            sig.name
                        ),
                    ));
                }
                let default_value = parse_prop_default(attr)?;
                props.push(HybridPropSpec {
                    name: sig.name,
                    ty: sig.return_type,
                    default_value,
                    doc: sig.doc,
                });
            } else if let Some(attr) = event_attr {
                if sig.return_type != HybridType::Void {
                    return Err(CodegenError::parse(
                        attr.line,
                        attr.col,
                        format!("#[event] '{}' must have void return type", sig.name),
                    ));
                }
                events.push(HybridEventSpec {
                    name: sig.name,
                    params: sig.params,
                    doc: sig.doc,
                });
            } else {
                methods.push(sig);
            }
        }
        self.expect(&TokKind::RBrace, "'}'")?;

        Ok(HybridViewSpec {
            name,
            trait_name,
            props,
            events,
            methods,
            doc,
            explicit_interface_id,
        })
    }

    fn parse_fn_signature(&mut self, doc: Option<String>) -> Result<HybridMethodSpec, CodegenError> {
        let fn_tok = self.peek().clone();
        if !self.eat_ident("fn") {
            return Err(CodegenError::parse(
                fn_tok.line,
                fn_tok.col,
                format!("expected 'fn' inside trait, found {:?}", fn_tok.kind),
            ));
        }
        let name = self.expect_ident("method")?;
        self.expect(&TokKind::LParen, "'('")?;

        let mut params = Vec::new();
        let mut has_self = false;
        while !matches!(self.peek().kind, TokKind::RParen | TokKind::Eof) {
            // Handle `&self`, `&mut self`, `self`, `mut self`
            if self.peek().kind == TokKind::Amp {
                let is_self_ref = matches!(
                    &self.peek_next().kind,
                    TokKind::Ident(s) if s == "self" || s == "mut"
                );
                if is_self_ref {
                    self.bump(); // '&'
                    self.eat_ident("mut");
                    if !self.eat_ident("self") {
                        let tok = self.peek().clone();
                        return Err(CodegenError::parse(
                            tok.line,
                            tok.col,
                            "expected 'self' receiver",
                        ));
                    }
                    has_self = true;
                    if !self.eat(&TokKind::Comma) {
                        break;
                    }
                    continue;
                }
            }
            if self.eat_ident("self") {
                has_self = true;
                if !self.eat(&TokKind::Comma) {
                    break;
                }
                continue;
            }
            if self.is_ident("mut") && matches!(&self.peek_next().kind, TokKind::Ident(s) if s == "self") {
                self.bump();
                self.bump();
                has_self = true;
                if !self.eat(&TokKind::Comma) {
                    break;
                }
                continue;
            }

            let pname = self.expect_ident("parameter")?;
            self.expect(&TokKind::Colon, "':'")?;
            let ty = self.parse_type()?;
            params.push(HybridParamSpec { name: pname, ty });
            if !self.eat(&TokKind::Comma) {
                break;
            }
        }
        self.expect(&TokKind::RParen, "')'")?;

        let return_type = if self.eat(&TokKind::Arrow) {
            self.parse_type()?
        } else {
            HybridType::Void
        };
        self.expect(&TokKind::Semi, "';'")?;

        Ok(HybridMethodSpec {
            name,
            params,
            return_type,
            doc,
            has_self,
            optional: false,
        })
    }

    fn parse_type(&mut self) -> Result<HybridType, CodegenError> {
        self.enter_depth()?;
        let res = self.parse_type_inner();
        self.exit_depth();
        res
    }

    fn parse_type_inner(&mut self) -> Result<HybridType, CodegenError> {
        let tok = self.peek().clone();
        // Unit `()` -> Void
        if self.eat(&TokKind::LParen) {
            self.expect(&TokKind::RParen, "')'")?;
            return Ok(HybridType::Void);
        }

        // Raw pointers: `*const T`, `*mut T`
        if self.eat(&TokKind::Star) {
            if self.eat_ident("const") {
                let inner = self.parse_type()?;
                return Ok(HybridType::ConstPtr(Box::new(inner)));
            } else if self.eat_ident("mut") {
                let inner = self.parse_type()?;
                return Ok(HybridType::MutPtr(Box::new(inner)));
            } else {
                let ptok = self.peek().clone();
                return Err(CodegenError::parse(
                    ptok.line,
                    ptok.col,
                    "expected 'const' or 'mut' after '*'",
                ));
            }
        }

        // Reference types: `&str`, `&[u8]`, `&[T]`, `&T`
        if self.eat(&TokKind::Amp) {
            self.eat_ident("mut");
            if self.eat(&TokKind::LBracket) {
                let elem = self.parse_type()?;
                self.expect(&TokKind::RBracket, "']'")?;
                return match elem {
                    HybridType::U8 => Ok(HybridType::Bytes),
                    other => Ok(HybridType::Vec(Box::new(other))),
                };
            }
            let inner = self.parse_type()?;
            return Ok(inner);
        }

        // Slice without `&` is invalid in Rust signatures; parse path identifier
        let mut path = vec![self.expect_ident("type")?];
        while self.eat(&TokKind::ColonColon) {
            path.push(self.expect_ident("type path segment")?);
        }
        let last = path
            .last()
            .ok_or_else(|| CodegenError::parse(tok.line, tok.col, "empty type path"))?
            .as_str();

        match last {
            "c_void" => Ok(HybridType::Void),
            "bool" => Ok(HybridType::Bool),
            "u8" => Ok(HybridType::U8),
            "u16" => Ok(HybridType::U16),
            "usize" => Ok(HybridType::Usize),
            "i32" => Ok(HybridType::I32),
            "u32" => Ok(HybridType::U32),
            "i64" => Ok(HybridType::I64),
            "u64" => Ok(HybridType::U64),
            "f32" => Ok(HybridType::F32),
            "f64" => Ok(HybridType::F64),
            "String" | "str" => Ok(HybridType::String),
            "NveValue" => Ok(HybridType::Value),
            "Option" => {
                self.expect(&TokKind::LAngle, "'<'")?;
                let inner = self.parse_type()?;
                self.expect(&TokKind::RAngle, "'>'")?;
                Ok(HybridType::Optional(Box::new(inner)))
            }
            "Vec" => {
                self.expect(&TokKind::LAngle, "'<'")?;
                let inner = self.parse_type()?;
                self.expect(&TokKind::RAngle, "'>'")?;
                match inner {
                    HybridType::U8 => Ok(HybridType::Bytes),
                    other => Ok(HybridType::Vec(Box::new(other))),
                }
            }
            "Result" => {
                self.expect(&TokKind::LAngle, "'<'")?;
                let ok = self.parse_type()?;
                self.expect(&TokKind::Comma, "','")?;
                let err = self.parse_type()?;
                self.expect(&TokKind::RAngle, "'>'")?;
                Ok(HybridType::Result {
                    ok: Box::new(ok),
                    err: Box::new(err),
                })
            }
            _ => {
                if self.peek().kind == TokKind::LAngle {
                    return Err(CodegenError::parse(
                        tok.line,
                        tok.col,
                        format!("unsupported generic type '{last}<...>' in hybrid spec"),
                    ));
                }
                Ok(HybridType::Struct(last.to_string()))
            }
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
        | HybridType::MutPtr(inner) => {
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
