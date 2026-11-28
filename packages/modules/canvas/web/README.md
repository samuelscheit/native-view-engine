# `nve-canvas-web`

Thin Web (`nve-backend-web`) leaf adapter rendering flat `nve.res.draw_list` (`FlatResource`) draw lists to HTML5 `<canvas>` 2D contexts and SSR attributes.

## Role & API

- Exports `register_canvas_web_system(registry: &mut WebTableRegistry)` implementing `WebTableSystem` for `Canvas`.

## Invariants

- Leaf backend adapter only; `nve-backend-web` has zero compile-time dependency on `nve-canvas`.
- Emits deterministic `<canvas>` markup for zero-mismatch SSR hydration and live DPR-scaled 2D painting in WASM.

## Verification

```bash
cargo test -p nve-canvas-web
```
