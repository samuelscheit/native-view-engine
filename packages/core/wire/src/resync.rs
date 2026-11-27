//! Shared client-side keyframe resynchronization state machine for `nve-wire` consumers
//! (`nve-runtime` `PairSession` and `nve-skia-shell` `PairedRemoteApp`).

/// Tracks `waiting_for_keyframe` and rate-limited `(session_id, resync_epoch)` `KeyframeRequest`
/// recovery across reconnects, bounded-queue overflows, and `NVE2` decode errors.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WireResyncState {
    waiting_for_keyframe: bool,
    resync_epoch: u64,
    keyframe_requested_key: Option<(u64, u64)>,
}

impl WireResyncState {
    #[inline]
    pub const fn new() -> Self {
        Self {
            waiting_for_keyframe: false,
            resync_epoch: 0,
            keyframe_requested_key: None,
        }
    }

    #[inline]
    pub const fn is_waiting_for_keyframe(&self) -> bool {
        self.waiting_for_keyframe
    }

    /// Handles [`crate::WireInboundItem::SessionStarted`].
    ///
    /// When `reconnected` is `true` and the consumer already had replica state (or was already
    /// resynchronizing), enters `waiting_for_keyframe` and records `(session_id, resync_epoch)`
    /// because `ReconnectingWireClient` already issued a reconnect `KeyframeRequest`.
    #[inline]
    pub fn on_session_started(
        &mut self,
        session_id: u64,
        reconnected: bool,
        has_prior_state: bool,
    ) {
        if reconnected && (has_prior_state || self.waiting_for_keyframe) {
            self.waiting_for_keyframe = true;
            self.keyframe_requested_key = Some((session_id, self.resync_epoch));
        }
    }

    /// Enters `waiting_for_keyframe` and invokes `send_request` at most once per
    /// `(session_id, resync_epoch)`. Records the epoch ONLY when `send_request()` succeeds so a
    /// transient full queue or disconnect never suppresses a retry.
    #[inline]
    pub fn request_keyframe_once(&mut self, session_id: u64, send_request: impl FnOnce() -> bool) {
        self.waiting_for_keyframe = true;
        let key = (session_id, self.resync_epoch);
        if self.keyframe_requested_key != Some(key) && send_request() {
            self.keyframe_requested_key = Some(key);
        }
    }

    /// Handles [`crate::WireInboundItem::Overflowed`] by entering `waiting_for_keyframe` and
    /// sending a rate-limited `KeyframeRequest`.
    #[inline]
    pub fn on_overflowed(&mut self, session_id: u64, send_request: impl FnOnce() -> bool) {
        self.request_keyframe_once(session_id, send_request);
    }

    /// Handles an `NVE2` decode/apply error by entering `waiting_for_keyframe` and sending a
    /// rate-limited `KeyframeRequest`.
    #[inline]
    pub fn on_decode_error(&mut self, session_id: u64, send_request: impl FnOnce() -> bool) {
        self.request_keyframe_once(session_id, send_request);
    }

    /// Returns `true` when an inbound scene frame should be applied rather than dropped.
    ///
    /// While `waiting_for_keyframe` is active, only `SceneKeyframe` or `frame == 1` is admitted.
    #[inline]
    pub const fn should_apply(&self, is_keyframe: bool, is_frame1: bool) -> bool {
        !self.waiting_for_keyframe || is_keyframe || is_frame1
    }

    /// Marks the replica synchronized after a `SceneKeyframe` or `frame == 1` applies cleanly,
    /// advancing `resync_epoch` so a future error in the same session may request one keyframe.
    #[inline]
    pub fn mark_keyframe_applied(&mut self) {
        if self.waiting_for_keyframe {
            self.waiting_for_keyframe = false;
            self.resync_epoch = self.resync_epoch.wrapping_add(1);
        }
    }
}
