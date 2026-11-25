use super::*;
use super::calls::*;

impl Parser {
    pub(super) fn build_ir(&self) -> Result<ParsedRnSpec, CodegenError> {
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

    pub(super) fn build_component_ir(&self, call: &ComponentCall) -> Result<RnComponentSpec, CodegenError> {
        let props_type_name = call
            .props_type
            .clone()
            .or_else(|| {
                if self.interfaces.contains_key("NativeProps")
                    || self.type_aliases.contains_key("NativeProps")
                {
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

    pub(super) fn collect_props_members(&self, type_name: &str) -> Result<(bool, Vec<RawMember>), CodegenError> {
        let mut extends_view_props = false;
        let mut members = Vec::new();
        let mut visited = HashSet::new();
        self.collect_named_members(type_name, &mut extends_view_props, &mut members, &mut visited)?;
        Ok((extends_view_props, members))
    }

    pub(super) fn collect_named_members(
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

    pub(super) fn collect_expr_members(
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
                if matches!(name.as_str(), "Readonly" | "$ReadOnly" | "Partial")
                    && let Some(inner) = args.first()
                {
                    return self.collect_expr_members(inner, extends_view_props, out, visited);
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

    pub(super) fn try_resolve_event(
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

    pub(super) fn resolve_prop(
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

    pub(super) fn resolve_commands(&self) -> Vec<CommandSpec> {
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

    pub(super) fn build_turbomodule_ir(&self, call: &TurboModuleCall) -> Result<TurboModuleSpec, CodegenError> {
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

    pub(super) fn resolve_params(&self, params: &[RawParam]) -> Result<Vec<ParamSpec>, CodegenError> {
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

    pub(super) fn resolve_type_expr(
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

