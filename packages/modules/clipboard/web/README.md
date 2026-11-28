# `nve-clipboard-web`

Thin browser `wasm32` leaf provider for `SERVICE_CLIPBOARD` (`0x0010`) implementing `nve_host_services::HostServiceHandler`.

## Key Public API

- `WebClipboard` — synchronous `write_text` + shadow buffer, DOM `copy`/`paste` listeners, and asynchronous `HandlerReply::Pending` completion for `navigator.clipboard.readText()` backed by a `thread_local!` closure slab (zero `unsafe impl Send`, zero leaked closures).
- `WebClipboard::reap_settled_closures` — drops settled `wasm-bindgen` closures safely outside active JS callbacks.

## Dependency Invariants

- Depends only on `nve-abi`, `nve-clipboard`, `nve-host-services`, and `wasm-bindgen` on `wasm32`.
- Zero `web-sys` feature bloat; compact `#[wasm_bindgen(inline_js = ...)]` bindings only.
- Never depended on by `packages/core/*` or `packages/backends/*`.

## Verification

```bash
cargo test -p nve-clipboard-web
```
