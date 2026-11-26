# `nve-backend-web`

Web DOM, SSR, hydration, and WASM `RenderBackend` (`packages/backends/web`, **Backend layer**) implementing Option 5 Hybrid CSS, streaming HTML, event delegation, and browser DOM patching.

## Key Public API

- `WebBackend`, `HydrationReport`, `HydrationMismatch`
- `css::{layout_to_css, style_to_css}`, `dom::{map_a11y_or_input_attr, raw_prop_route}`
- `events::{resolve_delegated_node, ImeCompositionGuard}`, `streaming`, `canvas`, `visual_viewport`

```rust
use nve_backend_web::WebBackend;
use nve_engine::{RenderBackend, Viewport};

let mut backend = WebBackend::new();
backend.set_viewport(Viewport { width: 1024.0, height: 768.0, scale_factor: 1.0 });
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-components`, `nve-engine`, and `wasm32`-gated `wasm-bindgen`/`web-sys` (optional `nve-camera` pending module extraction).
- **Must NOT depend on**: `dioxus*`, `skia-safe`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-backend-web
```
