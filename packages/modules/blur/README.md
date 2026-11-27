# `nve-blur`

Pure-spec capability crate defining the `BlurSurface` (`nve.blur_surface`) word table and `BlurMaterial` enum.

## Role & API

- Exposes `BlurSurface` (`#[derive(NveTable)]`) with `material`, `intensity`, and optional `tint_rgba`.
- Platform leaf adapters live in sibling subdirectories (`web/`, `apple/`, `windows/`, `flutter/`).

## Invariants

- Pure specification crate with zero backend or platform crate dependencies (`web-sys`, `skia-safe`, etc.).
- Optional module: core crates and backends never depend on `nve-blur`.

## Verification

```bash
cargo test -p nve-blur
```
