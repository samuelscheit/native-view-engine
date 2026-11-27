# `nve-server`

Runtime-agnostic, sans-I/O HTTP RPC server core for `native-view-engine`.

## Role & API

- **`RpcRegistry` / `RpcRegistryBuilder`**: registers `Procedure` handlers with by-value `Cx<S, U>`, compiles a sorted `Box<[(ProcId, Entry)]>` table (panicking on startup hash or name collisions), and dispatches with zero heap allocations on lookup.
- **`CsrfPolicy`**: standalone composable request-security layer enforcing canonical allowed `Origin`s, `Sec-Fetch-Site` fail-closed rules, strict `Content-Type` matching on CBOR RPC, and safe same-origin redirect validation.
- **`AuthResolver`**: unified principal extraction separating `Authorization: Bearer` (ignoring cookies when present) and `HttpOnly; SameSite=Lax` cookies.
- **`Server` / `DirectTransport`**: pure sans-I/O `http::Request<Bytes> -> http::Response<Bytes>` handler runnable on Hyper, Axum (`nve-http-hyper`), in-process tests, and `wasm32-wasip2`.

```rust
use nve_rpc::{procedures, DomainStatus, Procedure};
use nve_server::{AnonymousAuth, CsrfPolicy, Cx, RpcRegistry, Server, ServerConfig};

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct AddReq { #[n(0)] a: i32, #[n(1)] b: i32 }

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct AddRes { #[n(0)] sum: i32 }

procedures! {
    query Add = "math.v1.Calc/Add" (AddReq) -> AddRes, ();
}

let mut reg = RpcRegistry::<(), ()>::builder();
reg.register::<Add, _, _>(|_cx: Cx<(), ()>, req: AddReq| async move {
    Ok(AddRes { sum: req.a + req.b })
});
let _server = Server::new(
    (),
    reg.build(),
    AnonymousAuth,
    CsrfPolicy::allow_origins(["https://app.example.com"]),
    ServerConfig::default(),
);
```

## Invariants

- Zero `tokio`, `hyper`, `axum`, `dioxus`, or `serde_json` runtime dependencies; compiles cleanly to `wasm32-wasip2` and `wasm32-unknown-unknown`.
- `Server` is `Clone + Send + Sync + 'static` on native targets so `tower_service::Service` wrappers mount cleanly in Axum/Hyper.

## Verification

```bash
cargo test -p nve-server
cargo check -p nve-server --target wasm32-wasip2
cargo bench -p nve-server --no-run
```
