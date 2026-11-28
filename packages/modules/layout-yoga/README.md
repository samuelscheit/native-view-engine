# `nve-layout-yoga`

Opt-in React Native Yoga C++20 `StoreLayoutEngine` capability module (`packages/modules/layout-yoga`) compiled against `RN_REACT_COMMON_DIR` when present, with a clean `YogaUnavailableError` fallback otherwise. Core crates and backends never depend on it: hosts, examples, and benches inject it through the generic `with_layout(...)` / `RuntimeBuilder::layout(Box<dyn StoreLayoutEngine>)` seams.

## Key Public API

- `YogaLayoutEngine` (implements `nve_layout::StoreLayoutEngine`)
- `YogaUnavailableError`

```rust
use nve_layout_yoga::YogaLayoutEngine;

let engine = YogaLayoutEngine::new();
assert!(engine.is_ok() || engine.is_err());
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-tables`, `nve-engine`, `nve-layout`, `nve-store` (plus C++ Yoga via `build.rs` when `RN_REACT_COMMON_DIR` is set).
- **Must NOT depend on**: backends (outside dev-dependencies), `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, other `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-layout-yoga
```
