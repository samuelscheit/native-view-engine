# `nve-backend-headless`

Pure-Rust headless `RenderBackend` (`packages/backends/headless`, **Backend layer**) generic over any `LayoutEngine` (defaulting to `TaffyLayoutEngine`) for fast offscreen layout, hit-testing, and Frame-0 scene verification.

## Key Public API

- `HeadlessBackend<L = TaffyLayoutEngine>`, `HeadlessNode`, `HeadlessMutation`
- `TaffyLayoutEngine`, `to_taffy_style`, `to_taffy_style_with_ext`, `NodeSlotMap`
- `InteractionTracker`, `prerender_native_frame0`, `prerender_native_frame0_with_text_measurer`

```rust
use nve_backend_headless::HeadlessBackend;
use nve_engine::{RenderBackend, Viewport};

let mut backend = HeadlessBackend::new();
backend.set_viewport(Viewport { width: 800.0, height: 600.0, scale_factor: 1.0 });
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-components`, `nve-engine`, `taffy`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-backend-headless
```
