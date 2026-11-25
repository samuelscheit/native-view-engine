//! Interactive playground registry and built-in `nve-mdx` component schema registry.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use dioxus_core::Element;
use nve_mdx::{ComponentRegistry, ComponentSchema, PropSchema, PropType, normalize_route};

/// Metadata describing an interactive live Dioxus playground embedded inside docs pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaygroundMeta {
    /// Unique identifier referenced by `:::demo{id="..."}` or `<LiveDemo id="..." />`.
    pub id: String,
    /// Display title shown in the playground header bar.
    pub title: String,
    /// Optional short subtitle or caption.
    pub description: Option<String>,
    /// Highlighted source code shown when switching to the `Code` tab.
    pub source_code: String,
    /// Syntax highlighting language tag (default `"rust"`).
    pub language: String,
    /// Optional default preview height in logical points.
    pub default_height: Option<u32>,
}

impl PlaygroundMeta {
    /// Create metadata for an interactive playground with Rust source preview by default.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        source_code: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
            source_code: source_code.into(),
            language: "rust".to_string(),
            default_height: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    pub fn with_default_height(mut self, height: u32) -> Self {
        self.default_height = Some(height);
        self
    }
}

/// Thread-safe Dioxus factory closure for rendering a live interactive playground.
pub type PlaygroundRenderer = Arc<dyn Fn() -> Element + Send + Sync>;

/// Registered interactive playground entry combining metadata and its Dioxus renderer.
#[derive(Clone)]
pub struct PlaygroundEntry {
    pub meta: PlaygroundMeta,
    pub render: PlaygroundRenderer,
}

impl fmt::Debug for PlaygroundEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlaygroundEntry")
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

/// Registry mapping `demo_id` strings to live interactive Dioxus playgrounds.
///
/// Applications such as `examples/nve-docs-site` register interactive demos once and share
/// the `Arc<PlaygroundRegistry>` across Web SSR/hydration and native Skia/AppKit/UIKit hosts.
#[derive(Default, Clone)]
pub struct PlaygroundRegistry {
    entries: BTreeMap<String, PlaygroundEntry>,
}

impl fmt::Debug for PlaygroundRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlaygroundRegistry")
            .field("ids", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl PlaygroundRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a live playground under `meta.id`, returning `&mut Self` for chaining.
    pub fn register<F>(&mut self, meta: PlaygroundMeta, render: F) -> &mut Self
    where
        F: Fn() -> Element + Send + Sync + 'static,
    {
        let id = meta.id.clone();
        self.entries.insert(
            id,
            PlaygroundEntry {
                meta,
                render: Arc::new(render),
            },
        );
        self
    }

    /// Convenience registration taking `(id, title, source_code, renderer)`.
    pub fn register_simple<F>(
        &mut self,
        id: impl Into<String>,
        title: impl Into<String>,
        source_code: impl Into<String>,
        render: F,
    ) -> &mut Self
    where
        F: Fn() -> Element + Send + Sync + 'static,
    {
        self.register(PlaygroundMeta::new(id, title, source_code), render)
    }

    /// Look up a registered playground by `demo_id`.
    pub fn get(&self, id: &str) -> Option<&PlaygroundEntry> {
        self.entries.get(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }
}

/// Thread-safe Dioxus factory closure for rendering a full-bleed custom site route (such as `/`
/// landing pages or `/examples` galleries) inside [`crate::DocsShell`].
pub type CustomPageRenderer = Arc<dyn Fn() -> Element + Send + Sync>;

/// Registry mapping normalized application routes (e.g. `"/"`, `"/examples"`) to custom full-bleed
/// Dioxus page renderers that share the global navbar, theme toggle, and `⌘K` search modal.
#[derive(Default, Clone)]
pub struct CustomPageRegistry {
    entries: BTreeMap<String, CustomPageRenderer>,
}

impl fmt::Debug for CustomPageRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomPageRegistry")
            .field("routes", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl CustomPageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a custom page renderer at `route` (normalized via `normalize_route`).
    pub fn register<F>(&mut self, route: impl AsRef<str>, render: F) -> &mut Self
    where
        F: Fn() -> Element + Send + Sync + 'static,
    {
        self.entries
            .insert(normalize_route(route.as_ref()), Arc::new(render));
        self
    }

    /// Look up a custom page renderer by route.
    pub fn get(&self, route: &str) -> Option<&CustomPageRenderer> {
        self.entries.get(&normalize_route(route))
    }

    /// Returns `true` when a custom page renderer is registered for `route`.
    pub fn contains(&self, route: &str) -> bool {
        self.entries.contains_key(&normalize_route(route))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn routes(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }
}

/// Build the strict [`nve_mdx::ComponentRegistry`] covering all built-in `nve-docs` MDX components.
///
/// Registered components:
/// `Callout`, `Tabs`, `Tab`, `Steps`, `Step`, `Cards`, `Card`, `LiveDemo` (plus `Demo` alias),
/// `Badge`, `Kbd`, `ApiTable`, and `CodeGroup`.
pub fn default_component_registry() -> ComponentRegistry {
    let mut reg = ComponentRegistry::new();

    reg.register(
        ComponentSchema::new("Callout")
            .with_prop(PropSchema::optional("kind", PropType::String))
            .with_prop(PropSchema::optional("title", PropType::String))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("Tabs")
            .with_prop(PropSchema::optional("default", PropType::String))
            .with_prop(PropSchema::optional("id", PropType::String))
            .with_prop(PropSchema::optional("group", PropType::String))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("Tab")
            .with_prop(PropSchema::required("label", PropType::String))
            .with_prop(PropSchema::optional("value", PropType::String))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("Steps")
            .with_prop(PropSchema::optional("start", PropType::Int))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("Step")
            .with_prop(PropSchema::required("title", PropType::String))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("Cards")
            .with_prop(PropSchema::optional("cols", PropType::Int))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("Card")
            .with_prop(PropSchema::required("title", PropType::String))
            .with_prop(PropSchema::optional("href", PropType::String))
            .with_prop(PropSchema::optional("description", PropType::String))
            .with_prop(PropSchema::optional("icon", PropType::String))
            .with_prop(PropSchema::optional("badge", PropType::String))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("LiveDemo")
            .with_prop(PropSchema::required("id", PropType::String))
            .with_prop(PropSchema::optional("title", PropType::String))
            .with_prop(PropSchema::optional("hydrate", PropType::String))
            .with_prop(PropSchema::optional("height", PropType::Int))
            .with_prop(PropSchema::optional("code", PropType::String))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("Demo")
            .with_prop(PropSchema::required("id", PropType::String))
            .with_prop(PropSchema::optional("title", PropType::String))
            .with_prop(PropSchema::optional("hydrate", PropType::String))
            .with_prop(PropSchema::optional("height", PropType::Int))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("Badge")
            .with_prop(PropSchema::required("label", PropType::String))
            .with_prop(PropSchema::optional("tone", PropType::String))
            .with_prop(PropSchema::optional("variant", PropType::String))
            .allow_children(false),
    );

    reg.register(
        ComponentSchema::new("Kbd")
            .with_prop(PropSchema::required("keys", PropType::String))
            .allow_children(false),
    );

    reg.register(
        ComponentSchema::new("ApiTable")
            .with_prop(PropSchema::optional("title", PropType::String))
            .with_prop(PropSchema::optional("caption", PropType::String))
            .allow_children(true),
    );

    reg.register(
        ComponentSchema::new("CodeGroup")
            .with_prop(PropSchema::optional("title", PropType::String))
            .with_prop(PropSchema::optional("default", PropType::String))
            .allow_children(true),
    );

    reg
}
