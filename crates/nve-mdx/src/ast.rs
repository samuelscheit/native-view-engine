//! Core AST, frontmatter, and compiled-site types for `nve-mdx`.

use std::collections::{BTreeMap, HashMap};

use crate::highlight::HighlightLine;
use crate::nav::{NavTree, PrevNext};
use crate::search::SearchIndex;

/// 1-based line and column location in a source document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePos {
    pub line: usize,
    pub column: usize,
}

impl SourcePos {
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

impl Default for SourcePos {
    fn default() -> Self {
        Self { line: 1, column: 1 }
    }
}

/// Typed frontmatter metadata parsed from `---` (or `+++`) headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub description: Option<String>,
    pub order: Option<i64>,
    pub badge: Option<String>,
    pub icon: Option<String>,
    pub toc: bool,
    pub draft: bool,
    pub canonical: Option<String>,
    pub tags: Vec<String>,
    /// Custom string metadata map (plus stringified custom scalars).
    pub meta: BTreeMap<String, String>,
    /// Custom string-array metadata fields.
    pub arrays: BTreeMap<String, Vec<String>>,
}

impl Default for Frontmatter {
    fn default() -> Self {
        Self {
            title: None,
            description: None,
            order: None,
            badge: None,
            icon: None,
            toc: true,
            draft: false,
            canonical: None,
            tags: Vec::new(),
            meta: BTreeMap::new(),
            arrays: BTreeMap::new(),
        }
    }
}

impl Frontmatter {
    /// Look up a frontmatter key as a string (used by `{frontmatter.<key>}` interpolation).
    pub fn resolve_str(&self, key: &str) -> Option<String> {
        match key {
            "title" => self.title.clone(),
            "description" => self.description.clone(),
            "order" => self.order.map(|v| v.to_string()),
            "badge" => self.badge.clone(),
            "icon" => self.icon.clone(),
            "toc" => Some(self.toc.to_string()),
            "draft" => Some(self.draft.to_string()),
            "canonical" => self.canonical.clone(),
            "tags" if !self.tags.is_empty() => Some(self.tags.join(", ")),
            other => self
                .meta
                .get(other)
                .cloned()
                .or_else(|| self.arrays.get(other).map(|arr| arr.join(", "))),
        }
    }

    /// Look up a frontmatter key as a typed [`PropValue`].
    pub fn resolve_prop(&self, key: &str) -> Option<PropValue> {
        match key {
            "order" => self.order.map(PropValue::Int),
            "toc" => Some(PropValue::Bool(self.toc)),
            "draft" => Some(PropValue::Bool(self.draft)),
            _ => self.resolve_str(key).map(PropValue::String),
        }
    }
}

/// Table-of-contents heading entry extracted from a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocHeading {
    pub level: u8,
    pub id: String,
    pub text: String,
    pub line: usize,
}

/// Column alignment in a GFM table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAlign {
    None,
    Left,
    Center,
    Right,
}

/// Fenced code block AST node with pre-computed syntax highlighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    pub lang: Option<String>,
    pub title: Option<String>,
    pub highlight_lines: Vec<usize>,
    pub show_line_numbers: bool,
    pub live: bool,
    pub meta: String,
    pub code: String,
    pub highlighted: Vec<HighlightLine>,
    pub line: usize,
}

/// Extracted live RSX/Rust code snippet (` ```rsx live `).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveBlock {
    pub id: String,
    pub lang: String,
    pub title: Option<String>,
    pub code: String,
    pub line: usize,
}

/// Single list item (with optional GFM task-list checkbox state).
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
    pub line: usize,
}

/// Semantic kind of a `:::callout` directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutKind {
    Tip,
    Info,
    Warning,
    Danger,
    Note,
}

impl CalloutKind {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "tip" => Self::Tip,
            "warning" | "warn" | "caution" => Self::Warning,
            "danger" | "error" | "important" => Self::Danger,
            "note" => Self::Note,
            _ => Self::Info,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tip => "tip",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Danger => "danger",
            Self::Note => "note",
        }
    }
}

/// Hydration strategy for `:::demo` interactive islands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrateStrategy {
    Visible,
    Interaction,
    Load,
    Idle,
}

impl HydrateStrategy {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "interaction" => Self::Interaction,
            "load" => Self::Load,
            "idle" => Self::Idle,
            _ => Self::Visible,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Interaction => "interaction",
            Self::Load => "load",
            Self::Idle => "idle",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabItem {
    pub label: String,
    pub blocks: Vec<Block>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepItem {
    pub title: String,
    pub blocks: Vec<Block>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CardItem {
    pub title: String,
    pub href: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub blocks: Vec<Block>,
    pub line: usize,
}

/// Fenced `:::` directive AST variants.
#[derive(Debug, Clone, PartialEq)]
pub enum DirectiveBlock {
    Callout {
        kind: CalloutKind,
        title: Option<String>,
        blocks: Vec<Block>,
        line: usize,
    },
    Tabs {
        items: Vec<TabItem>,
        line: usize,
    },
    Steps {
        items: Vec<StepItem>,
        line: usize,
    },
    Cards {
        items: Vec<CardItem>,
        line: usize,
    },
    Demo {
        id: String,
        hydrate: HydrateStrategy,
        height: Option<u32>,
        title: Option<String>,
        blocks: Vec<Block>,
        line: usize,
    },
    Custom {
        name: String,
        attrs: BTreeMap<String, String>,
        blocks: Vec<Block>,
        line: usize,
    },
}

/// Typed prop value for declarative `.mdx` `<Component ... />` tags.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl PropValue {
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::Bool(_) => "bool",
        }
    }
}

/// Single component prop with its source location.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentProp {
    pub name: String,
    pub value: PropValue,
    pub pos: SourcePos,
}

/// Declarative `.mdx` component invocation (self-closing or paired).
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentNode {
    pub name: String,
    pub props: Vec<ComponentProp>,
    pub children: Vec<Block>,
    pub self_closing: bool,
    pub pos: SourcePos,
}

impl ComponentNode {
    /// Look up a prop's typed value by name.
    pub fn prop(&self, name: &str) -> Option<&PropValue> {
        self.props.iter().find(|p| p.name == name).map(|p| &p.value)
    }
}

/// Block-level AST node.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        id: String,
        content: Vec<Inline>,
        text: String,
        line: usize,
    },
    Paragraph {
        content: Vec<Inline>,
        line: usize,
    },
    CodeBlock(CodeBlock),
    List {
        ordered: bool,
        start: usize,
        items: Vec<ListItem>,
        line: usize,
    },
    Table {
        alignments: Vec<TableAlign>,
        headers: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        line: usize,
    },
    Blockquote {
        blocks: Vec<Block>,
        line: usize,
    },
    ThematicBreak {
        line: usize,
    },
    Directive(DirectiveBlock),
    Component(ComponentNode),
}

/// Inline-level AST node.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Code(String),
    Link {
        text: Vec<Inline>,
        url: String,
        title: Option<String>,
        pos: SourcePos,
    },
    Image {
        alt: String,
        url: String,
        title: Option<String>,
        pos: SourcePos,
    },
    Autolink {
        url: String,
        pos: SourcePos,
    },
    HardBreak,
    InlineComponent(ComponentNode),
}

/// Compiled Markdown / MDX page.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledPage {
    pub route: String,
    pub source_path: String,
    pub frontmatter: Frontmatter,
    pub title: String,
    pub description: String,
    pub toc: Vec<TocHeading>,
    pub blocks: Vec<Block>,
    pub live_blocks: Vec<LiveBlock>,
    pub plain_text: String,
}

/// Full compiled documentation site.
#[derive(Debug, Clone)]
pub struct CompiledSite {
    pub pages: Vec<CompiledPage>,
    pub nav: NavTree,
    pub prev_next: BTreeMap<String, PrevNext>,
    pub search_index: SearchIndex,
}

impl CompiledSite {
    /// Look up a compiled page by normalized route (e.g. `"/guides/routing"`).
    pub fn page(&self, route: &str) -> Option<&CompiledPage> {
        let norm = normalize_route(route);
        self.pages.iter().find(|p| p.route == norm)
    }
}

/// GitHub-style deterministic heading slugifier.
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.trim().chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
            prev_dash = false;
        } else if (ch == ' ' || ch == '-' || ch == '_') && !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "section".to_string()
    } else {
        out
    }
}

/// Stateful slug generator that disambiguates duplicates (`slug`, `slug-1`, `slug-2`, ...).
#[derive(Debug, Default)]
pub struct SlugGenerator {
    seen: HashMap<String, usize>,
}

impl SlugGenerator {
    pub fn next_slug(&mut self, heading_text: &str) -> String {
        let base = slugify(heading_text);
        if !self.seen.contains_key(&base) {
            self.seen.insert(base.clone(), 0);
            return base;
        }
        let mut count = self.seen[&base] + 1;
        let mut candidate = format!("{base}-{count}");
        while self.seen.contains_key(&candidate) {
            count += 1;
            candidate = format!("{base}-{count}");
        }
        self.seen.insert(base, count);
        self.seen.insert(candidate.clone(), 0);
        candidate
    }
}

/// Normalize an internal site route to start with `/` and have no trailing `/` (except `"/"`).
pub fn normalize_route(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    let without_query = trimmed.split(['?', '#']).next().unwrap_or("");
    let cleaned = without_query.trim_matches('/');
    if cleaned.is_empty() {
        "/".to_string()
    } else {
        format!("/{cleaned}")
    }
}
