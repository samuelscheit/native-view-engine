//! End-to-end `DesktopSession` tests for:
//! 1. `Press` firing on pointer release only (and drag-off cancellation).
//! 2. Declarative pointer capture (`OnPrimaryDown`) surviving `CursorLeft` + `Focused(false)` ->
//!    `PointerCancel`.
//! 3. `Tab` / `Shift+Tab` keyboard focus traversal (`FocusModality::Keyboard` / `:focus-visible`),
//!    `Interaction::focusable()`, `tab_index` ordering, and disabled skipping.

use std::path::Path;

use nve_abi::{
    ColorRgba, EventSet, FrameId, InteractionEvent, NodeId, SurfaceId, ValidatedEvents,
};
use nve_backend_skia::{FocusModality, InteractionState};
use nve_engine::Viewport;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore};
use nve_tables::tables::{
    register_core_tables, FlexDirection, Interaction, Layout, PointerCaptureMode,
};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::{DesktopApp, DesktopHostError, DesktopSession};

struct PointerAndFocusApp {
    store: NodeStore,
    encoder: WireEncoder,
    changes: ChangeSet,
    initialized: bool,
    frame_buf: Vec<u8>,
    log: Vec<(NodeId, InteractionEvent, bool, bool)>,
}

impl PointerAndFocusApp {
    fn new() -> Self {
        register_core_tables();
        Self {
            store: NodeStore::new(),
            encoder: WireEncoder::default(),
            changes: ChangeSet::default(),
            initialized: false,
            frame_buf: Vec::new(),
            log: Vec::new(),
        }
    }
}

impl DesktopApp for PointerAndFocusApp {
    fn generation(&self) -> u64 {
        1
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        let mut tx = self.store.tx();
        if !self.initialized {
            // 10: button (focusable, Press + PointerDown/Up)
            // 11: slider thumb (capture_mode = OnPrimaryDown, PointerDown/Move/Up/Cancel)
            // 12: tab_index=1 button (comes first in Tab order!)
            // 13: disabled focusable button (must be skipped!)
            // 14: tab_index=-1 node (clickable focus, skipped by Tab!)
            for id in [10u32, 11, 12, 13, 14] {
                let nid = NodeId::new(id, 0);
                let _ = tx.create_at(nid);
                tx.edit::<Layout>(nid)
                    .width(Length::points(120.0))
                    .height(Length::points(30.0));
            }
            tx.edit::<Layout>(NodeId::ROOT).flex_direction(FlexDirection::Column);

            tx.edit::<Interaction>(NodeId::new(10, 0))
                .focusable(true)
                .listeners(
                    EventSet::one(InteractionEvent::Press)
                        .with(InteractionEvent::PointerDown)
                        .with(InteractionEvent::PointerUp)
                        .with(InteractionEvent::Focus)
                        .with(InteractionEvent::Blur),
                );
            tx.edit::<Interaction>(NodeId::new(11, 0))
                .capture_mode(PointerCaptureMode::OnPrimaryDown)
                .listeners(
                    EventSet::one(InteractionEvent::PointerDown)
                        .with(InteractionEvent::PointerMove)
                        .with(InteractionEvent::PointerUp)
                        .with(InteractionEvent::PointerCancel),
                );
            tx.edit::<Interaction>(NodeId::new(12, 0))
                .focusable(true)
                .tab_index(Some(1))
                .listeners(EventSet::one(InteractionEvent::Focus));
            tx.edit::<Interaction>(NodeId::new(13, 0))
                .focusable(true)
                .disabled(true);
            tx.edit::<Interaction>(NodeId::new(14, 0))
                .focusable(true)
                .tab_index(Some(-1));

            tx.set_children(
                NodeId::ROOT,
                &[
                    NodeId::new(10, 0),
                    NodeId::new(11, 0),
                    NodeId::new(12, 0),
                    NodeId::new(13, 0),
                    NodeId::new(14, 0),
                ],
            );
            self.initialized = true;
        }
        tx.commit_into(&mut self.changes);
        self.frame_buf.clear();
        self.encoder
            .encode(&self.store, &self.changes, &mut self.frame_buf);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            if let Some(ie) = ev.interaction() {
                self.log
                    .push((ev.node(), ie, ev.is_capture(), ev.bubbles()));
                handled = true;
            }
        }
        handled
    }

    fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool {
        false
    }

    fn reload(
        &mut self,
        _new_dylib_path: &Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        Ok(1)
    }
}

#[test]
fn press_fires_on_release_only_and_capture_survives_cursor_left_until_window_blur() {
    let vp = Viewport {
        width: 300.0,
        height: 240.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(
        PointerAndFocusApp::new(),
        SurfaceId::ROOT,
        vp,
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().unwrap();

    let n10 = NodeId::new(10, 0);
    let n11 = NodeId::new(11, 0);
    let r10 = session.layout_of(n10).unwrap();
    let r11 = session.layout_of(n11).unwrap();
    let none = ModifiersState::empty();

    // 1. PointerDown on button 10 must NOT emit Press yet; PointerUp on button 10 emits Press.
    session
        .handle_pointer_down_logical(r10.x + 20.0, r10.y + 15.0, 0, none)
        .unwrap();
    assert!(
        !session
            .app()
            .log
            .iter()
            .any(|&(_, ev, _, _)| ev == InteractionEvent::Press),
        "Press must NOT fire on pointer down"
    );
    assert!(session
        .app()
        .log
        .iter()
        .any(|&(id, ev, _, _)| id == n10 && ev == InteractionEvent::PointerDown));

    session
        .handle_pointer_up_logical_ex(r10.x + 20.0, r10.y + 15.0, 0, none)
        .unwrap();
    assert!(session
        .app()
        .log
        .iter()
        .any(|&(id, ev, _, _)| id == n10 && ev == InteractionEvent::Press));

    // 2. Dragging off button 10 before release cancels Press.
    session.app_mut().log.clear();
    session
        .handle_pointer_down_logical(r10.x + 20.0, r10.y + 15.0, 0, none)
        .unwrap();
    session.handle_pointer_move_logical(260.0, 200.0).unwrap();
    session
        .handle_pointer_up_logical_ex(260.0, 200.0, 0, none)
        .unwrap();
    assert!(!session
        .app()
        .log
        .iter()
        .any(|&(_, ev, _, _)| ev == InteractionEvent::Press));

    // 3. Synchronous declarative capture on slider thumb 11 survives CursorLeft and cancels on blur.
    session.app_mut().log.clear();
    session
        .handle_pointer_down_logical(r11.x + 20.0, r11.y + 15.0, 0, none)
        .unwrap();
    assert_eq!(session.backend().captured_node(), Some(n11));

    // Cursor leaves window -> capture MUST survive!
    let _ = session.handle_pointer_leave();
    assert_eq!(session.backend().captured_node(), Some(n11));

    // Drag move outside window bounds still routes CAPTURE | NO_BUBBLE PointerMove to n11!
    session.handle_pointer_move_logical(280.0, 220.0).unwrap();
    assert!(session.app().log.iter().any(|&(id, ev, cap, bubbles)| {
        id == n11 && ev == InteractionEvent::PointerMove && cap && !bubbles
    }));

    // OS WindowEvent::Focused(false) -> releases capture and emits PointerCancel!
    session.handle_window_blur().unwrap();
    assert_eq!(session.backend().captured_node(), None);
    assert!(session.app().log.iter().any(|&(id, ev, cap, bubbles)| {
        id == n11 && ev == InteractionEvent::PointerCancel && cap && !bubbles
    }));
}

#[test]
fn tab_traversal_honors_tab_index_skips_disabled_and_sets_keyboard_focus_visible() {
    let vp = Viewport {
        width: 300.0,
        height: 240.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(
        PointerAndFocusApp::new(),
        SurfaceId::ROOT,
        vp,
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().unwrap();

    let n10 = NodeId::new(10, 0);
    let n12 = NodeId::new(12, 0);
    let n14 = NodeId::new(14, 0);
    let tab = Key::Named(NamedKey::Tab);
    let none = ModifiersState::empty();
    let shift = ModifiersState::SHIFT;

    // Tab 1 -> n12 (positive tab_index=1 comes first!), Keyboard modality -> FOCUS_VISIBLE!
    assert!(session.handle_keyboard_input(&tab, None, none).unwrap());
    assert_eq!(session.focused_node(), Some(n12));
    assert_eq!(
        session.backend().interaction_tracker().focus_modality(),
        FocusModality::Keyboard
    );
    assert_eq!(
        session.backend().interaction_state(n12),
        InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE
    );

    // Tab 2 -> n10 (natural order; skips disabled n13 and tab_index=-1 n14!).
    assert!(session.handle_keyboard_input(&tab, None, none).unwrap());
    assert_eq!(session.focused_node(), Some(n10));

    // Shift+Tab wraps back to n12.
    assert!(session.handle_keyboard_input(&tab, None, shift).unwrap());
    assert_eq!(session.focused_node(), Some(n12));

    // Clicking tab_index=-1 node n14 focuses it with Pointer modality (FOCUS without FOCUS_VISIBLE).
    let r14 = session.layout_of(n14).unwrap();
    session.handle_click_logical(r14.x + 15.0, r14.y + 15.0).unwrap();
    assert_eq!(session.focused_node(), Some(n14));
    assert_eq!(
        session.backend().interaction_tracker().focus_modality(),
        FocusModality::Pointer
    );
    let st14 = session.backend().interaction_state(n14);
    assert!(st14.contains(InteractionState::FOCUS));
    assert!(!st14.contains(InteractionState::FOCUS_VISIBLE));
}
