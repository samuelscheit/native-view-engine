# `nve-backend-skia`

Cross-platform 2D/GPU Skia `StoreBackend` (`packages/backends/skia`, **Backend layer**) supporting CPU raster, Metal (`skia-metal`), Vulkan (`skia-vulkan`), and Direct3D 12 (`skia-d3d`) swapchains with `cosmic-text` shaping and BiDi/grapheme editing.

## Key Public API

- `SkiaBackend`, `DisplayList`, `PaintOp`, `TextFocusState`
- `font::{BitmapFontMetrics, BitmapFontTextMeasurer}`, `text`, `text_bidi`, `text_grapheme`
- `gpu::{GaneshPresenter, NativeSwapchain, metal::MetalSwapchain, vulkan::VulkanSwapchain, d3d12::D3D12Swapchain}`, `raster`, `image_cache`

```rust
use nve_backend_skia::SkiaBackend;
use nve_engine::{StoreBackend, Viewport};

let mut backend = SkiaBackend::new();
backend.set_viewport(1, Viewport { width: 1280.0, height: 720.0, scale_factor: 2.0 });
assert_eq!(backend.viewport().scale_factor, 2.0);
```

## Dependency Invariants

- **Key dependencies**: `nve-abi`, `nve-components`, `nve-engine`, `nve-layout`, `nve-backend-headless`, `cosmic-text`, `unicode-segmentation`, `flate2`, `raw-window-handle`, plus optional `skia-safe` and platform GPU crates (`ash`, `objc2-*`, `windows`).
- **Must NOT depend on**: `dioxus*`, `web-sys`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-backend-skia
```
