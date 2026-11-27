use std::path::Path;

use nve_abi::{
    ColorRgba, EventSet, FrameId, InteractionEvent, NodeId, SurfaceId, TextInputEvent,
    ValidatedEvents,
};
use nve_backend_skia::{grapheme_count, is_grapheme_boundary};
use nve_tables::tables::{
    register_core_tables, FlexDirection, Interaction, Layout, TextInput, Typography, Visual,
};
use nve_engine::Viewport;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, Length, NodeStore, Rgba};
use winit::event::Ime;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::clipboard::InMemoryClipboard;
use super::input::{
    normalize_line_endings, paragraph_bounds_at, sanitize_pasted_multiline,
    sanitize_pasted_single_line, word_bounds_at,
};
use super::{DesktopApp, DesktopHostError, DesktopSession};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoggedEvent {
    TextChange,
    CompositionStart,
    CompositionUpdate,
    CompositionEnd,
    Focus,
    Blur,
}

struct TextMaturityApp {
    single_val: String,
    multi_val: String,
    events_log: Vec<(NodeId, LoggedEvent, String)>,
    store: NodeStore,
    encoder: WireEncoder,
    changes: ChangeSet,
    initialized: bool,
    frame_buf: Vec<u8>,
}

impl TextMaturityApp {
    fn new(single: &str, multi: &str) -> Self {
        register_core_tables();
        Self {
            single_val: single.to_owned(),
            multi_val: multi.to_owned(),
            events_log: Vec::new(),
            store: NodeStore::new(),
            encoder: WireEncoder::default(),
            changes: ChangeSet::default(),
            initialized: false,
            frame_buf: Vec::new(),
        }
    }
}

impl DesktopApp for TextMaturityApp {
    fn generation(&self) -> u64 {
        1
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        let mut tx = self.store.tx();
        if !self.initialized {
            for id in [1u32, 2, 10, 11] {
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

            // Node 10: single-line TextInput
            tx.attach::<TextInput>(NodeId::new(10, 0));
            tx.set(NodeId::new(10, 0), Layout::WIDTH, Length::points(340.0));
            tx.set(NodeId::new(10, 0), Layout::HEIGHT, Length::points(36.0));
            tx.set(NodeId::new(10, 0), Layout::PADDING_LEFT, Length::points(10.0));
            tx.set(NodeId::new(10, 0), Layout::PADDING_RIGHT, Length::points(10.0));
            tx.set(NodeId::new(10, 0), Visual::BACKGROUND, Rgba::rgb(30, 41, 59));
            tx.set(NodeId::new(10, 0), Typography::COLOR, Rgba::WHITE);
            tx.set(NodeId::new(10, 0), Typography::FONT_SIZE, 16.0);
            tx.edit::<Interaction>(NodeId::new(10, 0))
                .listeners(EventSet::one(InteractionEvent::Focus).with(InteractionEvent::Blur));
            tx.edit::<TextInput>(NodeId::new(10, 0)).listeners(
                EventSet::one(TextInputEvent::Change)
                    .with(TextInputEvent::CompositionStart)
                    .with(TextInputEvent::CompositionUpdate)
                    .with(TextInputEvent::CompositionEnd),
            );

            // Node 11: multiline TextInput
            tx.attach::<TextInput>(NodeId::new(11, 0));
            tx.set(NodeId::new(11, 0), Layout::WIDTH, Length::points(340.0));
            tx.set(NodeId::new(11, 0), Layout::HEIGHT, Length::points(60.0));
            tx.set(NodeId::new(11, 0), Layout::PADDING_LEFT, Length::points(10.0));
            tx.set(NodeId::new(11, 0), Layout::PADDING_RIGHT, Length::points(10.0));
            tx.set(NodeId::new(11, 0), Visual::BACKGROUND, Rgba::rgb(30, 41, 59));
            tx.set(NodeId::new(11, 0), Typography::COLOR, Rgba::WHITE);
            tx.set(NodeId::new(11, 0), Typography::FONT_SIZE, 16.0);
            tx.set(NodeId::new(11, 0), TextInput::MULTILINE, true);
            tx.edit::<Interaction>(NodeId::new(11, 0))
                .listeners(EventSet::one(InteractionEvent::Focus).with(InteractionEvent::Blur));
            tx.edit::<TextInput>(NodeId::new(11, 0)).listeners(
                EventSet::one(TextInputEvent::Change)
                    .with(TextInputEvent::CompositionStart)
                    .with(TextInputEvent::CompositionUpdate)
                    .with(TextInputEvent::CompositionEnd),
            );

            tx.set_children(NodeId::new(2, 0), &[NodeId::new(10, 0), NodeId::new(11, 0)]);
            tx.set_children(NodeId::ROOT, &[NodeId::new(2, 0)]);
            self.initialized = true;
        }
        tx.set_str(NodeId::new(10, 0), TextInput::VALUE, &self.single_val);
        tx.set_str(NodeId::new(11, 0), TextInput::VALUE, &self.multi_val);
        tx.commit_into(&mut self.changes);
        self.frame_buf.clear();
        self.encoder
            .encode(&self.store, &self.changes, &mut self.frame_buf);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            if let Some(te) = ev.text_input() {
                match te {
                    TextInputEvent::Change => {
                        let t = ev.text().to_owned();
                        if ev.node() == NodeId::new(10, 0) {
                            self.single_val = t.clone();
                        } else if ev.node() == NodeId::new(11, 0) {
                            self.multi_val = t.clone();
                        }
                        self.events_log.push((ev.node(), LoggedEvent::TextChange, t));
                        handled = true;
                    }
                    TextInputEvent::CompositionStart => {
                        self.events_log
                            .push((ev.node(), LoggedEvent::CompositionStart, ev.text().to_owned()));
                        handled = true;
                    }
                    TextInputEvent::CompositionUpdate => {
                        self.events_log.push((
                            ev.node(),
                            LoggedEvent::CompositionUpdate,
                            ev.text().to_owned(),
                        ));
                        handled = true;
                    }
                    TextInputEvent::CompositionEnd => {
                        self.events_log
                            .push((ev.node(), LoggedEvent::CompositionEnd, ev.text().to_owned()));
                        handled = true;
                    }
                    _ => {}
                }
            } else if let Some(ie) = ev.interaction() {
                match ie {
                    InteractionEvent::Focus => {
                        self.events_log.push((ev.node(), LoggedEvent::Focus, String::new()));
                        handled = true;
                    }
                    InteractionEvent::Blur => {
                        self.events_log.push((ev.node(), LoggedEvent::Blur, String::new()));
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

fn new_session(single: &str, multi: &str) -> DesktopSession<TextMaturityApp> {
    let vp = Viewport {
        width: 380.0,
        height: 220.0,
        scale_factor: 2.0,
    };
    let mut session = DesktopSession::new(
        TextMaturityApp::new(single, multi),
        SurfaceId::ROOT,
        vp,
        ColorRgba::BLACK,
        false,
    )
    .with_clipboard(Box::new(InMemoryClipboard::new()));
    session.render_and_rasterize().unwrap();
    session
}

#[test]
fn grapheme_safe_editing_for_zwj_family_flag_skin_tone_and_combining_accent() {
    let family = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
    let flag = "🇩🇪";
    let wave = "👋🏽";
    let accent = "e\u{0301}";
    let initial = format!("{family}{flag}{wave}{accent}");

    let mut session = new_session(&initial, "");
    session.focus_node(Some(NodeId::new(10, 0))).unwrap();
    let none = ModifiersState::empty();
    let shift = ModifiersState::SHIFT;

    assert_eq!(session.backend().focus_state().caret_byte_index, initial.len());
    assert_eq!(grapheme_count(&initial[..session.backend().focus_state().caret_byte_index]), 4);

    // ArrowLeft moves over the entire `e\u{0301}` combining cluster in ONE step.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, none)
        .unwrap());
    assert_eq!(
        session.backend().focus_state().caret_byte_index,
        initial.len() - accent.len()
    );

    // Shift+ArrowLeft selects the entire skin-tone wave `👋🏽` in ONE step.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, shift)
        .unwrap());
    assert_eq!(session.selected_text().as_deref(), Some(wave));

    // Backspace deletes the selected `👋🏽` cleanly without corrupting neighbors.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Backspace), None, none)
        .unwrap());
    assert_eq!(session.app().single_val, format!("{family}{flag}{accent}"));

    // Delete removes the entire `e\u{0301}` cluster forward in ONE keypress.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Delete), None, none)
        .unwrap());
    assert_eq!(session.app().single_val, format!("{family}{flag}"));

    // Backspace removes the entire flag `🇩🇪` in ONE keypress (not half a regional indicator!).
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Backspace), None, none)
        .unwrap());
    assert_eq!(session.app().single_val, family);

    // Backspace removes the entire 25-byte ZWJ family emoji `👨‍👩‍👧‍👦` in ONE keypress!
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::Backspace), None, none)
        .unwrap());
    assert_eq!(session.app().single_val, "");
    assert_eq!(session.backend().focus_state().caret_byte_index, 0);
}

#[test]
fn double_click_word_triple_click_paragraph_and_crlf_paste_normalization() {
    let mut session = new_session("hello world_42 test", "line one\nline two");
    let r10 = session.layout_of(NodeId::new(10, 0)).unwrap();
    let click_x = r10.x + 28.0;
    let click_y = r10.y + r10.height * 0.5;

    // Click 1: caret placement.
    session.handle_click_logical(click_x, click_y).unwrap();
    session.handle_pointer_up_logical();
    assert_eq!(session.backend().focus_state().selection_range(), None);

    // Click 2: double-click word select ("hello").
    session.handle_click_logical(click_x, click_y).unwrap();
    session.handle_pointer_up_logical();
    assert_eq!(session.selected_text().as_deref(), Some("hello"));

    // Click 3: triple-click paragraph/line select (entire line!).
    session.handle_click_logical(click_x, click_y).unwrap();
    session.handle_pointer_up_logical();
    assert_eq!(
        session.selected_text().as_deref(),
        Some("hello world_42 test")
    );

    // Paste with CRLF + CR into single-line normalizes to spaces cleanly.
    session
        .clipboard_mut()
        .set_text("alpha\r\nbeta\rgamma\ndelta");
    assert!(session
        .handle_keyboard_input(&Key::Character("v".into()), None, ModifiersState::SUPER)
        .unwrap());
    assert_eq!(session.app().single_val, "alpha beta gamma delta");

    // Paste with CRLF + CR into multiline (node 11) normalizes line endings to `\n`.
    session.focus_node(Some(NodeId::new(11, 0))).unwrap();
    session
        .handle_keyboard_input(&Key::Character("a".into()), None, ModifiersState::SUPER)
        .unwrap();
    assert!(session
        .handle_keyboard_input(&Key::Character("v".into()), None, ModifiersState::SUPER)
        .unwrap());
    assert_eq!(session.app().multi_val, "alpha\nbeta\ngamma\ndelta");

    assert_eq!(normalize_line_endings("x\r\ny\rz"), "x\ny\nz");
    assert_eq!(sanitize_pasted_single_line("x\r\ny\rz\tw"), "x y z w");
    assert_eq!(sanitize_pasted_multiline("x\r\ny\rz\x07"), "x\ny\nz");
    assert_eq!(word_bounds_at("café\u{0301} ok", 2), (0, "café\u{0301}".len()));
    assert_eq!(paragraph_bounds_at("a\r\nbb\nc", 4), (3, 5));
}

#[test]
fn bidi_visual_navigation_and_backspace_never_panic_or_split_clusters() {
    let mut session = new_session("Hello مرحبا world", "");
    session.focus_node(Some(NodeId::new(10, 0))).unwrap();
    let none = ModifiersState::empty();

    // Sweep ArrowLeft from end to start, then ArrowRight back to end.
    for _ in 0..30 {
        let _ = session
            .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, none)
            .unwrap();
        let val = session.app().single_val.clone();
        assert!(is_grapheme_boundary(&val, session.backend().focus_state().caret_byte_index));
    }
    for _ in 0..30 {
        let _ = session
            .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, none)
            .unwrap();
        let val = session.app().single_val.clone();
        assert!(is_grapheme_boundary(&val, session.backend().focus_state().caret_byte_index));
    }

    // Backspace across the entire mixed LTR+RTL string deletes one grapheme at a time cleanly.
    session
        .handle_keyboard_input(&Key::Named(NamedKey::End), None, none)
        .unwrap();
    while !session.app().single_val.is_empty() {
        let before_len = session.app().single_val.len();
        assert!(session
            .handle_keyboard_input(&Key::Named(NamedKey::Backspace), None, none)
            .unwrap());
        let val = &session.app().single_val;
        assert!(val.len() < before_len);
        assert!(is_grapheme_boundary(val, session.backend().focus_state().caret_byte_index));
    }
}

#[test]
fn unified_ime_lifecycle_emits_start_update_end_blur_cancel_and_tracks_live_caret_rect() {
    let mut session = new_session("", "");
    session.focus_node(Some(NodeId::new(10, 0))).unwrap();
    session.app_mut().events_log.clear();

    let r0 = session.ime_cursor_rect_logical().expect("focused caret rect");
    let r0_phys = session.ime_cursor_rect_physical().unwrap();
    assert!((r0_phys.x - r0.x * 2.0).abs() < 0.01);

    // 1. Preedit("ni") -> CompositionStart("ni"), caret/preedit rect widens.
    assert!(session
        .handle_ime(&Ime::Preedit("ni".into(), Some((0, 2))))
        .unwrap());
    let r_ni = session.ime_cursor_rect_logical().unwrap();
    assert!(r_ni.width > r0.width);

    // 2. Preedit("nihao") -> CompositionUpdate("nihao"), rect grows further.
    assert!(session
        .handle_ime(&Ime::Preedit("nihao".into(), Some((0, 5))))
        .unwrap());
    let r_nihao = session.ime_cursor_rect_logical().unwrap();
    assert!(r_nihao.width > r_ni.width);

    // 3. Commit("你好") -> CompositionEnd("你好") + TextChange("你好"), caret rect shifts right.
    assert!(session.handle_ime(&Ime::Commit("你好".into())).unwrap());
    let r_committed = session.ime_cursor_rect_logical().unwrap();
    assert!(
        r_committed.x > r0.x + 8.0,
        "caret x must advance after commit: r0={r0:?}, committed={r_committed:?}"
    );
    assert_eq!(
        session.app().events_log,
        vec![
            (NodeId::new(10, 0), LoggedEvent::CompositionStart, "ni".into()),
            (NodeId::new(10, 0), LoggedEvent::CompositionUpdate, "nihao".into()),
            (NodeId::new(10, 0), LoggedEvent::CompositionEnd, "你好".into()),
            (NodeId::new(10, 0), LoggedEvent::TextChange, "你好".into()),
        ]
    );

    // 4. Blur mid-composition -> emits clean CompositionEnd("") followed by Blur!
    session.app_mut().events_log.clear();
    assert!(session
        .handle_ime(&Ime::Preedit("zhong".into(), Some((0, 5))))
        .unwrap());
    assert!(session.focus_node(None).unwrap());
    assert_eq!(session.backend().focus_state().ime_preedit.as_deref(), None);
    assert_eq!(session.ime_cursor_rect_logical(), None);
    assert_eq!(
        session.app().events_log,
        vec![
            (NodeId::new(10, 0), LoggedEvent::CompositionStart, "zhong".into()),
            (NodeId::new(10, 0), LoggedEvent::CompositionEnd, String::new()),
            (NodeId::new(10, 0), LoggedEvent::Blur, String::new()),
        ]
    );

    // 5. Empty Preedit and Ime::Disabled mid-composition both emit clean CompositionEnd("").
    session.focus_node(Some(NodeId::new(10, 0))).unwrap();
    session.app_mut().events_log.clear();
    assert!(session
        .handle_ime(&Ime::Preedit("ka".into(), Some((0, 2))))
        .unwrap());
    assert!(session.handle_ime(&Ime::Preedit(String::new(), None)).unwrap());
    assert!(session
        .handle_ime(&Ime::Preedit("ki".into(), Some((0, 2))))
        .unwrap());
    assert!(session.handle_ime(&Ime::Disabled).unwrap());
    assert_eq!(
        session.app().events_log,
        vec![
            (NodeId::new(10, 0), LoggedEvent::CompositionStart, "ka".into()),
            (NodeId::new(10, 0), LoggedEvent::CompositionEnd, String::new()),
            (NodeId::new(10, 0), LoggedEvent::CompositionStart, "ki".into()),
            (NodeId::new(10, 0), LoggedEvent::CompositionEnd, String::new()),
        ]
    );
}
