# `nve-rpc`

Composable, transport-agnostic RPC contracts, integer-keyed CBOR (`minicbor`) payload codec, typed cache tags, and progressive form decoding for `native-view-engine`.

## Role & API

- **`Procedure` / `EffectKind` (`Query` | `Action`)**: zero-sized typed contracts with explicit stable wire names (`"issues.v1.Issues/List"`), compile-time `ProcId` (`xxh64`), and declarative `procedures!` sugar.
- **`RpcCodec` + `CborCodec`**: pluggable payload codec defaulting to RFC 8949 integer-keyed CBOR (`application/cbor`) with an iterative, zero-allocation ingress validator bounding nesting depth, collection lengths, and total items before decoding.
- **`Tag64` / `CacheTag` / `TagSink`**: zero-allocation cache invalidation keys carried in the standard `Nve-Invalidate` HTTP response header.
- **`Transport` / `call`**: bytes-only transport seam (`MaybeSend` on native and `wasm32`) using plain HTTP status codes and headers with zero custom framing envelope.
- **`FormDecode` / `decode_form_urlencoded_limited`**: bounded zero-dependency decoding for progressive 0-JS HTML `<form>` submissions.

```rust
use nve_rpc::{procedures, DomainStatus, EffectKind, Procedure, Query, Tag64};

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct PingInput {
    #[n(0)]
    pub msg: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct PingOutput {
    #[n(0)]
    pub reply: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
pub enum PingError {
    #[n(0)]
    Empty,
}
impl DomainStatus for PingError {}

procedures! {
    pub query Ping = "demo.v1.Health/Ping" (PingInput) -> PingOutput, PingError;
}

assert_eq!(Ping::NAME, "demo.v1.Health/Ping");
assert_eq!(<Ping as Procedure>::Effect::IS_QUERY, true);
let _tag = Tag64::keyed("ping", 1u64);
```

## Invariants

- Zero `tokio`, `hyper`, `axum`, `dioxus`, `web-sys`, or `serde_json` runtime dependencies; compiles cleanly for native, `wasm32-unknown-unknown`, and `wasm32-wasip2`.
- Pre-decode CBOR validation runs iteratively on a fixed stack array (zero heap allocations, zero recursion stack overflow) and rejects trailing bytes, over-deep nesting, and oversized collection counts before `minicbor::decode`.

## Verification

```bash
cargo test -p nve-rpc
cargo check -p nve-rpc --target wasm32-unknown-unknown
cargo check -p nve-rpc --target wasm32-wasip2
cargo bench -p nve-rpc --no-run
```
