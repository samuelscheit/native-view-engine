# `nve-hash`

Pure `no_std` compile-time and streaming 64-bit XXH64 hashing crate for `native-view-engine`.

## Role & API

- Provides `pub const fn xxh64(bytes: &[u8], seed: u64) -> u64`, `const_id`, `Key64`, and streaming `Xxh64Hasher`.
- Single source of truth for compile-time `TableKey`, `ResourceKind`, `NVE_CORE_ABI_SCHEMA_HASH`, `NVE_SCENE_SCHEMA_HASH`, and `nve-rpc` (`ProcId`, `Tag64`) digests across Rust and generated foreign host bindings.

## Invariants

- Strictly `#![no_std]` with zero dependencies and a single canonical XXH64 implementation across Rust and foreign bindings.
- Golden vectors locked against the canonical XXH64 reference specification and C++/Swift/Dart/Kotlin/TypeScript host mirrors.

## Verification

```bash
cargo test --locked -p nve-hash
```
