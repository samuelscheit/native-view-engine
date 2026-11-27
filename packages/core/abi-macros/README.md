# `nve-abi-macros`

Zero-dependency proc-macro crate (`packages/core/abi-macros`, **Core Foundation layer**) implementing `#[derive(NveAbi)]` for single-source-of-truth C ABI schema metadata and compile-time layout assertions (`size_of`, `align_of`, `offset_of!`).

## Key Public API

- `#[derive(NveAbi)]` (`derive_nve_abi`) with struct/enum attributes `#[nve_abi(size = N, size_64 = N, align = N)]` and field attributes `#[nve_abi(offset = N, offset64 = N, c_type = "...")]`.

```rust,ignore
// Expanded against `nve_abi::schema` in downstream crates (verified by `nve-abi` schema_derive tests).
use nve_abi::NveAbi;

#[derive(NveAbi)]
#[repr(C)]
#[nve_abi(size = 8, align = 4)]
pub struct Point { pub x: f32, pub y: f32 }
```

## Dependency Invariants

- **Depends on**: `proc_macro` only (zero third-party crate dependencies for minimal compile time).
- **Must NOT depend on**: `syn`, `quote`, `dioxus*`, `serde_json`, `web-sys`, `skia-safe`, or any `packages/modules/*` crate.

## Verification

```bash
cargo test -p nve-abi-macros
```
