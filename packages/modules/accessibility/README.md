# `nve-accessibility`

Platform-agnostic AccessKit accessibility tree builder, incremental `ChangeSet` diff engine, and
assistive-technology action translator over [`nve_store::NodeStore`].

Zero dependency on `winit`, `skia-safe`, or `dioxus-core`: desktop hosts (`hosts/skia-shell` via
`accesskit_winit`, `hosts/windows-shell` via `accesskit_windows`, and headless test harnesses)
share the same tree projection and epoch-guarded action routing.

```rust
use nve_accessibility::{A11yGeometryProvider, AccessKitTreeBridge};
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_tables::tables::{Accessibility, AriaRole, TextContent};

struct StaticGeo;
impl A11yGeometryProvider for StaticGeo {
    fn layout_rect(&self, _id: NodeId) -> Option<LayoutRect> {
        Some(LayoutRect { x: 0.0, y: 0.0, width: 200.0, height: 40.0 })
    }
}

let mut store = NodeStore::new();
let mut changes = ChangeSet::default();
let btn = {
    let mut tx = store.tx();
    let b = tx.create();
    tx.set_children(NodeId::ROOT, &[b]);
    tx.edit::<Accessibility>(b).role(Some(AriaRole::Button)).label("Submit");
    tx.edit::<TextContent>(b).text("Submit");
    tx.commit_into(&mut changes);
    b
};

let mut bridge = AccessKitTreeBridge::new();
bridge.activate();
let update = bridge.build_full_tree_update(&store, &StaticGeo, "Demo");
assert!(update.nodes.iter().any(|(id, node)| {
    bridge.decode_store_node_id(*id) == Some(btn) && node.label() == Some("Submit")
}));
```
