//! Drag/double-click/shift selection, clipboard roundtrip, secure-input copy refusal, and
//! hidden / `display: none` input rejection tests (split from `input_tests.rs` to stay `<= 650`
//! lines per file).

use std::path::Path;

use nve_abi::{ColorRgba, EventSet, FrameId, InteractionEvent, NodeId, SurfaceId, ValidatedEvents};
use nve_engine::Viewport;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore};
use nve_tables::tables::{
    register_core_tables, Display, InputKind, Interaction, Layout, TextInput,
};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::input::{is_focusable, word_bounds_at};
use super::input_tests::new_text_session;
use super::{DesktopApp, DesktopHostError, DesktopSession};

#[test]
fn drag_double_click_shift_selection_and_clipboard_copy_cut_paste_roundtrip() {
    assert_eq!(word_bounds_at("hello world_42!", 1), (0, 5));
    assert_eq!(word_bounds_at("hello world_42!", 8), (6, 14));
    assert_eq!(word_bounds_at("hello world_42!", 15), (14, 15));

    let mut session = new_text_session(false);
    let r10 = session.layout_of(NodeId::new(10, 0)).unwrap();
    let mid_y = r10.y + r10.height * 0.5;
    let none = ModifiersState::empty();
    let shift = ModifiersState::SHIFT;
    let cmd = ModifiersState::SUPER;

    // Focus input 10 and type "hello world".
    session.handle_click_logical(r10.x + 12.0, mid_y).unwrap();
    session.handle_pointer_up_logical();
    session
        .handle_keyboard_input(&Key::Character("hello world".into()), Some("hello world"), none)
        .unwrap();
    assert_eq!(session.app().title_value, "hello world");

    // Shift+Left x5 selects "world"; Cmd+C copies it without altering text.
    for _ in 0..5 {
        assert!(session
            .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, shift)
            .unwrap());
    }
    assert_eq!(session.selected_text().as_deref(), Some("world"));
    assert!(session
        .handle_keyboard_input(&Key::Character("c".into()), None, cmd)
        .unwrap());
    assert_eq!(
        session.clipboard_mut().get_text().as_deref(),
        Some("world")
    );
    assert_eq!(session.app().title_value, "hello world");

    // Move Home, Shift+Right x5 selects "hello", Cmd+X cuts it -> " world".
    session
        .handle_keyboard_input(&Key::Named(NamedKey::Home), None, none)
        .unwrap();
    assert_eq!(session.backend().focus_state().selection_range(), None);
    for _ in 0..5 {
        session
            .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, shift)
            .unwrap();
    }
    assert_eq!(session.selected_text().as_deref(), Some("hello"));
    assert!(session
        .handle_keyboard_input(&Key::Character("x".into()), None, cmd)
        .unwrap());
    assert_eq!(session.app().title_value, " world");
    assert_eq!(
        session.clipboard_mut().get_text().as_deref(),
        Some("hello")
    );

    // Move End, space, Cmd+V pastes "hello" -> " world hello".
    session
        .handle_keyboard_input(&Key::Named(NamedKey::End), None, none)
        .unwrap();
    session
        .handle_keyboard_input(&Key::Named(NamedKey::Space), Some(" "), none)
        .unwrap();
    assert!(session
        .handle_keyboard_input(&Key::Character("v".into()), None, cmd)
        .unwrap());
    assert_eq!(session.app().title_value, " world hello");

    // Shift+Home selects everything to start; ArrowRight collapses to right edge.
    session
        .handle_keyboard_input(&Key::Named(NamedKey::Home), None, shift)
        .unwrap();
    assert_eq!(
        session.backend().focus_state().selection_range(),
        Some((0, " world hello".len()))
    );
    session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, none)
        .unwrap();
    assert_eq!(session.backend().focus_state().selection_range(), None);
    assert_eq!(
        session.backend().focus_state().caret_byte_index,
        " world hello".len()
    );

    // Mouse press near start + drag to far right forms a non-empty range selection.
    session.handle_click_logical(r10.x + 10.0, mid_y).unwrap();
    assert_eq!(session.backend().focus_state().caret_byte_index, 0);
    assert!(session.handle_pointer_move_logical(r10.x + 220.0, mid_y).unwrap());
    let drag_range = session
        .backend()
        .focus_state()
        .selection_range()
        .expect("drag should create non-empty range selection");
    assert_eq!(drag_range.0, 0);
    assert_eq!(drag_range.1, " world hello".len());
    session.handle_pointer_up_logical();

    // Subsequent pointer move after mouse up does NOT change selection.
    assert!(!session.handle_pointer_move_logical(r10.x + 12.0, mid_y).unwrap());
    assert_eq!(
        session.backend().focus_state().selection_range(),
        Some(drag_range)
    );

    // Double-click over "world" selects the whole word "world".
    let word_x = r10.x + 28.0;
    session.handle_click_logical(word_x, mid_y).unwrap();
    session.handle_pointer_up_logical();
    session.handle_click_logical(word_x, mid_y).unwrap();
    session.handle_pointer_up_logical();
    assert_eq!(session.selected_text().as_deref(), Some("world"));

    // Secure input (node 11) must refuse Cmd+C so password plaintext never leaks to clipboard.
    let r11 = session.layout_of(NodeId::new(11, 0)).unwrap();
    session
        .handle_click_logical(r11.x + 15.0, r11.y + 15.0)
        .unwrap();
    session.handle_pointer_up_logical();
    session
        .handle_keyboard_input(&Key::Character("a".into()), None, cmd)
        .unwrap();
    assert!(!session
        .handle_keyboard_input(&Key::Character("c".into()), None, cmd)
        .unwrap());
    assert_eq!(
        session.clipboard_mut().get_text().as_deref(),
        Some("hello"),
        "clipboard must remain unchanged when copying from secureTextEntry"
    );
}

#[test]
fn hidden_input_and_display_none_nodes_have_zero_layout_paint_hit_and_reject_focus() {
    struct HiddenInputApp {
        store: NodeStore,
        encoder: WireEncoder,
        changes: ChangeSet,
        initialized: bool,
        frame_buf: Vec<u8>,
    }

    impl HiddenInputApp {
        fn new() -> Self {
            register_core_tables();
            Self {
                store: NodeStore::new(),
                encoder: WireEncoder::default(),
                changes: ChangeSet::default(),
                initialized: false,
                frame_buf: Vec::new(),
            }
        }
    }

    impl DesktopApp for HiddenInputApp {
        fn generation(&self) -> u64 {
            1
        }

        fn render(&mut self, _frame_id: FrameId) -> &[u8] {
            if !self.initialized {
                let mut tx = self.store.tx();
                for id in [10u32, 11, 12, 13] {
                    let _ = tx.create_at(NodeId::new(id, 0));
                }
                // Node 10: bare TextInput { input_kind: Hidden, value: "secret" } (no Layout::display)
                tx.edit::<TextInput>(NodeId::new(10, 0))
                    .input_kind(InputKind::Hidden)
                    .value("secret");
                tx.edit::<Interaction>(NodeId::new(10, 0)).listeners(
                    EventSet::one(InteractionEvent::Press)
                        .with(InteractionEvent::Focus)
                        .with(InteractionEvent::Blur),
                );
                // Node 11: Display::None TextInput
                tx.edit::<TextInput>(NodeId::new(11, 0)).value("display-none");
                tx.edit::<Layout>(NodeId::new(11, 0))
                    .display(Display::None)
                    .width(Length::points(180.0))
                    .height(Length::points(36.0));
                tx.edit::<Interaction>(NodeId::new(11, 0)).listeners(
                    EventSet::one(InteractionEvent::Focus).with(InteractionEvent::Blur),
                );
                // Node 12: Display::None parent containing visible-looking TextInput child 13
                tx.edit::<Layout>(NodeId::new(12, 0)).display(Display::None);
                tx.edit::<TextInput>(NodeId::new(13, 0)).value("under-none");
                tx.set_children(NodeId::new(12, 0), &[NodeId::new(13, 0)]);
                tx.set_children(
                    NodeId::ROOT,
                    &[NodeId::new(10, 0), NodeId::new(11, 0), NodeId::new(12, 0)],
                );
                tx.commit_into(&mut self.changes);
                self.initialized = true;
            }
            self.frame_buf.clear();
            self.encoder
                .encode(&self.store, &self.changes, &mut self.frame_buf);
            &self.frame_buf
        }

        fn dispatch_events(&mut self, _events: ValidatedEvents<'_>) -> bool {
            false
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

    let mut session = DesktopSession::new(
        HiddenInputApp::new(),
        SurfaceId::ROOT,
        Viewport {
            width: 320.0,
            height: 200.0,
            scale_factor: 1.0,
        },
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().unwrap();

    assert_eq!(
        session.backend().scene().live_op_count(),
        0,
        "hidden/display:none inputs must produce 0 paint ops"
    );
    for id in [NodeId::new(10, 0), NodeId::new(11, 0), NodeId::new(13, 0)] {
        let r = session.layout_of(id).expect("layout entry");
        assert_eq!((r.width, r.height), (0.0, 0.0), "node {id} must be 0x0");
        assert!(
            !is_focusable(session.replica.store(), id),
            "node {id} must not be focusable"
        );
        assert!(
            !session.focus_node(Some(id)).unwrap(),
            "focus_node({id}) must be rejected"
        );
        assert_eq!(session.backend().focused_node(), None);
    }
    assert!(!session.handle_click_logical(5.0, 5.0).unwrap());
    assert_eq!(session.backend().focused_node(), None);
}
