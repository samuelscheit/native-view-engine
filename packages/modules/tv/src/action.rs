//! Typed TV remote / gamepad / keyboard actions with a dense `u8` code space.

/// Cardinal 2D directional focus movement.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusDirection {
    Up = 0,
    Down = 1,
    Left = 2,
    Right = 3,
}

impl FocusDirection {
    pub const ALL: [Self; 4] = [Self::Up, Self::Down, Self::Left, Self::Right];

    /// Dense `0..4` slot used to index per-direction tables.
    #[inline]
    pub const fn slot(self) -> usize {
        self as usize
    }

    /// `true` for Left / Right.
    #[inline]
    pub const fn is_horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// A decimal remote digit, sealed to `0..=9` at construction.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalDigit(u8);

impl DecimalDigit {
    /// Returns `None` unless `value <= 9`.
    #[inline]
    pub const fn new(value: u8) -> Option<Self> {
        if value <= 9 { Some(Self(value)) } else { None }
    }

    /// Parses one ASCII digit byte (`b'0'..=b'9'`).
    #[inline]
    pub const fn from_ascii(byte: u8) -> Option<Self> {
        if byte.is_ascii_digit() {
            Some(Self(byte - b'0'))
        } else {
            None
        }
    }

    #[inline]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// High-level TV remote, gamepad, or keyboard action.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteAction {
    Move(FocusDirection),
    Select,
    Back,
    PlayPause,
    Play,
    Pause,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Menu,
    ContextMenu,
    Info,
    Guide,
    ChannelUp,
    ChannelDown,
    ColorRed,
    ColorGreen,
    ColorYellow,
    ColorBlue,
    Digit(DecimalDigit),
}

/// Unit actions in code order `4..24` (after the four moves, before the ten digits).
const UNIT_ACTIONS: [RemoteAction; 20] = [
    RemoteAction::Select,
    RemoteAction::Back,
    RemoteAction::PlayPause,
    RemoteAction::Play,
    RemoteAction::Pause,
    RemoteAction::Stop,
    RemoteAction::FastForward,
    RemoteAction::Rewind,
    RemoteAction::TrackNext,
    RemoteAction::TrackPrevious,
    RemoteAction::Menu,
    RemoteAction::ContextMenu,
    RemoteAction::Info,
    RemoteAction::Guide,
    RemoteAction::ChannelUp,
    RemoteAction::ChannelDown,
    RemoteAction::ColorRed,
    RemoteAction::ColorGreen,
    RemoteAction::ColorYellow,
    RemoteAction::ColorBlue,
];
const FIRST_UNIT_CODE: u8 = 4;
const FIRST_DIGIT_CODE: u8 = FIRST_UNIT_CODE + UNIT_ACTIONS.len() as u8;

/// `KeyboardEvent.key` digit names indexed by digit value.
const DIGIT_KEY_NAMES: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// Canonical W3C `KeyboardEvent.key` names in dense code order `0..24` (moves `0..4`, units `4..24`).
const CANONICAL_KEY_NAMES: [&str; FIRST_DIGIT_CODE as usize] = [
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Enter",
    "GoBack",
    "MediaPlayPause",
    "MediaPlay",
    "MediaPause",
    "MediaStop",
    "MediaFastForward",
    "MediaRewind",
    "MediaTrackNext",
    "MediaTrackPrevious",
    "Menu",
    "ContextMenu",
    "Info",
    "Guide",
    "ChannelUp",
    "ChannelDown",
    "ColorF0Red",
    "ColorF1Green",
    "ColorF2Yellow",
    "ColorF3Blue",
];

impl RemoteAction {
    /// Number of distinct dense codes (`0..CODE_COUNT`).
    pub const CODE_COUNT: usize = FIRST_DIGIT_CODE as usize + 10;

    /// Dense code: moves `0..4`, unit actions `4..24`, digits `24..34`.
    #[inline]
    pub const fn code(self) -> u8 {
        match self {
            Self::Move(dir) => dir as u8,
            Self::Select => 4,
            Self::Back => 5,
            Self::PlayPause => 6,
            Self::Play => 7,
            Self::Pause => 8,
            Self::Stop => 9,
            Self::FastForward => 10,
            Self::Rewind => 11,
            Self::TrackNext => 12,
            Self::TrackPrevious => 13,
            Self::Menu => 14,
            Self::ContextMenu => 15,
            Self::Info => 16,
            Self::Guide => 17,
            Self::ChannelUp => 18,
            Self::ChannelDown => 19,
            Self::ColorRed => 20,
            Self::ColorGreen => 21,
            Self::ColorYellow => 22,
            Self::ColorBlue => 23,
            Self::Digit(d) => FIRST_DIGIT_CODE + d.0,
        }
    }

    /// Inverse of [`Self::code`].
    #[inline]
    pub const fn from_code(code: u8) -> Option<Self> {
        if code < FIRST_UNIT_CODE {
            return Some(Self::Move(FocusDirection::ALL[code as usize]));
        }
        if code < FIRST_DIGIT_CODE {
            return Some(UNIT_ACTIONS[(code - FIRST_UNIT_CODE) as usize]);
        }
        match DecimalDigit::new(code - FIRST_DIGIT_CODE) {
            Some(d) => Some(Self::Digit(d)),
            None => None,
        }
    }

    /// `true` when holding the key should repeat the action (moves, channel zapping, seeking,
    /// digit entry). Holding Select / Back / play-pause / colors fires once: their OS
    /// auto-repeats are dropped.
    #[inline]
    pub const fn auto_repeats(self) -> bool {
        matches!(
            self,
            Self::Move(_)
                | Self::ChannelUp
                | Self::ChannelDown
                | Self::FastForward
                | Self::Rewind
                | Self::Digit(_)
        )
    }

    /// Returns the focus direction of a [`Self::Move`].
    #[inline]
    pub const fn direction(self) -> Option<FocusDirection> {
        match self {
            Self::Move(dir) => Some(dir),
            _ => None,
        }
    }

    /// Maps a W3C UI Events `KeyboardEvent.key` value (plus common TV aliases) to an action.
    pub fn from_key_name(key: &str) -> Option<Self> {
        if let [byte] = key.as_bytes() {
            return DecimalDigit::from_ascii(*byte).map(Self::Digit);
        }
        if let Some(idx) = CANONICAL_KEY_NAMES.iter().position(|&name| name == key) {
            return Self::from_code(idx as u8);
        }
        let action = match key {
            "Up" => Self::Move(FocusDirection::Up),
            "Down" => Self::Move(FocusDirection::Down),
            "Left" => Self::Move(FocusDirection::Left),
            "Right" => Self::Move(FocusDirection::Right),
            "Select" | "Accept" => Self::Select,
            "Escape" | "Esc" | "Back" | "BrowserBack" | "XF86Back" => Self::Back,
            "Pause" => Self::Pause,
            "MediaNextTrack" => Self::TrackNext,
            "MediaPreviousTrack" => Self::TrackPrevious,
            "Tools" => Self::ContextMenu,
            "Epg" => Self::Guide,
            _ => return None,
        };
        Some(action)
    }

    /// Canonical W3C `KeyboardEvent.key` name for this action.
    #[inline]
    pub const fn key_name(self) -> &'static str {
        match self {
            Self::Digit(d) => DIGIT_KEY_NAMES[d.0 as usize],
            _ => CANONICAL_KEY_NAMES[self.code() as usize],
        }
    }
}
