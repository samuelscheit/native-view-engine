# `nve-accessibility-windows`

Windows UI Automation (UIA) bridge for `hosts/windows-shell` built on `accesskit_windows` and
`nve-accessibility`.

Mirrors flat C-ABI `NveSceneView` snapshots into a replica `NodeStore` and projects them through
`nve_accessibility::AccessKitTreeBridge`, eliminating duplicate ARIA role, relation, selection, or
diff logic. Inactive frames and unchanged frames perform zero heap allocations; on Windows the C
ABI wires directly into `accesskit_windows::Adapter` (`WM_GETOBJECT`, `WM_SETFOCUS`/`WM_KILLFOCUS`,
and thread-safe `ActionHandler` callbacks).

```rust
use nve_accessibility_windows::{NoopResources, WinA11yBridge};
use nve_abi::NveSceneView;
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_tables::tables::{core_realization_order, Accessibility, AriaRole};

let mut store = NodeStore::new();
store.set_realization_order(core_realization_order());
let mut changes = ChangeSet::default();
let btn = {
    let mut tx = store.tx();
    let b = tx.create();
    tx.set_children(NodeId::ROOT, &[b]);
    tx.edit::<Accessibility>(b).role(Some(AriaRole::Button)).label("Save");
    tx.commit_into(&mut changes);
    b
};

let mut tables = Vec::new();
let mut dirty_buf = Vec::new();
store.publish_table_views(&changes, &mut tables, &mut dirty_buf);
let rows = store.node_rows();
let rects = vec![nve_abi::NveLayoutRect { x: 0.0, y: 0.0, width: 120.0, height: 32.0 }; rows.len()];
let scene = NveSceneView {
    frame_id: 1,
    scene_epoch: 1,
    node_capacity: rows.len() as u32,
    rows: rows.as_ptr().cast(),
    rects: rects.as_ptr(),
    text_slab: store.text_slab().as_ptr(),
    text_slab_len: store.text_slab().len() as u32,
    children_slab: store.children_slab().as_ptr().cast(),
    children_slab_len: store.children_slab().len() as u32,
    tables: tables.as_ptr(),
    table_count: tables.len() as u32,
    ..NveSceneView::default()
};

let mut bridge = WinA11yBridge::new();
bridge.activate();
// SAFETY: all pointers in `scene` reference valid local slices for the duration of this call.
let update = unsafe { bridge.sync_scene(&scene, &NoopResources, 0, 1.0, "Demo") }
    .expect("initial tree update");
assert!(update.nodes.iter().any(|(id, node)| {
    bridge.decode_store_node_id(*id) == Some(btn) && node.label() == Some("Save")
}));
```
