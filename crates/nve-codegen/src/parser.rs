//! Recursive-descent parser for React Native Fabric `*NativeComponent.{ts,js}`
//! and TurboModule `Native*.{ts,js}` specifications (both TypeScript and Flow syntax).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::error::CodegenError;
use crate::ir::{
    CommandSpec, ComponentOptions, DefaultValue, EventBubbling, EventSpec, ObjectField, ParamSpec,
    ParsedRnSpec, PropSpec, PropType, RnComponentSpec, TurboModuleMethod, TurboModuleSpec,
};
use crate::lexer::{tokenize, Token, TokenKind};

/// Parses a React Native Fabric component or TurboModule specification from source text.
pub fn parse_rn_spec(source: &str) -> Result<ParsedRnSpec, CodegenError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser::new(tokens);
    parser.parse_file()
}

/// Parses a React Native spec file from disk.
///
/// Automatically follows relative re-export wrapper files (common in React Native's
/// `Libraries/**/*NativeComponent.js` which re-export from `src/private/.../specs/...`).
pub fn parse_rn_spec_file(path: &Path) -> Result<ParsedRnSpec, CodegenError> {
    parse_rn_spec_file_inner(path, 0)
}

fn parse_rn_spec_file_inner(path: &Path, depth: usize) -> Result<ParsedRnSpec, CodegenError> {
    if depth > 4 {
        return Err(CodegenError::validation(format!(
            "exceeded max re-export indirection depth while reading {}",
            path.display()
        )));
    }
    let source = std::fs::read_to_string(path)
        .map_err(|e| CodegenError::io(path.display().to_string(), e))?;

    match parse_rn_spec(&source) {
        Ok(spec) => Ok(spec),
        Err(orig_err) => {
            if let Some(target_rel) = detect_reexport_target(&source) {
                if let Some(resolved) = resolve_relative_spec(path, &target_rel) {
                    return parse_rn_spec_file_inner(&resolved, depth + 1);
                }
            }
            Err(orig_err)
        }
    }
}

fn detect_reexport_target(source: &str) -> Option<String> {
    let tokens = tokenize(source).ok()?;
    for (i, tok) in tokens.iter().enumerate() {
        if matches!(&tok.kind, TokenKind::Ident(s) if s == "from") {
            if let Some(Token {
                kind: TokenKind::StringLit(rel),
                ..
            }) = tokens.get(i + 1)
            {
                if rel.starts_with('.') {
                    return Some(rel.clone());
                }
            }
        }
    }
    None
}

fn resolve_relative_spec(base_file: &Path, rel: &str) -> Option<PathBuf> {
    let parent = base_file.parent()?;
    let candidate = parent.join(rel);
    if candidate.is_file() {
        return Some(candidate);
    }
    for ext in ["ts", "tsx", "js", "jsx"] {
        let with_ext = parent.join(format!("{rel}.{ext}"));
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Internal AST collected in pass 1 before resolving aliases into final IR.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum TypeExpr {
    Named {
        name: String,
        args: Vec<TypeExpr>,
    },
    StringLit(String),
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),
    Null,
    Undefined,
    Void,
    Nullable(Box<TypeExpr>),
    Array(Box<TypeExpr>),
    Union(Vec<TypeExpr>),
    Intersection(Vec<TypeExpr>),
    Object {
        spreads: Vec<String>,
        members: Vec<RawMember>,
    },
    Function {
        params: Vec<RawParam>,
        ret: Box<TypeExpr>,
    },
    Unknown,
}

#[derive(Debug, Clone)]
struct RawParam {
    name: String,
    optional: bool,
    ty: TypeExpr,
}

#[derive(Debug, Clone)]
enum RawMember {
    Property {
        name: String,
        optional: bool,
        ty: TypeExpr,
        doc: Option<String>,
    },
    Method {
        name: String,
        params: Vec<RawParam>,
        ret: TypeExpr,
        doc: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct InterfaceDecl {
    name: String,
    extends: Vec<String>,
    members: Vec<RawMember>,
}

#[derive(Debug, Clone)]
struct ComponentCall {
    props_type: Option<String>,
    component_name: String,
    options: ComponentOptions,
}

#[derive(Debug, Clone)]
struct CommandsCall {
    interface_name: Option<String>,
    supported_commands: Vec<String>,
}

#[derive(Debug, Clone)]
struct TurboModuleCall {
    spec_name: Option<String>,
    module_name: String,
    enforcing: bool,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    type_aliases: HashMap<String, TypeExpr>,
    interfaces: HashMap<String, InterfaceDecl>,
    component_call: Option<ComponentCall>,
    commands_call: Option<CommandsCall>,
    turbomodule_call: Option<TurboModuleCall>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            type_aliases: HashMap::new(),
            interfaces: HashMap::new(),
            component_call: None,
            commands_call: None,
            turbomodule_call: None,
        }
    }

    fn peek(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .unwrap_or_else(|| self.tokens.last().unwrap())
    }

    fn peek_skip_docs(&self) -> &Token {
        let mut idx = self.pos;
        while let Some(tok) = self.tokens.get(idx) {
            if !matches!(tok.kind, TokenKind::DocComment(_)) {
                return tok;
            }
            idx += 1;
        }
        self.tokens.last().unwrap()
    }

    fn peek_nth_non_doc(&self, n: usize) -> &Token {
        let mut idx = self.pos;
        let mut seen = 0;
        while let Some(tok) = self.tokens.get(idx) {
            if !matches!(tok.kind, TokenKind::DocComment(_)) {
                if seen == n {
                    return tok;
                }
                seen += 1;
            }
            idx += 1;
        }
        self.tokens.last().unwrap()
    }

    fn take_doc_comments(&mut self) -> Option<String> {
        let mut last = None;
        while let TokenKind::DocComment(doc) = &self.peek().kind {
            last = Some(doc.clone());
            self.pos += 1;
        }
        last
    }

    fn skip_docs(&mut self) {
        while matches!(self.peek().kind, TokenKind::DocComment(_)) {
            self.pos += 1;
        }
    }

    fn next_tok(&mut self) -> Token {
        self.skip_docs();
        let tok = self.peek().clone();
        if !matches!(tok.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        tok
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek_skip_docs().kind, TokenKind::Eof)
    }

    fn at_ident(&self, expected: &str) -> bool {
        matches!(&self.peek_skip_docs().kind, TokenKind::Ident(s) if s == expected)
    }

    fn eat_ident(&mut self, expected: &str) -> bool {
        if self.at_ident(expected) {
            self.next_tok();
            true
        } else {
            false
        }
    }

    fn eat(&mut self, expected: &TokenKind) -> bool {
        if &self.peek_skip_docs().kind == expected {
            self.next_tok();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: &TokenKind, label: &str) -> Result<Token, CodegenError> {
        let tok = self.next_tok();
        if &tok.kind == expected {
            Ok(tok)
        } else {
            Err(CodegenError::parse(
                tok.line,
                tok.col,
                format!("expected {label}, found {:?}", tok.kind),
            ))
        }
    }

    fn expect_ident(&mut self, label: &str) -> Result<String, CodegenError> {
        let tok = self.next_tok();
        match tok.kind {
            TokenKind::Ident(s) => Ok(s),
            other => Err(CodegenError::parse(
                tok.line,
                tok.col,
                format!("expected {label}, found {other:?}"),
            )),
        }
    }

    fn parse_file(&mut self) -> Result<ParsedRnSpec, CodegenError> {
        while !self.at_eof() {
            let _doc = self.take_doc_comments();
            if self.at_eof() {
                break;
            }

            if self.at_ident("import") {
                self.skip_import();
                continue;
            }

            if self.at_ident("export") {
                self.next_tok();
                if self.eat_ident("default") {
                    self.parse_export_default()?;
                    continue;
                }
                if self.eat_ident("declare") {
                    // fall through to type/interface
                }
            } else if self.eat_ident("declare") {
                // fall through
            }

            if self.at_ident("interface") {
                let decl = self.parse_interface_decl()?;
                self.interfaces.insert(decl.name.clone(), decl);
                continue;
            }

            if self.at_ident("type") && matches!(self.peek_nth_non_doc(1).kind, TokenKind::Ident(_))
            {
                self.parse_type_alias()?;
                continue;
            }

            if self.at_ident("const") || self.at_ident("let") || self.at_ident("var") {
                self.parse_var_statement()?;
                continue;
            }

            // Check standalone calls (e.g. `codegenNativeComponent<...>(...)`)
            if self.at_ident("codegenNativeComponent") {
                let call = self.parse_codegen_native_component_call()?;
                self.component_call = Some(call);
                continue;
            }

            if self.at_ident("codegenNativeCommands") {
                let call = self.parse_codegen_native_commands_call()?;
                self.commands_call = Some(call);
                continue;
            }

            if self.at_ident("TurboModuleRegistry") {
                if let Some(call) = self.try_parse_turbomodule_call()? {
                    self.turbomodule_call = Some(call);
                    continue;
                }
            }

            // Skip any other top-level token cleanly
            self.next_tok();
        }

        self.build_ir()
    }

    fn skip_import(&mut self) {
        // Consume `import ... ;` or up to string literal after `from`
        self.next_tok(); // `import`
        while !self.at_eof() {
            let tok = self.next_tok();
            match tok.kind {
                TokenKind::Semi => break,
                TokenKind::StringLit(_) => {
                    self.eat(&TokenKind::Semi);
                    break;
                }
                _ => {}
            }
        }
    }

    fn parse_export_default(&mut self) -> Result<(), CodegenError> {
        // Handle optional parens: `export default (codegenNativeComponent<...>(...))`
        while self.eat(&TokenKind::LParen) {}

        if self.at_ident("codegenNativeComponent") {
            let call = self.parse_codegen_native_component_call()?;
            self.component_call = Some(call);
            self.skip_to_statement_end();
            return Ok(());
        }

        if self.at_ident("TurboModuleRegistry") {
            if let Some(call) = self.try_parse_turbomodule_call()? {
                self.turbomodule_call = Some(call);
                self.skip_to_statement_end();
                return Ok(());
            }
        }

        // Handle `(TurboModuleRegistry.get<Spec>(...) as ?Spec)` or other expressions
        let mut depth = 0usize;
        while !self.at_eof() {
            if self.at_ident("codegenNativeComponent") {
                let call = self.parse_codegen_native_component_call()?;
                self.component_call = Some(call);
                self.skip_to_statement_end();
                return Ok(());
            }
            if self.at_ident("TurboModuleRegistry") {
                if let Some(call) = self.try_parse_turbomodule_call()? {
                    self.turbomodule_call = Some(call);
                    self.skip_to_statement_end();
                    return Ok(());
                }
            }
            let tok = self.next_tok();
            match tok.kind {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Semi if depth == 0 => break,
                _ => {}
            }
        }
        Ok(())
    }

    fn parse_var_statement(&mut self) -> Result<(), CodegenError> {
        self.next_tok(); // const / let / var
        let mut depth = 0usize;
        while !self.at_eof() {
            if self.at_ident("codegenNativeComponent") {
                let call = self.parse_codegen_native_component_call()?;
                self.component_call = Some(call);
                continue;
            }
            if self.at_ident("codegenNativeCommands") {
                let call = self.parse_codegen_native_commands_call()?;
                self.commands_call = Some(call);
                continue;
            }
            if self.at_ident("TurboModuleRegistry") {
                if let Some(call) = self.try_parse_turbomodule_call()? {
                    self.turbomodule_call = Some(call);
                    continue;
                }
            }
            let tok = self.next_tok();
            match tok.kind {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Semi if depth == 0 => break,
                _ => {}
            }
        }
        Ok(())
    }

    fn skip_to_statement_end(&mut self) {
        let mut depth = 0usize;
        while !self.at_eof() {
            match self.peek_skip_docs().kind {
                TokenKind::Semi if depth == 0 => {
                    self.next_tok();
                    break;
                }
                TokenKind::Ident(ref s)
                    if depth == 0
                        && matches!(
                            s.as_str(),
                            "export" | "import" | "interface" | "type" | "const" | "declare"
                        ) =>
                {
                    break;
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => {
                    depth += 1;
                    self.next_tok();
                }
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1);
                    self.next_tok();
                }
                _ => {
                    self.next_tok();
                }
            }
        }
    }

    fn parse_interface_decl(&mut self) -> Result<InterfaceDecl, CodegenError> {
        self.next_tok(); // `interface`
        let name = self.expect_ident("interface name")?;

        // Optional generic parameters `<T>`
        if self.eat(&TokenKind::LAngle) {
            self.skip_balanced_angles();
        }

        let mut extends = Vec::new();
        if self.eat_ident("extends") {
            loop {
                let base = self.parse_qualified_name()?;
                if self.eat(&TokenKind::LAngle) {
                    self.skip_balanced_angles();
                }
                extends.push(base);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }

        let (_spreads, members) = self.parse_object_body()?;
        Ok(InterfaceDecl {
            name,
            extends,
            members,
        })
    }

    fn parse_type_alias(&mut self) -> Result<(), CodegenError> {
        self.next_tok(); // `type`
        let name = self.expect_ident("type alias name")?;

        if self.eat(&TokenKind::LAngle) {
            self.skip_balanced_angles();
        }

        self.expect(&TokenKind::Equals, "'=' in type alias")?;
        let ty = self.parse_type_expr()?;
        self.eat(&TokenKind::Semi);
        self.type_aliases.insert(name, ty);
        Ok(())
    }

    fn parse_object_body(&mut self) -> Result<(Vec<String>, Vec<RawMember>), CodegenError> {
        self.expect(&TokenKind::LBrace, "'{'")?;
        // Support Flow exact objects `{| ... |}`
        let exact = self.eat(&TokenKind::Pipe);

        let mut spreads = Vec::new();
        let mut members = Vec::new();

        while !self.at_eof() {
            let doc = self.take_doc_comments();
            if exact && self.peek().kind == TokenKind::Pipe {
                self.next_tok();
                self.expect(&TokenKind::RBrace, "'}' closing Flow exact object")?;
                return Ok((spreads, members));
            }
            if self.eat(&TokenKind::RBrace) {
                return Ok((spreads, members));
            }
            if self.eat(&TokenKind::Semi) || self.eat(&TokenKind::Comma) {
                continue;
            }

            // Flow spread: `...ViewProps,`
            if self.eat(&TokenKind::Ellipsis) {
                let spread_name = self.parse_qualified_name()?;
                if self.eat(&TokenKind::LAngle) {
                    self.skip_balanced_angles();
                }
                spreads.push(spread_name);
                let _ = self.eat(&TokenKind::Comma) || self.eat(&TokenKind::Semi);
                continue;
            }

            // Index signature: `[key: string]: ValueType`
            if self.peek().kind == TokenKind::LBracket {
                self.skip_balanced_brackets();
                if self.eat(&TokenKind::Colon) {
                    let _ = self.parse_type_expr()?;
                }
                let _ = self.eat(&TokenKind::Comma) || self.eat(&TokenKind::Semi);
                continue;
            }

            // Optional modifiers: `readonly`, `+`, `-`
            let _ = self.eat(&TokenKind::Plus) || self.eat(&TokenKind::Minus);
            if self.at_ident("readonly")
                && !matches!(
                    self.peek_nth_non_doc(1).kind,
                    TokenKind::Colon | TokenKind::Question | TokenKind::LParen
                )
            {
                self.next_tok();
            }

            let member_name = match self.next_tok() {
                Token {
                    kind: TokenKind::Ident(s),
                    ..
                }
                | Token {
                    kind: TokenKind::StringLit(s),
                    ..
                } => s,
                other => {
                    return Err(CodegenError::parse(
                        other.line,
                        other.col,
                        format!("expected member name in object/interface, found {:?}", other.kind),
                    ));
                }
            };

            let optional = self.eat(&TokenKind::Question);

            // Method signature: `foo(a: string): ReturnType;`
            if self.peek_skip_docs().kind == TokenKind::LParen {
                let params = self.parse_param_list()?;
                let ret = if self.eat(&TokenKind::Colon) {
                    self.parse_type_expr()?
                } else {
                    TypeExpr::Void
                };
                members.push(RawMember::Method {
                    name: member_name,
                    params,
                    ret,
                    doc,
                });
            } else {
                self.expect(&TokenKind::Colon, "':' after property name")?;
                let ty = self.parse_type_expr()?;
                members.push(RawMember::Property {
                    name: member_name,
                    optional,
                    ty,
                    doc,
                });
            }

            let _ = self.eat(&TokenKind::Comma) || self.eat(&TokenKind::Semi);
        }

        Err(CodegenError::parse(
            self.peek().line,
            self.peek().col,
            "unterminated object or interface body",
        ))
    }

    fn parse_param_list(&mut self) -> Result<Vec<RawParam>, CodegenError> {
        self.expect(&TokenKind::LParen, "'('")?;
        let mut params = Vec::new();
        while !self.at_eof() {
            self.skip_docs();
            if self.eat(&TokenKind::RParen) {
                return Ok(params);
            }
            if self.eat(&TokenKind::Comma) {
                continue;
            }
            // Optional rest `...args`
            let _ = self.eat(&TokenKind::Ellipsis);
            let name = self.expect_ident("parameter name")?;
            let optional = self.eat(&TokenKind::Question);
            let ty = if self.eat(&TokenKind::Colon) {
                self.parse_type_expr()?
            } else {
                TypeExpr::Unknown
            };
            params.push(RawParam { name, optional, ty });
            let _ = self.eat(&TokenKind::Comma);
        }
        Err(CodegenError::parse(
            self.peek().line,
            self.peek().col,
            "unterminated parameter list",
        ))
    }

    fn parse_type_expr(&mut self) -> Result<TypeExpr, CodegenError> {
        self.parse_union_type()
    }

    fn parse_union_type(&mut self) -> Result<TypeExpr, CodegenError> {
        // Support leading `|` in multiline unions (`| 'portrait' | 'landscape'`)
        let _ = self.eat(&TokenKind::Pipe);
        let first = self.parse_intersection_type()?;
        if self.peek_skip_docs().kind != TokenKind::Pipe {
            return Ok(first);
        }
        let mut variants = vec![first];
        while self.eat(&TokenKind::Pipe) {
            variants.push(self.parse_intersection_type()?);
        }
        Ok(TypeExpr::Union(variants))
    }

    fn parse_intersection_type(&mut self) -> Result<TypeExpr, CodegenError> {
        let _ = self.eat(&TokenKind::Amp);
        let first = self.parse_postfix_type()?;
        if self.peek_skip_docs().kind != TokenKind::Amp {
            return Ok(first);
        }
        let mut parts = vec![first];
        while self.eat(&TokenKind::Amp) {
            parts.push(self.parse_postfix_type()?);
        }
        Ok(TypeExpr::Intersection(parts))
    }

    fn parse_postfix_type(&mut self) -> Result<TypeExpr, CodegenError> {
        let mut ty = self.parse_primary_type()?;
        while self.peek_skip_docs().kind == TokenKind::LBracket {
            let next = self.peek_nth_non_doc(1);
            if next.kind == TokenKind::RBracket {
                self.next_tok();
                self.next_tok();
                ty = TypeExpr::Array(Box::new(ty));
            } else {
                break;
            }
        }
        Ok(ty)
    }

    fn parse_primary_type(&mut self) -> Result<TypeExpr, CodegenError> {
        self.skip_docs();
        let tok = self.peek().clone();
        match &tok.kind {
            // Flow nullable prefix `?T`
            TokenKind::Question => {
                self.next_tok();
                let inner = self.parse_postfix_type()?;
                Ok(TypeExpr::Nullable(Box::new(inner)))
            }
            // Parenthesized type OR arrow function `(a: T) => R`
            TokenKind::LParen => self.parse_paren_or_fn_type(),
            // Inline object `{ ... }`
            TokenKind::LBrace => {
                let (spreads, members) = self.parse_object_body()?;
                Ok(TypeExpr::Object { spreads, members })
            }
            // String literal `'small'`
            TokenKind::StringLit(s) => {
                let s = s.clone();
                self.next_tok();
                Ok(TypeExpr::StringLit(s))
            }
            // Negative numeric literal `-1` / `-0.5`
            TokenKind::Minus => {
                self.next_tok();
                let num = self.next_tok();
                match num.kind {
                    TokenKind::NumberLit(n) => parse_number_type_expr(&format!("-{n}"), num.line, num.col),
                    other => Err(CodegenError::parse(
                        num.line,
                        num.col,
                        format!("expected number after '-', found {other:?}"),
                    )),
                }
            }
            TokenKind::NumberLit(n) => {
                let n = n.clone();
                self.next_tok();
                parse_number_type_expr(&n, tok.line, tok.col)
            }
            // Tuple type `[A, B]` -> treat as array/mixed
            TokenKind::LBracket => {
                self.skip_balanced_brackets();
                Ok(TypeExpr::Array(Box::new(TypeExpr::Unknown)))
            }
            TokenKind::Ident(ident) => {
                let ident = ident.clone();
                match ident.as_str() {
                    "null" => {
                        self.next_tok();
                        Ok(TypeExpr::Null)
                    }
                    "undefined" => {
                        self.next_tok();
                        Ok(TypeExpr::Undefined)
                    }
                    "void" => {
                        self.next_tok();
                        Ok(TypeExpr::Void)
                    }
                    "true" => {
                        self.next_tok();
                        Ok(TypeExpr::BoolLit(true))
                    }
                    "false" => {
                        self.next_tok();
                        Ok(TypeExpr::BoolLit(false))
                    }
                    "readonly" => {
                        self.next_tok();
                        self.parse_postfix_type()
                    }
                    _ => {
                        let name = self.parse_qualified_name()?;
                        let mut args = Vec::new();
                        if self.eat(&TokenKind::LAngle) {
                            while !self.at_eof() {
                                self.skip_docs();
                                if self.eat(&TokenKind::RAngle) {
                                    break;
                                }
                                if self.eat(&TokenKind::Comma) {
                                    continue;
                                }
                                args.push(self.parse_type_expr()?);
                                let _ = self.eat(&TokenKind::Comma);
                            }
                        }
                        Ok(TypeExpr::Named { name, args })
                    }
                }
            }
            other => Err(CodegenError::parse(
                tok.line,
                tok.col,
                format!("unexpected token in type expression: {other:?}"),
            )),
        }
    }

    fn parse_paren_or_fn_type(&mut self) -> Result<TypeExpr, CodegenError> {
        let saved = self.pos;
        if let Ok(params) = self.parse_param_list() {
            if self.eat(&TokenKind::FatArrow) {
                let ret = self.parse_type_expr()?;
                return Ok(TypeExpr::Function {
                    params,
                    ret: Box::new(ret),
                });
            }
        }
        // Backtrack and parse as parenthesized type
        self.pos = saved;
        self.expect(&TokenKind::LParen, "'('")?;
        let inner = self.parse_type_expr()?;
        self.expect(&TokenKind::RParen, "')'")?;
        Ok(inner)
    }

    fn parse_qualified_name(&mut self) -> Result<String, CodegenError> {
        let mut name = self.expect_ident("type name")?;
        while self.eat(&TokenKind::Dot) {
            let seg = self.expect_ident("qualified name segment")?;
            name.push('.');
            name.push_str(&seg);
        }
        Ok(name)
    }

    fn skip_balanced_angles(&mut self) {
        let mut depth = 1usize;
        while !self.at_eof() && depth > 0 {
            match self.next_tok().kind {
                TokenKind::LAngle => depth += 1,
                TokenKind::RAngle => depth -= 1,
                _ => {}
            }
        }
    }

    fn skip_balanced_brackets(&mut self) {
        if !self.eat(&TokenKind::LBracket) {
            return;
        }
        let mut depth = 1usize;
        while !self.at_eof() && depth > 0 {
            match self.next_tok().kind {
                TokenKind::LBracket => depth += 1,
                TokenKind::RBracket => depth -= 1,
                _ => {}
            }
        }
    }

    fn parse_codegen_native_component_call(&mut self) -> Result<ComponentCall, CodegenError> {
        self.next_tok(); // `codegenNativeComponent`
        let mut props_type = None;
        if self.eat(&TokenKind::LAngle) {
            if let TokenKind::Ident(name) = self.peek_skip_docs().kind.clone() {
                props_type = Some(name);
            }
            self.skip_balanced_angles();
        }
        self.expect(&TokenKind::LParen, "'(' in codegenNativeComponent")?;
        let tok = self.next_tok();
        let component_name = match tok.kind {
            TokenKind::StringLit(s) => s,
            other => {
                return Err(CodegenError::parse(
                    tok.line,
                    tok.col,
                    format!(
                        "expected component name string literal in codegenNativeComponent, found {other:?}"
                    ),
                ));
            }
        };

        let mut options = ComponentOptions::default();
        if self.eat(&TokenKind::Comma) && self.peek_skip_docs().kind == TokenKind::LBrace {
            options = self.parse_component_options_object()?;
        }
        let _ = self.eat(&TokenKind::Comma);
        self.expect(&TokenKind::RParen, "')' closing codegenNativeComponent")?;

        Ok(ComponentCall {
            props_type,
            component_name,
            options,
        })
    }

    fn parse_component_options_object(&mut self) -> Result<ComponentOptions, CodegenError> {
        self.expect(&TokenKind::LBrace, "'{' in component options")?;
        let mut opts = ComponentOptions::default();
        while !self.at_eof() {
            self.skip_docs();
            if self.eat(&TokenKind::RBrace) {
                break;
            }
            if self.eat(&TokenKind::Comma) {
                continue;
            }
            let key = match self.next_tok().kind {
                TokenKind::Ident(s) | TokenKind::StringLit(s) => s,
                _ => continue,
            };
            if !self.eat(&TokenKind::Colon) {
                continue;
            }
            match key.as_str() {
                "paperComponentName" => {
                    if let TokenKind::StringLit(val) = self.next_tok().kind {
                        opts.paper_component_name = Some(val);
                    }
                }
                "interfaceOnly" => {
                    if let TokenKind::Ident(val) = self.next_tok().kind {
                        opts.interface_only = val == "true";
                    }
                }
                "excludedPlatforms" => {
                    if self.eat(&TokenKind::LBracket) {
                        while !self.at_eof() {
                            self.skip_docs();
                            if self.eat(&TokenKind::RBracket) {
                                break;
                            }
                            if let TokenKind::StringLit(p) = self.next_tok().kind {
                                opts.excluded_platforms.push(p);
                            }
                            let _ = self.eat(&TokenKind::Comma);
                        }
                    }
                }
                _ => {
                    // Skip unrecognized option value cleanly
                    let mut depth = 0usize;
                    while !self.at_eof() {
                        match self.peek_skip_docs().kind {
                            TokenKind::Comma | TokenKind::RBrace if depth == 0 => break,
                            TokenKind::LBrace | TokenKind::LBracket | TokenKind::LParen => {
                                depth += 1;
                                self.next_tok();
                            }
                            TokenKind::RBrace | TokenKind::RBracket | TokenKind::RParen => {
                                depth = depth.saturating_sub(1);
                                self.next_tok();
                            }
                            _ => {
                                self.next_tok();
                            }
                        }
                    }
                }
            }
            let _ = self.eat(&TokenKind::Comma);
        }
        Ok(opts)
    }

    fn parse_codegen_native_commands_call(&mut self) -> Result<CommandsCall, CodegenError> {
        self.next_tok(); // `codegenNativeCommands`
        let mut interface_name = None;
        if self.eat(&TokenKind::LAngle) {
            if let TokenKind::Ident(name) = self.peek_skip_docs().kind.clone() {
                interface_name = Some(name);
            }
            self.skip_balanced_angles();
        }
        let mut supported = Vec::new();
        if self.eat(&TokenKind::LParen) {
            let mut depth = 1usize;
            while !self.at_eof() && depth > 0 {
                if self.at_ident("supportedCommands") {
                    self.next_tok();
                    let _ = self.eat(&TokenKind::Colon);
                    if self.eat(&TokenKind::LBracket) {
                        while !self.at_eof() {
                            self.skip_docs();
                            if self.eat(&TokenKind::RBracket) {
                                break;
                            }
                            if let TokenKind::StringLit(cmd) = self.next_tok().kind {
                                supported.push(cmd);
                            }
                            let _ = self.eat(&TokenKind::Comma);
                        }
                    }
                    continue;
                }
                match self.next_tok().kind {
                    TokenKind::LParen => depth += 1,
                    TokenKind::RParen => depth -= 1,
                    _ => {}
                }
            }
        }
        Ok(CommandsCall {
            interface_name,
            supported_commands: supported,
        })
    }

    fn try_parse_turbomodule_call(&mut self) -> Result<Option<TurboModuleCall>, CodegenError> {
        let saved = self.pos;
        self.next_tok(); // `TurboModuleRegistry`
        if !self.eat(&TokenKind::Dot) {
            self.pos = saved;
            return Ok(None);
        }
        let method = match self.next_tok().kind {
            TokenKind::Ident(m) => m,
            _ => {
                self.pos = saved;
                return Ok(None);
            }
        };
        let enforcing = match method.as_str() {
            "getEnforcing" => true,
            "get" => false,
            _ => {
                self.pos = saved;
                return Ok(None);
            }
        };
        let mut spec_name = None;
        if self.eat(&TokenKind::LAngle) {
            if let TokenKind::Ident(name) = self.peek_skip_docs().kind.clone() {
                spec_name = Some(name);
            }
            self.skip_balanced_angles();
        }
        self.expect(&TokenKind::LParen, "'(' in TurboModuleRegistry call")?;
        let tok = self.next_tok();
        let module_name = match tok.kind {
            TokenKind::StringLit(s) => s,
            other => {
                return Err(CodegenError::parse(
                    tok.line,
                    tok.col,
                    format!(
                        "expected module name string literal in TurboModuleRegistry.{method}, found {other:?}"
                    ),
                ));
            }
        };
        let _ = self.eat(&TokenKind::Comma);
        self.expect(&TokenKind::RParen, "')' closing TurboModuleRegistry call")?;
        Ok(Some(TurboModuleCall {
            spec_name,
            module_name,
            enforcing,
        }))
    }

    // -----------------------------------------------------------------------
    // Pass 2: Resolve collected AST into final strongly-typed IR
    // -----------------------------------------------------------------------

    fn build_ir(&self) -> Result<ParsedRnSpec, CodegenError> {
        if let Some(comp_call) = &self.component_call {
            return self.build_component_ir(comp_call).map(ParsedRnSpec::Component);
        }
        if let Some(tm_call) = &self.turbomodule_call {
            return self
                .build_turbomodule_ir(tm_call)
                .map(ParsedRnSpec::TurboModule);
        }
        // Fallback: if file defines `interface Spec extends TurboModule` without registry call
        if let Some(spec_iface) = self.interfaces.values().find(|i| {
            i.extends
                .iter()
                .any(|e| e == "TurboModule" || e.ends_with(".TurboModule"))
        }) {
            let synthetic = TurboModuleCall {
                spec_name: Some(spec_iface.name.clone()),
                module_name: spec_iface.name.clone(),
                enforcing: true,
            };
            return self
                .build_turbomodule_ir(&synthetic)
                .map(ParsedRnSpec::TurboModule);
        }
        Err(CodegenError::validation(
            "no `codegenNativeComponent<...>(...)` or `TurboModule` spec found in file",
        ))
    }

    fn build_component_ir(&self, call: &ComponentCall) -> Result<RnComponentSpec, CodegenError> {
        let props_type_name = call
            .props_type
            .clone()
            .or_else(|| {
                if self.interfaces.contains_key("NativeProps") {
                    Some("NativeProps".to_owned())
                } else if self.type_aliases.contains_key("NativeProps") {
                    Some("NativeProps".to_owned())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                CodegenError::validation(format!(
                    "could not determine props type for component '{}'",
                    call.component_name
                ))
            })?;

        let (extends_view_props, raw_members) = self.collect_props_members(&props_type_name)?;

        let mut props = Vec::new();
        let mut events = Vec::new();

        for member in raw_members {
            match member {
                RawMember::Property {
                    name,
                    optional,
                    ty,
                    doc,
                } => {
                    if let Some(ev) = self.try_resolve_event(&name, optional, &ty, doc.clone())? {
                        events.push(ev);
                    } else {
                        let prop = self.resolve_prop(&name, optional, &ty, doc)?;
                        props.push(prop);
                    }
                }
                RawMember::Method { .. } => {}
            }
        }

        let commands = self.resolve_commands();

        Ok(RnComponentSpec {
            component_name: call.component_name.clone(),
            props_type_name,
            extends_view_props,
            props,
            events,
            commands,
            options: call.options.clone(),
        })
    }

    fn collect_props_members(&self, type_name: &str) -> Result<(bool, Vec<RawMember>), CodegenError> {
        let mut extends_view_props = false;
        let mut members = Vec::new();
        let mut visited = HashSet::new();
        self.collect_named_members(type_name, &mut extends_view_props, &mut members, &mut visited)?;
        Ok((extends_view_props, members))
    }

    fn collect_named_members(
        &self,
        name: &str,
        extends_view_props: &mut bool,
        out: &mut Vec<RawMember>,
        visited: &mut HashSet<String>,
    ) -> Result<(), CodegenError> {
        if name == "ViewProps" || name.ends_with(".ViewProps") {
            *extends_view_props = true;
            return Ok(());
        }
        if !visited.insert(name.to_owned()) {
            return Ok(());
        }
        if let Some(iface) = self.interfaces.get(name) {
            for base in &iface.extends {
                self.collect_named_members(base, extends_view_props, out, visited)?;
            }
            out.extend(iface.members.clone());
            return Ok(());
        }
        if let Some(alias) = self.type_aliases.get(name) {
            self.collect_expr_members(alias, extends_view_props, out, visited)?;
            return Ok(());
        }
        Err(CodegenError::validation(format!(
            "props type '{name}' was referenced by codegenNativeComponent but not defined in file"
        )))
    }

    fn collect_expr_members(
        &self,
        expr: &TypeExpr,
        extends_view_props: &mut bool,
        out: &mut Vec<RawMember>,
        visited: &mut HashSet<String>,
    ) -> Result<(), CodegenError> {
        match expr {
            TypeExpr::Object { spreads, members } => {
                for sp in spreads {
                    if sp == "ViewProps" || sp.ends_with(".ViewProps") {
                        *extends_view_props = true;
                    } else if self.interfaces.contains_key(sp) || self.type_aliases.contains_key(sp)
                    {
                        self.collect_named_members(sp, extends_view_props, out, visited)?;
                    }
                }
                out.extend(members.clone());
                Ok(())
            }
            TypeExpr::Named { name, args } => {
                if matches!(name.as_str(), "Readonly" | "$ReadOnly" | "Partial") {
                    if let Some(inner) = args.first() {
                        return self.collect_expr_members(inner, extends_view_props, out, visited);
                    }
                }
                if name == "ViewProps" || name.ends_with(".ViewProps") {
                    *extends_view_props = true;
                    return Ok(());
                }
                if self.interfaces.contains_key(name) || self.type_aliases.contains_key(name) {
                    return self.collect_named_members(name, extends_view_props, out, visited);
                }
                Ok(())
            }
            TypeExpr::Intersection(parts) => {
                for p in parts {
                    self.collect_expr_members(p, extends_view_props, out, visited)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn try_resolve_event(
        &self,
        name: &str,
        optional: bool,
        ty: &TypeExpr,
        doc: Option<String>,
    ) -> Result<Option<EventSpec>, CodegenError> {
        let unwrapped = unwrap_nullable_wrappers(ty);
        let TypeExpr::Named {
            name: type_name,
            args,
        } = unwrapped
        else {
            return Ok(None);
        };

        let bubbling = match type_name.as_str() {
            "DirectEventHandler" => EventBubbling::Direct,
            "BubblingEventHandler" => EventBubbling::Bubbling,
            _ => return Ok(None),
        };

        let paper_top_level_name = args.get(1).and_then(|arg| match arg {
            TypeExpr::StringLit(s) => Some(s.clone()),
            _ => None,
        });

        let (payload, payload_type_name) = match args.first() {
            None | Some(TypeExpr::Null) | Some(TypeExpr::Void) | Some(TypeExpr::Undefined) => {
                (Vec::new(), None)
            }
            Some(payload_expr) => {
                let alias_name = match unwrap_readonly(payload_expr) {
                    TypeExpr::Named { name, args } if args.is_empty() => Some(name.clone()),
                    _ => None,
                };
                let mut seen = HashSet::new();
                let resolved = self.resolve_type_expr(payload_expr, &mut seen)?;
                match resolved {
                    PropType::Object { fields } => (fields, alias_name),
                    PropType::Void => (Vec::new(), alias_name),
                    other => (
                        vec![ObjectField {
                            name: "value".to_owned(),
                            field_type: other,
                            optional: false,
                            nullable: false,
                        }],
                        alias_name,
                    ),
                }
            }
        };

        Ok(Some(EventSpec {
            name: name.to_owned(),
            bubbling,
            optional,
            paper_top_level_name,
            payload,
            payload_type_name,
            doc,
        }))
    }

    fn resolve_prop(
        &self,
        name: &str,
        optional: bool,
        ty: &TypeExpr,
        doc: Option<String>,
    ) -> Result<PropSpec, CodegenError> {
        let (inner, mut nullable) = strip_nullability(ty);
        let mut default_value = None;
        let core = if let TypeExpr::Named {
            name: wrapper,
            args,
        } = inner
        {
            if wrapper == "WithDefault" && !args.is_empty() {
                if let Some(def_expr) = args.get(1) {
                    default_value = extract_default_value(def_expr);
                    if matches!(default_value, Some(DefaultValue::Null)) {
                        nullable = true;
                    }
                }
                let (inner_t, inner_null) = strip_nullability(&args[0]);
                nullable |= inner_null;
                inner_t
            } else {
                inner
            }
        } else {
            inner
        };

        let mut seen = HashSet::new();
        let prop_type = self.resolve_type_expr(core, &mut seen)?;

        Ok(PropSpec {
            name: name.to_owned(),
            prop_type,
            optional: optional || default_value.is_some(),
            nullable,
            default_value,
            doc,
        })
    }

    fn resolve_commands(&self) -> Vec<CommandSpec> {
        let Some(cmd_call) = &self.commands_call else {
            return Vec::new();
        };
        let iface_name = cmd_call
            .interface_name
            .as_deref()
            .unwrap_or("NativeCommands");
        let Some(iface) = self.interfaces.get(iface_name) else {
            return cmd_call
                .supported_commands
                .iter()
                .map(|name| CommandSpec {
                    name: name.clone(),
                    params: Vec::new(),
                })
                .collect();
        };

        let mut out = Vec::new();
        for member in &iface.members {
            let (name, raw_params) = match member {
                RawMember::Method { name, params, .. } => (name.clone(), params.as_slice()),
                RawMember::Property {
                    name,
                    ty: TypeExpr::Function { params, .. },
                    ..
                } => (name.clone(), params.as_slice()),
                _ => continue,
            };
            if !cmd_call.supported_commands.is_empty() && !cmd_call.supported_commands.contains(&name)
            {
                continue;
            }
            // Drop the first `viewRef: React.ElementRef<...>` argument from RN command signatures
            let mut params = Vec::new();
            for (idx, p) in raw_params.iter().enumerate() {
                if idx == 0 && (p.name == "viewRef" || is_element_ref(&p.ty)) {
                    continue;
                }
                let mut seen = HashSet::new();
                let param_type = self
                    .resolve_type_expr(&p.ty, &mut seen)
                    .unwrap_or(PropType::Mixed);
                params.push(ParamSpec {
                    name: p.name.clone(),
                    param_type,
                    optional: p.optional,
                });
            }
            out.push(CommandSpec { name, params });
        }
        out
    }

    fn build_turbomodule_ir(&self, call: &TurboModuleCall) -> Result<TurboModuleSpec, CodegenError> {
        let spec_name = call.spec_name.as_deref().unwrap_or("Spec");
        let iface = self.interfaces.get(spec_name).ok_or_else(|| {
            CodegenError::validation(format!(
                "TurboModule interface '{spec_name}' not found for module '{}'",
                call.module_name
            ))
        })?;

        let mut methods = Vec::new();
        for member in &iface.members {
            match member {
                RawMember::Method {
                    name,
                    params,
                    ret,
                    doc,
                } => {
                    let resolved_params = self.resolve_params(params)?;
                    let mut seen = HashSet::new();
                    let return_type = self.resolve_type_expr(ret, &mut seen)?;
                    methods.push(TurboModuleMethod {
                        name: name.clone(),
                        params: resolved_params,
                        return_type,
                        doc: doc.clone(),
                    });
                }
                RawMember::Property {
                    name,
                    ty: TypeExpr::Function { params, ret },
                    doc,
                    ..
                } => {
                    let resolved_params = self.resolve_params(params)?;
                    let mut seen = HashSet::new();
                    let return_type = self.resolve_type_expr(ret, &mut seen)?;
                    methods.push(TurboModuleMethod {
                        name: name.clone(),
                        params: resolved_params,
                        return_type,
                        doc: doc.clone(),
                    });
                }
                RawMember::Property { .. } => {}
            }
        }

        Ok(TurboModuleSpec {
            module_name: call.module_name.clone(),
            enforcing: call.enforcing,
            methods,
        })
    }

    fn resolve_params(&self, params: &[RawParam]) -> Result<Vec<ParamSpec>, CodegenError> {
        let mut out = Vec::with_capacity(params.len());
        for p in params {
            let mut seen = HashSet::new();
            let param_type = self.resolve_type_expr(&p.ty, &mut seen)?;
            out.push(ParamSpec {
                name: p.name.clone(),
                param_type,
                optional: p.optional,
            });
        }
        Ok(out)
    }

    fn resolve_type_expr(
        &self,
        expr: &TypeExpr,
        seen: &mut HashSet<String>,
    ) -> Result<PropType, CodegenError> {
        match expr {
            TypeExpr::Nullable(inner) => self.resolve_type_expr(inner, seen),
            TypeExpr::Array(item) => Ok(PropType::Array {
                item: Box::new(self.resolve_type_expr(item, seen)?),
            }),
            TypeExpr::Null | TypeExpr::Undefined | TypeExpr::Void => Ok(PropType::Void),
            TypeExpr::StringLit(s) => Ok(PropType::StringEnum {
                variants: vec![s.clone()],
            }),
            TypeExpr::IntLit(i) => Ok(PropType::IntEnum {
                variants: vec![*i as i32],
            }),
            TypeExpr::FloatLit(_) => Ok(PropType::Float),
            TypeExpr::BoolLit(_) => Ok(PropType::Bool),
            TypeExpr::Unknown => Ok(PropType::Mixed),
            TypeExpr::Function { params, .. } => Ok(PropType::Callback {
                params: self.resolve_params(params)?,
            }),
            TypeExpr::Object { members, .. } => {
                let mut fields = Vec::new();
                for m in members {
                    if let RawMember::Property {
                        name,
                        optional,
                        ty,
                        ..
                    } = m
                    {
                        let (inner, nullable) = strip_nullability(ty);
                        let field_type = self.resolve_type_expr(inner, seen)?;
                        fields.push(ObjectField {
                            name: name.clone(),
                            field_type,
                            optional: *optional,
                            nullable,
                        });
                    }
                }
                Ok(PropType::Object { fields })
            }
            TypeExpr::Union(items) => {
                let non_null: Vec<&TypeExpr> = items
                    .iter()
                    .filter(|t| !matches!(t, TypeExpr::Null | TypeExpr::Undefined | TypeExpr::Void))
                    .collect();
                if non_null.is_empty() {
                    return Ok(PropType::Void);
                }
                if non_null.len() == 1 {
                    return self.resolve_type_expr(non_null[0], seen);
                }
                if non_null.iter().all(|t| matches!(t, TypeExpr::StringLit(_))) {
                    let variants = non_null
                        .into_iter()
                        .filter_map(|t| match t {
                            TypeExpr::StringLit(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    return Ok(PropType::StringEnum { variants });
                }
                if non_null.iter().all(|t| matches!(t, TypeExpr::IntLit(_))) {
                    let variants = non_null
                        .into_iter()
                        .filter_map(|t| match t {
                            TypeExpr::IntLit(v) => Some(*v as i32),
                            _ => None,
                        })
                        .collect();
                    return Ok(PropType::IntEnum { variants });
                }
                Ok(PropType::Mixed)
            }
            TypeExpr::Intersection(parts) => {
                let mut combined = Vec::new();
                for p in parts {
                    if let PropType::Object { fields } = self.resolve_type_expr(p, seen)? {
                        combined.extend(fields);
                    }
                }
                if combined.is_empty() {
                    Ok(PropType::Mixed)
                } else {
                    Ok(PropType::Object { fields: combined })
                }
            }
            TypeExpr::Named { name, args } => {
                let short = name.rsplit('.').next().unwrap_or(name.as_str());
                match short {
                    "boolean" | "Boolean" => Ok(PropType::Bool),
                    "Int32" => Ok(PropType::Int32),
                    "Float" => Ok(PropType::Float),
                    "Double" | "number" | "Number" => Ok(PropType::Double),
                    "string" | "String" | "Stringish" => Ok(PropType::String),
                    "ColorValue" | "ProcessedColorValue" => Ok(PropType::Color),
                    "ImageSource" => Ok(PropType::ImageSource),
                    "PointValue" => Ok(PropType::Point),
                    "EdgeInsetsValue" => Ok(PropType::EdgeInsets),
                    "DimensionValue" => Ok(PropType::Dimension),
                    "UnsafeMixed" | "Object" | "Record" | "Mixed" | "unknown" | "any" => {
                        Ok(PropType::Mixed)
                    }
                    "Readonly" | "$ReadOnly" | "Partial" | "WithDefault" => {
                        if let Some(first) = args.first() {
                            self.resolve_type_expr(first, seen)
                        } else {
                            Ok(PropType::Mixed)
                        }
                    }
                    "ReadonlyArray" | "$ReadOnlyArray" | "Array" => {
                        let item = if let Some(first) = args.first() {
                            self.resolve_type_expr(first, seen)?
                        } else {
                            PropType::Mixed
                        };
                        Ok(PropType::Array {
                            item: Box::new(item),
                        })
                    }
                    "Promise" => {
                        let inner = if let Some(first) = args.first() {
                            self.resolve_type_expr(first, seen)?
                        } else {
                            PropType::Void
                        };
                        Ok(PropType::Promise {
                            inner: Box::new(inner),
                        })
                    }
                    _ => {
                        if seen.contains(name) {
                            return Ok(PropType::Mixed);
                        }
                        if let Some(alias) = self.type_aliases.get(name) {
                            seen.insert(name.clone());
                            let resolved = self.resolve_type_expr(alias, seen)?;
                            seen.remove(name);
                            return Ok(resolved);
                        }
                        if let Some(iface) = self.interfaces.get(name) {
                            seen.insert(name.clone());
                            let mut fields = Vec::new();
                            for m in &iface.members {
                                if let RawMember::Property {
                                    name: fname,
                                    optional,
                                    ty,
                                    ..
                                } = m
                                {
                                    let (inner, nullable) = strip_nullability(ty);
                                    let field_type = self.resolve_type_expr(inner, seen)?;
                                    fields.push(ObjectField {
                                        name: fname.clone(),
                                        field_type,
                                        optional: *optional,
                                        nullable,
                                    });
                                }
                            }
                            seen.remove(name);
                            return Ok(PropType::Object { fields });
                        }
                        Ok(PropType::Mixed)
                    }
                }
            }
        }
    }
}

fn parse_number_type_expr(raw: &str, line: usize, col: usize) -> Result<TypeExpr, CodegenError> {
    let clean = raw.replace('_', "");
    if clean.contains('.') || clean.contains('e') || clean.contains('E') {
        clean
            .parse::<f64>()
            .map(TypeExpr::FloatLit)
            .map_err(|_| CodegenError::parse(line, col, format!("invalid float literal '{raw}'")))
    } else if let Some(hex) = clean.strip_prefix("0x").or_else(|| clean.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16)
            .map(TypeExpr::IntLit)
            .map_err(|_| CodegenError::parse(line, col, format!("invalid hex literal '{raw}'")))
    } else {
        clean
            .parse::<i64>()
            .map(TypeExpr::IntLit)
            .map_err(|_| CodegenError::parse(line, col, format!("invalid int literal '{raw}'")))
    }
}

fn unwrap_readonly(expr: &TypeExpr) -> &TypeExpr {
    match expr {
        TypeExpr::Named { name, args }
            if matches!(name.as_str(), "Readonly" | "$ReadOnly") && args.len() == 1 =>
        {
            unwrap_readonly(&args[0])
        }
        other => other,
    }
}

fn unwrap_nullable_wrappers(expr: &TypeExpr) -> &TypeExpr {
    match expr {
        TypeExpr::Nullable(inner) => unwrap_nullable_wrappers(inner),
        TypeExpr::Union(items) => {
            let non_null: Vec<&TypeExpr> = items
                .iter()
                .filter(|t| !matches!(t, TypeExpr::Null | TypeExpr::Undefined))
                .collect();
            if non_null.len() == 1 {
                unwrap_nullable_wrappers(non_null[0])
            } else {
                expr
            }
        }
        other => other,
    }
}

fn strip_nullability(expr: &TypeExpr) -> (&TypeExpr, bool) {
    match expr {
        TypeExpr::Nullable(inner) => {
            let (core, _) = strip_nullability(inner);
            (core, true)
        }
        TypeExpr::Union(items) => {
            let has_null = items
                .iter()
                .any(|t| matches!(t, TypeExpr::Null | TypeExpr::Undefined));
            if has_null {
                let non_null: Vec<&TypeExpr> = items
                    .iter()
                    .filter(|t| !matches!(t, TypeExpr::Null | TypeExpr::Undefined))
                    .collect();
                if non_null.len() == 1 {
                    let (core, _) = strip_nullability(non_null[0]);
                    return (core, true);
                }
            }
            (expr, has_null)
        }
        other => (other, false),
    }
}

fn extract_default_value(expr: &TypeExpr) -> Option<DefaultValue> {
    match expr {
        TypeExpr::Null | TypeExpr::Undefined => Some(DefaultValue::Null),
        TypeExpr::BoolLit(b) => Some(DefaultValue::Bool(*b)),
        TypeExpr::IntLit(i) => Some(DefaultValue::Int(*i)),
        TypeExpr::FloatLit(f) => Some(DefaultValue::Float(*f)),
        TypeExpr::StringLit(s) => Some(DefaultValue::String(s.clone())),
        _ => None,
    }
}

fn is_element_ref(expr: &TypeExpr) -> bool {
    match expr {
        TypeExpr::Named { name, .. } => name.contains("ElementRef") || name.contains("HostInstance"),
        _ => false,
    }
}
