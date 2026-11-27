# `nve-blur-web`

Thin Web (`nve-backend-web`) leaf adapter for `nve-blur` rendering CSS `backdrop-filter: blur(...)` surfaces.

## Role & API

- Exports `register_blur_web_system(registry: &mut WebTableRegistry)` implementing `WebTableSystem` for `BlurSurface`.

## Invariants

- Leaf backend adapter only; depends on `nve-blur` and `nve-backend-web` without leaking into core crates.
- Supports both SSR HTML/CSS generation and live WASM DOM patching.

## Verification

```bash
cargo test -p nve-blur-web
```
