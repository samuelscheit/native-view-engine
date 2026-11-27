use std::path::Path;

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask,
    FlexDirection, FrameId, LayoutProps, NativeEventKind, NodeId, RawPropValue, StyleProps,
    SurfaceId, ROOT_NODE_ID,
};
use nve_backend_skia::is_grapheme_boundary;
use nve_engine::{RenderBackend, Viewport};
use winit::event::Ime;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::clipboard::InMemoryClipboard;
use super::input::{
    normalize_line_endings, paragraph_bounds_at, sanitize_pasted_multiline,
    sanitize_pasted_single_line, word_bounds_at,
};
use super::{DesktopApp, DesktopHostError, DesktopSession};

struct TextMaturityApp {
    single_val: String,
    multi_val: String,
    events_log: Vec<(NodeId, NativeEventKind, String)>,
    frame_buf: Vec<u8>,
}

impl TextMaturityApp {
    fn new(single: &str, multi: &str) -> Self {
        Self {
            single_val: single.to_owned(),
            multi_val: multi.to_owned(),
            events_log: Vec::new(),
            frame_buf: Vec::new(),
        }
    }
}

impl DesktopApp for TextMaturityApp {
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

        // Node 10: single-line TextInput
        b.create_node(10, ComponentKind::TextInput, None);
        b.set_layout(
            10,
            &LayoutProps {
                width: Dimension::Points(340.0),
                height: Dimension::Points(36.0),
                padding: EdgeInsets::symmetric(0.0, 10.0),
                ..Default::default()
            },
        );
        b.set_style(
            10,
            &StyleProps {
                background: ColorRgba::rgb(30, 41, 59),
                text_color: ColorRgba::WHITE,
                font_size: 16.0,
                ..Default::default()
            },
        );
        b.set_raw_prop(10, "value", &RawPropValue::String(self.single_val.clone()));
        b.set_event_mask(
            10,
            EventMask::FOCUS | EventMask::BLUR | EventMask::TEXT_CHANGE | EventMask::COMPOSITION,
        );

        // Node 11: multiline TextInput
        b.create_node(11, ComponentKind::TextInput, None);
        b.set_layout(
            11,
            &LayoutProps {
                width: Dimension::Points(340.0),
                height: Dimension::Points(60.0),
                padding: EdgeInsets::symmetric(0.0, 10.0),
                ..Default::default()
            },
        );
        b.set_style(
            11,
            &StyleProps {
                background: ColorRgba::rgb(30, 41, 59),
                text_color: ColorRgba::WHITE,
                font_size: 16.0,
                ..Default::default()
            },
        );
        b.set_raw_prop(11, "value", &RawPropValue::String(self.multi_val.clone()));
        b.set_raw_prop(11, "multiline", &RawPropValue::Bool(true));
        b.set_event_mask(
            11,
            EventMask::FOCUS | EventMask::BLUR | EventMask::TEXT_CHANGE | EventMask::COMPOSITION,
        );

        b.set_children(2, &[10, 11]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        self.frame_buf = b.into_bytes();
        &self.frame_buf
    }

    fn dispatch_events(&mut self, events: nve_abi::NveEventBatchRef<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            match (ev.kind(), ev.payload_tag()) {
                (Some(NativeEventKind::TextChange), 3) => {
                    let t = ev.text().to_owned();
                    if ev.node_id() == 10 {
                        self.single_val = t.clone();
                    } else if ev.node_id() == 11 {
                        self.multi_val = t.clone();
                    }
                    self.events_log.push((ev.node_id(), NativeEventKind::TextChange, t));
                    handled = true;
                }
                (
                    Some(
                        kind @ (NativeEventKind::CompositionStart
                        | NativeEventKind::CompositionUpdate
                        | NativeEventKind::CompositionEnd),
                    ),
                    0 | 3,
                ) => {
                    self.events_log.push((ev.node_id(), kind, ev.text().to_owned()));
                    handled = true;
                }
                (Some(kind @ (NativeEventKind::Focus | NativeEventKind::Blur)), 0) => {
                    self.events_log.push((ev.node_id(), kind, String::new()));
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

fn new_session(single: &str, multi: &str) -> DesktopSession<TextMaturityApp> {
    let vp = Viewport {
        width: 380.0,
        height: 220.0,
        scale_factor: 2.0,
    };
    let mut session = DesktopSession::new(
        TextMaturityApp::new(single, multi),
        1,
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
    session.focus_node(Some(10)).unwrap();
    let none = ModifiersState::empty();
    let shift = ModifiersState::SHIFT;

    assert_eq!(session.backend().caret_byte_index(), initial.len());
    assert_eq!(session.backend().caret_grapheme_index(), 4);

    // ArrowLeft moves over the entire `e\u{0301}` combining cluster in ONE step.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, none)
        .unwrap());
    assert_eq!(
        session.backend().caret_byte_index(),
        initial.len() - accent.len()
    );

    // Shift+ArrowLeft selects the entire skin-tone wave `👋🏽` in ONE step.
    assert!(session
        .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, shift)
        .unwrap());
    assert_eq!(session.backend().selected_text().as_deref(), Some(wave));

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
    assert_eq!(session.backend().caret_byte_index(), 0);
}

#[test]
fn double_click_word_triple_click_paragraph_and_crlf_paste_normalization() {
    let mut session = new_session("hello world_42 test", "line one\nline two");
    let r10 = session.backend().layout_of(10).unwrap();
    let click_x = r10.x + 28.0;
    let click_y = r10.y + r10.height * 0.5;

    // Click 1: caret placement.
    session.handle_click_logical(click_x, click_y).unwrap();
    session.handle_pointer_up_logical();
    assert_eq!(session.backend().selection_range(), None);

    // Click 2: double-click word select ("hello").
    session.handle_click_logical(click_x, click_y).unwrap();
    session.handle_pointer_up_logical();
    assert_eq!(session.backend().selected_text().as_deref(), Some("hello"));

    // Click 3: triple-click paragraph/line select (entire line!).
    session.handle_click_logical(click_x, click_y).unwrap();
    session.handle_pointer_up_logical();
    assert_eq!(
        session.backend().selected_text().as_deref(),
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
    session.focus_node(Some(11)).unwrap();
    session.backend_mut().select_all_text();
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
    session.focus_node(Some(10)).unwrap();
    let none = ModifiersState::empty();

    // Sweep ArrowLeft from end to start, then ArrowRight back to end.
    for _ in 0..30 {
        let _ = session
            .handle_keyboard_input(&Key::Named(NamedKey::ArrowLeft), None, none)
            .unwrap();
        let val = session.app().single_val.clone();
        assert!(is_grapheme_boundary(&val, session.backend().caret_byte_index()));
    }
    for _ in 0..30 {
        let _ = session
            .handle_keyboard_input(&Key::Named(NamedKey::ArrowRight), None, none)
            .unwrap();
        let val = session.app().single_val.clone();
        assert!(is_grapheme_boundary(&val, session.backend().caret_byte_index()));
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
        assert!(is_grapheme_boundary(val, session.backend().caret_byte_index()));
    }
}

#[test]
fn unified_ime_lifecycle_emits_start_update_end_blur_cancel_and_tracks_live_caret_rect() {
    let mut session = new_session("", "");
    session.focus_node(Some(10)).unwrap();
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
            (10, NativeEventKind::CompositionStart, "ni".into()),
            (10, NativeEventKind::CompositionUpdate, "nihao".into()),
            (10, NativeEventKind::CompositionEnd, "你好".into()),
            (10, NativeEventKind::TextChange, "你好".into()),
        ]
    );

    // 4. Blur mid-composition -> emits clean CompositionEnd("") followed by Blur!
    session.app_mut().events_log.clear();
    assert!(session
        .handle_ime(&Ime::Preedit("zhong".into(), Some((0, 5))))
        .unwrap());
    assert!(session.focus_node(None).unwrap());
    assert_eq!(session.backend().ime_preedit(), None);
    assert_eq!(session.ime_cursor_rect_logical(), None);
    assert_eq!(
        session.app().events_log,
        vec![
            (10, NativeEventKind::CompositionStart, "zhong".into()),
            (10, NativeEventKind::CompositionEnd, String::new()),
            (10, NativeEventKind::Blur, String::new()),
        ]
    );

    // 5. Empty Preedit and Ime::Disabled mid-composition both emit clean CompositionEnd("").
    session.focus_node(Some(10)).unwrap();
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
            (10, NativeEventKind::CompositionStart, "ka".into()),
            (10, NativeEventKind::CompositionEnd, String::new()),
            (10, NativeEventKind::CompositionStart, "ki".into()),
            (10, NativeEventKind::CompositionEnd, String::new()),
        ]
    );
}
