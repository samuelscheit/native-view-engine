# `nve-hash`

Pure `no_std` compile-time and streaming 64-bit XXH64 hashing crate for `native-view-engine`.

## Role & API

- Provides `pub const fn xxh64(bytes: &[u8], seed: u64) -> u64`, `const_id`, `Key64`, and streaming `Xxh64Hasher`.
- Shared XXH64 implementation used by `nve-rpc` (`ProcId`, `Tag64`) and the upcoming unified `NodeStore` / `NVE2` hash layer (`packages/core/engine/src/hash.rs` remains until that branch merges).

## Invariants

- Strictly `#![no_std]` with zero dependencies and a single canonical XXH64 algorithm matching the official XXH64 specification.
- Golden vectors locked against the canonical XXH64 reference specification across seed 0 and seeded inputs.

## Verification

```bash
cargo test -p nve-hash
```
