# `nve-ui`

Accessible, themeable Dioxus UI component library (`packages/framework/ui`, **Framework layer**) built directly on `nve-dioxus` primitives, `nve-i18n`, and `nve-interop` theme/focus contexts with zero external CSS.

## Key Public API

- `components::{VirtualList, VirtualListState, VirtualSlice, FenwickHeightTree, RowHeights, HeightsRevision, ScrollAlign}` and calendar/combobox/overlay widgets
- `focus::{FocusScope, FocusScopeProvider, FocusTrap, use_focus_scope}`
- `form::{FormState, FieldHandle, FieldState, ValidationToken}`, `theme`, `positioning`, `history`
- Optional `spatial-focus` feature (opt-in via `features = ["spatial-focus"]`): `spatial_focus` re-exporting `nve-tv`

```rust
use nve_ui::{FenwickHeightTree, RowHeights};

let heights = RowHeights::uniform(1_000, 40);
let tree = FenwickHeightTree::from_fn(1_000, |i| 32 + ((i % 4) as u32) * 8);
assert_eq!(heights.len(), tree.len());
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-dioxus`, `nve-engine`, `nve-i18n`, `nve-interop`, `nve-web-html`, `base64`, `dioxus-*`, `wasm32`-gated `web-sys`, and optional `nve-tv` behind the opt-in `spatial-focus` feature (`criterion` is a native-only dev-dependency).
- **Must NOT depend on**: `skia-safe`, `serde_json`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-ui
cargo test -p nve-ui --features spatial-focus --test spatial_focus_tests
cargo test -p nve-ui --test virtual_list_perf_tests
cargo bench -p nve-ui --bench virtual_list_bench
```
