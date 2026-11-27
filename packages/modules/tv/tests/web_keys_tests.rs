//! Web TV key normalizer: user-agent detection, the three resolution tiers per vendor, and the
//! IME / text-input guard.

use nve_tv::web_keys::TIZEN_REGISTERED_KEYS;
use nve_tv::{
    DecimalDigit, FocusDirection, RemoteAction, TextInputContext, WebKeyInput, WebTvPlatform,
    map_web_tv_key,
};

const UP: RemoteAction = RemoteAction::Move(FocusDirection::Up);
const DOWN: RemoteAction = RemoteAction::Move(FocusDirection::Down);
const LEFT: RemoteAction = RemoteAction::Move(FocusDirection::Left);
const RIGHT: RemoteAction = RemoteAction::Move(FocusDirection::Right);

fn digit(value: u8) -> RemoteAction {
    RemoteAction::Digit(DecimalDigit::new(value).expect("digit"))
}

/// `keydown` with an unidentified `key` / empty `code`: only tier 3 can resolve it.
fn legacy(platform: WebTvPlatform, key_code: u32) -> Option<RemoteAction> {
    let input = WebKeyInput {
        key: "Unidentified",
        key_code,
        ..WebKeyInput::default()
    };
    map_web_tv_key(platform, &input)
}

fn keyed(platform: WebTvPlatform, key: &str, key_code: u32) -> Option<RemoteAction> {
    map_web_tv_key(
        platform,
        &WebKeyInput {
            key,
            key_code,
            ..WebKeyInput::default()
        },
    )
}

fn in_text(key: &str, context: TextInputContext) -> Option<RemoteAction> {
    let input = WebKeyInput {
        key,
        text_input: context,
        ..WebKeyInput::default()
    };
    map_web_tv_key(WebTvPlatform::Standard, &input)
}

#[test]
fn user_agent_detection_covers_every_vendor_family() {
    let cases = [
        (
            "Mozilla/5.0 (SMART-TV; LINUX; Tizen 6.0) AppleWebKit/537.36 (KHTML, like Gecko) \
             76.0.3809.146/6.0 TV Safari/537.36",
            WebTvPlatform::Tizen,
        ),
        (
            "Mozilla/5.0 (Web0S; Linux/SmartTV) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/79.0.3945.79 Safari/537.36 WebAppManager",
            WebTvPlatform::WebOs,
        ),
        (
            "Mozilla/5.0 (Linux; NetCast; U) AppleWebKit/537.31",
            WebTvPlatform::WebOs,
        ),
        (
            "Mozilla/5.0 (Linux; Android 9; AFTKA Build/PS7633; wv) AppleWebKit/537.36 \
             (KHTML, like Gecko) Version/4.0 Chrome/120.0 Mobile Safari/537.36",
            WebTvPlatform::AndroidTv,
        ),
        (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; Xbox; Xbox One) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0 Safari/537.36 Edg/120.0",
            WebTvPlatform::Xbox,
        ),
        (
            "Mozilla/5.0 (PlayStation; PlayStation 5/2.26) AppleWebKit/605.1.15 \
             (KHTML, like Gecko) Version/13.0 Safari/605.1.15",
            WebTvPlatform::PlayStation,
        ),
        (
            "Mozilla/5.0 (Linux; U) AppleWebKit/537.36 HbbTV/1.4.1 (+DRM; Vestel; MB130; 3.0;)",
            WebTvPlatform::Standard,
        ),
        (
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4) AppleWebKit/537.36 Chrome/124.0",
            WebTvPlatform::Standard,
        ),
        ("", WebTvPlatform::Standard),
    ];
    for (ua, expected) in cases {
        assert_eq!(WebTvPlatform::from_user_agent(ua), expected, "{ua}");
    }
}

#[test]
fn tier1_w3c_key_names_win_on_every_platform() {
    let platforms = [
        WebTvPlatform::Standard,
        WebTvPlatform::Tizen,
        WebTvPlatform::WebOs,
        WebTvPlatform::AndroidTv,
        WebTvPlatform::Xbox,
        WebTvPlatform::PlayStation,
    ];
    let cases = [
        ("ArrowUp", UP),
        ("Down", DOWN),
        ("Enter", RemoteAction::Select),
        ("Accept", RemoteAction::Select),
        ("BrowserBack", RemoteAction::Back),
        ("XF86Back", RemoteAction::Back),
        ("MediaNextTrack", RemoteAction::TrackNext),
        ("Tools", RemoteAction::ContextMenu),
        ("Epg", RemoteAction::Guide),
        ("ColorF3Blue", RemoteAction::ColorBlue),
        ("7", digit(7)),
    ];
    for platform in platforms {
        for (key, expected) in cases {
            // A conflicting legacy keyCode must never override an exact key name.
            assert_eq!(
                keyed(platform, key, 19),
                Some(expected),
                "{platform:?} {key}"
            );
        }
    }
}

#[test]
fn tier2_code_resolves_unidentified_keys_but_never_remaps_typed_characters() {
    let code = |key, code| {
        map_web_tv_key(
            WebTvPlatform::Standard,
            &WebKeyInput {
                key,
                code,
                ..Default::default()
            },
        )
    };
    assert_eq!(code("Unidentified", "ArrowLeft"), Some(LEFT));
    assert_eq!(code("", "NumpadEnter"), Some(RemoteAction::Select));
    assert_eq!(code("Unidentified", "Numpad4"), Some(digit(4)));
    assert_eq!(code("Unidentified", "Digit0"), Some(digit(0)));
    assert_eq!(code("Unidentified", "Digit10"), None);
    // AZERTY: `&` on the Digit1 key is typed text, not a channel digit.
    assert_eq!(code("&", "Digit1"), None);
    assert_eq!(code("a", "KeyA"), None);
    // Typed characters never fall through to tier 3 either (`y` is Android KEYCODE_REWIND 89).
    assert_eq!(keyed(WebTvPlatform::AndroidTv, "y", 89), None);
}

#[test]
fn tier3_key_code_19_is_disambiguated_by_platform() {
    assert_eq!(legacy(WebTvPlatform::AndroidTv, 19), Some(UP));
    for platform in [
        WebTvPlatform::Standard,
        WebTvPlatform::Tizen,
        WebTvPlatform::WebOs,
    ] {
        assert_eq!(
            legacy(platform, 19),
            Some(RemoteAction::Pause),
            "{platform:?}"
        );
    }
}

#[test]
fn tier3_standard_hbbtv_table() {
    let p = WebTvPlatform::Standard;
    let cases = [
        (37, LEFT),
        (40, DOWN),
        (13, RemoteAction::Select),
        (461, RemoteAction::Back),
        (166, RemoteAction::Back),
        (415, RemoteAction::Play),
        (413, RemoteAction::Stop),
        (412, RemoteAction::Rewind),
        (417, RemoteAction::FastForward),
        (425, RemoteAction::TrackNext),
        (424, RemoteAction::TrackPrevious),
        (179, RemoteAction::PlayPause),
        (457, RemoteAction::Info),
        (458, RemoteAction::Guide),
        (427, RemoteAction::ChannelUp),
        (428, RemoteAction::ChannelDown),
        (403, RemoteAction::ColorRed),
        (406, RemoteAction::ColorBlue),
        (57, digit(9)),
        (96, digit(0)),
    ];
    for (key_code, expected) in cases {
        assert_eq!(legacy(p, key_code), Some(expected), "{key_code}");
    }
    assert_eq!(legacy(p, 10009), None, "Tizen codes stay Tizen-scoped");
    assert_eq!(legacy(p, 33), None, "PageUp is not a channel key off webOS");
    assert_eq!(legacy(p, 0), None);
}

#[test]
fn tier3_tizen_table() {
    let p = WebTvPlatform::Tizen;
    assert_eq!(legacy(p, 10009), Some(RemoteAction::Back));
    assert_eq!(legacy(p, 10252), Some(RemoteAction::PlayPause));
    assert_eq!(legacy(p, 415), Some(RemoteAction::Play));
    assert_eq!(legacy(p, 19), Some(RemoteAction::Pause));
    assert_eq!(legacy(p, 413), Some(RemoteAction::Stop));
    assert_eq!(legacy(p, 412), Some(RemoteAction::Rewind));
    assert_eq!(legacy(p, 417), Some(RemoteAction::FastForward));
    assert_eq!(legacy(p, 10232), Some(RemoteAction::TrackPrevious));
    assert_eq!(legacy(p, 10233), Some(RemoteAction::TrackNext));
    assert_eq!(legacy(p, 10135), Some(RemoteAction::ContextMenu));
    assert_eq!(legacy(p, 427), Some(RemoteAction::ChannelUp));
    assert_eq!(legacy(p, 428), Some(RemoteAction::ChannelDown));
    assert_eq!(legacy(p, 404), Some(RemoteAction::ColorGreen));
    assert_eq!(legacy(p, 405), Some(RemoteAction::ColorYellow));
    // Every key that needs `registerKeyBatch` is one the normalizer understands.
    for key in TIZEN_REGISTERED_KEYS {
        assert!(RemoteAction::from_key_name(key).is_some(), "{key}");
    }
}

#[test]
fn tier3_webos_table() {
    let p = WebTvPlatform::WebOs;
    assert_eq!(legacy(p, 461), Some(RemoteAction::Back));
    assert_eq!(keyed(p, "PageUp", 33), Some(RemoteAction::ChannelUp));
    assert_eq!(keyed(p, "PageDown", 34), Some(RemoteAction::ChannelDown));
    assert_eq!(legacy(p, 415), Some(RemoteAction::Play));
    assert_eq!(legacy(p, 412), Some(RemoteAction::Rewind));
    assert_eq!(legacy(p, 403), Some(RemoteAction::ColorRed));
    assert_eq!(legacy(p, 10009), None);
}

#[test]
fn tier3_android_and_fire_tv_table() {
    let p = WebTvPlatform::AndroidTv;
    let cases = [
        (20, DOWN),
        (21, LEFT),
        (22, RIGHT),
        (23, RemoteAction::Select),
        (96, RemoteAction::Select),
        (4, RemoteAction::Back),
        (97, RemoteAction::Back),
        (82, RemoteAction::Menu),
        (85, RemoteAction::PlayPause),
        (179, RemoteAction::PlayPause),
        (126, RemoteAction::Play),
        (127, RemoteAction::Pause),
        (227, RemoteAction::Rewind),
        (228, RemoteAction::FastForward),
        (166, RemoteAction::ChannelUp),
        (167, RemoteAction::ChannelDown),
        (183, RemoteAction::ColorRed),
        (186, RemoteAction::ColorBlue),
        (38, UP),
    ];
    for (key_code, expected) in cases {
        assert_eq!(legacy(p, key_code), Some(expected), "{key_code}");
    }
}

#[test]
fn tier3_xbox_and_playstation_tables() {
    let x = WebTvPlatform::Xbox;
    assert_eq!(keyed(x, "GamepadA", 195), Some(RemoteAction::Select));
    assert_eq!(keyed(x, "GamepadB", 196), Some(RemoteAction::Back));
    assert_eq!(keyed(x, "GamepadDPadUp", 203), Some(UP));
    assert_eq!(keyed(x, "GamepadDPadDown", 204), Some(DOWN));
    assert_eq!(keyed(x, "GamepadDPadLeft", 205), Some(LEFT));
    assert_eq!(keyed(x, "GamepadDPadRight", 206), Some(RIGHT));
    assert_eq!(keyed(x, "GamepadLeftThumbstickUp", 211), Some(UP));
    assert_eq!(keyed(x, "GamepadLeftThumbstickRight", 213), Some(RIGHT));
    assert_eq!(keyed(x, "GamepadMenu", 207), Some(RemoteAction::Menu));
    assert_eq!(
        keyed(x, "GamepadView", 208),
        Some(RemoteAction::ContextMenu)
    );
    assert_eq!(
        legacy(WebTvPlatform::Standard, 195),
        None,
        "Xbox codes stay Xbox-scoped"
    );

    let ps = WebTvPlatform::PlayStation;
    assert_eq!(keyed(ps, "Enter", 13), Some(RemoteAction::Select), "Cross");
    assert_eq!(keyed(ps, "Escape", 27), Some(RemoteAction::Back), "Circle");
    assert_eq!(legacy(ps, 40), Some(DOWN));
    assert_eq!(legacy(ps, 19), Some(RemoteAction::Pause));
}

#[test]
fn tier0_ime_composition_and_command_modifiers_pass_through() {
    let composing = WebKeyInput {
        key: "ArrowDown",
        is_composing: true,
        ..Default::default()
    };
    assert_eq!(map_web_tv_key(WebTvPlatform::Standard, &composing), None);
    let process = WebKeyInput {
        key: "Process",
        key_code: 229,
        ..Default::default()
    };
    assert_eq!(map_web_tv_key(WebTvPlatform::AndroidTv, &process), None);
    let ime_arrow = WebKeyInput {
        key: "ArrowDown",
        key_code: 229,
        ..Default::default()
    };
    assert_eq!(map_web_tv_key(WebTvPlatform::Standard, &ime_arrow), None);
    let shortcut = WebKeyInput {
        key: "ArrowLeft",
        command_modifier: true,
        ..Default::default()
    };
    assert_eq!(map_web_tv_key(WebTvPlatform::Standard, &shortcut), None);
}

#[test]
fn tier0_single_line_input_keeps_caret_keys_but_allows_vertical_escape_and_back() {
    let ctx = TextInputContext::SingleLine;
    for kept in [
        "ArrowLeft",
        "ArrowRight",
        "Enter",
        "Backspace",
        "5",
        "a",
        " ",
    ] {
        assert_eq!(in_text(kept, ctx), None, "{kept}");
    }
    assert_eq!(in_text("ArrowUp", ctx), Some(UP));
    assert_eq!(in_text("ArrowDown", ctx), Some(DOWN));
    assert_eq!(in_text("Escape", ctx), Some(RemoteAction::Back));
    assert_eq!(in_text("GoBack", ctx), Some(RemoteAction::Back));
    assert_eq!(
        in_text("MediaPlayPause", ctx),
        Some(RemoteAction::PlayPause)
    );
}

#[test]
fn tier0_multiline_input_keeps_every_arrow() {
    let ctx = TextInputContext::from_flags(true, true);
    assert_eq!(ctx, TextInputContext::MultiLine);
    for kept in [
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "Enter",
        "3",
    ] {
        assert_eq!(in_text(kept, ctx), None, "{kept}");
    }
    assert_eq!(in_text("Escape", ctx), Some(RemoteAction::Back));
    assert_eq!(in_text("XF86Back", ctx), Some(RemoteAction::Back));
    assert_eq!(
        TextInputContext::from_flags(false, true),
        TextInputContext::None
    );
}

#[test]
fn named_non_tv_keys_never_fall_back_to_code_or_key_code() {
    let android = WebTvPlatform::AndroidTv;
    // Phone / tablet Android browsers match `AndroidTv`: their named keys keep their meaning.
    for (key, key_code) in [
        ("CapsLock", 20),
        ("HangulMode", 21),
        ("KanaMode", 21),
        ("BrowserForward", 167),
        ("BrowserSearch", 166),
        ("F15", 126),
        ("F16", 127),
        ("Meta", 93),
    ] {
        assert_eq!(keyed(android, key, key_code), None, "{key}");
        assert_eq!(keyed(WebTvPlatform::Standard, key, key_code), None, "{key}");
    }
    // NumLock-off numpad keys report navigation names on numpad codes: not digits.
    for (key, code) in [
        ("Insert", "Numpad0"),
        ("End", "Numpad1"),
        ("Clear", "Numpad5"),
        ("Home", "Numpad7"),
        ("PageUp", "Numpad9"),
    ] {
        let input = WebKeyInput {
            key,
            code,
            ..WebKeyInput::default()
        };
        assert_eq!(
            map_web_tv_key(WebTvPlatform::Standard, &input),
            None,
            "{key}"
        );
    }
    // Vendor-named keys still reach their platform table.
    assert_eq!(
        keyed(WebTvPlatform::Tizen, "XF86RaiseChannel", 427),
        Some(RemoteAction::ChannelUp)
    );
    assert_eq!(
        keyed(WebTvPlatform::Tizen, "XF86Back", 10009),
        Some(RemoteAction::Back)
    );
    assert_eq!(
        keyed(WebTvPlatform::Standard, "XF86RaiseChannel", 427),
        None
    );
    assert_eq!(
        keyed(WebTvPlatform::Standard, "PageUp", 33),
        None,
        "PageUp scrolls on desktop"
    );
    assert_eq!(keyed(WebTvPlatform::WebOs, "GamepadA", 195), None);
}

#[test]
fn composition_keys_pause_key_and_android_gamepad_codes() {
    for key in ["Dead", "Process", "Compose"] {
        for platform in [WebTvPlatform::Standard, WebTvPlatform::AndroidTv] {
            assert_eq!(keyed(platform, key, 186), None, "{key}");
        }
    }
    assert_eq!(
        keyed(WebTvPlatform::AndroidTv, "Pause", 19),
        Some(RemoteAction::Pause)
    );
    assert_eq!(
        keyed(WebTvPlatform::Standard, "Pause", 19),
        Some(RemoteAction::Pause)
    );
    for key_code in 98..=105 {
        assert_eq!(
            legacy(WebTvPlatform::AndroidTv, key_code),
            None,
            "{key_code}"
        );
    }
    assert_eq!(
        legacy(WebTvPlatform::Standard, 101),
        Some(digit(5)),
        "desktop numpad 5"
    );
}

#[test]
fn back_releases_text_controls_before_reaching_the_app() {
    assert!(TextInputContext::SingleLine.blurs_on(RemoteAction::Back));
    assert!(TextInputContext::MultiLine.blurs_on(RemoteAction::Back));
    assert!(!TextInputContext::None.blurs_on(RemoteAction::Back));
    assert!(!TextInputContext::SingleLine.blurs_on(RemoteAction::Select));
    assert_eq!(
        in_text("Escape", TextInputContext::SingleLine),
        Some(RemoteAction::Back),
        "Escape still maps; the controller blurs instead of dispatching"
    );
}
