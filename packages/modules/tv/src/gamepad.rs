//! W3C Gamepad "standard" mapping -> [`RemoteAction`] with press-edge detection and
//! hold-to-repeat, fed by any poller (`navigator.getGamepads()`, SDL, GameController).

use crate::action::{FocusDirection, RemoteAction};
use crate::spatial_focus::RemoteInput;

/// Delay before a held button starts auto-repeating.
pub const GAMEPAD_HOLD_DELAY_US: u64 = 400_000;
/// Auto-repeat spacing for a held button.
pub const GAMEPAD_REPEAT_INTERVAL_US: u64 = 120_000;
/// Left-stick deflection that presses a D-Pad direction.
pub const GAMEPAD_AXIS_THRESHOLD: f32 = 0.5;
/// Deflection below which a stick-held direction releases (hysteresis against jitter around
/// [`GAMEPAD_AXIS_THRESHOLD`]).
pub const GAMEPAD_AXIS_RELEASE: f32 = 0.3;

/// Standard-mapping buttons the repeater tracks (`buttons[0..16]`).
const TRACKED_BUTTONS: usize = 16;
const DPAD_UP: u32 = 12;
const DPAD_DOWN: u32 = 13;
const DPAD_LEFT: u32 = 14;
const DPAD_RIGHT: u32 = 15;

/// Maps a W3C standard-gamepad button index to a remote action.
pub const fn standard_gamepad_button(index: u32) -> Option<RemoteAction> {
    match index {
        0 => Some(RemoteAction::Select),
        1 => Some(RemoteAction::Back),
        8 => Some(RemoteAction::ContextMenu),
        9 => Some(RemoteAction::Menu),
        DPAD_UP => Some(RemoteAction::Move(FocusDirection::Up)),
        DPAD_DOWN => Some(RemoteAction::Move(FocusDirection::Down)),
        DPAD_LEFT => Some(RemoteAction::Move(FocusDirection::Left)),
        DPAD_RIGHT => Some(RemoteAction::Move(FocusDirection::Right)),
        _ => None,
    }
}

/// One polled gamepad frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GamepadFrame {
    /// Bit `i` set when standard button `i` is pressed.
    pub pressed: u32,
    /// Left stick (`axes[0]`, `axes[1]`), `-1.0..=1.0`, +y pointing down.
    pub axis_x: f32,
    pub axis_y: f32,
}

impl GamepadFrame {
    /// D-Pad bits pressed by the left stick: only the dominant axis counts (a diagonal never
    /// fires two moves), and a direction the stick already holds (`stick_held`) stays pressed
    /// until it falls below [`GAMEPAD_AXIS_RELEASE`].
    #[inline]
    fn stick_mask(&self, stick_held: u32) -> u32 {
        let (value, neg, pos) = if self.axis_x.abs() >= self.axis_y.abs() {
            (self.axis_x, DPAD_LEFT, DPAD_RIGHT)
        } else {
            (self.axis_y, DPAD_UP, DPAD_DOWN)
        };
        let bit = if value < 0.0 { 1 << neg } else { 1 << pos };
        let threshold = if stick_held & bit != 0 {
            GAMEPAD_AXIS_RELEASE
        } else {
            GAMEPAD_AXIS_THRESHOLD
        };
        if value.abs() >= threshold { bit } else { 0 }
    }
}

const MAPPED_MASK: u32 = {
    let mut mask = 0u32;
    let mut i = 0;
    while i < TRACKED_BUTTONS as u32 {
        if standard_gamepad_button(i).is_some() {
            mask |= 1 << i;
        }
        i += 1;
    }
    mask
};

/// Edge-detecting, hold-to-repeat state for one gamepad. Zero heap.
#[derive(Debug, Clone, Default)]
pub struct GamepadRepeater {
    held: u32,
    /// D-Pad bits currently pressed by the left stick (hysteresis state).
    stick: u32,
    next_repeat_us: [u64; TRACKED_BUTTONS],
}

impl GamepadRepeater {
    pub const fn new() -> Self {
        Self {
            held: 0,
            stick: 0,
            next_repeat_us: [0; TRACKED_BUTTONS],
        }
    }

    /// Returns `true` while any mapped button is held.
    #[inline]
    pub const fn is_active(&self) -> bool {
        self.held != 0
    }

    /// Consumes one frame, emitting fresh presses and due auto-repeats in button order.
    pub fn update(&mut self, frame: GamepadFrame, now_us: u64, mut emit: impl FnMut(RemoteInput)) {
        self.stick = frame.stick_mask(self.stick);
        let mask = (frame.pressed | self.stick) & MAPPED_MASK;
        let mut bits = mask;
        while bits != 0 {
            let index = bits.trailing_zeros();
            bits &= bits - 1;
            let slot = index as usize;
            let fresh = self.held & (1 << index) == 0;
            let repeat_due = !fresh && now_us >= self.next_repeat_us[slot];
            if !(fresh || repeat_due) {
                continue;
            }
            self.next_repeat_us[slot] = now_us.saturating_add(if fresh {
                GAMEPAD_HOLD_DELAY_US
            } else {
                GAMEPAD_REPEAT_INTERVAL_US
            });
            if let Some(action) = standard_gamepad_button(index) {
                emit(RemoteInput {
                    action,
                    repeat: !fresh,
                    timestamp_us: now_us,
                });
            }
        }
        self.held = mask;
    }

    /// Forgets held buttons (gamepad disconnected).
    pub fn reset(&mut self) {
        self.held = 0;
        self.stick = 0;
    }
}
