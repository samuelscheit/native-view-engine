//! Per-action auto-repeat filter (held D-Pad / channel keys) over a fixed dense table.

use crate::action::RemoteAction;

/// Default minimum spacing between accepted auto-repeat events (matches the Android TV bridge).
pub const DEFAULT_REPEAT_INTERVAL_US: u64 = 100_000;

/// Drops every auto-repeat of an action that does not [`RemoteAction::auto_repeats`], and
/// auto-repeats of the others arriving faster than `interval_us`.
///
/// Fresh (non-repeat) presses always pass and restart the window. Each action is throttled
/// independently, so holding Right never delays a Select press. Zero heap: one fixed slot per
/// [`RemoteAction::code`].
#[derive(Debug, Clone)]
pub struct RepeatThrottle {
    interval_us: u64,
    last_accepted_us: [u64; RemoteAction::CODE_COUNT],
    seen: u64,
}

const _: () = assert!(RemoteAction::CODE_COUNT <= u64::BITS as usize);

impl Default for RepeatThrottle {
    fn default() -> Self {
        Self::new(DEFAULT_REPEAT_INTERVAL_US)
    }
}

impl RepeatThrottle {
    /// `interval_us == 0` disables rate limiting (non-repeating actions still never repeat).
    pub const fn new(interval_us: u64) -> Self {
        Self {
            interval_us,
            last_accepted_us: [0; RemoteAction::CODE_COUNT],
            seen: 0,
        }
    }

    #[inline]
    pub const fn interval_us(&self) -> u64 {
        self.interval_us
    }

    #[inline]
    pub fn set_interval_us(&mut self, interval_us: u64) {
        self.interval_us = interval_us;
    }

    /// Returns `true` when [`Self::admit`] would accept the event, without recording it (lets
    /// callers skip layout work for inputs that are dropped anyway).
    ///
    /// A timestamp earlier than the last accepted one (clock reset) is admitted.
    pub fn would_admit(&self, action: RemoteAction, repeat: bool, timestamp_us: u64) -> bool {
        if !repeat {
            return true;
        }
        if !action.auto_repeats() {
            return false;
        }
        let slot = usize::from(action.code());
        if self.interval_us == 0 || self.seen & (1u64 << slot) == 0 {
            return true;
        }
        let last = self.last_accepted_us[slot];
        timestamp_us < last || timestamp_us - last >= self.interval_us
    }

    /// Returns `true` when the event should be dispatched, recording its timestamp.
    pub fn admit(&mut self, action: RemoteAction, repeat: bool, timestamp_us: u64) -> bool {
        if !self.would_admit(action, repeat, timestamp_us) {
            return false;
        }
        let slot = usize::from(action.code());
        self.seen |= 1u64 << slot;
        self.last_accepted_us[slot] = timestamp_us;
        true
    }

    /// Forgets every recorded press.
    pub fn reset(&mut self) {
        self.seen = 0;
    }
}
