# `nve-interop`

Universal framework-compatibility orchestration layer (`packages/core/interop`, **Core Interop layer**) providing typed channels, gesture arbitration, intrinsic layout negotiation, multi-window sync, plugin hosting, and local-first CRDT reconciliation.

## Key Public API

- `InteropRuntime`, `InteropError`, `surface`, `window`
- `channel::{NveChannelBus, ChannelId, ChannelMessage, CHANNEL_ID_NAVIGATION, CHANNEL_ID_THEME}`
- `gesture::{GestureArbitrator, ArbitrationDecision}`, `layout::{LayoutNegotiator, SizeConstraints}`, `plugin_host::NvePluginHost`, `crdt`

```rust
use nve_interop::{ChannelId, NveChannelBus};

let mut bus = NveChannelBus::new();
let _ch: ChannelId = 1;
let _ = &mut bus;
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-tables`, `nve-engine`, `serde`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-interop
```
