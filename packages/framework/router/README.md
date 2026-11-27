# `nve-router`

Type-safe stack, tab, modal, and deep-link router (`packages/framework/router`, **Framework layer**) for `nve-dioxus`, bridged to `nve-interop::NveChannelBus` (`CHANNEL_ID_NAVIGATION = 4`) via compact binary packets.

## Key Public API

- `Routable`, `normalize_path`, `Navigator`, `Link`, `Outlet`, `use_navigator`, `use_route`, `use_tab_navigator`
- `guard::{NavigationGuard, GuardChain, GuardDecision}`, `deeplink::{DeepLinkConfig, DeepLinkMatch}`
- `query::{QueryParams, percent_encode, percent_decode}`, `channel::{encode_nav_packet, decode_nav_packet}`

```rust
use nve_router::normalize_path;

assert_eq!(normalize_path("/docs/guide/").as_deref(), Some("/docs/guide"));
assert_eq!(normalize_path("/docs//guide/"), None);
```

## Dependency Invariants

- **Depends on**: `nve-dioxus`, `nve-interop`, `dioxus-*`, and `wasm32`-gated `wasm-bindgen`/`web-sys`.
- **Must NOT depend on**: `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-router
```
