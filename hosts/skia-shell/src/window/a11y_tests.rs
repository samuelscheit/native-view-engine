//! Headless end-to-end tests for `skia-shell` AccessKit tree sync and action dispatch
//! (`Click`, `Focus`, `Blur`, `SetValue`, `ReplaceSelectedText`, `SetTextSelection`,
//! `ScrollIntoView`, `SetScrollOffset`, `display:none`/`aria-hidden`, and `DylibReload` epoch
//! invalidation).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use accesskit::{
    Action, ActionData, ActionRequest, Affine, Point, Role, TextPosition, TextSelection, TreeId,
};
use nve_abi::{
    ColorRgba, FrameId, InteractionEvent, NodeId, ScrollEvent, SurfaceId, TextInputEvent,
    ValidatedEvents,
};
use nve_accessibility::{encode_text_run_id, AccessKitTreeBridge};
use nve_backend_skia::SkiaFrameHook;
use nve_engine::{HotPatchMsg, Viewport};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore};
use nve_tables::tables::{
    register_core_tables, Accessibility, AriaRole, Display, FlexDirection, InputKind, Interaction,
    Layout, Overflow, Position, Scroll, ScrollAxes, TextContent, TextInput, Visual,
};
use nve_tables::transform::Transform2D;
use winit::event::Ime;
use winit::keyboard::{Key, ModifiersState};

use super::session::DesktopSession;
use super::{DesktopApp, DesktopHostError};

struct A11yTestApp {
    store: NodeStore,
    changes: ChangeSet,
    encoder: WireEncoder,
    frame_buf: Vec<u8>,
    btn_id: NodeId,
    input_id: NodeId,
    scroll_id: NodeId,
    deep_item_id: NodeId,
    hidden_id: NodeId,
    press_count: Arc<AtomicU32>,
    last_text: Arc<Mutex<String>>,
    generation: u64,
}

impl A11yTestApp {
    fn new() -> Self {
        register_core_tables();
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let (btn_id, input_id, scroll_id, deep_item_id, hidden_id) = {
            let mut tx = store.tx();
            tx.edit::<Layout>(NodeId::ROOT)
                .width(Length::points(320.0))
                .height(Length::points(260.0))
                .flex_direction(FlexDirection::Column);

            let btn = tx.create();
            tx.edit::<Layout>(btn)
                .width(Length::points(120.0))
                .height(Length::points(32.0))
                .flex_shrink(0.0);
            tx.edit::<Accessibility>(btn)
                .role(Some(AriaRole::Button))
                .label("Increment");
            tx.edit::<Interaction>(btn)
                .listen(InteractionEvent::Press)
                .listen(InteractionEvent::Focus)
                .listen(InteractionEvent::Blur)
                .focusable(true);

            let input = tx.create();
            tx.edit::<Layout>(input)
                .width(Length::points(200.0))
                .height(Length::points(32.0))
                .flex_shrink(0.0);
            tx.edit::<Accessibility>(input).label("Username");
            tx.edit::<TextInput>(input)
                .value("alice")
                .listen(TextInputEvent::Change);
            tx.edit::<Interaction>(input)
                .listen(InteractionEvent::Focus)
                .listen(InteractionEvent::Blur);

            let scroll = tx.create();
            tx.edit::<Layout>(scroll)
                .width(Length::points(280.0))
                .height(Length::points(100.0))
                .flex_direction(FlexDirection::Column)
                .flex_shrink(0.0);
            tx.edit::<Scroll>(scroll).axes(ScrollAxes::Vertical);
            tx.edit::<Visual>(scroll).overflow_y(Overflow::Scroll);

            let mut scroll_children = Vec::new();
            for i in 0..8u32 {
                let item = tx.create();
                tx.edit::<Layout>(item)
                    .width(Length::points(260.0))
                    .height(Length::points(40.0))
                    .flex_shrink(0.0);
                tx.edit::<TextContent>(item).text(&format!("Row {i}"));
                scroll_children.push(item);
            }
            let deep_item = scroll_children[7];
            tx.set_children(scroll, &scroll_children);

            let hidden = tx.create();
            tx.edit::<Layout>(hidden)
                .display(Display::None)
                .width(Length::points(80.0))
                .height(Length::points(24.0));
            tx.edit::<TextContent>(hidden).text("Hidden node");

            tx.set_children(NodeId::ROOT, &[btn, input, scroll, hidden]);
            tx.commit_into(&mut changes);
            (btn, input, scroll, deep_item, hidden)
        };
        let mut encoder = WireEncoder::default();
        let mut frame_buf = Vec::new();
        encoder.encode_full(&store, &mut frame_buf);
        Self {
            store,
            changes,
            encoder,
            frame_buf,
            btn_id,
            input_id,
            scroll_id,
            deep_item_id,
            hidden_id,
            press_count: Arc::new(AtomicU32::new(0)),
            last_text: Arc::new(Mutex::new("alice".to_owned())),
            generation: 1,
        }
    }
}

impl DesktopApp for A11yTestApp {
    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        &self.frame_buf
    }

    fn dispatch_events(&mut self, batch: ValidatedEvents<'_>) -> bool {
        let mut changed = false;
        for ev in batch.iter() {
            if ev.node() == self.btn_id && ev.interaction() == Some(InteractionEvent::Press) {
                let count = self.press_count.fetch_add(1, Ordering::Relaxed) + 1;
                let mut tx = self.store.tx();
                tx.edit::<Accessibility>(self.btn_id)
                    .label(&format!("Clicks: {count}"));
                tx.commit_into(&mut self.changes);
                self.frame_buf.clear();
                self.encoder
                    .encode(&self.store, &self.changes, &mut self.frame_buf);
                changed = true;
            } else if ev.node() == self.input_id && ev.text_input() == Some(TextInputEvent::Change)
            {
                let text = ev.text();
                *self.last_text.lock().expect("last_text lock") = text.to_owned();
                let mut tx = self.store.tx();
                tx.edit::<TextInput>(self.input_id).value(text);
                tx.commit_into(&mut self.changes);
                self.frame_buf.clear();
                self.encoder
                    .encode(&self.store, &self.changes, &mut self.frame_buf);
                changed = true;
            }
        }
        changed
    }

    fn apply_hot_patch(&mut self, _patch_bytes: &[u8]) -> bool {
        false
    }

    fn reload(
        &mut self,
        _dylib_path: &std::path::Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        self.generation += 1;
        self.frame_buf.clear();
        self.encoder.encode_full(&self.store, &mut self.frame_buf);
        Ok(self.generation)
    }

    fn generation(&self) -> u64 {
        self.generation
    }
}

#[test]
fn desktop_session_accesskit_full_sync_actions_scroll_and_reload_epoch_invalidation() {
    let app = A11yTestApp::new();
    let btn_id = app.btn_id;
    let input_id = app.input_id;
    let scroll_id = app.scroll_id;
    let deep_item_id = app.deep_item_id;
    let hidden_id = app.hidden_id;
    let press_count = Arc::clone(&app.press_count);
    let last_text = Arc::clone(&app.last_text);

    let mut session = DesktopSession::new(
        app,
        SurfaceId::ROOT,
        Viewport { width: 320.0, height: 260.0, scale_factor: 1.0 },
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().expect("initial render");

    let mut bridge = AccessKitTreeBridge::new();
    bridge.activate();
    let full = session.build_a11y_full_tree(&mut bridge, "A11y Demo");
    let by_id: HashMap<_, _> = full.nodes.into_iter().collect();
    assert!(!by_id.contains_key(&bridge.encode_store_node_id(hidden_id)));
    assert_eq!(by_id[&bridge.encode_store_node_id(btn_id)].role(), Role::Button);
    assert_eq!(by_id[&bridge.encode_store_node_id(input_id)].role(), Role::TextInput);
    assert_eq!(by_id[&bridge.encode_store_node_id(input_id)].value(), Some("alice"));

    // 1. Click action increments counter and triggers minimal incremental label update.
    let click_req = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: bridge.encode_store_node_id(btn_id),
        data: None,
    };
    let resolved = bridge.translate_action(&click_req).expect("click resolved");
    assert!(session.dispatch_a11y_action(resolved).expect("dispatch click"));
    assert_eq!(press_count.load(Ordering::Relaxed), 1);
    let inc = session
        .sync_a11y_incremental(&mut bridge, "A11y Demo", false)
        .expect("incremental label update");
    assert_eq!(inc.nodes.len(), 1);
    assert_eq!(inc.nodes[0].1.label(), Some("Clicks: 1"));

    // 2. Focus + SetValue + SetTextSelection + ReplaceSelectedText on TextInput.
    let set_val = bridge
        .translate_action(&ActionRequest {
            action: Action::SetValue,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(input_id),
            data: Some(ActionData::Value("hello world".into())),
        })
        .expect("set_value");
    assert!(session.dispatch_a11y_action(set_val).expect("dispatch set_value"));
    assert_eq!(session.focused_node(), Some(input_id));
    assert_eq!(last_text.lock().expect("last_text lock").as_str(), "hello world");

    let run_id = encode_text_run_id(bridge.epoch(), input_id);
    let set_sel = bridge
        .translate_action(&ActionRequest {
            action: Action::SetTextSelection,
            target_tree: TreeId::ROOT,
            target_node: run_id,
            data: Some(ActionData::SetTextSelection(TextSelection {
                anchor: TextPosition { node: run_id, character_index: 6 },
                focus: TextPosition { node: run_id, character_index: 11 },
            })),
        })
        .expect("set_selection");
    assert!(session.dispatch_a11y_action(set_sel).expect("dispatch sel"));
    assert_eq!(session.selected_text().as_deref(), Some("world"));

    let replace_sel = bridge
        .translate_action(&ActionRequest {
            action: Action::ReplaceSelectedText,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(input_id),
            data: Some(ActionData::Value("NVE".into())),
        })
        .expect("replace");
    assert!(session.dispatch_a11y_action(replace_sel).expect("dispatch replace"));
    assert_eq!(last_text.lock().expect("last_text lock").as_str(), "hello NVE");

    // 3. Blur clears focus cleanly.
    let blur = bridge
        .translate_action(&ActionRequest {
            action: Action::Blur,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(input_id),
            data: None,
        })
        .expect("blur");
    assert!(session.dispatch_a11y_action(blur).expect("dispatch blur"));
    assert_eq!(session.focused_node(), None);

    // 4. ScrollIntoView scrolls deep item into viewport, ignores display:none, & SetScrollOffset jumps cleanly.
    assert_eq!(session.backend().scroll_offset(scroll_id), (0.0, 0.0));
    let hidden_siv = bridge
        .translate_action(&ActionRequest {
            action: Action::ScrollIntoView,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(hidden_id),
            data: None,
        })
        .expect("hidden scroll_into_view");
    assert!(!session.dispatch_a11y_action(hidden_siv).expect("dispatch hidden siv"));

    let siv = bridge
        .translate_action(&ActionRequest {
            action: Action::ScrollIntoView,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(deep_item_id),
            data: None,
        })
        .expect("scroll_into_view");
    assert!(session.dispatch_a11y_action(siv).expect("dispatch scroll_into_view"));
    assert!(session.backend().scroll_offset(scroll_id).1 > 100.0);

    let abs_scroll = bridge
        .translate_action(&ActionRequest {
            action: Action::SetScrollOffset,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(scroll_id),
            data: Some(ActionData::SetScrollOffset(Point { x: 0.0, y: 24.0 })),
        })
        .expect("set_scroll_offset");
    assert!(session.dispatch_a11y_action(abs_scroll).expect("dispatch abs scroll"));
    assert!((session.backend().scroll_offset(scroll_id).1 - 24.0).abs() < 1e-3);

    // 5. Tier-2 DylibReload advances store_generation -> advancing bridge epoch rejects old IDs.
    let gen_before = session.store_generation();
    let stale_btn_id = bridge.encode_store_node_id(btn_id);
    session
        .handle_hot_patch(HotPatchMsg::DylibReload {
            generation: 2,
            dylib_path: "dummy.dylib".into(),
            build_ms: 10,
        })
        .expect("dylib reload");
    assert_ne!(session.store_generation(), gen_before);
    bridge.advance_epoch();
    assert!(bridge
        .translate_action(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: stale_btn_id,
            data: None,
        })
        .is_none());
}

struct RecordingApp {
    frame_buf: Vec<u8>,
    presses: Arc<Mutex<Vec<NodeId>>>,
    keydowns: Arc<Mutex<Vec<(NodeId, String)>>>,
    scrolls: Arc<Mutex<Vec<(NodeId, f32, f32)>>>,
    generation: u64,
}

impl RecordingApp {
    fn from_store(store: &NodeStore) -> Self {
        let mut enc = WireEncoder::default();
        let mut frame_buf = Vec::new();
        enc.encode_full(store, &mut frame_buf);
        Self {
            frame_buf,
            presses: Arc::new(Mutex::new(Vec::new())),
            keydowns: Arc::new(Mutex::new(Vec::new())),
            scrolls: Arc::new(Mutex::new(Vec::new())),
            generation: 1,
        }
    }
}

impl DesktopApp for RecordingApp {
    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        &self.frame_buf
    }
    fn dispatch_events(&mut self, batch: ValidatedEvents<'_>) -> bool {
        for ev in batch.iter() {
            match ev.interaction() {
                Some(InteractionEvent::Press) => self.presses.lock().unwrap().push(ev.node()),
                Some(InteractionEvent::KeyDown) => self
                    .keydowns
                    .lock()
                    .unwrap()
                    .push((ev.node(), ev.key().to_owned())),
                _ => {}
            }
            if ev.scroll_event() == Some(ScrollEvent::Scroll) {
                self.scrolls
                    .lock()
                    .unwrap()
                    .push((ev.node(), ev.x(), ev.y()));
            }
        }
        false
    }
    fn apply_hot_patch(&mut self, _: &[u8]) -> bool {
        false
    }
    fn reload(&mut self, _: &std::path::Path, _: SurfaceId) -> Result<u64, DesktopHostError> {
        self.generation += 1;
        Ok(self.generation)
    }
    fn generation(&self) -> u64 {
        self.generation
    }
}

#[test]
fn display_contents_subtree_and_virtualized_sticky_coords_match_skia_visual_rect() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (contents_wrap, contents_btn, scroll_id, sticky_hdr, sticky_btn, normal_row) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(320.0))
            .height(Length::points(260.0))
            .flex_direction(FlexDirection::Column);

        let wrap = tx.create();
        tx.edit::<Layout>(wrap).display(Display::Contents);
        let cbtn = tx.create();
        tx.edit::<Layout>(cbtn)
            .width(Length::points(120.0))
            .height(Length::points(32.0))
            .flex_shrink(0.0);
        tx.edit::<Accessibility>(cbtn)
            .role(Some(AriaRole::Button))
            .label("Inside Contents");
        tx.edit::<Interaction>(cbtn).listen(InteractionEvent::Press);
        tx.set_children(wrap, &[cbtn]);

        let scroll = tx.create();
        tx.edit::<Layout>(scroll)
            .width(Length::points(300.0))
            .height(Length::points(160.0))
            .flex_direction(FlexDirection::Column)
            .flex_shrink(0.0);
        tx.edit::<Scroll>(scroll)
            .axes(ScrollAxes::Vertical)
            .virtual_top_spacer(Some(400.0));
        tx.edit::<Visual>(scroll).overflow_y(Overflow::Scroll);

        let hdr = tx.create();
        tx.edit::<Layout>(hdr)
            .position(Position::Sticky)
            .inset_top(Length::points(0.0))
            .width(Length::points(300.0))
            .height(Length::points(32.0))
            .flex_shrink(0.0);
        let sbtn = tx.create();
        tx.edit::<Layout>(sbtn)
            .width(Length::points(80.0))
            .height(Length::points(24.0))
            .flex_shrink(0.0);
        tx.edit::<Accessibility>(sbtn)
            .role(Some(AriaRole::Button))
            .label("Pinned Action");
        tx.set_children(hdr, &[sbtn]);

        let mut rows = vec![hdr];
        for i in 0..20u32 {
            let r = tx.create();
            tx.edit::<Layout>(r)
                .width(Length::points(280.0))
                .height(Length::points(40.0))
                .flex_shrink(0.0);
            tx.edit::<TextContent>(r).text(&format!("VRow {i}"));
            rows.push(r);
        }
        let nrow = rows[3];
        tx.set_children(scroll, &rows);
        tx.set_children(NodeId::ROOT, &[wrap, scroll]);
        tx.commit_into(&mut changes);
        (wrap, cbtn, scroll, hdr, sbtn, nrow)
    };

    let app = RecordingApp::from_store(&store);
    let presses = Arc::clone(&app.presses);
    let mut session = DesktopSession::new(
        app,
        SurfaceId::ROOT,
        Viewport { width: 320.0, height: 260.0, scale_factor: 2.0 },
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().expect("render");

    let mut bridge = AccessKitTreeBridge::new();
    bridge.activate();
    let full = session.build_a11y_full_tree(&mut bridge, "VirtSticky");
    let by_id: HashMap<_, _> = full.nodes.into_iter().collect();
    assert!(!by_id.contains_key(&bridge.encode_store_node_id(contents_wrap)));
    let cbtn_node = by_id
        .get(&bridge.encode_store_node_id(contents_btn))
        .expect("display:contents child MUST stay in AccessKit tree");
    assert_eq!(cbtn_node.label(), Some("Inside Contents"));
    assert!(cbtn_node.bounds().is_some_and(|b| b.width() > 100.0));

    // Clicking the child inside display:contents succeeds!
    let click = bridge
        .translate_action(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(contents_btn),
            data: None,
        })
        .unwrap();
    assert!(session.dispatch_a11y_action(click).unwrap());
    assert_eq!(presses.lock().unwrap().as_slice(), &[contents_btn]);

    // Scroll virtualized container past virtual_top_spacer=400.0 -> visual_shift = 60.0.
    let scroll_act = bridge
        .translate_action(&ActionRequest {
            action: Action::SetScrollOffset,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(scroll_id),
            data: Some(ActionData::SetScrollOffset(Point { x: 0.0, y: 460.0 })),
        })
        .unwrap();
    assert!(session.dispatch_a11y_action(scroll_act).unwrap());

    let post_full = session.build_a11y_full_tree(&mut bridge, "VirtSticky");
    let map: HashMap<_, _> = post_full.nodes.into_iter().collect();
    let tf_of = |id: NodeId| {
        map[&bridge.encode_store_node_id(id)]
            .transform()
            .copied()
            .unwrap_or(Affine::IDENTITY)
    };
    let root_tf = tf_of(NodeId::ROOT);
    let scroll_tf = tf_of(scroll_id);
    let sticky_tf = tf_of(sticky_hdr);

    let check_rect = |chain: Affine, target: NodeId| {
        let b = map[&bridge.encode_store_node_id(target)].bounds().unwrap();
        let p0 = chain * Point::new(b.x0, b.y0);
        let p1 = chain * Point::new(b.x1, b.y1);
        let expected = session.backend().visual_rect_of(session.store(), target).unwrap();
        assert!((p0.x - f64::from(expected.x * 2.0)).abs() < 1e-2, "x mismatch for {target:?}");
        assert!((p0.y - f64::from(expected.y * 2.0)).abs() < 1e-2, "y mismatch for {target:?}");
        assert!((p1.y - p0.y - f64::from(expected.height * 2.0)).abs() < 1e-2);
    };
    check_rect(root_tf * scroll_tf * sticky_tf, sticky_btn);
    check_rect(root_tf * scroll_tf, normal_row);
}

#[test]
fn real_desktop_session_scroll_typing_ime_drag_and_animation_sync_in_o1() {
    struct StepTransformHook {
        target: NodeId,
        tx: f32,
        remaining: u8,
    }
    impl SkiaFrameHook for StepTransformHook {
        fn register_tables(&self) {}
        fn after_commit(
            &mut self,
            _: &NodeStore,
            _: &ChangeSet,
            _: nve_backend_skia::TargetsOf<'_>,
        ) {
        }
        fn sync_nodes(&mut self, _: &NodeStore, _: &[NodeId], _: nve_backend_skia::TargetsOf<'_>) {}
        fn advance(&mut self, _: f32) -> bool {
            if self.remaining == 0 {
                return false;
            }
            self.remaining -= 1;
            self.tx += 12.0;
            true
        }
        fn advance_collecting(
            &mut self,
            dt_s: f32,
            composite_dirty: &mut Vec<NodeId>,
            _: &mut Vec<NodeId>,
        ) -> bool {
            if self.advance(dt_s) {
                composite_dirty.push(self.target);
                true
            } else {
                false
            }
        }
        fn needs_next_frame(&self) -> bool {
            self.remaining > 0
        }
        fn set_reduced_motion(&mut self, _: bool) -> bool {
            false
        }
        fn resolve_transform(&self, node: NodeId, target: Option<Transform2D>) -> Option<Transform2D> {
            if node == self.target && self.tx != 0.0 {
                let mut t = target.unwrap_or_default();
                t.translate_x = self.tx;
                Some(t)
            } else {
                target
            }
        }
        fn reset(&mut self) {}
    }

    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (input_id, anim_id, scroll_id) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(320.0))
            .height(Length::points(260.0))
            .flex_direction(FlexDirection::Column);
        let input = tx.create();
        tx.edit::<Layout>(input)
            .width(Length::points(240.0))
            .height(Length::points(32.0))
            .flex_shrink(0.0);
        tx.edit::<TextInput>(input).value("hello world");

        let anim = tx.create();
        tx.edit::<Layout>(anim)
            .width(Length::points(100.0))
            .height(Length::points(24.0))
            .flex_shrink(0.0);
        tx.edit::<Accessibility>(anim).role(Some(AriaRole::Button)).label("Animated");

        let scroll = tx.create();
        tx.edit::<Layout>(scroll)
            .width(Length::points(300.0))
            .height(Length::points(160.0))
            .flex_direction(FlexDirection::Column)
            .flex_shrink(0.0);
        tx.edit::<Scroll>(scroll).axes(ScrollAxes::Vertical);
        tx.edit::<Visual>(scroll).overflow_y(Overflow::Scroll);
        let mut rows = Vec::with_capacity(210);
        for i in 0..210u32 {
            let r = tx.create();
            tx.edit::<Layout>(r)
                .width(Length::points(280.0))
                .height(Length::points(24.0))
                .flex_shrink(0.0);
            tx.edit::<TextContent>(r).text(&format!("Item {i}"));
            rows.push(r);
        }
        tx.set_children(scroll, &rows);
        tx.set_children(NodeId::ROOT, &[input, anim, scroll]);
        tx.commit_into(&mut changes);
        (input, anim, scroll)
    };

    let mut session = DesktopSession::new(
        RecordingApp::from_store(&store),
        SurfaceId::ROOT,
        Viewport { width: 320.0, height: 260.0, scale_factor: 1.0 },
        ColorRgba::BLACK,
        false,
    );
    session.replica.backend_mut().register_frame_hook(StepTransformHook {
        target: anim_id,
        tx: 0.0,
        remaining: 1,
    });
    session.render_and_rasterize().expect("render");
    let mut bridge = AccessKitTreeBridge::new();
    bridge.activate();
    let full = session.build_a11y_full_tree(&mut bridge, "Perf");
    assert!(full.nodes.len() > 210);

    // 1. Real scroll path -> O(1) incremental sync!
    assert!(session.handle_scroll_logical(60.0, 120.0, 0.0, 48.0).unwrap());
    let sc_upd = session
        .sync_a11y_incremental(&mut bridge, "Perf", false)
        .expect("scroll incremental");
    assert_eq!(bridge.last_sync_nodes_built(), 1, "scroll must build 1 node");
    assert_eq!(sc_upd.nodes[0].0, bridge.encode_store_node_id(scroll_id));
    assert_eq!(sc_upd.nodes[0].1.scroll_y(), Some(48.0));

    // 2. Real typing + IME preedit/commit -> O(1) incremental sync!
    assert!(session.focus_node(Some(input_id)).unwrap());
    let _ = session.sync_a11y_incremental(&mut bridge, "Perf", false);
    assert!(session
        .handle_keyboard_input(&Key::Character("!".into()), Some("!"), ModifiersState::empty())
        .unwrap());
    let key_upd = session
        .sync_a11y_incremental(&mut bridge, "Perf", false)
        .expect("typing incremental");
    assert!(bridge.last_sync_nodes_built() <= 2);
    let key_map: HashMap<_, _> = key_upd.nodes.into_iter().collect();
    assert!(key_map[&bridge.encode_store_node_id(input_id)]
        .value()
        .unwrap()
        .contains('!'));

    assert!(session.handle_ime(&Ime::Preedit("ni".into(), Some((0, 2)))).unwrap());
    let _ = session.sync_a11y_incremental(&mut bridge, "Perf", false);
    assert!(bridge.last_sync_nodes_built() <= 2);
    assert!(session.handle_ime(&Ime::Commit("你".into())).unwrap());
    let ime_upd = session
        .sync_a11y_incremental(&mut bridge, "Perf", false)
        .expect("ime commit incremental");
    assert!(bridge.last_sync_nodes_built() <= 2);
    let ime_map: HashMap<_, _> = ime_upd.nodes.into_iter().collect();
    assert!(ime_map[&bridge.encode_store_node_id(input_id)]
        .value()
        .unwrap()
        .contains('你'));

    // 3. Real drag selection -> marks dirty & syncs selection in O(1)!
    assert!(session
        .handle_pointer_down_logical(4.0, 16.0, 0, winit::keyboard::ModifiersState::empty())
        .unwrap());
    let _ = session.sync_a11y_incremental(&mut bridge, "Perf", false);
    assert!(session.handle_pointer_move_logical(96.0, 16.0).unwrap());
    assert!(session.has_pending_a11y_dirty());
    let drag_upd = session
        .sync_a11y_incremental(&mut bridge, "Perf", false)
        .expect("drag selection incremental");
    assert!(bridge.last_sync_nodes_built() <= 2);
    let drag_map: HashMap<_, _> = drag_upd.nodes.into_iter().collect();
    let sel = drag_map[&bridge.encode_store_node_id(input_id)]
        .text_selection()
        .expect("drag selection present");
    assert_ne!(sel.anchor.character_index, sel.focus.character_index);
    session.handle_pointer_up_logical();

    // 4. Real transform animation tick -> marks dirty & updates AccessKit transform in O(1)!
    let _ = session.tick_animations();
    assert!(session.has_pending_a11y_dirty());
    let anim_upd = session
        .sync_a11y_incremental(&mut bridge, "Perf", false)
        .expect("animation transform incremental");
    assert!(bridge.last_sync_nodes_built() <= 2);
    assert_eq!(anim_upd.nodes.len(), 1);
    assert_eq!(anim_upd.nodes[0].0, bridge.encode_store_node_id(anim_id));
    assert_eq!(
        anim_upd.nodes[0].1.transform(),
        Some(&Affine::translate((12.0, 0.0)))
    );
}

#[test]
fn accesskit_action_dispatch_enforces_visibility_focusable_target_scroll_and_no_spurious_gen_bump() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (
        focus_only_btn,
        tab_index_btn,
        slider_ok,
        slider_disabled,
        disp_none_child,
        aria_hidden_child,
        hidden_input,
        detached,
        scroll_id,
        deep_row,
    ) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(320.0))
            .height(Length::points(260.0))
            .flex_direction(FlexDirection::Column);

        let fbtn = tx.create();
        tx.edit::<Layout>(fbtn).width(Length::points(80.0)).height(Length::points(24.0));
        tx.edit::<Interaction>(fbtn).focusable(true);

        let tbtn = tx.create();
        tx.edit::<Layout>(tbtn).width(Length::points(80.0)).height(Length::points(24.0));
        tx.edit::<Interaction>(tbtn).tab_index(Some(0));

        let sl_ok = tx.create();
        tx.edit::<Layout>(sl_ok).width(Length::points(120.0)).height(Length::points(24.0));
        tx.edit::<Accessibility>(sl_ok).role(Some(AriaRole::Slider));
        tx.edit::<Interaction>(sl_ok)
            .focusable(true)
            .listen(InteractionEvent::KeyDown);

        let sl_dis = tx.create();
        tx.edit::<Layout>(sl_dis).width(Length::points(120.0)).height(Length::points(24.0));
        tx.edit::<Accessibility>(sl_dis).role(Some(AriaRole::Slider));
        tx.edit::<Interaction>(sl_dis)
            .focusable(true)
            .disabled(true)
            .listen(InteractionEvent::KeyDown);

        let dn_parent = tx.create();
        tx.edit::<Layout>(dn_parent).display(Display::None);
        let dn_child = tx.create();
        tx.edit::<Interaction>(dn_child).focusable(true).listen(InteractionEvent::Press);
        tx.set_children(dn_parent, &[dn_child]);

        let ah_parent = tx.create();
        tx.edit::<Accessibility>(ah_parent).hidden(true);
        let ah_child = tx.create();
        tx.edit::<Layout>(ah_child).width(Length::points(80.0)).height(Length::points(24.0));
        tx.edit::<Interaction>(ah_child).focusable(true).listen(InteractionEvent::Press);
        tx.set_children(ah_parent, &[ah_child]);

        let hid_in = tx.create();
        tx.edit::<Layout>(hid_in).width(Length::points(80.0)).height(Length::points(24.0));
        tx.edit::<TextInput>(hid_in).input_kind(InputKind::Hidden).value("secret");

        let det = tx.create();
        tx.edit::<Interaction>(det).focusable(true).listen(InteractionEvent::Press);

        let scroll = tx.create();
        tx.edit::<Layout>(scroll)
            .width(Length::points(260.0))
            .height(Length::points(80.0))
            .flex_direction(FlexDirection::Column)
            .flex_shrink(0.0);
        tx.edit::<Scroll>(scroll).axes(ScrollAxes::Vertical);
        tx.edit::<Visual>(scroll).overflow_y(Overflow::Scroll);
        let mut rows = Vec::new();
        for i in 0..8u32 {
            let r = tx.create();
            tx.edit::<Layout>(r)
                .width(Length::points(240.0))
                .height(Length::points(36.0))
                .flex_shrink(0.0);
            tx.edit::<TextContent>(r).text(&format!("SRow {i}"));
            rows.push(r);
        }
        let drow = rows[7];
        tx.set_children(scroll, &rows);

        tx.set_children(
            NodeId::ROOT,
            &[fbtn, tbtn, sl_ok, sl_dis, dn_parent, ah_parent, hid_in, scroll],
        );
        tx.commit_into(&mut changes);
        (fbtn, tbtn, sl_ok, sl_dis, dn_child, ah_child, hid_in, det, scroll, drow)
    };

    let app = RecordingApp::from_store(&store);
    let presses = Arc::clone(&app.presses);
    let keydowns = Arc::clone(&app.keydowns);
    let scrolls = Arc::clone(&app.scrolls);
    let mut session = DesktopSession::new(
        app,
        SurfaceId::ROOT,
        Viewport { width: 320.0, height: 260.0, scale_factor: 1.0 },
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().expect("render");

    // P1#8: Repeated identical wire frame (frame == 1? No, create a delta frame 2 and repeat it!)
    let mut delta_enc = WireEncoder::default();
    let mut init_bytes = Vec::new();
    delta_enc.encode_full(&store, &mut init_bytes);
    {
        let mut tx = store.tx();
        tx.edit::<Accessibility>(focus_only_btn).label("Focused");
        tx.commit_into(&mut changes);
    }
    let mut frame2 = Vec::new();
    delta_enc.encode(&store, &changes, &mut frame2);
    session.app_mut().frame_buf = frame2;
    let gen_before = session.store_generation();
    session.render_and_rasterize().expect("frame 2 first");
    session.render_and_rasterize().expect("frame 2 repeated");
    assert_eq!(
        session.store_generation(),
        gen_before,
        "repeating a cached wire frame number must NOT bump store_generation"
    );

    let mut bridge = AccessKitTreeBridge::new();
    bridge.activate();
    let _ = session.build_a11y_full_tree(&mut bridge, "Actions");

    // P1#7a: Focus works on focusable(true) and tab_index(Some(0)) buttons WITHOUT listeners!
    let focus_act = |node: NodeId| {
        bridge
            .translate_action(&ActionRequest {
                action: Action::Focus,
                target_tree: TreeId::ROOT,
                target_node: bridge.encode_store_node_id(node),
                data: None,
            })
            .unwrap()
    };
    assert!(session.dispatch_a11y_action(focus_act(focus_only_btn)).unwrap());
    assert_eq!(session.focused_node(), Some(focus_only_btn));
    assert!(session.dispatch_a11y_action(focus_act(tab_index_btn)).unwrap());
    assert_eq!(session.focused_node(), Some(tab_index_btn));

    // P1#7b: Hidden / detached / ancestor-excluded / InputKind::Hidden reject ALL actions!
    for &excluded in &[disp_none_child, aria_hidden_child, hidden_input, detached] {
        for action in [
            Action::Click,
            Action::Focus,
            Action::Increment,
            Action::Decrement,
            Action::ShowContextMenu,
            Action::ScrollIntoView,
        ] {
            let res = bridge
                .translate_action(&ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: bridge.encode_store_node_id(excluded),
                    data: None,
                })
                .unwrap();
            assert!(
                !session.dispatch_a11y_action(res).unwrap(),
                "excluded target {excluded:?} must reject {action:?}"
            );
        }
        let sv = bridge
            .translate_action(&ActionRequest {
                action: Action::SetValue,
                target_tree: TreeId::ROOT,
                target_node: bridge.encode_store_node_id(excluded),
                data: Some(ActionData::Value("hack".into())),
            })
            .unwrap();
        assert!(!session.dispatch_a11y_action(sv).unwrap());
    }
    assert!(presses.lock().unwrap().is_empty());

    // P1#7c: Increment/Decrement/ShowContextMenu reject disabled & route to resolved.target!
    for action in [Action::Increment, Action::Decrement, Action::ShowContextMenu] {
        let dis = bridge
            .translate_action(&ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node: bridge.encode_store_node_id(slider_disabled),
                data: None,
            })
            .unwrap();
        assert!(!session.dispatch_a11y_action(dis).unwrap());
    }
    assert!(keydowns.lock().unwrap().is_empty());

    // While `tab_index_btn` holds focus, targeting `slider_ok` focuses `slider_ok` and delivers
    // ArrowUp / ArrowDown / ContextMenu directly to `slider_ok`!
    assert_eq!(session.focused_node(), Some(tab_index_btn));
    for (action, expected_key) in [
        (Action::Increment, "ArrowUp"),
        (Action::Decrement, "ArrowDown"),
        (Action::ShowContextMenu, "ContextMenu"),
    ] {
        let req = bridge
            .translate_action(&ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node: bridge.encode_store_node_id(slider_ok),
                data: None,
            })
            .unwrap();
        let _ = session.dispatch_a11y_action(req).unwrap();
        assert_eq!(session.focused_node(), Some(slider_ok));
        assert_eq!(
            keydowns.lock().unwrap().last().cloned(),
            Some((slider_ok, expected_key.to_owned()))
        );
    }

    // P1#7d: ScrollIntoView emits ScrollEvent::Scroll to app!
    let siv = bridge
        .translate_action(&ActionRequest {
            action: Action::ScrollIntoView,
            target_tree: TreeId::ROOT,
            target_node: bridge.encode_store_node_id(deep_row),
            data: None,
        })
        .unwrap();
    assert!(session.dispatch_a11y_action(siv).unwrap());
    let emitted = scrolls.lock().unwrap();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].0, scroll_id);
    assert!(emitted[0].2 > 100.0);
}
