# `nve-layout-yoga`

Opt-in React Native Yoga C++20 `LayoutEngine` adapter (`packages/core/layout-yoga`, **Core Optional Layout layer**) compiled against `RN_REACT_COMMON_DIR` when present, with a clean `YogaUnavailableError` fallback otherwise.

## Key Public API

- `YogaLayoutEngine` (implements `nve_engine::LayoutEngine`)
- `YogaUnavailableError`

```rust
use nve_layout_yoga::YogaLayoutEngine;

let engine = YogaLayoutEngine::new();
assert!(engine.is_ok() || engine.is_err());
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-components`, `nve-engine` (plus C++ Yoga via `build.rs` when `RN_REACT_COMMON_DIR` is set).
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-layout-yoga
```
