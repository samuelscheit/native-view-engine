# `nve-haptics-web`

Thin browser `wasm32` leaf provider for `SERVICE_HAPTICS` (`0x0016`) implementing `nve_host_services::HostServiceHandler`.

## Key Public API

- `WebHaptics` — dispatches tactile impact, notification, and selection feedback through `navigator.vibrate(pattern)`, returning `ServiceError::Unsupported` when the Vibration API is unavailable.
- `impact_pattern_ms` / `notification_pattern_ms` / `SELECTION_PATTERN_MS` — zero-allocation static vibration pattern tables.

## Dependency Invariants

- Depends only on `nve-abi`, `nve-haptics`, `nve-host-services`, and `wasm-bindgen` on `wasm32`.
- Never depended on by `packages/core/*` or `packages/backends/*`.

## Verification

```bash
cargo test -p nve-haptics-web
```
