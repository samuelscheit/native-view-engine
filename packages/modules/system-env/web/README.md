# `nve-system-env-web`

Thin browser `wasm32` leaf provider for `SERVICE_SYSTEM_ENV` (`0x0014`) implementing `nve_host_services::HostServiceHandler` and live `WebThemeWatcher` push delivery.

## Key Public API

- `WebSystemEnv` — reads `matchMedia` (`prefers-color-scheme`, `prefers-contrast`, `prefers-reduced-motion`, `prefers-reduced-transparency`), `devicePixelRatio`, `navigator.language`, and `Intl` timezone.
- `WebThemeWatcher` — subscribes to browser `matchMedia` change events via a `thread_local!` listener slab (detached cleanly on `Drop`) and pushes `SERVICE_SYSTEM_ENV` `EVENT_THEME_CHANGED` records into a `CompletionSink` for `NveHostEventBatch` dispatch.
- `theme_from_media_flags` — zero-allocation pure snapshot builder.

## Dependency Invariants

- Depends only on `nve-abi`, `nve-system-env`, `nve-host-services`, and `wasm-bindgen` on `wasm32`.
- Never depended on by `packages/core/*` or `packages/backends/*`.

## Verification

```bash
cargo test -p nve-system-env-web
```
