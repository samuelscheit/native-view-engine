# `nve-ui`

Accessible, themeable Dioxus UI component library (`packages/framework/ui`, **Framework layer**) built directly on `nve-dioxus` primitives, `nve-i18n`, and `nve-interop` theme/focus contexts with zero external CSS.

## Key Public API

- `components::{VirtualList, VirtualListState, FenwickHeightTree, RowHeights, ScrollAlign}` and calendar/combobox/overlay widgets
- `focus::{FocusScope, FocusScopeProvider, FocusTrap, use_focus_scope}`
- `form::{FormState, FieldHandle, FieldState, ValidationToken}`, `theme`, `positioning`, `spatial_focus`, `history`

```rust
use nve_ui::FenwickHeightTree;

let tree = FenwickHeightTree::from_heights(vec![40; 1_000]);
assert_eq!(tree.len(), 1_000);
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-dioxus`, `nve-engine`, `nve-i18n`, `nve-interop`, `dioxus-*`, and `wasm32`-gated `web-sys`.
- **Must NOT depend on**: `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-ui
```
