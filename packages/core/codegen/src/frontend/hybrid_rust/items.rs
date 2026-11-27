use super::*;

impl Parser {
    pub(super) fn parse_docs_and_attrs(&mut self) -> Result<(Option<String>, Vec<ParsedAttr>), CodegenError> {
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

    pub(super) fn parse_attr_body(&mut self, line: usize, col: usize) -> Result<ParsedAttr, CodegenError> {
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

    pub(super) fn parse_attr_value(&mut self) -> Result<AttrValue, CodegenError> {
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

    pub(super) fn skip_visibility(&mut self) -> Result<(), CodegenError> {
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

    pub(super) fn skip_use_stmt(&mut self) -> Result<(), CodegenError> {
        while !matches!(self.peek().kind, TokKind::Semi | TokKind::Eof) {
            self.bump();
        }
        self.expect(&TokKind::Semi, "';' after use statement")?;
        Ok(())
    }

    pub(super) fn parse_file(&mut self) -> Result<HybridFileSpec, CodegenError> {
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
                    let explicit_interface_id =
                        attr_u32_arg(attr, "interface_id")?.map(nve_abi::InterfaceId);
                    let c_name = attr_string_arg(attr, "c_name");
                    file.modules.push(self.parse_module_trait(
                        explicit_name,
                        explicit_interface_id,
                        c_name,
                        doc,
                    )?);
                } else if let Some(attr) = view_attr {
                    let explicit_name = attr_string_arg(attr, "name");
                    let explicit_interface_id =
                        attr_u32_arg(attr, "interface_id")?.map(nve_abi::InterfaceId);
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

    pub(super) fn parse_const(&mut self, doc: Option<String>) -> Result<HybridConstSpec, CodegenError> {
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

    pub(super) fn parse_alias(&mut self, doc: Option<String>) -> Result<HybridAliasSpec, CodegenError> {
        let name = self.expect_ident("type alias")?;
        self.expect(&TokKind::Eq, "'='")?;
        let target = self.parse_type()?;
        self.expect(&TokKind::Semi, "';'")?;
        Ok(HybridAliasSpec { name, target, doc })
    }

    pub(super) fn parse_enum(&mut self, doc: Option<String>) -> Result<HybridEnumSpec, CodegenError> {
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

    pub(super) fn parse_struct(
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

}
