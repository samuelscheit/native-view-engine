# `nve-arch-invariants`

Standalone mechanical architectural invariant verifier for the `native-view-engine` workspace (L2 hybrid linting layer alongside workspace Clippy lints, `clippy.toml`, `ast-grep`, and `cargo-deny`).

## Role

- Enforces workspace dependency DAG boundaries using `guppy` and real `target-spec` `cfg(...)` evaluation (`wasm32` dev-dependency gating, framework-agnostic core, zero `packages/** -> hosts/**`, module root spec purity).
- Enforces cross-file Rust AST invariants via `syn` + `proc-macro2` (duplicate internal enums, redundant NVE-to-NVE converters, mirrored `<->` match pairs, compact discriminant matches, core string tables, cross-file `#[repr(C)]` layout/cache-line locks, and typed domain fields).
- Enforces repository hygiene (`docs/BACKLOG.md` open-only, generated-file naming & zero tracked artifacts, personal path/endpoint ratchet, crate `README.md` + doctest hooks, 650-line production file ratchet across `packages/` and `hosts/`, and per-crate benchmark / `CountingAlloc` coverage).

## Verification

```bash
cargo test --locked -p nve-arch-invariants
NVE_REPORT_INVARIANTS=1 cargo test --locked -p nve-arch-invariants -- --nocapture
```
