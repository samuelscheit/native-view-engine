//! `WebTvController` key decisions through `WebBackend::intercept_key` (native: nodes are
//! registered through `with_engine` instead of DOM rect collection).

use std::cell::RefCell;
use std::rc::Rc;

use nve_backend_web::{WebBackend, WebKeyDisposition, WebKeyEventInfo};
use nve_tv::{NodeId, RemoteAction, SpatialFocusNode, SpatialRect, WebTvPlatform};
use nve_tv_web::{SelectActivation, WebTvConfig, WebTvController};

const CONSUMED: WebKeyDisposition = WebKeyDisposition::ConsumedPreventDefault;
const PASS: WebKeyDisposition = WebKeyDisposition::PassThrough;

fn n(index: u32) -> NodeId {
    NodeId::new(index, 0)
}

fn key(key: &str, key_code: u32) -> WebKeyEventInfo<'_> {
    WebKeyEventInfo {
        key,
        code: "",
        key_code,
        modifiers: 0,
        repeat: false,
        is_composing: false,
        in_text_input: false,
        is_multiline_input: false,
        target_node: None,
        timestamp_us: 0,
    }
}

/// Backend + controller over a 3-card row with ids 1..=3.
fn setup(platform: WebTvPlatform) -> (WebBackend, WebTvController) {
    let mut backend = WebBackend::new();
    let tv = WebTvController::install(&mut backend, WebTvConfig::for_platform(platform));
    tv.with_engine(|engine| {
        for i in 0..3 {
            let rect = SpatialRect::new(i as f32 * 120.0, 0.0, 100.0, 80.0);
            engine.register(SpatialFocusNode::new(n(i + 1), rect));
        }
    });
    (backend, tv)
}

fn focused(tv: &WebTvController) -> Option<NodeId> {
    tv.with_engine(|e| e.focused())
}

#[test]
fn directional_keys_move_focus_and_prevent_default_even_when_blocked() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    assert_eq!(backend.intercept_key(&key("ArrowRight", 39)), CONSUMED);
    assert_eq!(focused(&tv), Some(n(1)), "first move initializes focus");
    assert_eq!(backend.intercept_key(&key("ArrowRight", 39)), CONSUMED);
    assert_eq!(backend.intercept_key(&key("ArrowRight", 39)), CONSUMED);
    assert_eq!(focused(&tv), Some(n(3)));
    assert_eq!(
        backend.intercept_key(&key("ArrowRight", 39)),
        CONSUMED,
        "edge still eats the key"
    );
    assert_eq!(backend.intercept_key(&key("ArrowUp", 38)), CONSUMED);
    assert_eq!(focused(&tv), Some(n(3)));
}

#[test]
fn pages_without_focusables_keep_native_arrow_scrolling() {
    let mut backend = WebBackend::new();
    let _tv = WebTvController::install(&mut backend, WebTvConfig::detect());
    assert_eq!(backend.intercept_key(&key("ArrowDown", 40)), PASS);
}

#[test]
fn dom_focus_target_resyncs_engine_focus_before_moving() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    let mut info = key("ArrowLeft", 37);
    info.target_node = Some(n(3));
    assert_eq!(backend.intercept_key(&info), CONSUMED);
    assert_eq!(
        focused(&tv),
        Some(n(2)),
        "moved from the DOM-focused node, not engine state"
    );
}

#[test]
fn vendor_legacy_codes_are_resolved_per_platform() {
    let (backend, tv) = setup(WebTvPlatform::AndroidTv);
    assert_eq!(
        backend.intercept_key(&key("Unidentified", 22)),
        CONSUMED,
        "DPAD_RIGHT"
    );
    assert_eq!(focused(&tv), Some(n(1)));

    let (backend, tv) = setup(WebTvPlatform::Tizen);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&seen);
    tv.set_action_handler(move |action, node| {
        log.borrow_mut().push((action, node));
        true
    });
    assert_eq!(
        backend.intercept_key(&key("Unidentified", 19)),
        CONSUMED,
        "VK_PAUSE"
    );
    assert_eq!(backend.intercept_key(&key("XF86Back", 10009)), CONSUMED);
    assert_eq!(
        *seen.borrow(),
        [(RemoteAction::Pause, None), (RemoteAction::Back, None)]
    );
    assert_eq!(tv.platform(), WebTvPlatform::Tizen);
}

#[test]
fn text_inputs_ime_and_shortcuts_pass_through() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    let mut caret = key("ArrowLeft", 37);
    caret.in_text_input = true;
    assert_eq!(backend.intercept_key(&caret), PASS);
    let mut escape_up = key("ArrowDown", 40);
    escape_up.in_text_input = true;
    assert_eq!(
        backend.intercept_key(&escape_up),
        CONSUMED,
        "single-line vertical escape"
    );
    escape_up.is_multiline_input = true;
    assert_eq!(
        backend.intercept_key(&escape_up),
        PASS,
        "textarea keeps every arrow"
    );
    let mut composing = key("ArrowDown", 229);
    composing.is_composing = true;
    assert_eq!(backend.intercept_key(&composing), PASS);
    let mut shortcut = key("ArrowRight", 39);
    shortcut.modifiers = nve_abi::NVE_KEY_MOD_CTRL;
    assert_eq!(backend.intercept_key(&shortcut), PASS);
    assert_eq!(backend.intercept_key(&key("a", 65)), PASS);
    assert_eq!(focused(&tv), Some(n(1)), "only the escape key moved focus");
}

#[test]
fn unhandled_actions_pass_through_and_handler_sees_focused_node() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    assert_eq!(
        backend.intercept_key(&key("MediaPlayPause", 179)),
        PASS,
        "no handler"
    );
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&calls);
    let reentrant = tv.clone();
    tv.set_action_handler(move |action, node| {
        // Handlers may use the controller re-entrantly without a borrow panic.
        let engine_focus = reentrant.with_engine(|e| e.focused());
        log.borrow_mut().push((action, node, engine_focus));
        action == RemoteAction::ColorRed
    });
    assert_eq!(backend.intercept_key(&key("ArrowRight", 39)), CONSUMED);
    assert_eq!(backend.intercept_key(&key("ColorF0Red", 403)), CONSUMED);
    assert_eq!(backend.intercept_key(&key("ColorF1Green", 404)), PASS);
    // Natively there is no DOM element to activate: Select passes through.
    assert_eq!(backend.intercept_key(&key("Enter", 13)), PASS);
    assert_eq!(
        *calls.borrow(),
        [
            (RemoteAction::ColorRed, Some(n(1)), Some(n(1))),
            (RemoteAction::ColorGreen, Some(n(1)), Some(n(1))),
            (RemoteAction::Select, Some(n(1)), Some(n(1))),
        ]
    );
}

#[test]
fn held_keys_are_throttled_and_consumed() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    let mut press = key("ArrowRight", 39);
    assert_eq!(backend.intercept_key(&press), CONSUMED);
    press.repeat = true;
    press.timestamp_us = 40_000;
    assert_eq!(
        backend.intercept_key(&press),
        CONSUMED,
        "throttled repeat is still eaten"
    );
    assert_eq!(focused(&tv), Some(n(1)));
    press.timestamp_us = 140_000;
    assert_eq!(backend.intercept_key(&press), CONSUMED);
    assert_eq!(focused(&tv), Some(n(2)));
}

#[test]
fn earlier_interceptors_win_and_later_ones_see_pass_through_keys() {
    let mut backend = WebBackend::new();
    backend.add_key_interceptor(|info| {
        if info.key == "ArrowLeft" {
            WebKeyDisposition::ConsumedAllowDefault
        } else {
            WebKeyDisposition::PassThrough
        }
    });
    let tv = WebTvController::install(&mut backend, WebTvConfig::for_platform(WebTvPlatform::Xbox));
    tv.with_engine(|e| {
        e.register(SpatialFocusNode::new(
            n(7),
            SpatialRect::new(0.0, 0.0, 10.0, 10.0),
        ));
    });
    assert_eq!(
        backend.intercept_key(&key("ArrowLeft", 37)),
        WebKeyDisposition::ConsumedAllowDefault
    );
    assert_eq!(focused(&tv), None, "the TV controller never saw ArrowLeft");
    assert_eq!(
        backend.intercept_key(&key("GamepadDPadDown", 204)),
        CONSUMED
    );
    assert_eq!(focused(&tv), Some(n(7)));
}

#[test]
fn select_activation_is_native_only_for_enter_on_natively_activatable_elements() {
    use SelectActivation::{Native, SyntheticClick};
    assert_eq!(SelectActivation::for_target("Enter", "BUTTON", ""), Native);
    assert_eq!(SelectActivation::for_target("Enter", "A", ""), Native);
    assert_eq!(SelectActivation::for_target("Enter", "SUMMARY", ""), Native);
    assert_eq!(
        SelectActivation::for_target("Enter", "INPUT", "Submit"),
        Native
    );
    assert_eq!(
        SelectActivation::for_target("Enter", "INPUT", "checkbox"),
        SyntheticClick
    );
    assert_eq!(
        SelectActivation::for_target("Enter", "DIV", ""),
        SyntheticClick
    );
    assert_eq!(
        SelectActivation::for_target("GamepadA", "BUTTON", ""),
        SyntheticClick
    );
    assert_eq!(
        SelectActivation::for_target("", "A", ""),
        SyntheticClick,
        "gamepad poller"
    );
    assert_eq!(
        nve_tv_web::platform::register_platform_keys(WebTvPlatform::Tizen),
        0
    );
    assert!(!nve_tv_web::platform::exit_to_platform(
        WebTvPlatform::WebOs
    ));
    assert_eq!(
        nve_tv_web::platform::detect_platform(),
        WebTvPlatform::Standard
    );
}
