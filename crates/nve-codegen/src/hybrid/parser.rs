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
    HybridDefaultValue, HybridEnumSpec, HybridEnumVariant, HybridEventSpec, HybridFieldSpec,
    HybridFileSpec, HybridMethodSpec, HybridModuleSpec, HybridParamSpec, HybridPropSpec,
    HybridStructSpec, HybridType, HybridViewSpec,
};

#[derive(Debug, Clone, PartialEq)]
enum TokKind {
    Ident(String),
    Int(u64),
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
                text.push(advance(&mut i, &mut line, &mut col).unwrap());
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
                    let ch = advance(&mut i, &mut line, &mut col).unwrap();
                    if ch != '_' {
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
                    kind: TokKind::Int(val),
                    line: start_line,
                    col: start_col,
                });
                continue;
            }
            let mut is_float = false;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                let ch = advance(&mut i, &mut line, &mut col).unwrap();
                if ch != '_' {
                    raw.push(ch);
                }
            }
            if i < chars.len()
                && chars[i] == '.'
                && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())
            {
                is_float = true;
                raw.push(advance(&mut i, &mut line, &mut col).unwrap());
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    let ch = advance(&mut i, &mut line, &mut col).unwrap();
                    if ch != '_' {
                        raw.push(ch);
                    }
                }
            }
            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                is_float = true;
                raw.push(advance(&mut i, &mut line, &mut col).unwrap());
                if i < chars.len() && matches!(chars[i], '+' | '-') {
                    raw.push(advance(&mut i, &mut line, &mut col).unwrap());
                }
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    let ch = advance(&mut i, &mut line, &mut col).unwrap();
                    if ch != '_' {
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
                    kind: TokKind::Int(val),
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
                ident.push(advance(&mut i, &mut line, &mut col).unwrap());
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

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Tok>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Tok {
        self.tokens.get(self.pos).unwrap_or(self.tokens.last().unwrap())
    }

    fn peek_next(&self) -> &Tok {
        self.tokens
            .get(self.pos + 1)
            .unwrap_or(self.tokens.last().unwrap())
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
                let key = self.expect_ident("attribute argument")?;
                if self.eat(&TokKind::Eq) {
                    let val = self.parse_attr_value()?;
                    args.push((key, val));
                } else if self.eat(&TokKind::LParen) {
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
                } else {
                    args.push((key, AttrValue::Flag));
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
                    TokKind::Int(u) => {
                        let signed = i64::try_from(u).map_err(|_| {
                            CodegenError::parse(next.line, next.col, "integer literal out of i64 range")
                        })?;
                        Ok(AttrValue::Int(-signed))
                    }
                    TokKind::Float(f) => Ok(AttrValue::Float(-f)),
                    _ => Err(CodegenError::parse(
                        next.line,
                        next.col,
                        "expected numeric literal after '-'",
                    )),
                }
            }
            TokKind::Int(u) => {
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
                if attr_matches(&attr.name, "nve_hybrid_namespace") {
                    if let Some(ns) = attr_string_arg(attr, "name") {
                        file.namespace = Some(ns);
                    }
                }
            }

            self.skip_visibility()?;

            if self.eat_ident("use") {
                self.skip_use_stmt()?;
                continue;
            }

            if self.eat_ident("enum") {
                let is_hybrid = attrs.iter().any(|a| attr_matches(&a.name, "nve_hybrid_enum"));
                let enum_spec = self.parse_enum(doc)?;
                if is_hybrid {
                    file.enums.push(enum_spec);
                }
                continue;
            }

            if self.eat_ident("struct") {
                let is_hybrid = attrs
                    .iter()
                    .any(|a| attr_matches(&a.name, "nve_hybrid_struct"));
                let struct_spec = self.parse_struct(doc)?;
                if is_hybrid {
                    file.structs.push(struct_spec);
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
                if mod_attr.is_some() && view_attr.is_some() {
                    let a = mod_attr.unwrap();
                    return Err(CodegenError::parse(
                        a.line,
                        a.col,
                        "trait cannot carry both #[nve_hybrid_module] and #[nve_hybrid_view]",
                    ));
                }
                if let Some(attr) = mod_attr {
                    let explicit_name = attr_string_arg(attr, "name");
                    file.modules
                        .push(self.parse_module_trait(explicit_name, doc)?);
                } else if let Some(attr) = view_attr {
                    let explicit_name = attr_string_arg(attr, "name");
                    file.views.push(self.parse_view_trait(explicit_name, doc)?);
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
                    "unexpected token {:?} at top level (expected enum, struct, trait, or use)",
                    tok.kind
                ),
            ));
        }

        // Resolve named type references against declared enums (parsed initially as Struct(name)).
        resolve_enum_types(&mut file);
        file.validate()?;
        Ok(file)
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
                    TokKind::Int(v) => u32::try_from(v).map_err(|_| {
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
        })
    }

    fn parse_struct(&mut self, doc: Option<String>) -> Result<HybridStructSpec, CodegenError> {
        let name = self.expect_ident("struct")?;
        self.expect(&TokKind::LBrace, "'{'")?;
        let mut fields = Vec::new();

        while !matches!(self.peek().kind, TokKind::RBrace | TokKind::Eof) {
            let (fdoc, _fattrs) = self.parse_docs_and_attrs()?;
            if self.peek().kind == TokKind::RBrace {
                break;
            }
            self.skip_visibility()?;
            let fname = self.expect_ident("struct field")?;
            self.expect(&TokKind::Colon, "':'")?;
            let ty = self.parse_type()?;
            fields.push(HybridFieldSpec {
                name: fname,
                ty,
                doc: fdoc,
            });
            if !self.eat(&TokKind::Comma) {
                break;
            }
        }
        self.expect(&TokKind::RBrace, "'}'")?;
        Ok(HybridStructSpec { name, fields, doc })
    }

    fn parse_module_trait(
        &mut self,
        explicit_name: Option<String>,
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
            }
            methods.push(self.parse_fn_signature(mdoc)?);
        }
        self.expect(&TokKind::RBrace, "'}'")?;

        Ok(HybridModuleSpec {
            name,
            trait_name,
            methods,
            doc,
        })
    }

    fn parse_view_trait(
        &mut self,
        explicit_name: Option<String>,
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
                    if !self.eat(&TokKind::Comma) {
                        break;
                    }
                    continue;
                }
            }
            if self.eat_ident("self") {
                if !self.eat(&TokKind::Comma) {
                    break;
                }
                continue;
            }
            if self.is_ident("mut") && matches!(&self.peek_next().kind, TokKind::Ident(s) if s == "self") {
                self.bump();
                self.bump();
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
        })
    }

    fn parse_type(&mut self) -> Result<HybridType, CodegenError> {
        let tok = self.peek().clone();
        // Unit `()` -> Void
        if self.eat(&TokKind::LParen) {
            self.expect(&TokKind::RParen, "')'")?;
            return Ok(HybridType::Void);
        }

        // Reference types: `&str`, `&[u8]`, `&[T]`, `&T`
        if self.eat(&TokKind::Amp) {
            self.eat_ident("mut");
            if self.eat(&TokKind::LBracket) {
                let elem = self.parse_type()?;
                self.expect(&TokKind::RBracket, "']'")?;
                return match elem {
                    HybridType::Struct(ref s) if s == "u8" => Ok(HybridType::Bytes),
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
        let last = path.last().unwrap().as_str();

        match last {
            "bool" => Ok(HybridType::Bool),
            "i32" => Ok(HybridType::I32),
            "u32" => Ok(HybridType::U32),
            "i64" => Ok(HybridType::I64),
            "u64" => Ok(HybridType::U64),
            "f32" => Ok(HybridType::F32),
            "f64" => Ok(HybridType::F64),
            "String" | "str" => Ok(HybridType::String),
            "NveValue" => Ok(HybridType::Value),
            "u8" => Ok(HybridType::Struct("u8".to_string())), // resolved inside Vec<u8> / &[u8]
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
                    HybridType::Struct(ref s) if s == "u8" => Ok(HybridType::Bytes),
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
        if k == key {
            if let AttrValue::Str(s) = v {
                return Some(s.clone());
            }
        }
        None
    })
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

fn resolve_enum_types(file: &mut HybridFileSpec) {
    let enum_names: BTreeSet<String> = file.enums.iter().map(|e| e.name.clone()).collect();
    let rewrite = |ty: &mut HybridType| rewrite_type_enums(ty, &enum_names);
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
}

fn rewrite_type_enums(ty: &mut HybridType, enum_names: &BTreeSet<String>) {
    match ty {
        HybridType::Struct(name) => {
            if enum_names.contains(name) {
                *ty = HybridType::Enum(name.clone());
            }
        }
        HybridType::Optional(inner) | HybridType::Vec(inner) => {
            rewrite_type_enums(inner, enum_names);
        }
        HybridType::Result { ok, err } => {
            rewrite_type_enums(ok, enum_names);
            rewrite_type_enums(err, enum_names);
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
