# `nve-backend-skia`

Cross-platform 2D/GPU Skia `RenderBackend` (`packages/backends/skia`, **Backend layer**) supporting CPU raster, Metal (`skia-metal`), Vulkan (`skia-vulkan`), and Direct3D 12 (`skia-d3d`) swapchains with `cosmic-text` shaping and BiDi/grapheme editing.

## Key Public API

- `SkiaBackend`, `DisplayList`, `PaintOp`, `TextFocusState`
- `font::{BitmapFontMetrics, BitmapFontTextMeasurer}`, `text`, `text_bidi`, `text_grapheme`
- `gpu::{metal::MetalSwapchain, vulkan::VulkanSwapchain, d3d12::D3D12Swapchain}`, `raster`, `image_cache`

```rust
use nve_backend_skia::SkiaBackend;
use nve_engine::{RenderBackend, Viewport};

let mut backend = SkiaBackend::new();
backend.set_viewport(Viewport { width: 1280.0, height: 720.0, scale_factor: 2.0 });
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-components`, `nve-engine`, `nve-backend-headless`, `cosmic-text`, `unicode-segmentation`, optional `skia-safe`/`nve-layout-yoga`/platform GPU crates.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-backend-skia
```
