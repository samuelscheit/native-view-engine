use std::path::Path;

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask,
    FlexDirection, FrameId, LayoutProps, NativeEventKind, NodeId, RawPropValue, StyleProps,
    SurfaceId, ROOT_NODE_ID,
};
use nve_backend_skia::FOCUS_RING_COLOR;
use nve_engine::{RenderBackend, Viewport};
use winit::event::Ime;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::clipboard::InMemoryClipboard;
use super::input::{format_keydown_shorthand, word_bounds_at};
use super::{DesktopApp, DesktopHostError, DesktopSession};

struct MockTextInputApp {
    title_value: String,
    password_value: String,
    has_custom_listener: bool,
    focus_log: Vec<(NodeId, &'static str)>,
    text_changes: Vec<(NodeId, String)>,
    submits: Vec<(NodeId, String)>,
    keydowns: Vec<(NodeId, String)>,
    frame_buf: Vec<u8>,
}

impl MockTextInputApp {
    fn new(has_custom_listener: bool) -> Self {
        Self {
            title_value: String::new(),
            password_value: "pw".to_string(),
            has_custom_listener,
            focus_log: Vec::new(),
            text_changes: Vec::new(),
            submits: Vec::new(),
            keydowns: Vec::new(),
            frame_buf: Vec::new(),
        }
    }
}

impl DesktopApp for MockTextInputApp {
    fn generation(&self) -> u64 {
        1
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let mut b = CommandBufferBuilder::new(1, frame_id);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Percent(100.0),
                padding: EdgeInsets::all(12.0),
                gap: 10.0,
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(15, 23, 42),
                ..Default::default()
            },
        );
        if self.has_custom_listener {
            b.set_event_mask(2, EventMask::KEY_DOWN | EventMask::CUSTOM);
        }

        // Node 10: controlled primary TextInput
        b.create_node(10, ComponentKind::TextInput, None);
        b.set_layout(
            10,
            &LayoutProps {
                width: Dimension::Points(260.0),
                height: Dimension::Points(36.0),
                padding: EdgeInsets::symmetric(0.0, 10.0),
                ..Default::default()
            },
        );
        b.set_style(
            10,
            &StyleProps {
                background: ColorRgba::rgb(30, 41, 59),
                border_color: ColorRgba::rgb(71, 85, 105),
                border_width: 1.0,
                border_radius: 6.0,
                text_color: ColorRgba::rgb(241, 245, 249),
                font_size: 14.0,
                ..Default::default()
            },
        );
        b.set_raw_prop(10, "placeholder", &RawPropValue::String("Search...".into()));
        b.set_raw_prop(10, "value", &RawPropValue::String(self.title_value.clone()));
        b.set_event_mask(
            10,
            EventMask::FOCUS
                | EventMask::BLUR
                | EventMask::TEXT_CHANGE
                | EventMask::SUBMIT
                | EventMask::KEY_DOWN
                | EventMask::CUSTOM,
        );

        // Node 11: controlled secure TextInput
        b.create_node(11, ComponentKind::TextInput, None);
        b.set_layout(
            11,
            &LayoutProps {
                width: Dimension::Points(260.0),
                height: Dimension::Points(36.0),
                padding: EdgeInsets::symmetric(0.0, 10.0),
                ..Default::default()
            },
        );
        b.set_style(
            11,
            &StyleProps {
                background: ColorRgba::rgb(30, 41, 59),
                border_radius: 6.0,
                text_color: ColorRgba::rgb(241, 245, 249),
                font_size: 14.0,
                ..Default::default()
            },
        );
        b.set_raw_prop(
            11,
            "value",
            &RawPropValue::String(self.password_value.clone()),
        );
        b.set_raw_prop(11, "secureTextEntry", &RawPropValue::Bool(true));
        b.set_event_mask(11, EventMask::FOCUS | EventMask::BLUR | EventMask::TEXT_CHANGE);

        // Node 12: uncontrolled TextInput (no `value` prop)
        b.create_node(12, ComponentKind::TextInput, None);
        b.set_layout(
            12,
            &LayoutProps {
                width: Dimension::Points(260.0),
                height: Dimension::Points(36.0),
                padding: EdgeInsets::symmetric(0.0, 10.0),
                ..Default::default()
            },
        );
        b.set_style(
            12,
            &StyleProps {
                background: ColorRgba::rgb(30, 41, 59),
                text_color: ColorRgba::rgb(241, 245, 249),
                font_size: 14.0,
                ..Default::default()
            },
        );

        b.set_children(2, &[10, 11, 12]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        self.frame_buf = b.into_bytes();
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: nve_abi::NveEventBatchRef<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            match (ev.kind(), ev.payload_tag()) {
                (Some(NativeEventKind::Focus), 0) => {
                    self.focus_log.push((ev.node_id(), "focus"));
                    handled = true;
                }
                (Some(NativeEventKind::Blur), 0) => {
                    self.focus_log.push((ev.node_id(), "blur"));
                    handled = true;
                }
                (Some(NativeEventKind::TextChange), 3) => {
                    let text = ev.text().to_owned();
                    if ev.node_id() == 10 {
                        self.title_value = text.clone();
                        handled = true;
                    } else if ev.node_id() == 11 {
                        self.password_value = text.clone();
                        handled = true;
                    }
                    self.text_changes.push((ev.node_id(), text));
                }
                (Some(NativeEventKind::Submit), 0 | 3) => {
                    self.submits.push((ev.node_id(), ev.text().to_owned()));
                    handled = true;
                }
                (Some(NativeEventKind::KeyDown), 6) => {
                    let shorthand = format_keydown_shorthand(
                        ev.key(),
                        ev.meta(),
                        ev.ctrl(),
                        ev.shift(),
                        ev.alt(),
                    );
                    self.keydowns.push((ev.node_id(), shorthand));
                    handled = true;
                }
                _ => {}
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
        1,
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
    let r10 = session.backend().layout_of(10).unwrap();
    let r11 = session.backend().layout_of(11).unwrap();

    // Click node 10 -> Focus(10) and sky-blue focus ring painted.
    assert!(session
        .handle_click_logical(r10.x + 20.0, r10.y + 18.0)
        .unwrap());
    assert_eq!(session.focused_node(), Some(10));
    assert_eq!(session.app().focus_log, vec![(10, "focus")]);
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
    assert_eq!(session.focused_node(), Some(11));
    assert_eq!(
        session.app().focus_log,
        vec![(10, "focus"), (10, "blur"), (11, "focus")]
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
        vec![(10, "focus"), (10, "blur"), (11, "focus"), (11, "blur")]
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
    let r10 = session.backend().layout_of(10).unwrap();
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
    assert_eq!(session.backend().caret_byte_index(), 0);
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Delete), None, none)
        .unwrap());
    assert_eq!(session.app().title_value, "afe 🚀");

    // ArrowRight -> End -> type '!' -> "afe 🚀!".
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, none)
        .unwrap());
    assert_eq!(session.backend().caret_char_index(), 1);
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
    assert_eq!(session.backend().selection_range(), Some((0, "afe 🚀!".len())));
    assert!(session
        .handle_keyboard_input(&Key::Character("done".into()), Some("done"), none)
        .unwrap());
    assert_eq!(session.backend().selection_range(), None);
    assert_eq!(session.app().title_value, "done");

    // Enter emits typed Submit on the focused TextInput node.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Enter), None, none)
        .unwrap());
    assert_eq!(session.app().submits, vec![(10, "done".to_string())]);
}

#[test]
fn cjk_ime_preedit_commit_and_escape_behavior() {
    let mut session = new_text_session(true);
    let r10 = session.backend().layout_of(10).unwrap();
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
    assert_eq!(session.backend().ime_preedit(), Some("にほん"));
    assert_eq!(session.app().text_changes.len(), changes_before);

    // Pressing Escape while IME preedit is active cancels preedit without blurring.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Escape), None, none)
        .unwrap());
    assert_eq!(session.backend().ime_preedit(), None);
    assert_eq!(session.focused_node(), Some(10));

    // Re-enter IME composition and Commit CJK text -> emits TextChange and clears preedit.
    assert!(session
        .handle_ime(&Ime::Preedit("かんじ".to_string(), Some((0, "かんじ".len()))))
        .unwrap());
    assert!(session
        .handle_ime(&Ime::Commit("漢字".to_string()))
        .unwrap());
    assert_eq!(session.backend().ime_preedit(), None);
    assert_eq!(session.app().title_value, "NVE 漢字");
    assert_eq!(session.backend().caret_byte_index(), "NVE 漢字".len());

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
    assert_eq!(session.app().focus_log.last(), Some(&(10, "blur")));
    assert_eq!(
        session.app().keydowns.last().map(|(_, s)| s.as_str()),
        Some("Escape")
    );
}

#[test]
fn uncontrolled_text_input_updates_value_and_caret_locally() {
    let mut session = new_text_session(false);
    let r12 = session.backend().layout_of(12).unwrap();
    session
        .handle_click_logical(r12.x + 15.0, r12.y + 15.0)
        .unwrap();
    assert_eq!(session.focused_node(), Some(12));

    let none = ModifiersState::empty();
    assert!(session
        .handle_keyboard_input(&Key::Character("hi".into()), Some("hi"), none)
        .unwrap());
    assert_eq!(session.backend().text_input_value(12).as_deref(), Some("hi"));
    assert_eq!(session.backend().caret_byte_index(), 2);

    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Backspace), None, none)
        .unwrap());
    assert_eq!(session.backend().text_input_value(12).as_deref(), Some("h"));
    assert_eq!(session.backend().caret_byte_index(), 1);
}


#[test]
fn drag_double_click_shift_selection_and_clipboard_copy_cut_paste_roundtrip() {
    assert_eq!(word_bounds_at("hello world_42!", 1), (0, 5));
    assert_eq!(word_bounds_at("hello world_42!", 8), (6, 14));
    assert_eq!(word_bounds_at("hello world_42!", 15), (14, 15));

    let mut session = new_text_session(false);
    let r10 = session.backend().layout_of(10).unwrap();
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
    assert_eq!(session.backend().selected_text().as_deref(), Some("world"));
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
    assert_eq!(session.backend().selection_range(), None);
    for _ in 0..5 {
        session
            .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, shift)
            .unwrap();
    }
    assert_eq!(session.backend().selected_text().as_deref(), Some("hello"));
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
    assert_eq!(session.backend().selection_range(), Some((0, " world hello".len())));
    session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, none)
        .unwrap();
    assert_eq!(session.backend().selection_range(), None);
    assert_eq!(session.backend().caret_byte_index(), " world hello".len());

    // Mouse press near start + drag to far right forms a non-empty range selection.
    session.handle_click_logical(r10.x + 10.0, mid_y).unwrap();
    assert_eq!(session.backend().caret_byte_index(), 0);
    assert!(session.handle_pointer_move_logical(r10.x + 220.0, mid_y).unwrap());
    let drag_range = session
        .backend()
        .selection_range()
        .expect("drag should create non-empty range selection");
    assert_eq!(drag_range.0, 0);
    assert_eq!(drag_range.1, " world hello".len());
    session.handle_pointer_up_logical();

    // Subsequent pointer move after mouse up does NOT change selection.
    assert!(!session.handle_pointer_move_logical(r10.x + 12.0, mid_y).unwrap());
    assert_eq!(session.backend().selection_range(), Some(drag_range));

    // Double-click over "world" selects the whole word "world".
    let word_x = r10.x + 28.0;
    session.handle_click_logical(word_x, mid_y).unwrap();
    session.handle_pointer_up_logical();
    session.handle_click_logical(word_x, mid_y).unwrap();
    session.handle_pointer_up_logical();
    assert_eq!(session.backend().selected_text().as_deref(), Some("world"));

    // Secure input (node 11) must refuse Cmd+C so password plaintext never leaks to clipboard.
    let r11 = session.backend().layout_of(11).unwrap();
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
