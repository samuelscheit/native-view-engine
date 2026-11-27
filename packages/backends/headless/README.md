# `nve-backend-headless`

Pure-Rust headless `StoreBackend` (`packages/backends/headless`, **Backend layer**) generic over any `StoreLayoutEngine` (defaulting to `TaffyStoreLayout`) for fast offscreen layout, hit-testing, and Frame-0 scene verification.

## Key Public API

- `HeadlessBackend<L = TaffyStoreLayout>`, `TaffyStoreLayout`
- `InteractionTracker`, `prerender_native_frame0`, `prerender_native_frame0_with_text_measurer`

```rust
use nve_abi::SurfaceId;
use nve_backend_headless::HeadlessBackend;
use nve_engine::{StoreBackend, Viewport};

let mut backend = HeadlessBackend::new();
backend.set_viewport(SurfaceId::ROOT, Viewport { width: 800.0, height: 600.0, scale_factor: 1.0 });
assert_eq!(backend.viewport().width, 800.0);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-components`, `nve-engine`, `taffy`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-backend-headless
```
