use std::path::Path;

use nve_abi::{
    ColorRgba, EventSet, FrameId, InteractionEvent, NodeId, SurfaceId, TextInputEvent,
    ValidatedEvents,
};
use nve_tables::tables::{
    register_core_tables, Display, FlexDirection, InputKind, Interaction, Layout, TextInput,
    Typography, Visual,
};
use nve_engine::Viewport;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore, Rgba};
use winit::event::Ime;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::clipboard::InMemoryClipboard;
use super::input::{format_keydown_shorthand, is_focusable, word_bounds_at};
use super::{DesktopApp, DesktopHostError, DesktopSession};

const FOCUS_RING_COLOR: ColorRgba = ColorRgba::rgb(56, 189, 248);

struct MockTextInputApp {
    title_value: String,
    password_value: String,
    has_custom_listener: bool,
    focus_log: Vec<(NodeId, &'static str)>,
    text_changes: Vec<(NodeId, String)>,
    submits: Vec<(NodeId, String)>,
    keydowns: Vec<(NodeId, String)>,
    store: NodeStore,
    encoder: WireEncoder,
    changes: ChangeSet,
    initialized: bool,
    frame_buf: Vec<u8>,
}

impl MockTextInputApp {
    fn new(has_custom_listener: bool) -> Self {
        register_core_tables();
        Self {
            title_value: String::new(),
            password_value: "pw".to_string(),
            has_custom_listener,
            focus_log: Vec::new(),
            text_changes: Vec::new(),
            submits: Vec::new(),
            keydowns: Vec::new(),
            store: NodeStore::new(),
            encoder: WireEncoder::default(),
            changes: ChangeSet::default(),
            initialized: false,
            frame_buf: Vec::new(),
        }
    }
}

impl DesktopApp for MockTextInputApp {
    fn generation(&self) -> u64 {
        1
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        let mut tx = self.store.tx();
        if !self.initialized {
            for id in [1u32, 2, 10, 11, 12] {
                let _ = tx.create_at(NodeId::new(id, 0));
            }
            tx.set(NodeId::new(2, 0), Layout::WIDTH, Length::percent(100.0));
            tx.set(NodeId::new(2, 0), Layout::HEIGHT, Length::percent(100.0));
            let p12 = Length::points(12.0);
            tx.set(NodeId::new(2, 0), Layout::PADDING_TOP, p12);
            tx.set(NodeId::new(2, 0), Layout::PADDING_RIGHT, p12);
            tx.set(NodeId::new(2, 0), Layout::PADDING_BOTTOM, p12);
            tx.set(NodeId::new(2, 0), Layout::PADDING_LEFT, p12);
            tx.set(NodeId::new(2, 0), Layout::ROW_GAP, Length::points(10.0));
            tx.set(NodeId::new(2, 0), Layout::COLUMN_GAP, Length::points(10.0));
            tx.set(NodeId::new(2, 0), Layout::FLEX_DIRECTION, FlexDirection::Column);
            tx.set(NodeId::new(2, 0), Visual::BACKGROUND, Rgba::rgb(15, 23, 42));
            if self.has_custom_listener {
                tx.edit::<Interaction>(NodeId::new(2, 0))
                    .listeners(EventSet::one(InteractionEvent::KeyDown));
            }

            for id in [10u32, 11, 12] {
                let nid = NodeId::new(id, 0);
                tx.attach::<TextInput>(nid);
                tx.set(nid, Layout::WIDTH, Length::points(260.0));
                tx.set(nid, Layout::HEIGHT, Length::points(36.0));
                tx.set(nid, Layout::PADDING_LEFT, Length::points(10.0));
                tx.set(nid, Layout::PADDING_RIGHT, Length::points(10.0));
                tx.set(nid, Visual::BACKGROUND, Rgba::rgb(30, 41, 59));
                tx.set(nid, Typography::COLOR, Rgba::rgb(241, 245, 249));
                tx.set(nid, Typography::FONT_SIZE, 14.0);
            }
            tx.set(NodeId::new(10, 0), Visual::BORDER_COLOR, Rgba::rgb(71, 85, 105));
            tx.set(NodeId::new(10, 0), Visual::BORDER_WIDTH, 1.0);
            tx.set(NodeId::new(10, 0), Visual::BORDER_RADIUS, 6.0);
            tx.set_str(NodeId::new(10, 0), TextInput::PLACEHOLDER, "Search...");
            tx.edit::<Interaction>(NodeId::new(10, 0)).listeners(
                EventSet::one(InteractionEvent::Focus)
                    .with(InteractionEvent::Blur)
                    .with(InteractionEvent::KeyDown),
            );
            tx.edit::<TextInput>(NodeId::new(10, 0)).listeners(
                EventSet::one(TextInputEvent::Change).with(TextInputEvent::Submit),
            );

            tx.set(NodeId::new(11, 0), Visual::BORDER_RADIUS, 6.0);
            tx.set(NodeId::new(11, 0), TextInput::SECURE, true);
            tx.edit::<Interaction>(NodeId::new(11, 0))
                .listeners(EventSet::one(InteractionEvent::Focus).with(InteractionEvent::Blur));
            tx.edit::<TextInput>(NodeId::new(11, 0))
                .listeners(EventSet::one(TextInputEvent::Change));

            tx.set_children(
                NodeId::new(2, 0),
                &[NodeId::new(10, 0), NodeId::new(11, 0), NodeId::new(12, 0)],
            );
            tx.set_children(NodeId::ROOT, &[NodeId::new(2, 0)]);
            self.initialized = true;
        }
        tx.set_str(NodeId::new(10, 0), TextInput::VALUE, &self.title_value);
        tx.set_str(NodeId::new(11, 0), TextInput::VALUE, &self.password_value);
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
                match ie {
                    InteractionEvent::Focus => {
                        self.focus_log.push((ev.node(), "focus"));
                        handled = true;
                    }
                    InteractionEvent::Blur => {
                        self.focus_log.push((ev.node(), "blur"));
                        handled = true;
                    }
                    InteractionEvent::KeyDown => {
                        let shorthand = format_keydown_shorthand(
                            ev.key(),
                            ev.meta(),
                            ev.ctrl(),
                            ev.shift(),
                            ev.alt(),
                        );
                        self.keydowns.push((ev.node(), shorthand));
                        handled = true;
                    }
                    _ => {}
                }
            } else if let Some(te) = ev.text_input() {
                match te {
                    TextInputEvent::Change => {
                        let text = ev.text().to_owned();
                        if ev.node() == NodeId::new(10, 0) {
                            self.title_value = text.clone();
                            handled = true;
                        } else if ev.node() == NodeId::new(11, 0) {
                            self.password_value = text.clone();
                            handled = true;
                        }
                        self.text_changes.push((ev.node(), text));
                    }
                    TextInputEvent::Submit => {
                        self.submits.push((ev.node(), ev.text().to_owned()));
                        handled = true;
                    }
                    _ => {}
                }
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

fn new_text_session(has_custom_listener: bool) -> DesktopSession<MockTextInputApp> {
    let viewport = Viewport {
        width: 320.0,
        height: 220.0,
        scale_factor: 1.0,
    };
    let mut session = DesktopSession::new(
        MockTextInputApp::new(has_custom_listener),
        SurfaceId::ROOT,
        viewport,
        ColorRgba::BLACK,
        false,
    )
    .with_clipboard(Box::new(InMemoryClipboard::new()));
    session.render_and_rasterize().unwrap();
    session
}

#[test]
fn format_keydown_shorthand_produces_compact_strings() {
    assert_eq!(format_keydown_shorthand("k", true, false, false, false), "Cmd+k");
    assert_eq!(
        format_keydown_shorthand("Enter", false, true, true, false),
        "Ctrl+Shift+Enter"
    );
    assert_eq!(
        format_keydown_shorthand("ArrowDown", false, false, false, false),
        "ArrowDown"
    );
    assert_eq!(
        format_keydown_shorthand("Escape", false, false, false, false),
        "Escape"
    );
    assert_eq!(
        format_keydown_shorthand("x", true, true, true, true),
        "Cmd+Ctrl+Shift+Alt+x"
    );
}

#[test]
fn click_focus_and_blur_transitions_emit_events_and_update_focus_ring_pixels() {
    let mut session = new_text_session(false);
    let r10 = session.layout_of(NodeId::new(10, 0)).unwrap();
    let r11 = session.layout_of(NodeId::new(11, 0)).unwrap();

    // Click node 10 -> Focus(10) and sky-blue focus ring painted.
    assert!(session
        .handle_click_logical(r10.x + 20.0, r10.y + 18.0)
        .unwrap());
    assert_eq!(session.focused_node(), Some(NodeId::new(10, 0)));
    assert_eq!(session.app().focus_log, vec![(NodeId::new(10, 0), "focus")]);
    assert_eq!(
        session
            .pixel_buffer()
            .get_rgb((r10.x + r10.width * 0.5) as u32, r10.y as u32),
        Some((FOCUS_RING_COLOR.r, FOCUS_RING_COLOR.g, FOCUS_RING_COLOR.b))
    );

    // Click node 11 -> Blur(10) + Focus(11), focus ring moves from 10 to 11.
    assert!(session
        .handle_click_logical(r11.x + 20.0, r11.y + 18.0)
        .unwrap());
    assert_eq!(session.focused_node(), Some(NodeId::new(11, 0)));
    assert_eq!(
        session.app().focus_log,
        vec![(NodeId::new(10, 0), "focus"), (NodeId::new(10, 0), "blur"), (NodeId::new(11, 0), "focus")]
    );
    assert_ne!(
        session
            .pixel_buffer()
            .get_rgb((r10.x + r10.width * 0.5) as u32, r10.y as u32),
        Some((FOCUS_RING_COLOR.r, FOCUS_RING_COLOR.g, FOCUS_RING_COLOR.b))
    );
    assert_eq!(
        session
            .pixel_buffer()
            .get_rgb((r11.x + r11.width * 0.5) as u32, r11.y as u32),
        Some((FOCUS_RING_COLOR.r, FOCUS_RING_COLOR.g, FOCUS_RING_COLOR.b))
    );

    // Click outside all inputs -> Blur(11), focus ring disappears.
    assert!(session.handle_click_logical(310.0, 210.0).unwrap());
    assert_eq!(session.focused_node(), None);
    assert_eq!(
        session.app().focus_log,
        vec![
            (NodeId::new(10, 0), "focus"),
            (NodeId::new(10, 0), "blur"),
            (NodeId::new(11, 0), "focus"),
            (NodeId::new(11, 0), "blur")
        ]
    );
    assert_ne!(
        session
            .pixel_buffer()
            .get_rgb((r11.x + r11.width * 0.5) as u32, r11.y as u32),
        Some((FOCUS_RING_COLOR.r, FOCUS_RING_COLOR.g, FOCUS_RING_COLOR.b))
    );
}

#[test]
fn keyboard_editing_supports_utf8_navigation_backspace_delete_select_all_and_submit() {
    let mut session = new_text_session(false);
    let r10 = session.layout_of(NodeId::new(10, 0)).unwrap();
    session
        .handle_click_logical(r10.x + 15.0, r10.y + 15.0)
        .unwrap();

    let none = ModifiersState::empty();
    let cmd = ModifiersState::SUPER;

    // Type "café" + space + "🚀".
    for ch in ["c", "a", "f", "é"] {
        assert!(session
            .handle_keyboard_input(&Key::Character(ch.into()), Some(ch), none)
            .unwrap());
    }
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Space), Some(" "), none)
        .unwrap());
    assert!(session
        .handle_keyboard_input(&Key::Character("🚀".into()), Some("🚀"), none)
        .unwrap());
    assert_eq!(session.app().title_value, "café 🚀");

    // ArrowLeft moves cleanly over the 4-byte emoji scalar and space; Backspace deletes 'é'.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, none)
        .unwrap());
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, none)
        .unwrap());
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Backspace), None, none)
        .unwrap());
    assert_eq!(session.app().title_value, "caf 🚀");

    // Insert 'e' at caret -> "cafe 🚀".
    assert!(session
        .handle_keyboard_input(&Key::Character("e".into()), Some("e"), none)
        .unwrap());
    assert_eq!(session.app().title_value, "cafe 🚀");

    // Home -> Delete removes leading 'c' -> "afe 🚀".
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Home), None, none)
        .unwrap());
    assert_eq!(session.backend().focus_state().caret_byte_index, 0);
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Delete), None, none)
        .unwrap());
    assert_eq!(session.app().title_value, "afe 🚀");

    // ArrowRight -> End -> type '!' -> "afe 🚀!".
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, none)
        .unwrap());
    assert_eq!(session.backend().focus_state().caret_byte_index, 1);
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::End), None, none)
        .unwrap());
    assert!(session
        .handle_keyboard_input(&Key::Character("!".into()), Some("!"), none)
        .unwrap());
    assert_eq!(session.app().title_value, "afe 🚀!");

    // Cmd+A selects all; typing replaces entire text -> "done".
    assert!(session
        .handle_keyboard_input(&Key::Character("a".into()), None, cmd)
        .unwrap());
    assert_eq!(session.backend().focus_state().selection_range(), Some((0, "afe 🚀!".len())));
    assert!(session
        .handle_keyboard_input(&Key::Character("done".into()), Some("done"), none)
        .unwrap());
    assert_eq!(session.backend().focus_state().selection_range(), None);
    assert_eq!(session.app().title_value, "done");

    // Enter emits typed Submit on the focused TextInput node.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Enter), None, none)
        .unwrap());
    assert_eq!(session.app().submits, vec![(NodeId::new(10, 0), "done".to_string())]);
}

#[test]
fn cjk_ime_preedit_commit_and_escape_behavior() {
    let mut session = new_text_session(true);
    let r10 = session.layout_of(NodeId::new(10, 0)).unwrap();
    session
        .handle_click_logical(r10.x + 15.0, r10.y + 15.0)
        .unwrap();

    let none = ModifiersState::empty();
    // Type ASCII prefix "NVE " first.
    assert!(session
        .handle_keyboard_input(&Key::Character("NVE ".into()), Some("NVE "), none)
        .unwrap());
    let changes_before = session.app().text_changes.len();

    // IME Preedit renders composition preview without firing premature TextChange.
    assert!(session
        .handle_ime(&Ime::Preedit("にほん".to_string(), Some((0, "にほん".len()))))
        .unwrap());
    assert_eq!(session.backend().focus_state().ime_preedit.as_deref(), Some("にほん"));
    assert_eq!(session.app().text_changes.len(), changes_before);

    // Pressing Escape while IME preedit is active cancels preedit without blurring.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Escape), None, none)
        .unwrap());
    assert_eq!(session.backend().focus_state().ime_preedit.as_deref(), None);
    assert_eq!(session.focused_node(), Some(NodeId::new(10, 0)));

    // Re-enter IME composition and Commit CJK text -> emits TextChange and clears preedit.
    assert!(session
        .handle_ime(&Ime::Preedit("かんじ".to_string(), Some((0, "かんじ".len()))))
        .unwrap());
    assert!(session
        .handle_ime(&Ime::Commit("漢字".to_string()))
        .unwrap());
    assert_eq!(session.backend().focus_state().ime_preedit.as_deref(), None);
    assert_eq!(session.app().title_value, "NVE 漢字");
    assert_eq!(session.backend().focus_state().caret_byte_index, "NVE 漢字".len());

    // Cmd+K while focused forwards binary keydown shortcut instead of typing 'k'.
    assert!(session
        .handle_keyboard_input(&Key::Character("k".into()), None, ModifiersState::SUPER)
        .unwrap());
    assert_eq!(session.app().title_value, "NVE 漢字");
    assert_eq!(session.app().keydowns.last().map(|(_, s)| s.as_str()), Some("Cmd+k"));

    // Escape when no IME preedit is active blurs the TextInput and forwards Escape keydown.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Escape), None, none)
        .unwrap());
    assert_eq!(session.focused_node(), None);
    assert_eq!(session.app().focus_log.last(), Some(&(NodeId::new(10, 0), "blur")));
    assert_eq!(
        session.app().keydowns.last().map(|(_, s)| s.as_str()),
        Some("Escape")
    );
}

#[test]
fn uncontrolled_text_input_updates_value_and_caret_locally() {
    let mut session = new_text_session(false);
    let r12 = session.layout_of(NodeId::new(12, 0)).unwrap();
    session
        .handle_click_logical(r12.x + 15.0, r12.y + 15.0)
        .unwrap();
    assert_eq!(session.focused_node(), Some(NodeId::new(12, 0)));

    let none = ModifiersState::empty();
    assert!(session
        .handle_keyboard_input(&Key::Character("hi".into()), Some("hi"), none)
        .unwrap());
    assert_eq!(session.backend().text_input_value(session.store(), NodeId::new(12, 0)).as_deref(), Some("hi"));
    assert_eq!(session.backend().focus_state().caret_byte_index, 2);

    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Backspace), None, none)
        .unwrap());
    assert_eq!(session.backend().text_input_value(session.store(), NodeId::new(12, 0)).as_deref(), Some("h"));
    assert_eq!(session.backend().focus_state().caret_byte_index, 1);
}


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
    assert_eq!(session.backend().focus_state().selection_range(), Some((0, " world hello".len())));
    session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, none)
        .unwrap();
    assert_eq!(session.backend().focus_state().selection_range(), None);
    assert_eq!(session.backend().focus_state().caret_byte_index, " world hello".len());

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
    assert_eq!(session.backend().focus_state().selection_range(), Some(drag_range));

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
        assert!(!session.focus_node(Some(id)).unwrap(), "focus_node({id}) must be rejected");
        assert_eq!(session.backend().focused_node(), None);
    }
    assert!(!session.handle_click_logical(5.0, 5.0).unwrap());
    assert_eq!(session.backend().focused_node(), None);
}
