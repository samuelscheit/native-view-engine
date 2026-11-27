# `nve-tv`

Standalone opt-in TV input module (`packages/modules/tv`, **Capability Module layer**): spatial focus engine, remote / web key normalizer, gamepad repeater, and title-safe overscan calculator for D-Pad, Siri Remote, TV remote, and gamepad navigation.

## Key Public API

- `SpatialFocusEngine` keyed by `nve_store::NodeId` (dense reused arena, explicit `with_next` overrides incl. self-pinning, bounded-LRU per-group focus memory, eligible-only modal traps that pull stray focus in, wrap-around, viewport-first initial focus, `would_admit` pre-check), `SpatialFocusNode`, `SpatialFocusGroupId`, `RemoteInput` / `FocusOutcome`
- `RemoteAction` (`#[repr(u8)]`, dense `code()` / `from_code()`, W3C `key_name()`, `auto_repeats()`), `FocusDirection`, `DecimalDigit`
- `map_web_tv_key` + `WebTvPlatform` (HbbTV / Tizen / webOS / Android TV & Fire TV / Xbox / PlayStation) + `TextInputContext`
- `RepeatThrottle` (non-repeating actions never repeat), `GamepadRepeater` (dominant-axis stick with press / release hysteresis), `SpatialRect`, `ScrollRegion`, `ScrollAdjustment`, `TvOverscanInsets`

```rust
use nve_tv::{
    map_web_tv_key, FocusOutcome, NodeId, RemoteInput, SpatialFocusEngine, SpatialFocusNode,
    SpatialRect, WebKeyInput, WebTvPlatform,
};

let mut engine = SpatialFocusEngine::new();
for i in 0..3 {
    let rect = SpatialRect::new(i as f32 * 120.0, 0.0, 100.0, 80.0);
    engine.register(SpatialFocusNode::new(NodeId::new(i + 1, 0), rect));
}

// A Tizen remote's "Right" arrives as a plain W3C key; Back arrives as legacy keyCode 10009.
let platform = WebTvPlatform::from_user_agent("Mozilla/5.0 (SMART-TV; Linux; Tizen 6.5)");
let right = WebKeyInput { key: "ArrowRight", code: "ArrowRight", key_code: 39, ..Default::default() };
let action = map_web_tv_key(platform, &right).expect("D-Pad key");
let outcome = engine.handle_input(RemoteInput { action, repeat: false, timestamp_us: 0 });
assert_eq!(outcome, FocusOutcome::Moved(NodeId::new(1, 0)));

let back = WebKeyInput { key: "Unidentified", key_code: 10009, ..Default::default() };
assert_eq!(map_web_tv_key(platform, &back), Some(nve_tv::RemoteAction::Back));
```

## Dependency Invariants

- **Depends on**: `nve-store` (for `NodeId`) only.
- **Must NOT depend on**: `nve-backend-*`, `web-sys`, `skia-safe`, `dioxus*`, `serde_json`, or `hosts/*`. Browser glue lives in `packages/modules/tv/web` (`nve-tv-web`).

## Verification

```bash
cargo test -p nve-tv                       # incl. thread-local CountingAlloc 0-alloc guards
cargo bench -p nve-tv --bench tv_bench     # spatial_move_500_focusables, map_web_tv_key
```
