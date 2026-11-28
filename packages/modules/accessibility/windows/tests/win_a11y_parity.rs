//! End-to-end parity tests for `nve-accessibility-windows` `WinA11yBridge` and C ABI entry points.

use core::ffi::{c_char, c_int, c_void};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use accesskit::{Action, ActionData, ActionRequest, Live, Orientation, Role, TreeId};
use nve_abi::table::NveTableView;
use nve_abi::{NveLayoutRect, NveSceneView, NveScrollOffset};
use nve_accessibility_windows::ffi::{
    nve_a11y_win_activate, nve_a11y_win_create, nve_a11y_win_destroy,
    nve_a11y_win_handle_wm_getobject, nve_a11y_win_is_active, nve_a11y_win_set_window_focus,
    nve_a11y_win_sync_scene, nve_a11y_win_wants_wm_getobject,
};
use nve_accessibility_windows::{
    NveA11yWinAction, NveA11yWinActionKind, ResourceReader, WinA11yBridge,
};
use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore, ResourceId};
use nve_tables::tables::{
    core_realization_order, Accessibility, AriaLive, AriaOrientation, AriaRelations, AriaRole,
    DebugId, Interaction, Scroll, ScrollAxes, TextContent, TextInput,
};

struct MapResources(HashMap<u32, (u64, Vec<u8>)>);

impl MapResources {
    fn from_store(store: &NodeStore, ids: &[(ResourceId, u64)]) -> Self {
        let mut map = HashMap::new();
        for &(rid, kind) in ids {
            if let Some(words) = store.resources().raw_words(rid) {
                let mut bytes = Vec::with_capacity(words.len() * 4);
                for &w in words {
                    bytes.extend_from_slice(&w.to_le_bytes());
                }
                map.insert(rid.0, (kind, bytes));
            }
        }
        Self(map)
    }
}

impl ResourceReader for MapResources {
    fn read(&self, resource_id: u32, expected_kind: u64) -> Option<&[u8]> {
        self.0
            .get(&resource_id)
            .filter(|(k, _)| *k == expected_kind)
            .map(|(_, b)| b.as_slice())
    }
}

struct PublishedScene {
    tables: Vec<NveTableView>,
    _dirty_buf: Vec<u32>,
    rects: Vec<NveLayoutRect>,
    scrolls: Vec<NveScrollOffset>,
    dirty_indices: Vec<u32>,
    removed_ids: Vec<u32>,
    moved_indices: Vec<u32>,
    dirty_resources: Vec<u32>,
    frame_id: u64,
    scene_epoch: u32,
}

impl PublishedScene {
    fn build(
        store: &NodeStore,
        changes: &ChangeSet,
        frame_id: u64,
        scene_epoch: u32,
        scrolls: Vec<NveScrollOffset>,
    ) -> Self {
        let mut tables = Vec::new();
        let mut dirty_buf = Vec::new();
        store.publish_table_views(changes, &mut tables, &mut dirty_buf);
        let cap = store.node_rows().len();
        let rects = (0..cap)
            .map(|i| NveLayoutRect {
                x: 0.0,
                y: (i as f32) * 32.0,
                width: 320.0,
                height: 28.0,
            })
            .collect();
        let mut dirty_indices: Vec<u32> = changes
            .created
            .iter()
            .chain(changes.touched.iter())
            .chain(changes.children.iter())
            .map(|id| id.index() as u32)
            .collect();
        dirty_indices.sort_unstable();
        dirty_indices.dedup();
        let removed_ids = changes.removed.iter().map(|id| id.raw()).collect();
        Self {
            tables,
            _dirty_buf: dirty_buf,
            rects,
            scrolls,
            dirty_indices,
            removed_ids,
            moved_indices: Vec::new(),
            dirty_resources: Vec::new(),
            frame_id,
            scene_epoch,
        }
    }

    fn view(&self, store: &NodeStore) -> NveSceneView {
        let rows = store.node_rows();
        NveSceneView {
            frame_id: self.frame_id,
            scene_epoch: self.scene_epoch,
            node_capacity: rows.len() as u32,
            dirty_count: self.dirty_indices.len() as u32,
            removed_count: self.removed_ids.len() as u32,
            moved_count: self.moved_indices.len() as u32,
            dirty_resource_count: self.dirty_resources.len() as u32,
            text_slab_len: store.text_slab().len() as u32,
            children_slab_len: store.children_slab().len() as u32,
            table_count: self.tables.len() as u32,
            scroll_count: self.scrolls.len() as u32,
            rows: rows.as_ptr().cast(),
            rects: self.rects.as_ptr(),
            text_slab: store.text_slab().as_ptr(),
            children_slab: store.children_slab().as_ptr().cast(),
            dirty_indices: self.dirty_indices.as_ptr(),
            removed_ids: self.removed_ids.as_ptr(),
            moved_indices: self.moved_indices.as_ptr(),
            dirty_resources: self.dirty_resources.as_ptr(),
            tables: self.tables.as_ptr(),
            scroll_offsets: self.scrolls.as_ptr(),
            ..NveSceneView::default()
        }
    }
}

#[test]
fn mirror_projects_aria_relations_inputs_scroll_and_incremental_updates() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();

    let (title_node, dialog, input, scroller, rel_id) = {
        let mut tx = store.tx();
        let title_node = tx.create();
        tx.edit::<DebugId>(title_node).id("dlg-title");
        tx.edit::<TextContent>(title_node).text("Confirm Delete");

        let dialog = tx.create();
        tx.edit::<Accessibility>(dialog)
            .role(Some(AriaRole::Dialog))
            .modal(true)
            .live(Some(AriaLive::Assertive))
            .orientation(Some(AriaOrientation::Vertical))
            .relations(Some(AriaRelations {
                labelled_by: "dlg-title".into(),
                ..AriaRelations::default()
            }));

        let input = tx.create();
        tx.edit::<TextInput>(input)
            .value("secret")
            .placeholder("Password")
            .secure(true)
            .read_only(false);
        tx.edit::<Interaction>(input).focusable(true);

        let scroller = tx.create();
        tx.edit::<Scroll>(scroller).axes(ScrollAxes::Vertical);
        let item = tx.create();
        tx.edit::<TextContent>(item).text("Row 1");
        tx.set_children(scroller, &[item]);

        tx.set_children(dialog, &[title_node, input]);
        tx.set_children(NodeId::ROOT, &[dialog, scroller]);
        tx.commit_into(&mut changes);
        let rel_id = store
            .try_row::<Accessibility>(dialog)
            .map_or(ResourceId::NONE, |a| a.relations());
        (title_node, dialog, input, scroller, rel_id)
    };

    let resources = MapResources::from_store(&store, &[(rel_id, AriaRelations::KIND.0)]);
    let pub1 = PublishedScene::build(
        &store,
        &changes,
        1,
        1,
        vec![NveScrollOffset {
            node_id: scroller.raw(),
            x: 0.0,
            y: 12.0,
        }],
    );
    let view1 = pub1.view(&store);

    let mut mirror = WinA11yBridge::new();
    mirror.activate();
    // SAFETY: `view1` references live slices owned by `store` and `pub1`.
    let upd1 = unsafe { mirror.sync_scene(&view1, &resources, input.raw(), 2.0, "WinHost") };
    assert!(upd1.is_some(), "expected initial TreeUpdate");
    let Some(upd1) = upd1 else {
        return;
    };

    let dialog_ak = mirror.encode_store_node_id(dialog);
    let title_ak = mirror.encode_store_node_id(title_node);
    let input_ak = mirror.encode_store_node_id(input);
    let scroll_ak = mirror.encode_store_node_id(scroller);

    assert_eq!(upd1.focus, input_ak);
    let dialog_node = upd1.nodes.iter().find(|(id, _)| *id == dialog_ak).map(|(_, n)| n);
    assert!(dialog_node.is_some());
    if let Some(dn) = dialog_node {
        assert_eq!(dn.role(), Role::Dialog);
        assert!(dn.is_modal());
        assert_eq!(dn.live(), Some(Live::Assertive));
        assert_eq!(dn.orientation(), Some(Orientation::Vertical));
        assert_eq!(dn.labelled_by(), &[title_ak]);
    }
    let input_node = upd1.nodes.iter().find(|(id, _)| *id == input_ak).map(|(_, n)| n);
    assert!(input_node.is_some());
    if let Some(in_node) = input_node {
        assert_eq!(in_node.role(), Role::PasswordInput);
        assert_eq!(in_node.placeholder(), Some("Password"));
        assert_eq!(in_node.value(), Some("••••••"));
    }
    let scroll_node = upd1.nodes.iter().find(|(id, _)| *id == scroll_ak).map(|(_, n)| n);
    assert!(scroll_node.is_some());
    if let Some(sn) = scroll_node {
        assert_eq!(sn.scroll_y(), Some(12.0));
    }

    // Incremental single-label edit + scroll offset change on frame 2.
    {
        let mut tx = store.tx();
        tx.edit::<Accessibility>(dialog).label("Updated Dialog");
        tx.commit_into(&mut changes);
    }
    let pub2 = PublishedScene::build(
        &store,
        &changes,
        2,
        1,
        vec![NveScrollOffset {
            node_id: scroller.raw(),
            x: 0.0,
            y: 48.0,
        }],
    );
    let view2 = pub2.view(&store);
    // SAFETY: `view2` references live slices owned by `store` and `pub2`.
    let upd2 = unsafe { mirror.sync_scene(&view2, &resources, input.raw(), 2.0, "WinHost") };
    assert!(upd2.is_some(), "expected incremental TreeUpdate");
    let Some(upd2) = upd2 else {
        return;
    };
    assert!(upd2.tree.is_none(), "incremental update should not resend TreeInfo");
    assert!(upd2
        .nodes
        .iter()
        .any(|(id, n)| *id == dialog_ak && n.label() == Some("Updated Dialog")));
    assert!(upd2
        .nodes
        .iter()
        .any(|(id, n)| *id == scroll_ak && n.scroll_y() == Some(48.0)));

    // Scene epoch bump invalidates prior AccessKit IDs so stale actions are rejected.
    let stale_req = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: dialog_ak,
        data: None,
    };
    assert!(mirror.translate_action(&stale_req).is_some());

    let pub3 = PublishedScene::build(&store, &changes, 1, 2, vec![]);
    let view3 = pub3.view(&store);
    // SAFETY: `view3` references live slices owned by `store` and `pub3`.
    let upd3 = unsafe { mirror.sync_scene(&view3, &resources, 0, 1.0, "WinHost") };
    assert!(upd3.is_some());
    assert!(
        mirror.translate_action(&stale_req).is_none(),
        "stale epoch action must be rejected after scene_epoch change"
    );
}

#[test]
fn c_abi_lifecycle_wm_getobject_and_action_dispatch() {
    assert_eq!(nve_a11y_win_wants_wm_getobject(-25), 1);
    assert_eq!(nve_a11y_win_wants_wm_getobject(-4), 1);
    assert_eq!(nve_a11y_win_wants_wm_getobject(-1), 0); // OBJID_SYSMENU

    struct CallbackState {
        calls: AtomicUsize,
        last_node: AtomicUsize,
        last_kind: AtomicUsize,
        last_text_len: AtomicUsize,
    }
    unsafe extern "C" fn on_action(ctx: *mut c_void, act: *const NveA11yWinAction) {
        if ctx.is_null() || act.is_null() {
            return;
        }
        // SAFETY: `ctx` points to `CallbackState` alive for the test; `act` is valid for call.
        let st = unsafe { &*(ctx as *const CallbackState) };
        // SAFETY: `act` is a valid C-ABI struct passed by `ActionBridge::dispatch`.
        let a = unsafe { &*act };
        st.calls.fetch_add(1, Ordering::SeqCst);
        st.last_node.store(a.node_id as usize, Ordering::SeqCst);
        st.last_kind.store(a.kind as usize, Ordering::SeqCst);
        st.last_text_len.store(a.text_len, Ordering::SeqCst);
    }

    let state = CallbackState {
        calls: AtomicUsize::new(0),
        last_node: AtomicUsize::new(0),
        last_kind: AtomicUsize::new(0),
        last_text_len: AtomicUsize::new(0),
    };

    // SAFETY: portable test handle creation with valid `state` pointer.
    let handle = unsafe {
        nve_a11y_win_create(
            core::ptr::null_mut(),
            1,
            Some(on_action),
            (&state as *const CallbackState).cast_mut().cast(),
        )
    };
    assert!(!handle.is_null());
    // SAFETY: `handle` is live.
    assert_eq!(unsafe { nve_a11y_win_is_active(handle) }, 0);

    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let input = {
        let mut tx = store.tx();
        let inp = tx.create();
        tx.edit::<TextInput>(inp).value("hello");
        tx.set_children(NodeId::ROOT, &[inp]);
        tx.commit_into(&mut changes);
        inp
    };
    let pub1 = PublishedScene::build(&store, &changes, 1, 1, vec![]);
    let view1 = pub1.view(&store);
    let title = b"Title\0";

    // SAFETY: `handle` and `view1` are live; inactive handle skips work and returns 0.
    let skipped = unsafe {
        nve_a11y_win_sync_scene(
            handle,
            &view1,
            None,
            core::ptr::null_mut(),
            input.raw(),
            1.0,
            title.as_ptr().cast::<c_char>(),
        )
    };
    assert_eq!(skipped, 0);

    // Activate, sync, handle WM_GETOBJECT, focus toggle, and verify action translation.
    let mut lres: isize = 0;
    // SAFETY: `handle` and `view1` are live.
    unsafe {
        nve_a11y_win_activate(handle);
        assert_eq!(nve_a11y_win_is_active(handle), 1);
        let synced = nve_a11y_win_sync_scene(
            handle,
            &view1,
            None,
            core::ptr::null_mut(),
            input.raw(),
            1.0,
            title.as_ptr().cast::<c_char>(),
        );
        assert_eq!(synced, 1);
        assert_eq!(nve_a11y_win_handle_wm_getobject(handle, 0, -25, &mut lres), 1);
        assert_eq!(nve_a11y_win_handle_wm_getobject(handle, 0, -1, &mut lres), 0);
        nve_a11y_win_set_window_focus(handle, 0);
        nve_a11y_win_set_window_focus(handle, 1);
    }

    // Dispatch a SetValue action through the handle's ActionBridge.
    // SAFETY: `handle` is live.
    let h_ref = unsafe { &*handle };
    let ak_id = nve_accessibility::encode_store_node_id(h_ref.current_epoch(), input);
    h_ref.dispatch_test_action(&ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node: ak_id,
        data: Some(ActionData::Value("world".into())),
    });
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.last_node.load(Ordering::SeqCst), input.raw() as usize);
    assert_eq!(
        state.last_kind.load(Ordering::SeqCst),
        NveA11yWinActionKind::SetValue as usize
    );
    assert_eq!(state.last_text_len.load(Ordering::SeqCst), 5);

    // SAFETY: `handle` was created by `nve_a11y_win_create` and is destroyed once.
    unsafe { nve_a11y_win_destroy(handle) };
    let _ = (0 as c_int, lres);
}

#[test]
fn moved_container_updates_descendant_bounds_and_skipped_frame_rebuilds() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();

    let (container, child, alt_title, rel_id) = {
        let mut tx = store.tx();
        let container = tx.create();
        tx.edit::<Accessibility>(container)
            .role(Some(AriaRole::Region))
            .relations(Some(AriaRelations {
                labelled_by: "orig-lbl".into(),
                ..AriaRelations::default()
            }));
        let child = tx.create();
        tx.edit::<DebugId>(child).id("orig-lbl");
        tx.edit::<TextContent>(child).text("Nested Item");
        let alt_title = tx.create();
        tx.edit::<DebugId>(alt_title).id("new-lbl");
        tx.edit::<TextContent>(alt_title).text("New Label");
        tx.set_children(container, &[child]);
        tx.set_children(NodeId::ROOT, &[container, alt_title]);
        tx.commit_into(&mut changes);
        let rel_id = store
            .try_row::<Accessibility>(container)
            .map_or(ResourceId::NONE, |a| a.relations());
        (container, child, alt_title, rel_id)
    };

    let mut resources = MapResources::from_store(&store, &[(rel_id, AriaRelations::KIND.0)]);
    let pub1 = PublishedScene::build(&store, &changes, 1, 1, vec![]);
    let view1 = pub1.view(&store);

    let mut mirror = WinA11yBridge::new();
    mirror.activate();
    // SAFETY: `view1` references live local slices.
    let upd1 = unsafe { mirror.sync_scene(&view1, &resources, 0, 1.0, "MoveTest") };
    assert!(upd1.is_some());

    // Frame 2: only `container` is in `moved_indices`, while BOTH `container` and `child` shift
    // down by +120px in `rects` (matching `nve-runtime`'s parent-relative `moved_indices` contract).
    let empty_changes = ChangeSet::default();
    let mut pub2 = PublishedScene::build(&store, &empty_changes, 2, 1, vec![]);
    pub2.rects[container.index()].y += 120.0;
    pub2.rects[child.index()].y += 120.0;
    pub2.moved_indices = vec![container.index() as u32];
    let expected_child_y0 = f64::from(pub2.rects[child.index()].y);
    let view2 = pub2.view(&store);

    // SAFETY: `view2` references live local slices.
    let upd2 = unsafe { mirror.sync_scene(&view2, &resources, 0, 1.0, "MoveTest") };
    let child_ak = mirror.encode_store_node_id(child);
    let container_ak = mirror.encode_store_node_id(container);
    let alt_ak = mirror.encode_store_node_id(alt_title);
    assert!(upd2.is_some(), "moved container must emit incremental bounds update");
    if let Some(u) = upd2 {
        assert!(u.tree.is_none(), "moved subtree update should stay incremental");
        let child_node = u.nodes.iter().find(|(id, _)| *id == child_ak).map(|(_, n)| n);
        assert!(
            child_node.is_some(),
            "descendant of moved container must be re-emitted with updated bounds"
        );
        if let Some(cn) = child_node {
            let bounds = cn.bounds().unwrap_or_default();
            assert!((bounds.y0 - expected_child_y0).abs() < 1e-3);
        }
    }

    // Frame 3: in-place `dirty_resources` update on `AriaRelations` refreshes `container` even
    // though `dirty_indices` is empty.
    let mut temp_store = NodeStore::new();
    temp_store.set_realization_order(core_realization_order());
    let mut temp_changes = ChangeSet::default();
    let new_rel_bytes = {
        let mut tx = temp_store.tx();
        let n = tx.create();
        tx.edit::<Accessibility>(n).relations(Some(AriaRelations {
            labelled_by: "new-lbl".into(),
            ..AriaRelations::default()
        }));
        tx.commit_into(&mut temp_changes);
        let temp_rid = temp_store
            .try_row::<Accessibility>(n)
            .map_or(ResourceId::NONE, |a| a.relations());
        let mut bytes = Vec::new();
        if let Some(words) = temp_store.resources().raw_words(temp_rid) {
            for &w in words {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        bytes
    };
    resources.0.insert(rel_id.0, (AriaRelations::KIND.0, new_rel_bytes));
    let mut pub3 = PublishedScene::build(&store, &empty_changes, 3, 1, vec![]);
    pub3.dirty_resources = vec![rel_id.0];
    let view3 = pub3.view(&store);
    // SAFETY: `view3` references live local slices.
    let upd3 = unsafe { mirror.sync_scene(&view3, &resources, 0, 1.0, "MoveTest") };
    assert!(upd3.is_some(), "dirty_resources AriaRelations update must emit TreeUpdate");
    if let Some(u) = upd3 {
        let cn = u.nodes.iter().find(|(id, _)| *id == container_ak).map(|(_, n)| n);
        assert!(cn.is_some());
        if let Some(cn) = cn {
            assert_eq!(cn.labelled_by(), &[alt_ak]);
        }
    }

    // Frame 6 (skipped frames 4 & 5, empty dirty_indices): forces full rebuild so mutations
    // performed during skipped frames are reflected without losing synchronization.
    {
        let mut tx = store.tx();
        tx.edit::<TextContent>(child).text("Updated After Skip");
        tx.commit_into(&mut changes);
    }
    let pub_skip = PublishedScene::build(&store, &empty_changes, 6, 1, vec![]);
    let view_skip = pub_skip.view(&store);
    // SAFETY: `view_skip` references live local slices.
    let upd_skip = unsafe { mirror.sync_scene(&view_skip, &resources, 0, 1.0, "MoveTest") };
    assert!(upd_skip.is_some(), "skipped frame_id must trigger full rebuild");
    if let Some(u) = upd_skip {
        assert!(u.tree.is_some(), "skipped frame_id must emit a full TreeInfo update");
        assert!(u
            .nodes
            .iter()
            .any(|(id, n)| *id == child_ak && n.value() == Some("Updated After Skip") || n.label() == Some("Updated After Skip")));
    }
}
