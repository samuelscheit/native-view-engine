# `nve-wire`

Unified QUIC (`nve-wire-2`), HTTP/3 WebTransport (`h3`), and TLS WebSocket (`wss://`) Happy-Eyeballs transport plus C ABI (`packages/core/wire`, **Core Wire Transport layer**) built on `nve-wire-core`.

## Key Public API

- `NveWireServer`, `NveWireClient`, `auth::{generate_bearer_token, verify_bearer_token}`
- `tls`, `framing`, `c_api`, plus full re-export of `nve_wire_core::*`

```rust
use nve_wire::{generate_bearer_token, verify_bearer_token};

let token = generate_bearer_token();
assert!(verify_bearer_token(&token, &token));
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-engine`, `nve-wire-core`, and native async/TLS/QUIC transport crates (`quinn`, `rustls`, `tokio`, `tokio-tungstenite`).
- **Must NOT depend on**: `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-wire
```
