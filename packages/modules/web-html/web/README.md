# `nve-web-html-web`

Thin Web (`nve-backend-web`) leaf adapter emitting semantic HTML5 tags and attributes for `nve-web-html` tables across SSR and live WASM DOM.

## Role & API

- Exports `register_web_html_systems(registry: &mut WebTableRegistry)` implementing `WebTableSystem` for all `nve-web-html` tables.

## Invariants

- Leaf backend adapter only; `nve-backend-web` stays free of HTML5-specific framework widgets when unused.
- Maintains exact SSR-to-WASM attribute parity for zero-mismatch hydration.

## Verification

```bash
cargo test -p nve-web-html-web
```
