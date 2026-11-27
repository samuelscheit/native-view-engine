# `nve-tables`

Stateless cross-platform core word tables, pure value types, and resources (`packages/core/tables`, **Core layer**): layout extensions, paint/gradients, box shadows, transforms, media queries, input hints, keyboard insets, and interaction states. Zero CSS/HTML formatting (that lives in `nve-backend-web`).

## Key Public API

- `interaction::{InteractionState, CursorKind, StateStyleRule}`
- `layout_ext::{LayoutExt, GridLayout, PositionMode}`, `paint::{BackgroundFill, LinearGradient}`, `transform::Transform2D`
- `tables::*` (`Layout`, `Visual`, `Typography`, …), `input_hints::{AutofillHint, InputMode}`, `keyboard::KeyboardInsets`, `media::MediaQuery`

```rust
use nve_tables::{AutofillHint, InputMode};

let hint = AutofillHint::Email;
let mode = InputMode::Email;
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-codec`, `nve-store`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-tables
```
