# `nve-web-html`

Pure-spec capability crate defining semantic HTML5 word tables (`HtmlTag`, `HtmlForm`, `HtmlDialog`, `HtmlPopover`, `HtmlDetails`, `HtmlDataAttrs`, `HtmlHeading`, `HtmlCode`, `HtmlTable`) used by `nve-ui` and `nve-docs`.

## Role & API

- Provides typed `#[derive(NveTable)]` tables for native HTML5 semantics (`<dialog>`, `<details>`, `<form>`, `popover`, `commandfor`, `data-*` attributes) without polluting core `nve-components`.
- Web leaf adapter lives in `web/`.

## Invariants

- Pure specification crate with zero backend or platform dependencies.
- Zero `set_raw_prop` or stringly typed maps on the runtime hot path.

## Verification

```bash
cargo test -p nve-web-html
```
