# `nve-query`

Runtime-agnostic, sans-I/O data-oriented query & mutation cache for `native-view-engine`.

## Role & Architecture

- **Hot/Cold Dense Slab (`QueryMeta` 64B aligned + `QueryPayload`)**: 1 cache line per slot for sweeps, generational handles (`QueryHandle`), intrusive hash-collision chaining over `(ProcId, xxh64(encoded_args))` with exact `Bytes` equality verification, and zero-clone `Shared<T>` (`Arc<T>` on native, `Rc<T>` on `wasm32`) typed reads.
- **O(k) `Tag64` Index & O(1) Wildcard `*` Epoch**: flat free-list `TagEdge` pool with doubly-linked tag lists (zero per-tag `Vec` heap allocations) and monotonic `wildcard_epoch` invalidation.
- **Deduplication & Race Protection**: shares one in-flight fetch across N subscribers, bumps per-slot `fetch_gen` on invalidation/mutation so slow pre-mutation replies can never clobber fresh state, and supports structural sharing via `raw_body_hash`.
- **Rebaseable Layered Optimistic Updates**: separates server-committed truth from ordered `OptimisticLayer` patches so concurrent overlapping mutations roll back cleanly without clobbering sibling mutations or flickering on mid-flight refetches.
- **Zero-Encode Hydration Seam**: preserves `CallReply::raw_body` for compact binary `dehydrate_bytes` / `hydrate_bytes` without a global type registry.

```rust
use nve_query::{ManualClock, QueryClient, QueryClientConfig, QueryOptions, Shared};
use nve_rpc::{procedures, BoxFuture, CborCodec, RawCall, RawReply, ReplyKind, RpcCodec, Transport, TransportError};
use bytes::Bytes;

procedures! {
    query Echo = "demo.v1.Echo/Ping" (u64) -> u64, ();
}

struct MockTransport;
impl Transport for MockTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        Box::pin(async move {
            Ok(RawReply {
                http_status: 200,
                kind: ReplyKind::Ok,
                invalidated_tags: Vec::new(),
                invalidate_all: false,
                body: req.body,
            })
        })
    }
}

let client = QueryClient::with_clock(
    MockTransport,
    CborCodec::default(),
    Shared::new(ManualClock::new(1_000)),
    QueryClientConfig::default(),
);
let handle = client.subscribe::<Echo>(&42, QueryOptions::default()).unwrap();
let snap = client.read::<Echo>(handle);
assert!(snap.is_pending());
client.unsubscribe(handle);
```

## Invariants

- Strictly sans-I/O: zero `dioxus`, `web-sys`, `wasm-bindgen`, `tokio`, `http`, or `serde_json` dependencies. Compiles cleanly on native, `wasm32-unknown-unknown`, and `wasm32-wasip2`.
- Interior lock is NEVER held across `.await` or user callbacks.

## Verification

```bash
cargo test -p nve-query
cargo check -p nve-query --target wasm32-unknown-unknown
cargo check -p nve-query --target wasm32-wasip2
cargo bench -p nve-query --no-run
```
