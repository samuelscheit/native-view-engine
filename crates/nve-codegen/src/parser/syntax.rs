use super::*;
use super::calls::parse_number_type_expr;

impl Parser {
    pub(super) fn parse_interface_decl(&mut self) -> Result<InterfaceDecl, CodegenError> {
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

    pub(super) fn parse_type_alias(&mut self) -> Result<(), CodegenError> {
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

    pub(super) fn parse_object_body(&mut self) -> Result<(Vec<String>, Vec<RawMember>), CodegenError> {
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

    pub(super) fn parse_param_list(&mut self) -> Result<Vec<RawParam>, CodegenError> {
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

    pub(super) fn parse_type_expr(&mut self) -> Result<TypeExpr, CodegenError> {
        self.parse_union_type()
    }

    pub(super) fn parse_union_type(&mut self) -> Result<TypeExpr, CodegenError> {
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

    pub(super) fn parse_intersection_type(&mut self) -> Result<TypeExpr, CodegenError> {
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

    pub(super) fn parse_postfix_type(&mut self) -> Result<TypeExpr, CodegenError> {
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

    pub(super) fn parse_primary_type(&mut self) -> Result<TypeExpr, CodegenError> {
        self.enter_depth()?;
        let res = self.parse_primary_type_inner();
        self.exit_depth();
        res
    }

    pub(super) fn parse_primary_type_inner(&mut self) -> Result<TypeExpr, CodegenError> {
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

    pub(super) fn parse_paren_or_fn_type(&mut self) -> Result<TypeExpr, CodegenError> {
        let saved = self.pos;
        if let Ok(params) = self.parse_param_list()
            && self.eat(&TokenKind::FatArrow)
        {
            let ret = self.parse_type_expr()?;
            return Ok(TypeExpr::Function {
                params,
                ret: Box::new(ret),
            });
        }
        // Backtrack and parse as parenthesized type
        self.pos = saved;
        self.expect(&TokenKind::LParen, "'('")?;
        let inner = self.parse_type_expr()?;
        self.expect(&TokenKind::RParen, "')'")?;
        Ok(inner)
    }

    pub(super) fn parse_qualified_name(&mut self) -> Result<String, CodegenError> {
        let mut name = self.expect_ident("type name")?;
        while self.eat(&TokenKind::Dot) {
            let seg = self.expect_ident("qualified name segment")?;
            name.push('.');
            name.push_str(&seg);
        }
        Ok(name)
    }

    pub(super) fn skip_balanced_angles(&mut self) {
        let mut depth = 1usize;
        while !self.at_eof() && depth > 0 {
            match self.next_tok().kind {
                TokenKind::LAngle => depth += 1,
                TokenKind::RAngle => depth -= 1,
                _ => {}
            }
        }
    }

    pub(super) fn skip_balanced_brackets(&mut self) {
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

}
