//! Shared fixture site and headless test harness for `nve-docs` integration tests.

use std::sync::Arc;

use nve_abi::{
    EventPayload, NativeEvent, NativeEventKind, NodeId, RawPropValue, pack_key_modifiers,
};
use nve_backend_headless::HeadlessBackend;
use nve_dioxus::DioxusSurface;
use nve_dioxus::dioxus_elements::backend::{RenderBackend, Viewport};
use nve_dioxus::prelude::*;
use nve_docs::{
    DocsNavSection, DocsSiteConfig, DocsViewport, PlaygroundMeta, PlaygroundRegistry,
    create_docs_surface_with_viewport, default_component_registry,
};
use nve_clipboard::MemoryClipboard;
use nve_host_services::{HostServiceProvider, MemoryHostServices};
use nve_mdx::{CompileOptions, CompiledSite, SiteSource, compile_site_with_validation};
use nve_system_env::MemorySystemEnv;

pub const RUST_SNIPPET: &str =
    "fn main() {\n    let surface = create_docs_surface();\n    println!(\"ready\");\n}";

#[component]
fn LiveCounterDemo() -> Element {
    let mut count = use_signal(|| 0i32);
    rsx! {
        view {
            test_id: "demo-counter-root",
            style: "flex-direction: row; align-items: center; gap: 12px;",
            text {
                test_id: "demo-counter-value",
                "Count: {count}"
            }
            pressable {
                test_id: "demo-counter-inc",
                accessibility_role: "button",
                onpress: move |_| count += 1,
                text { "+1" }
            }
        }
    }
}

pub fn build_fixture_site() -> Arc<CompiledSite> {
    let mut src = SiteSource::new();
    src.insert(
        "_meta.toml",
        r#"
index = { title = "Overview", badge = "ABI v5" }
guides = { title = "Guides", badge = "New" }
"#,
    );
    src.insert(
        "guides/_meta.toml",
        r#"
quickstart = { title = "Quickstart", badge = "Zero-Copy" }
components = "Interactive Components"
"#,
    );
    src.insert(
        "index.mdx",
        format!(
            r#"---
title: "Native View Engine Docs"
description: "Single binary IR across Web SSR, Desktop Skia, and Native Mobile hosts."
badge: "ABI v5"
tags: ["rust", "zero-copy"]
---

# Native View Engine Docs

Welcome to **native-view-engine** — explore the [Quickstart Guide](/guides/quickstart) or inspect the architecture.

## Architecture Overview

Every frame crosses the FFI boundary as one flat `CommandBuffer`.

| Target | Backend | Overhead |
| :--- | :---: | ---: |
| Web | WASM DOM | Binary ABI |
| Desktop | Skia GPU | Batched |

### Binary Pipeline

```rust title="main.rs" {{2}} showLineNumbers
{RUST_SNIPPET}
```

:::callout{{kind="tip" title="Zero-Copy Guarantee"}}
No per-prop FFI crossings or JSON serialization anywhere on the hot path.
:::

:::tabs
::tab{{label="Cargo"}}
Run `cargo test --workspace` to verify every crate.
::tab{{label="CMake"}}
Run `ctest --test-dir cxx/build` for the C++20 Fabric bridge.
:::

<LiveDemo id="counter" title="Interactive Counter" hydrate="interaction" height={{160}} />
"#
        ),
    );
    src.insert(
        "guides/quickstart.mdx",
        r#"---
title: "Quickstart"
description: "Launch your first cross-platform NVE surface in 60 seconds."
---

# Quickstart

Follow the steps below to boot a surface and jump to [Interactive Components](/guides/components).

## Install Toolchain

:::steps
::step{title="Build workspace"}
Run `cargo build --workspace`.
::step{title="Run Skia shell"}
Execute `cargo run -p nve-skia-shell`.
:::

## Next Steps

:::cards
::card{title="Components" href="/guides/components" description="Callouts, tabs, and live playgrounds"}
:::
"#,
    );
    src.insert(
        "guides/components.mdx",
        r#"---
title: "Interactive Components"
description: "Built-in MDX components validated at compile time."
---

# Interactive Components

Check out the <Badge label="Zero-Copy" tone="success" /> badge and press <Kbd keys="⌘K" /> to search!

## Quantum Telemetry Index

Unique searchable token `quantum-flux-capacitor` lives exclusively on this components reference page.

<Callout kind="warning" title="Strict Schema">
All props are validated by `default_component_registry()`.
</Callout>
"#,
    );

    let registry = default_component_registry();
    let (site, diagnostics) =
        compile_site_with_validation(&src, &CompileOptions::default(), &registry);
    assert!(
        diagnostics.is_empty(),
        "fixture MDX site must validate cleanly, got diagnostics: {diagnostics:#?}"
    );
    Arc::new(site)
}

pub fn build_config() -> DocsSiteConfig {
    DocsSiteConfig::new("NVE Docs")
        .with_tagline("Zero-copy native & web UI engine")
        .with_brand_mark("NVE")
        .with_github_repo("https://github.com/example/native-view-engine")
        .with_edit_base_url("https://github.com/example/native-view-engine/edit/main/docs")
        .with_sections(vec![
            DocsNavSection::new("overview", "Overview", "/", "/"),
            DocsNavSection::new("guides", "Guides", "/guides", "/guides/quickstart")
                .with_badge("v5"),
        ])
}

pub fn build_playgrounds() -> Arc<PlaygroundRegistry> {
    let mut reg = PlaygroundRegistry::new();
    reg.register(
        PlaygroundMeta::new(
            "counter",
            "Interactive Counter",
            "let mut count = use_signal(|| 0);\nrsx! { button { onclick: move |_| count += 1, \"{count}\" } }",
        )
        .with_default_height(140),
        || rsx! { LiveCounterDemo {} },
    );
    Arc::new(reg)
}

pub struct DocsHarness {
    pub surface: DioxusSurface,
    pub backend: HeadlessBackend,
    pub services: Arc<MemoryHostServices>,
    frame_id: u64,
    ts: u64,
}

impl DocsHarness {
    pub fn mount(route: &str, viewport: DocsViewport) -> Self {
        let services = Arc::new(MemoryHostServices::new());
        services.register(MemoryClipboard::new());
        services.register(MemorySystemEnv::new());
        let host: Arc<dyn HostServiceProvider> = services.clone();
        let surface = create_docs_surface_with_viewport(
            1,
            route,
            viewport,
            build_config(),
            build_fixture_site(),
            build_playgrounds(),
            host,
        );
        let mut backend = HeadlessBackend::new();
        backend.set_viewport(
            1,
            Viewport {
                width: viewport.width,
                height: viewport.height,
                scale_factor: 1.0,
            },
        );
        let mut h = Self {
            surface,
            backend,
            services,
            frame_id: 0,
            ts: 0,
        };
        h.render();
        h
    }

    pub fn render(&mut self) -> Vec<u8> {
        self.frame_id += 1;
        let (_, _, bytes) = self.surface.render_frame(self.frame_id);
        self.backend
            .submit_frame(&bytes)
            .expect("headless submit_frame");
        bytes
    }

    pub fn maybe_find(&self, tid: &str) -> Option<NodeId> {
        self.backend
            .nodes()
            .values()
            .find_map(|n| match n.raw_props.get("testID") {
                Some(RawPropValue::String(s)) if s == tid => Some(n.id),
                _ => None,
            })
    }

    pub fn find(&self, tid: &str) -> NodeId {
        self.maybe_find(tid).unwrap_or_else(|| {
            panic!(
                "expected node with testID='{tid}' in tree:\n{}",
                self.backend.render_ascii_tree()
            )
        })
    }

    pub fn find_prefix(&self, prefix: &str) -> NodeId {
        self.backend
            .nodes()
            .values()
            .find_map(|n| match n.raw_props.get("testID") {
                Some(RawPropValue::String(s)) if s.starts_with(prefix) => Some(n.id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected node with testID prefix '{prefix}'"))
    }

    pub fn raw_str(&self, id: NodeId, key: &str) -> Option<&str> {
        match self.backend.node(id)?.raw_props.get(key) {
            Some(RawPropValue::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn collect_text(&self, id: NodeId) -> String {
        let Some(node) = self.backend.node(id) else {
            return String::new();
        };
        let mut out = String::new();
        if let Some(t) = node.text.as_deref() {
            out.push_str(t);
        }
        for &cid in &node.children {
            out.push_str(&self.collect_text(cid));
        }
        out
    }

    pub fn text_of(&self, tid: &str) -> String {
        let id = self.find(tid);
        self.collect_text(id)
    }

    fn next_ts(&mut self) -> u64 {
        self.ts += 1_000;
        self.ts
    }

    pub fn press(&mut self, tid: &str) {
        let id = self.find(tid);
        self.press_id(id);
    }

    pub fn press_id(&mut self, id: NodeId) {
        let ts = self.next_ts();
        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 8.0, y: 8.0 },
        }]);
        assert!(handled, "press on node {id} should be handled");
        self.render();
    }

    pub fn type_text(&mut self, tid: &str, value: &str) {
        let id = self.find(tid);
        let ts = self.next_ts();
        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(value.to_string()),
        }]);
        assert!(handled, "text change on '{tid}' should be handled");
        self.render();
    }

    pub fn keydown(&mut self, tid: &str, key: &str, meta: bool, ctrl: bool, shift: bool, alt: bool) {
        let id = self.find(tid);
        let ts = self.next_ts();
        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::KeyDown,
            payload: EventPayload::Key {
                key: key.to_string(),
                code: String::new(),
                key_code: 0,
                modifiers: pack_key_modifiers(meta, ctrl, shift, alt, false),
                is_composing: false,
            },
        }]);
        assert!(handled, "keydown '{key}' on '{tid}' should be handled");
        self.render();
    }
}
