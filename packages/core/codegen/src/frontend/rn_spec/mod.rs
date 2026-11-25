//! Recursive-descent parser for React Native Fabric `*NativeComponent.{ts,js}`
//! and TurboModule `Native*.{ts,js}` specifications (both TypeScript and Flow syntax).

mod calls;
pub mod lexer;
mod lowering;
mod syntax;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::error::CodegenError;
use crate::schema::export::{
    CommandSpec, ComponentOptions, DefaultValue, EventBubbling, EventSpec, ObjectField, ParamSpec,
    ParsedRnSpec, PropSpec, PropType, RnComponentSpec, TurboModuleMethod, TurboModuleSpec,
};
use lexer::{tokenize, Token, TokenKind};

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
            if let Some(target_rel) = detect_reexport_target(&source)
                && let Some(resolved) = resolve_relative_spec(path, &target_rel)
            {
                return parse_rn_spec_file_inner(&resolved, depth + 1);
            }
            Err(orig_err)
        }
    }
}

fn detect_reexport_target(source: &str) -> Option<String> {
    let tokens = tokenize(source).ok()?;
    for (i, tok) in tokens.iter().enumerate() {
        if matches!(&tok.kind, TokenKind::Ident(s) if s == "from")
            && let Some(Token {
                kind: TokenKind::StringLit(rel),
                ..
            }) = tokens.get(i + 1)
            && rel.starts_with('.')
        {
            return Some(rel.clone());
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

const MAX_PARSE_DEPTH: usize = 128;

struct Parser {
    tokens: Vec<Token>,
    eof: Token,
    pos: usize,
    depth: usize,
    type_aliases: HashMap<String, TypeExpr>,
    interfaces: HashMap<String, InterfaceDecl>,
    component_call: Option<ComponentCall>,
    commands_call: Option<CommandsCall>,
    turbomodule_call: Option<TurboModuleCall>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        let eof = tokens
            .last()
            .filter(|t| matches!(t.kind, TokenKind::Eof))
            .cloned()
            .unwrap_or(Token {
                kind: TokenKind::Eof,
                line: 1,
                col: 1,
            });
        Self {
            tokens,
            eof,
            pos: 0,
            depth: 0,
            type_aliases: HashMap::new(),
            interfaces: HashMap::new(),
            component_call: None,
            commands_call: None,
            turbomodule_call: None,
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

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&self.eof)
    }

    fn peek_skip_docs(&self) -> &Token {
        let mut idx = self.pos;
        while let Some(tok) = self.tokens.get(idx) {
            if !matches!(tok.kind, TokenKind::DocComment(_)) {
                return tok;
            }
            idx += 1;
        }
        &self.eof
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
        &self.eof
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

            if self.at_ident("TurboModuleRegistry")
                && let Some(call) = self.try_parse_turbomodule_call()?
            {
                self.turbomodule_call = Some(call);
                continue;
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

        if self.at_ident("TurboModuleRegistry")
            && let Some(call) = self.try_parse_turbomodule_call()?
        {
            self.turbomodule_call = Some(call);
            self.skip_to_statement_end();
            return Ok(());
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
            if self.at_ident("TurboModuleRegistry")
                && let Some(call) = self.try_parse_turbomodule_call()?
            {
                self.turbomodule_call = Some(call);
                self.skip_to_statement_end();
                return Ok(());
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
            if self.at_ident("TurboModuleRegistry")
                && let Some(call) = self.try_parse_turbomodule_call()?
            {
                self.turbomodule_call = Some(call);
                continue;
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

}
