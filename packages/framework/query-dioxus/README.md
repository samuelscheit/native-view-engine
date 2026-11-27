# `nve-query-dioxus`

Dioxus 0.7 reactive integration for `nve-query` and `nve-ui`.

## Role & Architecture

- **Transport-Erased Context (`ErasedQueryClient` / `DynTransport`)**: shared UI components write `use_query::<ListIssues>(filter)` and `use_action::<CreateIssue>()` once with zero `<T: Transport>` generics across Web WASM (`FetchTransport`), Native Desktop/Mobile (`HyperTransport`), and SSR (`DirectTransport`).
- **Dense Recycled Slot Signals (`DioxusQueryBridge`)**: maintains `Vec<Option<(Signal<u32>, u32)>>` indexed by `QuerySlotId`, allocated in `QueryClientProvider`'s scope and freed via `manually_drop()` when refcount hits `0` (zero `Signal` leaks on long-lived apps).
- **Zero-Polling Async Wake Bridge**: wires `QueryClient::set_wake_callback` onto a single `dioxus_core::spawn` task inside `QueryClientProvider` so background Tokio completions and browser `FetchTransport` Promises trigger incremental frames with zero user input events.
- **Progressive `<ActionForm<P>>`**: emits semantic `<form method="post" action="...">` for 0-JS browsers while running local `P::Args::from_form(&pairs)` (0ms client validation) -> `use_action::<P>()` CBOR RPC on hydrated Web WASM and Native, preserving drafts on error and attaching server `FieldErrors` inline.

## Verification

```bash
cargo test --locked -p nve-query-dioxus
cargo clippy --locked -p nve-query-dioxus --all-targets -- -D warnings
cargo clippy --locked --target wasm32-unknown-unknown -p nve-query-dioxus --all-targets -- -D warnings
```
