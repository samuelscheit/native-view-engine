//! Headless end-to-end tests for `skia-shell` AccessKit tree sync and action dispatch
//! (`Click`, `Focus`, `Blur`, `SetValue`, `ReplaceSelectedText`, `SetTextSelection`,
//! `ScrollIntoView`, `SetScrollOffset`, `display:none`/`aria-hidden`, and `DylibReload` epoch
//! invalidation).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use accesskit::{
    Action, ActionData, ActionRequest, Point, Role, TextPosition, TextSelection, TreeId,
};
use nve_abi::{
    ColorRgba, FrameId, InteractionEvent, NodeId, SurfaceId, TextInputEvent, ValidatedEvents,
};
use nve_accessibility::{encode_text_run_id, AccessKitTreeBridge};
use nve_engine::{HotPatchMsg, Viewport};
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore};
use nve_tables::tables::{
    register_core_tables, Accessibility, AriaRole, Display, FlexDirection, Interaction, Layout,
    Overflow, Scroll, ScrollAxes, TextContent, TextInput, Visual,
};

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
