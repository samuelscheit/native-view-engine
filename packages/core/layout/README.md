# `nve-layout`

Shared `NodeStore` layout layer (`packages/core/layout`, **Core layer**): the `StoreLayoutEngine` / `TextMeasurer` traits, the deterministic `HeuristicTextMeasurer`, and the default `TaffyStoreLayout` engine (direct Taffy 0.16 low-level layout over `NodeStore` word columns, zero `TaffyTree` duplication).

## Key Public API

- `StoreLayoutEngine`, `DynLayoutEngine`, `TextMeasurer`, `HeuristicTextMeasurer`
- `TaffyStoreLayout`, `ResolvedTextStyle`, `MAX_TREE_DEPTH`

```rust
use nve_engine::Viewport;
use nve_layout::{StoreLayoutEngine, TaffyStoreLayout};
use nve_store::{ChangeSet, NodeId, NodeStore};

let store = NodeStore::new();
let mut layout = TaffyStoreLayout::new();
layout.sync_changes(&store, &ChangeSet::default());
layout.compute_layout(&store, NodeId::ROOT, Viewport::default());
assert_eq!(layout.name(), "taffy");
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-components`, `nve-engine`, `nve-store`, `taffy`.
- **Used by**: `nve-backend-headless`, `nve-backend-skia`, `nve-runtime`, `packages/modules/layout-yoga`.
- **Must NOT depend on**: backends, `dioxus*`, `web-sys`, `skia-safe`, `serde_json`, `packages/modules/*`, or `hosts/*`.

## Verification

```bash
cargo test -p nve-layout
cargo bench -p nve-layout --bench taffy_hoisted_bench -- --test
```
