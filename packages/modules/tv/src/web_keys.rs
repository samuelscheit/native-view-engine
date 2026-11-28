//! Zero-allocation `KeyboardEvent` -> [`RemoteAction`] normalizer for TV browsers and WebViews.
//!
//! Resolution order:
//! 0. Text-input / IME guard: composition (`Process`, `Dead`, `Compose`, keyCode 229), command
//!    modifiers, and editing keys inside text controls pass through untouched.
//! 1. Exact W3C `KeyboardEvent.key` (plus common TV aliases).
//! 2. W3C `KeyboardEvent.code`, only when `key` is missing / `"Unidentified"`: a named non-TV
//!    key (`CapsLock`, NumLock-off `Insert` on `Numpad0`, ...) is never remapped by position.
//! 3. Platform-scoped legacy `keyCode` tables, so ambiguous codes resolve per vendor (Android
//!    `19` is `DPAD_UP`, everywhere else it is CE-HTML `VK_PAUSE`). Same `Unidentified` gate,
//!    except for vendor-named keys (webOS `PageUp` / `PageDown` channel keys, Xbox `Gamepad*`,
//!    Tizen `XF86*`).
//!
//! Pure Rust (no `web-sys`): browser glue lives in `nve-tv-web`.

use crate::action::{DecimalDigit, FocusDirection, RemoteAction};

/// `keyCode` reported while an IME is processing a key.
pub const IME_PROCESS_KEY_CODE: u32 = 229;

/// TV browser / WebView family selecting the tier-3 legacy `keyCode` table.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WebTvPlatform {
    /// Desktop browsers, CE-HTML, HbbTV / OIPF `VK_*` codes.
    #[default]
    Standard,
    /// Samsung Tizen (`tizen.tvinputdevice` key codes).
    Tizen,
    /// LG webOS / NetCast.
    WebOs,
    /// Android TV / Google TV / Fire TV WebViews (raw Android `KEYCODE_*` leaks).
    AndroidTv,
    /// Xbox Edge / WebView2 gamepad-as-keyboard codes.
    Xbox,
    /// PlayStation WebKit (pad normalized to Enter / Escape / arrows).
    PlayStation,
}

impl WebTvPlatform {
    /// Detects the platform from `navigator.userAgent`; vendor tokens win over `HbbTV`.
    pub fn from_user_agent(user_agent: &str) -> Self {
        let has = |token: &str| user_agent.contains(token);
        if has("Tizen") {
            Self::Tizen
        } else if has("Web0S") || has("webOS") || has("WebOS") || has("NetCast") {
            Self::WebOs
        } else if has("Xbox") {
            Self::Xbox
        } else if has("PlayStation") {
            Self::PlayStation
        } else if has("Android") {
            Self::AndroidTv
        } else {
            Self::Standard
        }
    }

    /// Tier 3 for a named `key` the W3C table does not know: only this platform's own key
    /// names fall back to its `keyCode` table.
    fn map_named_key(self, key: &str, key_code: u32) -> Option<RemoteAction> {
        let vendor_named = match self {
            Self::WebOs => matches!(key, "PageUp" | "PageDown"),
            Self::Xbox => key.starts_with("Gamepad"),
            Self::Tizen => key.starts_with("XF86"),
            Self::Standard | Self::AndroidTv | Self::PlayStation => false,
        };
        if vendor_named {
            self.map_key_code(key_code)
        } else {
            None
        }
    }

    /// Tier-3 legacy `keyCode` lookup for this platform.
    pub const fn map_key_code(self, key_code: u32) -> Option<RemoteAction> {
        match self {
            Self::Standard | Self::PlayStation => standard_key_code(key_code),
            Self::Tizen => tizen_key_code(key_code),
            Self::WebOs => web_os_key_code(key_code),
            Self::AndroidTv => android_key_code(key_code),
            Self::Xbox => xbox_key_code(key_code),
        }
    }
}

/// Where keyboard focus sits, deciding which keys belong to text editing.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextInputContext {
    #[default]
    None,
    /// `<input>` text-like controls: caret keys, Enter, and digits stay with the control;
    /// ArrowUp / ArrowDown escape to spatial navigation.
    SingleLine,
    /// `<textarea>` / `[contenteditable]`: every arrow stays with the control.
    MultiLine,
}

impl TextInputContext {
    #[inline]
    pub const fn from_flags(in_text_input: bool, multiline: bool) -> Self {
        match (in_text_input, multiline) {
            (false, _) => Self::None,
            (true, false) => Self::SingleLine,
            (true, true) => Self::MultiLine,
        }
    }

    /// Returns `true` when `action` should first release the focused text control (blur it,
    /// closing the on-screen keyboard) instead of reaching the app: Back / Escape.
    #[inline]
    pub const fn blurs_on(self, action: RemoteAction) -> bool {
        !matches!(self, Self::None) && matches!(action, RemoteAction::Back)
    }

    /// Returns `true` when `action` must stay with the focused text control.
    #[inline]
    pub const fn keeps(self, action: RemoteAction) -> bool {
        match self {
            Self::None => false,
            Self::SingleLine => matches!(
                action,
                RemoteAction::Move(FocusDirection::Left | FocusDirection::Right)
                    | RemoteAction::Select
                    | RemoteAction::Digit(_)
            ),
            Self::MultiLine => matches!(
                action,
                RemoteAction::Move(_) | RemoteAction::Select | RemoteAction::Digit(_)
            ),
        }
    }
}

/// Borrowed view of one `keydown` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WebKeyInput<'a> {
    pub key: &'a str,
    pub code: &'a str,
    pub key_code: u32,
    pub is_composing: bool,
    /// Ctrl, Meta, or Alt held: the key belongs to an application / OS shortcut.
    pub command_modifier: bool,
    pub text_input: TextInputContext,
}

/// Normalizes one `keydown` to a [`RemoteAction`], or `None` when it must pass through.
pub fn map_web_tv_key(platform: WebTvPlatform, input: &WebKeyInput<'_>) -> Option<RemoteAction> {
    if input.is_composing
        || input.key_code == IME_PROCESS_KEY_CODE
        || input.command_modifier
        || matches!(input.key, "Process" | "Dead" | "Compose")
    {
        return None;
    }
    let action = resolve_key(platform, input)?;
    (!input.text_input.keeps(action)).then_some(action)
}

fn resolve_key(platform: WebTvPlatform, input: &WebKeyInput<'_>) -> Option<RemoteAction> {
    if let Some(action) = RemoteAction::from_key_name(input.key) {
        return Some(action);
    }
    if input.key.is_empty() || input.key == "Unidentified" {
        return from_code_name(input.code).or_else(|| platform.map_key_code(input.key_code));
    }
    platform.map_named_key(input.key, input.key_code)
}

/// Physical `KeyboardEvent.code` actions whose W3C `code` matches their canonical `key_name()`.
const PHYSICAL_CODE_ACTIONS: &[RemoteAction] = &[
    RemoteAction::Move(FocusDirection::Up),
    RemoteAction::Move(FocusDirection::Down),
    RemoteAction::Move(FocusDirection::Left),
    RemoteAction::Move(FocusDirection::Right),
    RemoteAction::Select,
    RemoteAction::PlayPause,
    RemoteAction::Stop,
    RemoteAction::TrackNext,
    RemoteAction::TrackPrevious,
    RemoteAction::ContextMenu,
];

/// Tier 2: W3C `KeyboardEvent.code` physical key names.
fn from_code_name(code: &str) -> Option<RemoteAction> {
    if let Some(&action) = PHYSICAL_CODE_ACTIONS.iter().find(|a| a.key_name() == code) {
        return Some(action);
    }
    match code {
        "NumpadEnter" => Some(RemoteAction::Select),
        "Escape" | "BrowserBack" => Some(RemoteAction::Back),
        _ => {
            let digit = code
                .strip_prefix("Digit")
                .or_else(|| code.strip_prefix("Numpad"))?;
            match digit.as_bytes() {
                [byte] => DecimalDigit::from_ascii(*byte).map(RemoteAction::Digit),
                _ => None,
            }
        }
    }
}

const fn digit(value: u32) -> Option<RemoteAction> {
    match DecimalDigit::new(value as u8) {
        Some(d) if value <= 9 => Some(RemoteAction::Digit(d)),
        _ => None,
    }
}

const UP: Option<RemoteAction> = Some(RemoteAction::Move(FocusDirection::Up));
const DOWN: Option<RemoteAction> = Some(RemoteAction::Move(FocusDirection::Down));
const LEFT: Option<RemoteAction> = Some(RemoteAction::Move(FocusDirection::Left));
const RIGHT: Option<RemoteAction> = Some(RemoteAction::Move(FocusDirection::Right));

/// Desktop virtual keys + CE-HTML / HbbTV / OIPF `VK_*` codes.
const fn standard_key_code(key_code: u32) -> Option<RemoteAction> {
    match key_code {
        37 => LEFT,
        38 => UP,
        39 => RIGHT,
        40 => DOWN,
        13 => Some(RemoteAction::Select),
        27 | 166 | 461 => Some(RemoteAction::Back),
        179 => Some(RemoteAction::PlayPause),
        415 => Some(RemoteAction::Play),
        19 => Some(RemoteAction::Pause),
        178 | 413 => Some(RemoteAction::Stop),
        417 => Some(RemoteAction::FastForward),
        412 => Some(RemoteAction::Rewind),
        176 | 425 => Some(RemoteAction::TrackNext),
        177 | 424 => Some(RemoteAction::TrackPrevious),
        93 => Some(RemoteAction::ContextMenu),
        457 => Some(RemoteAction::Info),
        458 => Some(RemoteAction::Guide),
        427 => Some(RemoteAction::ChannelUp),
        428 => Some(RemoteAction::ChannelDown),
        403 => Some(RemoteAction::ColorRed),
        404 => Some(RemoteAction::ColorGreen),
        405 => Some(RemoteAction::ColorYellow),
        406 => Some(RemoteAction::ColorBlue),
        48..=57 => digit(key_code - 48),
        96..=105 => digit(key_code - 96),
        _ => None,
    }
}

/// Samsung Tizen `tizen.tvinputdevice` codes over the CE-HTML base.
const fn tizen_key_code(key_code: u32) -> Option<RemoteAction> {
    match key_code {
        10009 => Some(RemoteAction::Back),
        10252 => Some(RemoteAction::PlayPause),
        10232 => Some(RemoteAction::TrackPrevious),
        10233 => Some(RemoteAction::TrackNext),
        10135 => Some(RemoteAction::ContextMenu),
        _ => standard_key_code(key_code),
    }
}

/// LG webOS: Back `461`, channel keys reuse PageUp / PageDown (`33` / `34`).
const fn web_os_key_code(key_code: u32) -> Option<RemoteAction> {
    match key_code {
        33 => Some(RemoteAction::ChannelUp),
        34 => Some(RemoteAction::ChannelDown),
        _ => standard_key_code(key_code),
    }
}

/// Android TV / Fire TV WebViews leaking raw `android.view.KeyEvent` codes. `96..=105` are
/// `KEYCODE_BUTTON_*` gamepad keys there, never the desktop numpad digits.
const fn android_key_code(key_code: u32) -> Option<RemoteAction> {
    match key_code {
        19 => UP,
        20 => DOWN,
        21 => LEFT,
        22 => RIGHT,
        23 | 96 => Some(RemoteAction::Select),
        4 | 97 => Some(RemoteAction::Back),
        98..=105 => None,
        82 => Some(RemoteAction::Menu),
        85 => Some(RemoteAction::PlayPause),
        126 => Some(RemoteAction::Play),
        127 => Some(RemoteAction::Pause),
        86 => Some(RemoteAction::Stop),
        87 => Some(RemoteAction::TrackNext),
        88 => Some(RemoteAction::TrackPrevious),
        89 | 227 => Some(RemoteAction::Rewind),
        90 | 228 => Some(RemoteAction::FastForward),
        165 => Some(RemoteAction::Info),
        172 => Some(RemoteAction::Guide),
        166 => Some(RemoteAction::ChannelUp),
        167 => Some(RemoteAction::ChannelDown),
        183 => Some(RemoteAction::ColorRed),
        184 => Some(RemoteAction::ColorGreen),
        185 => Some(RemoteAction::ColorYellow),
        186 => Some(RemoteAction::ColorBlue),
        _ => standard_key_code(key_code),
    }
}

/// Xbox gamepad-as-keyboard codes (`GamepadA` 195 ... left thumbstick 211-214).
const fn xbox_key_code(key_code: u32) -> Option<RemoteAction> {
    match key_code {
        195 => Some(RemoteAction::Select),
        196 => Some(RemoteAction::Back),
        203 | 211 => UP,
        204 | 212 => DOWN,
        205 | 214 => LEFT,
        206 | 213 => RIGHT,
        207 => Some(RemoteAction::Menu),
        208 => Some(RemoteAction::ContextMenu),
        _ => standard_key_code(key_code),
    }
}

/// Tizen keys delivered to web apps only after `tizen.tvinputdevice.registerKeyBatch`.
pub const TIZEN_REGISTERED_KEYS: [&str; 27] = [
    "MediaPlayPause",
    "MediaPlay",
    "MediaPause",
    "MediaStop",
    "MediaFastForward",
    "MediaRewind",
    "MediaTrackNext",
    "MediaTrackPrevious",
    "ChannelUp",
    "ChannelDown",
    "ColorF0Red",
    "ColorF1Green",
    "ColorF2Yellow",
    "ColorF3Blue",
    "Info",
    "Guide",
    "Tools",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
];
