# `nve-webview-web`

Thin Web (`nve-backend-web`) leaf adapter rendering `nve-webview` nodes as sandboxed HTML5 `<iframe>` elements.

## Role & API

- Exports `register_webview_web_system(registry: &mut WebTableRegistry)` implementing `WebTableSystem` for `WebView`.

## Invariants

- Leaf backend adapter only; `nve-backend-web` has zero dependency on `nve-webview`.
- Rejects `javascript:` / `data:` / control-character URLs before setting `iframe.src`.

## Verification

```bash
cargo test -p nve-webview-web
```
