# `nve-ui`

Accessible, themeable Dioxus UI component library (`packages/framework/ui`, **Framework layer**) built directly on `nve-dioxus` primitives, `nve-i18n`, and `nve-interop` theme/focus contexts with zero external CSS.

## Key Public API

- `components::{VirtualList, VirtualListState, VirtualSlice, FenwickHeightTree, RowHeights, HeightsRevision, ScrollAlign}` and calendar/combobox/overlay widgets
- `focus::{FocusScope, FocusScopeProvider, FocusTrap, use_focus_scope}`
- `form::{FormState, FieldHandle, FieldState, ValidationToken}`, `theme`, `positioning`, `spatial_focus`, `history`

```rust
use nve_ui::{FenwickHeightTree, RowHeights};

let heights = RowHeights::uniform(1_000, 40);
let tree = FenwickHeightTree::from_fn(1_000, |i| 32 + ((i % 4) as u32) * 8);
assert_eq!(heights.len(), tree.len());
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-dioxus`, `nve-engine`, `nve-i18n`, `nve-interop`, `dioxus-*`, and `wasm32`-gated `web-sys` (`criterion` is native-only dev-dep).
- **Must NOT depend on**: `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-ui
cargo test -p nve-ui --test virtual_list_perf_tests
cargo bench -p nve-ui --bench virtual_list_bench
```
