//! `WebTvController` key decisions through `WebBackend::intercept_key` (native: nodes are
//! registered through `with_engine` instead of DOM rect collection).

use std::cell::RefCell;
use std::rc::Rc;

use nve_backend_web::{ArrowAxes, WebBackend, WebKeyDisposition, WebKeyEventInfo};
use nve_tv::{NodeId, RemoteAction, SpatialFocusNode, SpatialRect, WebTvPlatform};
use nve_tv_web::{SelectActivation, WebTvConfig, WebTvController};

const CONSUMED: WebKeyDisposition = WebKeyDisposition::ConsumedPreventDefault;
const PASS: WebKeyDisposition = WebKeyDisposition::PassThrough;

fn n(index: u32) -> NodeId {
    NodeId::new(index, 0)
}

fn key(key: &str, key_code: u32) -> WebKeyEventInfo<'_> {
    WebKeyEventInfo::new(key, "", key_code)
}

fn held(key: &str, key_code: u32, timestamp_us: u64) -> WebKeyEventInfo<'_> {
    WebKeyEventInfo {
        repeat: true,
        timestamp_us,
        ..WebKeyEventInfo::new(key, "", key_code)
    }
}

fn register_row(tv: &WebTvController) {
    tv.with_engine(|engine| {
        for i in 0..3 {
            let rect = SpatialRect::new(i as f32 * 120.0, 0.0, 100.0, 80.0);
            engine.register(SpatialFocusNode::new(n(i + 1), rect));
        }
    })
    .expect("engine is free");
}

/// Backend + controller over a 3-card row with ids 1..=3.
fn setup(platform: WebTvPlatform) -> (WebBackend, WebTvController) {
    let mut backend = WebBackend::new();
    let tv = WebTvController::install(&mut backend, WebTvConfig::for_platform(platform));
    register_row(&tv);
    (backend, tv)
}

fn focused(tv: &WebTvController) -> Option<NodeId> {
    tv.with_engine(|e| e.focused()).flatten()
}

type Calls = Rc<RefCell<Vec<(RemoteAction, Option<NodeId>)>>>;

/// Records every action; handles those in `handled`.
fn record(tv: &WebTvController, handled: &'static [RemoteAction]) -> Calls {
    let calls = Calls::default();
    let log = Rc::clone(&calls);
    tv.set_action_handler(move |action, node| {
        log.borrow_mut().push((action, node));
        handled.contains(&action)
    });
    calls
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
fn pages_without_focusables_keep_native_arrow_scrolling_and_its_repeats() {
    let mut backend = WebBackend::new();
    let _tv = WebTvController::install(&mut backend, WebTvConfig::detect());
    assert_eq!(backend.intercept_key(&key("ArrowDown", 40)), PASS);
    assert_eq!(
        backend.intercept_key(&held("ArrowDown", 40, 10)),
        PASS,
        "throttled repeat too"
    );
}

#[test]
fn dom_focus_target_resyncs_engine_focus_before_moving() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    let info = WebKeyEventInfo {
        target_node: Some(n(3)),
        ..key("ArrowLeft", 37)
    };
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
    let calls = record(&tv, &[RemoteAction::Pause, RemoteAction::Back]);
    assert_eq!(
        backend.intercept_key(&key("Unidentified", 19)),
        CONSUMED,
        "VK_PAUSE"
    );
    assert_eq!(backend.intercept_key(&key("XF86Back", 10009)), CONSUMED);
    assert_eq!(
        *calls.borrow(),
        [(RemoteAction::Pause, None), (RemoteAction::Back, None)]
    );
    assert_eq!(tv.platform(), WebTvPlatform::Tizen);
}

#[test]
fn text_inputs_ime_and_shortcuts_pass_through_and_back_releases_the_input() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    let calls = record(&tv, &[RemoteAction::Back]);
    let caret = WebKeyEventInfo {
        in_text_input: true,
        ..key("ArrowLeft", 37)
    };
    assert_eq!(backend.intercept_key(&caret), PASS);
    let mut escape_up = WebKeyEventInfo {
        in_text_input: true,
        ..key("ArrowDown", 40)
    };
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
    let composing = WebKeyEventInfo {
        is_composing: true,
        ..key("ArrowDown", 229)
    };
    assert_eq!(backend.intercept_key(&composing), PASS);
    let shortcut = WebKeyEventInfo {
        modifiers: nve_abi::NVE_KEY_MOD_CTRL,
        ..key("ArrowRight", 39)
    };
    assert_eq!(backend.intercept_key(&shortcut), PASS);
    assert_eq!(backend.intercept_key(&key("a", 65)), PASS);
    assert_eq!(focused(&tv), Some(n(1)), "only the escape key moved focus");
    // Back inside a text control blurs it first; the app only sees the next Back.
    let back_in_input = WebKeyEventInfo {
        in_text_input: true,
        ..key("Escape", 27)
    };
    assert_eq!(backend.intercept_key(&back_in_input), CONSUMED);
    assert!(
        calls.borrow().is_empty(),
        "the blur press never reaches the app"
    );
    assert_eq!(
        backend.intercept_key(&held("Escape", 27, 50_000)),
        CONSUMED,
        "its repeat too"
    );
    assert_eq!(backend.intercept_key(&key("Escape", 27)), CONSUMED);
    assert_eq!(*calls.borrow(), [(RemoteAction::Back, Some(n(1)))]);
}

#[test]
fn widgets_that_own_arrows_receive_them_on_their_axes() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    assert_eq!(backend.intercept_key(&key("ArrowRight", 39)), CONSUMED);
    let in_tablist = |k, code| WebKeyEventInfo {
        owned_arrows: ArrowAxes::HORIZONTAL,
        target_node: Some(n(1)),
        ..key(k, code)
    };
    assert_eq!(
        backend.intercept_key(&in_tablist("ArrowRight", 39)),
        PASS,
        "tab switching"
    );
    let repeat = WebKeyEventInfo {
        repeat: true,
        timestamp_us: 10,
        ..in_tablist("ArrowRight", 39)
    };
    assert_eq!(
        backend.intercept_key(&repeat),
        PASS,
        "held arrow keeps reaching the widget"
    );
    assert_eq!(focused(&tv), Some(n(1)));
    assert_eq!(
        backend.intercept_key(&in_tablist("ArrowDown", 40)),
        CONSUMED,
        "leave vertically"
    );
    let opted_out = WebKeyEventInfo {
        owned_arrows: ArrowAxes::BOTH,
        ..key("ArrowUp", 38)
    };
    assert_eq!(
        backend.intercept_key(&opted_out),
        PASS,
        "data-nve-spatial-nav=off"
    );
    // Non-arrow TV keys inside such widgets are still TV keys.
    let enter = WebKeyEventInfo {
        owned_arrows: ArrowAxes::BOTH,
        ..key("MediaPlayPause", 179)
    };
    let calls = record(&tv, &[RemoteAction::PlayPause]);
    assert_eq!(backend.intercept_key(&enter), CONSUMED);
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn unhandled_actions_pass_through_and_handlers_reenter_the_controller() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    assert_eq!(
        backend.intercept_key(&key("MediaPlayPause", 179)),
        PASS,
        "no handler"
    );
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&calls);
    let weak = tv.downgrade();
    tv.set_action_handler(move |action, node| {
        let Some(tv) = weak.upgrade() else {
            return false;
        };
        // The state is free while app code runs: engine access and a nested key both work.
        let engine_focus = tv.with_engine(|e| e.focused()).flatten();
        let nested =
            (action == RemoteAction::ColorGreen).then(|| tv.handle_key(&key("ArrowRight", 39)));
        log.borrow_mut().push((action, node, engine_focus, nested));
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
            (RemoteAction::ColorRed, Some(n(1)), Some(n(1)), None),
            (
                RemoteAction::ColorGreen,
                Some(n(1)),
                Some(n(1)),
                Some(CONSUMED)
            ),
            (RemoteAction::Select, Some(n(2)), Some(n(2)), None),
        ]
    );
}

#[test]
fn handlers_replaced_or_cleared_while_running_are_never_restored() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let (log, weak) = (Rc::clone(&calls), tv.downgrade());
    tv.set_action_handler(move |action, _| {
        log.borrow_mut().push(("first", action));
        if let Some(tv) = weak.upgrade() {
            let log = Rc::clone(&log);
            tv.set_action_handler(move |action, _| {
                log.borrow_mut().push(("second", action));
                true
            });
        }
        true
    });
    assert_eq!(backend.intercept_key(&key("Info", 457)), CONSUMED);
    assert_eq!(backend.intercept_key(&key("Info", 457)), CONSUMED);
    assert_eq!(
        *calls.borrow(),
        [
            ("first", RemoteAction::Info),
            ("second", RemoteAction::Info)
        ]
    );

    let weak = tv.downgrade();
    tv.set_action_handler(move |_, _| {
        weak.upgrade()
            .inspect(WebTvController::clear_action_handler)
            .is_some()
    });
    assert_eq!(backend.intercept_key(&key("Guide", 458)), CONSUMED);
    assert_eq!(
        backend.intercept_key(&key("Guide", 458)),
        PASS,
        "cleared from inside itself"
    );
    // Engine access is exclusive: a nested `with_engine` reports busy instead of panicking.
    assert_eq!(tv.with_engine(|_| tv.with_engine(|e| e.len())), Some(None));
}

#[test]
fn held_keys_repeat_only_for_auto_repeating_actions_and_mirror_their_first_press() {
    let (backend, tv) = setup(WebTvPlatform::Standard);
    assert_eq!(backend.intercept_key(&key("ArrowRight", 39)), CONSUMED);
    assert_eq!(
        backend.intercept_key(&held("ArrowRight", 39, 40_000)),
        CONSUMED,
        "throttled, eaten"
    );
    assert_eq!(focused(&tv), Some(n(1)));
    assert_eq!(
        backend.intercept_key(&held("ArrowRight", 39, 140_000)),
        CONSUMED
    );
    assert_eq!(focused(&tv), Some(n(2)));

    let calls = record(&tv, &[RemoteAction::Back, RemoteAction::ChannelUp]);
    // Handled Back: one dispatch, its OS repeats consumed but never re-dispatched.
    assert_eq!(backend.intercept_key(&key("GoBack", 0)), CONSUMED);
    for t in [200_000, 400_000, 600_000] {
        assert_eq!(backend.intercept_key(&held("GoBack", 0, t)), CONSUMED);
    }
    // Unhandled Enter / PlayPause: the first press passes, so do its repeats (native behavior).
    assert_eq!(backend.intercept_key(&key("MediaPlayPause", 179)), PASS);
    assert_eq!(
        backend.intercept_key(&held("MediaPlayPause", 179, 700_000)),
        PASS
    );
    // Channel zapping auto-repeats through the handler at the throttle rate.
    let zap_press = WebKeyEventInfo {
        timestamp_us: 800_000,
        ..key("ChannelUp", 427)
    };
    assert_eq!(backend.intercept_key(&zap_press), CONSUMED);
    assert_eq!(
        backend.intercept_key(&held("ChannelUp", 427, 810_000)),
        CONSUMED
    );
    assert_eq!(
        backend.intercept_key(&held("ChannelUp", 427, 950_000)),
        CONSUMED
    );
    let back = (RemoteAction::Back, Some(n(2)));
    let zap = (RemoteAction::ChannelUp, Some(n(2)));
    assert_eq!(
        *calls.borrow(),
        [back, (RemoteAction::PlayPause, Some(n(2))), zap, zap],
        "Back once, the 10 ms channel repeat dropped, the 140 ms one dispatched"
    );
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
fn controllers_never_leak_through_their_handler_and_uninstall_releases_everything() {
    let mut backend = WebBackend::new();
    let tv = WebTvController::install(&mut backend, WebTvConfig::detect());
    let probe = tv.downgrade();
    let weak = tv.downgrade();
    tv.set_action_handler(move |_, _| weak.upgrade().is_some());
    drop(tv);
    assert!(
        probe.upgrade().is_some(),
        "the backend's interceptor keeps it alive"
    );
    drop(backend);
    assert!(
        probe.upgrade().is_none(),
        "weak handler capture: no Rc cycle"
    );

    // A strong capture is a cycle until `uninstall` breaks it.
    let mut backend = WebBackend::new();
    let tv = WebTvController::install(&mut backend, WebTvConfig::detect());
    register_row(&tv);
    let strong = tv.clone();
    tv.set_action_handler(move |_, _| strong.platform() == WebTvPlatform::Standard);
    let probe = tv.downgrade();
    tv.uninstall(&mut backend);
    assert_eq!(
        backend.intercept_key(&key("ArrowRight", 39)),
        PASS,
        "interceptor removed"
    );
    drop(tv);
    assert!(probe.upgrade().is_none());
}

#[test]
fn select_activation_is_native_only_for_enter_on_focused_natively_activatable_elements() {
    use SelectActivation::{Native, ShowPicker, SyntheticClick};
    let focused_target =
        |key, tag, input_type| SelectActivation::for_target(key, tag, input_type, true);
    assert_eq!(focused_target("Enter", "BUTTON", ""), Native);
    assert_eq!(focused_target("Enter", "A", ""), Native);
    assert_eq!(focused_target("Enter", "SUMMARY", ""), Native);
    assert_eq!(focused_target("Enter", "INPUT", "Submit"), Native);
    assert_eq!(focused_target("Enter", "INPUT", "checkbox"), SyntheticClick);
    assert_eq!(focused_target("Enter", "DIV", ""), SyntheticClick);
    assert_eq!(focused_target("GamepadA", "BUTTON", ""), SyntheticClick);
    assert_eq!(
        focused_target("", "A", ""),
        SyntheticClick,
        "gamepad poller"
    );
    assert_eq!(
        SelectActivation::for_target("Enter", "BUTTON", "", false),
        SyntheticClick,
        "DOM focus drifted to <body>: the browser would activate nothing"
    );
    for (key, holds) in [("Enter", true), ("Enter", false), ("", false)] {
        assert_eq!(
            SelectActivation::for_target(key, "SELECT", "", holds),
            ShowPicker
        );
    }
    assert_eq!(
        nve_tv_web::platform::register_platform_keys(WebTvPlatform::Tizen),
        0
    );
    assert!(!nve_tv_web::platform::exit_to_platform(
        WebTvPlatform::WebOs
    ));
    assert!(!nve_tv_web::platform::set_xbox_gamepad_emulation(false));
    assert_eq!(
        nve_tv_web::platform::detect_platform(),
        WebTvPlatform::Standard
    );
}
