# `nve-dioxus`

Dioxus (`dioxus-core 0.7`) frontend adapter (`packages/core/dioxus`, **Core Frontend Adapter layer**) implementing `WriteMutations` to compile `rsx!` updates directly into flat binary NVE command buffers.

## Key Public API

- `DioxusSurface`, `elements` (`dioxus_elements`), `use_signal`, `CustomEventData`
- `ErrorBoundary`, `throw_error`, `CanvasDrawList`, `svg`, `native_view`, `typed_style`
- `hot_patch`, `signal_archive`, `stream_slot`, `traits`

```rust
use nve_dioxus::prelude::*;

fn Counter() -> Element {
    let count = use_signal(|| 0i32);
    rsx! { text { "{count}" } }
}
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-tables`, `nve-engine`, `nve-interop`, `dioxus-core`, `dioxus-signals`, `dioxus-core-macro`.
- **Must NOT depend on**: `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-dioxus
```
