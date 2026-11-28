# `nve-canvas-skia`

Thin Skia (`nve-backend-skia`) leaf adapter executing direct `DirectCanvasRegistry` callbacks and flat `nve.res.draw_list` (`FlatResource`) draw lists through unified `SkiaCanvasPainter` (`GuardedPainter<SkiaPixelSink>`) and, under the optional `skia` feature, direct GPU `SkiaNativeCanvasPainter` (`GuardedPainter<SkiaNativeSink>`).

## Role & API

- Exports `CanvasSkiaSystem::{new, with_direct, direct}` implementing `SkiaTableSystem` (`paint_node`, `remove_node`, `reset`, and `#[cfg(feature = "skia")]` `prefers_skia_canvas` / `paint_skia_canvas`) for `Canvas`.
- Exports `SkiaCanvasPainter` and `SkiaPixelSink` for software rendering and cached static recorded canvases.
- Under `--features skia`, exports `SkiaNativeCanvasPainter` and `SkiaNativeSink` to execute direct-mode canvases straight onto `&skia_safe::Canvas` without CPU `PixelBuffer` rasterization or GPU texture uploads, while static recorded canvases remain GPU-texture cached.

## Invariants

- Leaf backend adapter only; `nve-backend-skia` has zero compile-time dependency on `nve-canvas`, and default features have zero `skia-safe` dependency.
- Validates and sanitizes all geometry and clip stacks identically across direct (`SkiaPixelSink` / `SkiaNativeSink`) and recorded paths with zero allocations on steady-state painting.

## Verification

```bash
cargo test -p nve-canvas-skia
cargo test -p nve-canvas-skia --features skia
```
