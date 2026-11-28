# `nve-canvas-skia`

Thin Skia (`nve-backend-skia`) leaf adapter executing flat `nve.res.draw_list` (`FlatResource`) draw lists directly onto `skia_safe::Canvas` and headless Skia surfaces.

## Role & API

- Exports `register_canvas_skia_system(registry: &mut SkiaTableRegistry)` implementing `SkiaTableSystem` for `Canvas`.

## Invariants

- Leaf backend adapter only; `nve-backend-skia` has zero compile-time dependency on `nve-canvas`.
- Validates draw lists before rasterization with zero panics on malformed payloads.

## Verification

```bash
cargo test -p nve-canvas-skia
```
