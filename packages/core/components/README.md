# `nve-components`

Typed cross-platform UI primitive specs and value types (`packages/core/components`, **Core layer**) covering layout extensions, paint/gradients, transforms, SVG/canvas IR, media queries, input hints, keyboard insets, interaction states, and native-view codecs.

## Key Public API

- `interaction::{InteractionState, InteractionTracker, CursorKind, StateStyleRule}`
- `layout_ext::{LayoutExt, GridLayout, PositionMode}`, `paint::{BackgroundFill, LinearGradient}`, `transform::Transform2D`
- `canvas_ir::{DrawList, DrawListRecorder, CanvasOp}`, `svg::SvgPicture`, `input_hints::{AutofillHint, InputMode}`, `keyboard::KeyboardInsets`, `media::MediaQuery`

```rust
use nve_components::{AutofillHint, InputMode};

let hint = AutofillHint::Email;
let mode = InputMode::Email;
```

## Dependency Invariants

- **Depends on**: `nve-abi`.
- **Must NOT depend on**: `dioxus*`, `web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-components
```
