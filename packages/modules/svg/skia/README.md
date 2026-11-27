# `nve-svg-skia`

Thin Skia (`nve-backend-skia`) leaf adapter rasterizing `nve-svg` `Svg` nodes onto Skia canvases.

## Role & API

- Exports `register_svg_skia_system(registry: &mut SkiaTableRegistry)` implementing `SkiaTableSystem` for `Svg`.

## Invariants

- Leaf backend adapter only; `nve-backend-skia` remains free of SVG parsing dependencies when unused.
- Handles malformed or empty SVG markup safely without panicking.

## Verification

```bash
cargo test -p nve-svg-skia
```
