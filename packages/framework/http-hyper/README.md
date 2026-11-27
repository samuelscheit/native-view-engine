# `nve-http-hyper`

Thin Tokio + Hyper HTTP/1.1 leaf adapter for `nve-server` and `nve-rpc` (intentionally HTTP/1.1-only so `h2` does not pull the workspace-banned `fnv` crate; reverse proxies terminate TLS/HTTP/2 in production and Slice 2 adds native QUIC/HTTP/3 over `nve-wire`).

## Role & API

- **`NveHyperService`** (`server` feature): wraps `nve_server::Server` in a `Clone + Send + Sync + 'static` service implementing both `hyper::service::Service` and `tower_service::Service`, running route/CSRF/auth preflight BEFORE reading any body bytes. Use `NveHyperService::new(server)` for full paths or `axum::Router::route_service("/rpc/{*rest}", ...)`, and `NveHyperService::nested(server)` for `axum::Router::nest_service("/rpc", ...)`.
- **`collect_body_limited` / `collect_body_limited_timeout`**: enforces streaming per-frame byte limits (`413 Payload Too Large`) and body read deadlines (`408 Request Timeout`) with zero heap allocation on single-frame bodies.
- **`serve_service_with_shutdown` / `serve_service_with_config` / `serve_rpc_with_shutdown`**: runs Hyper HTTP/1.1 over a `tokio::net::TcpListener` with `TCP_NODELAY`, header-read timeout, connection concurrency semaphore, transient `accept()` error backoff, and `GracefulShutdown` connection draining.
- **`HyperTransport`** (`client` feature): native plain HTTP/1.1 (`http://`) `nve_rpc::Transport` with `TCP_NODELAY`, connect/request timeouts, bounded idle pool timer, and response byte caps.

```rust,no_run
use nve_http_hyper::NveHyperService;
use nve_server::{AnonymousAuth, CsrfPolicy, RpcRegistry, Server, ServerConfig};

let server = Server::new(
    (),
    RpcRegistry::<(), ()>::builder().build(),
    AnonymousAuth,
    CsrfPolicy::allow_origins(["http://127.0.0.1:8080"]),
    ServerConfig::default(),
);
let _svc = NveHyperService::new(server);
```

## Invariants

- Zero `serde_json` or runtime `axum` dependency (`axum` is a dev-dependency solely to verify `Router::nest_service` and `Router::route_service` mountability).
- Preflight rejects `405`/`404`/`403`/`415`/`401` before polling a single body frame; streaming body cap + timeout abort oversized or stalled payloads immediately.

## Verification

```bash
cargo test -p nve-http-hyper
cargo bench -p nve-http-hyper --bench http_hyper_bench -- --quick
```
