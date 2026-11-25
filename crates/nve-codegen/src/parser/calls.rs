use super::*;

impl Parser {
    pub(super) fn parse_codegen_native_component_call(&mut self) -> Result<ComponentCall, CodegenError> {
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

    pub(super) fn parse_component_options_object(&mut self) -> Result<ComponentOptions, CodegenError> {
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

    pub(super) fn parse_codegen_native_commands_call(&mut self) -> Result<CommandsCall, CodegenError> {
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

    pub(super) fn try_parse_turbomodule_call(&mut self) -> Result<Option<TurboModuleCall>, CodegenError> {
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

}

pub(super) fn parse_number_type_expr(raw: &str, line: usize, col: usize) -> Result<TypeExpr, CodegenError> {
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

pub(super) fn unwrap_readonly(expr: &TypeExpr) -> &TypeExpr {
    match expr {
        TypeExpr::Named { name, args }
            if matches!(name.as_str(), "Readonly" | "$ReadOnly") && args.len() == 1 =>
        {
            unwrap_readonly(&args[0])
        }
        other => other,
    }
}

pub(super) fn unwrap_nullable_wrappers(expr: &TypeExpr) -> &TypeExpr {
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

pub(super) fn strip_nullability(expr: &TypeExpr) -> (&TypeExpr, bool) {
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

pub(super) fn extract_default_value(expr: &TypeExpr) -> Option<DefaultValue> {
    match expr {
        TypeExpr::Null | TypeExpr::Undefined => Some(DefaultValue::Null),
        TypeExpr::BoolLit(b) => Some(DefaultValue::Bool(*b)),
        TypeExpr::IntLit(i) => Some(DefaultValue::Int(*i)),
        TypeExpr::FloatLit(f) => Some(DefaultValue::Float(*f)),
        TypeExpr::StringLit(s) => Some(DefaultValue::String(s.clone())),
        _ => None,
    }
}

pub(super) fn is_element_ref(expr: &TypeExpr) -> bool {
    match expr {
        TypeExpr::Named { name, .. } => name.contains("ElementRef") || name.contains("HostInstance"),
        _ => false,
    }
}
