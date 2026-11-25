use super::*;

impl Parser {
    pub(super) fn parse_module_trait(
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

    pub(super) fn parse_view_trait(
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

    pub(super) fn parse_fn_signature(&mut self, doc: Option<String>) -> Result<HybridMethodSpec, CodegenError> {
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

    pub(super) fn parse_type(&mut self) -> Result<HybridType, CodegenError> {
        self.enter_depth()?;
        let res = self.parse_type_inner();
        self.exit_depth();
        res
    }

    pub(super) fn parse_type_inner(&mut self) -> Result<HybridType, CodegenError> {
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

        // Fixed-size array types: `[T; N]`
        if self.eat(&TokKind::LBracket) {
            let elem = self.parse_type()?;
            self.expect(&TokKind::Semi, "';'")?;
            let len_tok = self.bump();
            let len = match len_tok.kind {
                TokKind::Int(v, _) => usize::try_from(v).map_err(|_| {
                    CodegenError::parse(
                        len_tok.line,
                        len_tok.col,
                        "fixed array length exceeds usize::MAX",
                    )
                })?,
                _ => {
                    return Err(CodegenError::parse(
                        len_tok.line,
                        len_tok.col,
                        "expected integer length after ';' in fixed array type",
                    ));
                }
            };
            self.expect(&TokKind::RBracket, "']'")?;
            return Ok(HybridType::FixedArray {
                elem: Box::new(elem),
                len,
            });
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

