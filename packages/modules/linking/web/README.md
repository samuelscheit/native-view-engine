# `nve-linking-web`

Thin browser `wasm32` leaf provider for `SERVICE_LINKING` (`0x0013`) implementing `nve_host_services::HostServiceHandler`.

## Key Public API

- `WebLinking` — validates URLs via `validate_url` + strict scheme allowlist (`http`, `https`, `mailto`, `tel`), explicitly rejects `javascript:`/`data:`/`file:`, and launches safe external tabs via `window.open(url, '_blank', 'noopener,noreferrer')`.
- `is_allowed_web_url_scheme` / `validate_web_url` — zero-allocation URL scheme allowlist helpers.

## Dependency Invariants

- Depends only on `nve-abi`, `nve-linking`, `nve-host-services`, and `wasm-bindgen` on `wasm32`.
- Never depended on by `packages/core/*` or `packages/backends/*`.

## Verification

```bash
cargo test -p nve-linking-web
```
