# `nve-ssr`

Dedicated `tokio::task::LocalSet` streaming SSR orchestrator for `native-view-engine` web applications. Bridges `!Send` Dioxus `VirtualDom` instances (`nve-dioxus`), `QuerySuspense` (`nve-query-dioxus`), `NVEQ` v2 incremental query dehydration (`nve-query`), Declarative Shadow DOM out-of-order slot deltas (`nve-backend-web`), and backpressured Hyper `HttpBody` responses (`nve-http-hyper`).

## Role & API

- **`SsrLocalPool`** (`Clone`): Fixed pool of OS worker threads, each owning a single-threaded `current_thread` Tokio runtime and `LocalSet` that interleaves concurrent streaming SSR requests via `spawn_local`, dispatched round-robin with non-panicking bounded backpressure (`SsrError::PoolOverloaded` / `SsrError::PoolClosed`) and non-blocking `Drop`.
- **`SsrLocalPool::render_streaming`**: Runs Frame 1 pre-headers shell rendering and Declarative Shadow DOM placement hygiene validation before emitting HTTP headers:
  - **Adaptive fast path**: When zero `<QuerySuspense>` slots suspend, returns buffered `Response<HttpBody::Full>` Light-DOM HTML (no shadow root), optional `seq=0` `NVEQ` v2 hydration script, `<template data-nve-stream-end></template>`, and optional async module boot script.
  - **Streaming path**: Returns `Response<HttpBody::Stream>` (`ChannelBody`) immediately with the DSD shell, `seq=0` (`StreamPending` + settled entries) strictly before `<script type="module" async>`, and streams out-of-order atomic `[NVEQ delta + ack + slot delta CSS/HTML]` chunks with keepalive heartbeats, `total_deadline` recovery, and immediate `VirtualDom`/RPC cancellation when the client disconnects.
- **`SsrPageOptions`**, **`StreamPlan`**, **`SsrLocalPoolConfig`**, and **`generate_csp_nonce()`**: Per-request CSP nonce generation (16 CSPRNG bytes via `getrandom::fill`, Standard Base64), strict zero-inline-JS default (`enable_event_replay: false`), and safe HTML attribute/script escaping.

```rust,no_run
use nve_ssr::{generate_csp_nonce, SsrLocalPool, SsrPageOptions, StreamPlan};

let _pool = SsrLocalPool::new().expect("spawn SSR pool");
let nonce = generate_csp_nonce().expect("CSPRNG nonce");
let _page = SsrPageOptions::new("Issues")
    .with_style_nonce(nonce)
    .with_modulepreload_src("/assets/app.js")
    .with_async_boot_src("/assets/app.js");
let _plan = StreamPlan::default();
```

## Invariants

- Zero `serde_json` and zero runtime `axum` coupling (enforced mechanically by `arch_invariants::crate_dag`).
- DSD placement hygiene (`<form>` ancestors, table/select foster-parenting traps, nested slots) fails closed with `SsrError::Streaming(StreamingError::InvalidSlotPlacement(_))` strictly BEFORE headers are sent.
- Every per-slot chunk emits its `NVEQ` v2 `<script type="application/x-nve-query">` + `<template data-nve-q-ack>` strictly BEFORE the matching `<div slot="nve-slot-...">` inside one atomic `Bytes` frame, with zero duplicate CSS rules or `NVEQ` entries.
- Client TCP disconnect closes `ChannelBody` -> `tx.closed()` aborts the `spawn_local` task and drops `VirtualDom` and all in-flight RPC futures immediately.

## Verification

```bash
cargo test --locked -p nve-ssr
cargo clippy --locked -p nve-ssr --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps -p nve-ssr
cargo bench --locked -p nve-ssr --bench ssr_bench -- --quick
cargo test --locked -p nve-cli --test arch_invariants
```
