# `nve-svg-web`

Thin Web (`nve-backend-web`) leaf adapter rendering `nve-svg` `Svg` nodes as sanitized inline `<svg>` elements across SSR and WASM DOM.

## Role & API

- Exports `register_svg_web_system(registry: &mut WebTableRegistry)` implementing `WebTableSystem` for `Svg`.

## Invariants

- Leaf backend adapter only; `nve-backend-web` has zero dependency on `nve-svg`.
- Strips unsafe script/event-handler SVG payloads before emitting markup.

## Verification

```bash
cargo test -p nve-svg-web
```
