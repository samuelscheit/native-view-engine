# `nve-http-web`

Minimal-WASM browser `FetchTransport`, `BrowserClock`, and `visibilitychange` / `focus` / `online` lifecycle bridge for `nve-rpc` and `nve-query`.

## Role & Design

- **Zero `web-sys`, zero `reqwest`, zero `http`, zero `serde_json`, zero `wasm-bindgen-futures`**: binds directly to `globalThis.fetch`, `globalThis.AbortController`, `ReadableStream`, and `performance.now()` via a compact `#[wasm_bindgen(inline_js = ...)]` helper + 45-line Rust `Promise` future.
- **Streaming byte-cap enforcement before WASM allocation**: checks `Content-Length` and reads response chunks via `response.body.getReader()`, cancelling the stream immediately if `max_response_bytes` is exceeded.
- **Safety & Cancellation**: enforces `POST`, `Content-Type: application/cbor`, `redirect: "error"`, `cache: "no-store"`, configurable `CredentialsMode` (`SameOrigin` default for `HttpOnly; SameSite=Lax` cookies), per-request `AbortController` timeout, and automatic `AbortOnDrop` cancellation when a Rust future is dropped.
- **Single Source of Truth**: feeds raw status + `Nve-Error-Kind` + `Nve-Invalidate` + body straight into `nve_rpc::classify_http_reply`.

```rust
use nve_http_web::{CredentialsMode, FetchConfig, FetchTransport};

let cfg = FetchConfig::same_origin()
    .with_timeout_ms(15_000)
    .with_credentials(CredentialsMode::SameOrigin);
assert_eq!(
    cfg.build_url("issues.v1.Issues/List"),
    "/rpc/issues.v1.Issues/List"
);
let _transport = FetchTransport::with_config(cfg);
```

## Verification

```bash
cargo test -p nve-http-web
cargo clippy -p nve-http-web --target wasm32-unknown-unknown -- -D warnings
```
