# `nve-backend-web`

Web DOM, SSR, hydration, and WASM `StoreBackend` (`packages/backends/web`, **Backend layer**) implementing Option 5 Hybrid CSS, streaming HTML, event delegation, and browser DOM patching.

## Key Public API

- `WebBackend`, `HydrationReport`, `HydrationMismatch`, `SsrOutput`, `SsrDocumentOptions`
- Crate-root re-exports: `layout_to_css`, `style_to_css`, `ImeCompositionGuard`, `KeyboardInsets`
- Public submodules: `canvas`, `native_view`, `streaming`

```rust
use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_engine::{StoreBackend, Viewport};

let mut backend = WebBackend::new();
backend.set_viewport(SurfaceId::ROOT, Viewport { width: 1024.0, height: 768.0, scale_factor: 1.0 });
assert_eq!(backend.viewport().width, 1024.0);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-tables`, `nve-engine`, `nve-store`, `base64`, and `wasm32`-gated `wasm-bindgen`/`web-sys`.
- **Must NOT depend on**: `dioxus*`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-backend-web
```
