# `nve-mdx`

Zero-dependency pure-Rust Markdown & declarative `.mdx` compiler, syntax highlighter, `_meta.toml` navigation builder, compile-time validator, and binary full-text search index (`packages/framework/mdx`, **Framework layer**).

## Key Public API

- `CompileOptions`, `compile_document`, `compile_site`, `compile_site_with_validation`
- `ast::{CompiledPage, CompiledSite, Block, Frontmatter}`, `frontmatter::parse_frontmatter`
- `highlight::{highlight_code, Language, TokenKind}`, `nav::{build_navigation, NavTree}`, `search::SearchIndex`, `codegen::generate_rust_module`

```rust
use nve_mdx::{compile_document, CompileOptions};

let page = compile_document("/intro", "# Hello\n\nBody.", &CompileOptions::default()).unwrap();
assert!(!page.blocks.is_empty());
```

## Dependency Invariants

- **Depends on**: `std` only (zero third-party crate dependencies).
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-mdx
```
