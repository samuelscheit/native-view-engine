//! Compile-time MDX component schema, internal route, and heading-anchor validator.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ast::{
    Block, CompiledPage, CompiledSite, ComponentNode, DirectiveBlock, Inline, PropValue, SourcePos,
    normalize_route,
};

/// Expected type of a declarative `.mdx` component prop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropType {
    String,
    Int,
    Float,
    Bool,
    Any,
}

impl PropType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::Any => "any",
        }
    }

    pub fn matches(self, val: &PropValue) -> bool {
        matches!(
            (self, val),
            (Self::Any, _)
                | (Self::String, PropValue::String(_))
                | (Self::Int, PropValue::Int(_))
                | (Self::Float, PropValue::Float(_) | PropValue::Int(_))
                | (Self::Bool, PropValue::Bool(_))
        )
    }
}

/// Schema specification for a single component prop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropSchema {
    pub name: String,
    pub ty: PropType,
    pub required: bool,
}

impl PropSchema {
    pub fn required(name: impl Into<String>, ty: PropType) -> Self {
        Self {
            name: name.into(),
            ty,
            required: true,
        }
    }

    pub fn optional(name: impl Into<String>, ty: PropType) -> Self {
        Self {
            name: name.into(),
            ty,
            required: false,
        }
    }
}

/// Schema specification for an allowed declarative `.mdx` component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSchema {
    pub name: String,
    pub props: Vec<PropSchema>,
    pub allow_children: bool,
    pub allow_unknown_props: bool,
}

impl ComponentSchema {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            props: Vec::new(),
            allow_children: true,
            allow_unknown_props: false,
        }
    }

    pub fn with_prop(mut self, prop: PropSchema) -> Self {
        self.props.push(prop);
        self
    }

    pub fn allow_children(mut self, allow: bool) -> Self {
        self.allow_children = allow;
        self
    }

    pub fn allow_unknown_props(mut self, allow: bool) -> Self {
        self.allow_unknown_props = allow;
        self
    }
}

/// Registry of allowed MDX components and their prop schemas.
#[derive(Debug, Clone, Default)]
pub struct ComponentRegistry {
    components: BTreeMap<String, ComponentSchema>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, schema: ComponentSchema) -> &mut Self {
        self.components.insert(schema.name.clone(), schema);
        self
    }

    pub fn get(&self, name: &str) -> Option<&ComponentSchema> {
        self.components.get(name)
    }

    /// Pre-populated registry with standard `nve-docs` MDX components.
    pub fn with_default_docs_components() -> Self {
        let mut reg = Self::new();
        reg.register(
            ComponentSchema::new("Callout")
                .with_prop(PropSchema::optional("kind", PropType::String))
                .with_prop(PropSchema::optional("title", PropType::String)),
        );
        reg.register(
            ComponentSchema::new("Badge")
                .with_prop(PropSchema::required("label", PropType::String))
                .with_prop(PropSchema::optional("tone", PropType::String))
                .allow_children(false),
        );
        reg.register(
            ComponentSchema::new("Card")
                .with_prop(PropSchema::required("title", PropType::String))
                .with_prop(PropSchema::optional("href", PropType::String))
                .with_prop(PropSchema::optional("description", PropType::String))
                .with_prop(PropSchema::optional("icon", PropType::String)),
        );
        reg.register(
            ComponentSchema::new("Demo")
                .with_prop(PropSchema::required("id", PropType::String))
                .with_prop(PropSchema::optional("hydrate", PropType::String))
                .with_prop(PropSchema::optional("height", PropType::Int))
                .with_prop(PropSchema::optional("title", PropType::String)),
        );
        reg.register(
            ComponentSchema::new("Kbd")
                .with_prop(PropSchema::required("keys", PropType::String))
                .allow_children(false),
        );
        reg
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    UnknownComponent,
    MissingRequiredProp,
    UnknownProp,
    PropTypeMismatch,
    ChildrenNotAllowed,
    BrokenInternalLink,
    BrokenAnchor,
    EmptyTitle,
    DuplicateRoute,
}

/// Actionable compile-time validation diagnostic with 1-based file location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub kind: DiagnosticKind,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Validate a full [`CompiledSite`] against `registry` and cross-page links/anchors.
pub fn validate_site(site: &CompiledSite, registry: &ComponentRegistry) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut routes: HashMap<String, HashSet<String>> = HashMap::new();
    let mut route_owner: HashMap<String, String> = HashMap::new();

    for page in &site.pages {
        if let Some(first_file) = route_owner.get(&page.route) {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                kind: DiagnosticKind::DuplicateRoute,
                file: page.source_path.clone(),
                line: 1,
                column: 1,
                message: format!(
                    "Duplicate route '{}' (already defined by '{}')",
                    page.route, first_file
                ),
            });
        } else {
            route_owner.insert(page.route.clone(), page.source_path.clone());
        }

        if page.title.trim().is_empty() {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                kind: DiagnosticKind::EmptyTitle,
                file: page.source_path.clone(),
                line: 1,
                column: 1,
                message: format!(
                    "Page '{}' has an empty title; set `title` in frontmatter or add a `# Heading`",
                    page.source_path
                ),
            });
        }

        let anchors: HashSet<String> = page.toc.iter().map(|h| h.id.clone()).collect();
        routes.entry(page.route.clone()).or_default().extend(anchors);
    }

    for page in &site.pages {
        validate_blocks(
            &page.blocks,
            page,
            &routes,
            registry,
            &mut diagnostics,
        );
    }

    diagnostics.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.column.cmp(&b.column))
    });
    diagnostics
}

fn validate_blocks(
    blocks: &[Block],
    page: &CompiledPage,
    routes: &HashMap<String, HashSet<String>>,
    registry: &ComponentRegistry,
    out: &mut Vec<Diagnostic>,
) {
    for block in blocks {
        match block {
            Block::Heading { content, .. } | Block::Paragraph { content, .. } => {
                validate_inlines(content, page, routes, registry, out);
            }
            Block::List { items, .. } => {
                for item in items {
                    validate_blocks(&item.blocks, page, routes, registry, out);
                }
            }
            Block::Table { headers, rows, .. } => {
                for h in headers {
                    validate_inlines(h, page, routes, registry, out);
                }
                for row in rows {
                    for cell in row {
                        validate_inlines(cell, page, routes, registry, out);
                    }
                }
            }
            Block::Blockquote { blocks, .. } => {
                validate_blocks(blocks, page, routes, registry, out);
            }
            Block::Directive(dir) => match dir {
                DirectiveBlock::Callout { blocks, .. }
                | DirectiveBlock::Demo { blocks, .. }
                | DirectiveBlock::Custom { blocks, .. } => {
                    validate_blocks(blocks, page, routes, registry, out);
                }
                DirectiveBlock::Tabs { items, .. } => {
                    for item in items {
                        validate_blocks(&item.blocks, page, routes, registry, out);
                    }
                }
                DirectiveBlock::Steps { items, .. } => {
                    for item in items {
                        validate_blocks(&item.blocks, page, routes, registry, out);
                    }
                }
                DirectiveBlock::Cards { items, .. } => {
                    for item in items {
                        if let Some(href) = &item.href {
                            validate_link(href, SourcePos::new(item.line, 1), page, routes, out);
                        }
                        validate_blocks(&item.blocks, page, routes, registry, out);
                    }
                }
            },
            Block::Component(comp) => {
                validate_component(comp, page, routes, registry, out);
            }
            Block::CodeBlock(_) | Block::ThematicBreak { .. } => {}
        }
    }
}

fn validate_inlines(
    inlines: &[Inline],
    page: &CompiledPage,
    routes: &HashMap<String, HashSet<String>>,
    registry: &ComponentRegistry,
    out: &mut Vec<Diagnostic>,
) {
    for inline in inlines {
        match inline {
            Inline::Strong(c) | Inline::Emphasis(c) | Inline::Strikethrough(c) => {
                validate_inlines(c, page, routes, registry, out);
            }
            Inline::Link { text, url, pos, .. } => {
                validate_link(url, *pos, page, routes, out);
                validate_inlines(text, page, routes, registry, out);
            }
            Inline::InlineComponent(comp) => {
                validate_component(comp, page, routes, registry, out);
            }
            Inline::Text(_)
            | Inline::Code(_)
            | Inline::Image { .. }
            | Inline::Autolink { .. }
            | Inline::HardBreak => {}
        }
    }
}

fn validate_component(
    comp: &ComponentNode,
    page: &CompiledPage,
    routes: &HashMap<String, HashSet<String>>,
    registry: &ComponentRegistry,
    out: &mut Vec<Diagnostic>,
) {
    let Some(schema) = registry.get(&comp.name) else {
        out.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            kind: DiagnosticKind::UnknownComponent,
            file: page.source_path.clone(),
            line: comp.pos.line,
            column: comp.pos.column,
            message: format!(
                "Unknown MDX component `<{}>`; register it in ComponentRegistry",
                comp.name
            ),
        });
        validate_blocks(&comp.children, page, routes, registry, out);
        return;
    };

    if !schema.allow_children && !comp.children.is_empty() {
        out.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            kind: DiagnosticKind::ChildrenNotAllowed,
            file: page.source_path.clone(),
            line: comp.pos.line,
            column: comp.pos.column,
            message: format!(
                "Component `<{}>` does not accept children; use self-closing syntax",
                comp.name
            ),
        });
    }

    for req in schema.props.iter().filter(|p| p.required) {
        if comp.prop(&req.name).is_none() {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                kind: DiagnosticKind::MissingRequiredProp,
                file: page.source_path.clone(),
                line: comp.pos.line,
                column: comp.pos.column,
                message: format!(
                    "Component `<{}>` is missing required prop `{}` (expected {})",
                    comp.name,
                    req.name,
                    req.ty.as_str()
                ),
            });
        }
    }

    for prop in &comp.props {
        if let Some(prop_schema) = schema.props.iter().find(|p| p.name == prop.name) {
            if !prop_schema.ty.matches(&prop.value) {
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    kind: DiagnosticKind::PropTypeMismatch,
                    file: page.source_path.clone(),
                    line: prop.pos.line,
                    column: prop.pos.column,
                    message: format!(
                        "Prop `{}` on `<{}>` expects {}, got {}",
                        prop.name,
                        comp.name,
                        prop_schema.ty.as_str(),
                        prop.value.type_name()
                    ),
                });
            }
            if prop.name == "href"
                && let PropValue::String(href) = &prop.value
            {
                validate_link(href, prop.pos, page, routes, out);
            }
        } else if !schema.allow_unknown_props {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                kind: DiagnosticKind::UnknownProp,
                file: page.source_path.clone(),
                line: prop.pos.line,
                column: prop.pos.column,
                message: format!(
                    "Unknown prop `{}` on component `<{}>`",
                    prop.name, comp.name
                ),
            });
        }
    }

    validate_blocks(&comp.children, page, routes, registry, out);
}

fn validate_link(
    url: &str,
    pos: SourcePos,
    page: &CompiledPage,
    routes: &HashMap<String, HashSet<String>>,
    out: &mut Vec<Diagnostic>,
) {
    let trimmed = url.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("mailto:")
        || trimmed.starts_with("tel:")
        || trimmed.starts_with("data:")
    {
        return;
    }

    // Same-page anchor `#heading-id`.
    if let Some(anchor) = trimmed.strip_prefix('#') {
        if !anchor.is_empty() && !page.toc.iter().any(|h| h.id == anchor) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                kind: DiagnosticKind::BrokenAnchor,
                file: page.source_path.clone(),
                line: pos.line,
                column: pos.column,
                message: format!(
                    "Broken heading anchor `#{anchor}` in '{}'",
                    page.source_path
                ),
            });
        }
        return;
    }

    // Internal site route `/foo/bar` or `/foo/bar#anchor`.
    if trimmed.starts_with('/') {
        let (path_part, anchor_part) = match trimmed.split_once('#') {
            Some((p, a)) => (p, Some(a)),
            None => (trimmed, None),
        };
        let target_route = normalize_route(path_part);
        let Some(target_anchors) = routes.get(&target_route) else {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                kind: DiagnosticKind::BrokenInternalLink,
                file: page.source_path.clone(),
                line: pos.line,
                column: pos.column,
                message: format!(
                    "Broken internal link `{trimmed}` (route `{target_route}` does not exist)"
                ),
            });
            return;
        };
        if let Some(anchor) = anchor_part
            && !anchor.is_empty()
            && !target_anchors.contains(anchor)
        {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                kind: DiagnosticKind::BrokenAnchor,
                file: page.source_path.clone(),
                line: pos.line,
                column: pos.column,
                message: format!(
                    "Broken anchor `#{anchor}` on internal route `{target_route}`"
                ),
            });
        }
    }
}
