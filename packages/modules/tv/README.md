# `nve-tv`

Standalone opt-in TV spatial focus engine, remote action mapper, and title-safe overscan calculator (`packages/modules/tv`, **Capability Module layer**) for D-Pad, Siri Remote, and gamepad navigation.

## Key Public API

- `SpatialFocusEngine`, `SpatialFocusNode`, `SpatialFocusGroupId`, `SpatialRect`, `ScrollRegion`, `ScrollAdjustment`
- `FocusDirection`, `RemoteAction`, `TvOverscanInsets`

```rust
use nve_tv::{FocusDirection, SpatialFocusEngine};

let mut engine = SpatialFocusEngine::new();
let _next = engine.move_focus(FocusDirection::Right);
```

## Dependency Invariants

- **Depends on**: `std` only (zero third-party or engine dependencies).
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-tv
```
