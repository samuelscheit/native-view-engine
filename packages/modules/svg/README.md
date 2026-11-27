# `nve-svg`

Pure-spec capability crate defining the `Svg` (`nve.svg`) word table and validated `SvgMarkup` resource.

## Role & API

- Exposes `Svg` (`#[derive(NveTable)]`) with `markup` resource reference and `view_box` geometry.
- Leaf backend adapters live in `web/` and `skia/`.

## Invariants

- Pure specification crate with zero backend or platform dependencies.
- Optional capability module never imported by core crates or backends.

## Verification

```bash
cargo test -p nve-svg
```
