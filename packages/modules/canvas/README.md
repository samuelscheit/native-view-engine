# `nve-canvas`

Pure-spec capability crate defining the `Canvas` (`nve.canvas`) word table and compact `NVE_DRAW1` binary draw-list IR (`DrawList`, `DrawListBuilder`, `DrawOp`).

## Role & API

- Exposes `Canvas` (`#[derive(NveTable)]`) referencing a `DrawList` binary resource plus deterministic `DrawList` validation/decoding.
- Leaf backend rasterizers live in `web/` and `skia/`.

## Invariants

- Pure specification crate with zero backend or platform dependencies.
- Bounds-checked, NaN/Inf-rejecting binary draw-list decoder shared across backends.

## Verification

```bash
cargo test -p nve-canvas
```
