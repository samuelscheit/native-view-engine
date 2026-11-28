# `nve-backend-headless`

Pure-Rust headless `StoreBackend` (`packages/backends/headless`, **Backend layer**) generic over any `nve_layout::StoreLayoutEngine` (defaulting to `nve_layout::TaffyStoreLayout`) for fast offscreen layout, hit-testing, and Frame-0 scene verification.

## Key Public API

- `HeadlessBackend<L = nve_layout::TaffyStoreLayout>`
- `InteractionTracker`

```rust
use nve_abi::SurfaceId;
use nve_backend_headless::HeadlessBackend;
use nve_engine::{StoreBackend, Viewport};

let mut backend = HeadlessBackend::new();
backend.set_viewport(SurfaceId::ROOT, Viewport { width: 800.0, height: 600.0, scale_factor: 1.0 });
assert_eq!(backend.viewport().width, 800.0);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-tables`, `nve-engine`, `nve-layout`, `nve-store`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-backend-headless
```
