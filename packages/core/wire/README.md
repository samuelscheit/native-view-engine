# `nve-wire`

Unified QUIC (`nve-wire-2`), HTTP/3 WebTransport (`h3`), and TLS WebSocket (`wss://`) Happy-Eyeballs transport (`packages/core/wire`, **Core Wire Transport layer**) built on `nve-wire-core`.

## Feature Split

- `client` (default): `NveWireClient`, `ReconnectingWireClient` (single `current_thread` Tokio runtime + exponential backoff `100ms..=5s` + automatic `KeyframeRequest` recovery), SHA-256 cert pinning — embedded directly inside `nve-runtime`.
- `server` (default): `NveWireServer`, `NveDevCertificate` (`rcgen`), Bearer token generation (`getrandom`) — used by `nve pair` CLI and dev servers.

## Key Public API

- `NveWireServer`, `NveWireClient`, `ReconnectingWireClient`, `auth::{generate_bearer_token, verify_bearer_token}`
- `tls`, `framing`, `reconnect`, plus full re-export of `nve_wire_core::*`

```rust
use nve_wire::{generate_bearer_token, verify_bearer_token};

let token = generate_bearer_token();
assert!(verify_bearer_token(&token, &token));
```

## Dependency Invariants

- **Key dependencies**: `nve-wire-core`, `quinn`, `web-transport-quinn`, `rustls`, `tokio`, `tokio-rustls`, `tokio-tungstenite`, `futures-util`, `subtle`, `bytes`, `crossbeam-channel`, plus optional `url` (`client`) and `rcgen`/`base64`/`getrandom` (`server`).
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-wire
cargo check -p nve-wire --no-default-features --features client
cargo check -p nve-wire --no-default-features --features server
```
