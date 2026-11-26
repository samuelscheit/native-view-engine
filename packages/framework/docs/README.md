# `nve-docs`

Reusable documentation site framework (`packages/framework/docs`, **Framework layer**) built on `nve-ui`, `nve-router`, `nve-mdx`, `nve-clipboard`, and `nve-system-env` with search modal, sidebar, TOC, and playground registry.

## Key Public API

- `DocsApp`, `create_docs_surface`, `create_docs_surface_at_route`, `create_docs_surface_with_pages`
- `DocsSiteConfig`, `DocsNavSection`, `DocsRoute`, `DocsTheme`, `DocsAppState`
- `DocsShell`, `DocsNavbar`, `DocsSidebar`, `DocsToc`, `Breadcrumbs`, `SearchModal`, `PlaygroundRegistry`

```rust
use nve_docs::{DocsRoute, DocsSiteConfig};

let route = DocsRoute::new("/getting-started");
let _ = (route, DocsSiteConfig::default);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-dioxus`, `nve-interop`, `nve-mdx`, `nve-router`, `nve-host-services`, `nve-clipboard`, `nve-system-env`, `nve-ui`, `dioxus-*`.
- **Must NOT depend on**: `skia-safe`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-docs
```
