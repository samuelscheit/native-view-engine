# `nve-store`

Data-oriented word-table `NodeStore`, transactional `Transaction`/`ChangeSet` commits, and the compact 4-byte-aligned `NVE2` wire codec.

## Role & API

- `NodeStore`: dense 32-byte cache-line-aligned `NodeRow` storage, dense/sparse `u32` word columns (`NveTable` + `#[derive(NveTable)]` with sealed `RowRef<'a>` / `RowWriter<'a>`), sealed UTF-8 `StrArena`, typed `Span<T>`, `ResourceStore`, and `RealizationOrder`.
- `wire::{WireEncoder, WireDecoder}`: table-major `NVE2` binary wire framing with zero-allocation steady-state buffer reuse.

## Invariants

- Framework-agnostic (zero `dioxus-core` or platform backend dependencies).
- Zero steady-state heap allocations proven by `tests/zero_alloc.rs` (`CountingAlloc`).
- `#[derive(NveTable)]` `Str` getters return `&'a str` directly via sealed `RowRef<'a>` (O(1) unchecked over the live store's validated UTF-8 arena); detached `StrArena::str(span)` validates UTF-8 safely on arbitrary spans.

## Verification

```bash
cargo test -p nve-store && cargo bench -p nve-store --bench store_bench
```
