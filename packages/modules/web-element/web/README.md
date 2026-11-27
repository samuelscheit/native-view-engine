# `nve-web-element-web`

Thin Web (`nve-backend-web`) leaf adapter rendering `nve-web-element` custom HTML tags and attributes across SSR and WASM DOM.

## Role & API

- Exports `register_web_element_web_system(registry: &mut WebTableRegistry)` implementing `WebTableSystem` for `WebElement`.

## Invariants

- Leaf backend adapter only; `nve-backend-web` has zero dependency on `nve-web-element`.
- Rejects raw `<script>`/event-handler attribute injection on both SSR and WASM paths.

## Verification

```bash
cargo test -p nve-web-element-web
```
