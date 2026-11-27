# `nve-web-element`

Pure-spec capability crate defining the `WebElement` (`nve.web_element`) word table for custom HTML/Web Component tag interop.

## Role & API

- Exposes `WebElement` (`#[derive(NveTable)]`) with validated tag name and attribute list resource (`WebAttrList`).
- Web leaf adapter lives in `web/`.

## Invariants

- Pure specification crate with zero backend or platform dependencies.
- Validates custom element tag names and strips unsafe attribute names.

## Verification

```bash
cargo test -p nve-web-element
```
