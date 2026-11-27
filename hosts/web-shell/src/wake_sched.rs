//! Target-independent coalesced async wake scheduler for `hosts/web-shell`.
//!
//! Absorbs wakes fired during an active `pump_async_work_with` pass into a single deferred
//! follow-up decision and guarantees that a `MAX_PUMPS` macrotask paint yield (`setTimeout(0)`)
//! always wins over microtasks, invalidating any earlier stale microtask token.

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "wake_sched_tests.rs"]
mod tests;

/// Scheduling tier for the next async pump tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduleKind {
    Microtask,
    Macrotask,
}

/// Reason a synchronous direct pump window could not be entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectPumpBlocked {
    AlreadyPumping,
    Shutdown,
}

/// Action requested by `pump_surface_async_work` / `run_direct_pump` upon exiting a pump pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PumpNext {
    Done,
    Microtask,
    Macrotask,
}

impl PumpNext {
    #[inline]
    pub(crate) const fn from_hit_cap(hit_cap: bool) -> Self {
        if hit_cap {
            Self::Macrotask
        } else {
            Self::Done
        }
    }
}

/// Pure deterministic state machine coordinating `Waker::wake`, direct event/boot pumps, and
/// browser microtask/macrotask ticks without re-entrancy or paint starvation.
#[derive(Debug)]
pub(crate) struct WakeScheduler {
    alive: bool,
    pumping: bool,
    woken_during_pump: bool,
    pending: Option<(ScheduleKind, u64)>,
    next_token: u64,
}

impl WakeScheduler {
    pub(crate) fn new() -> Self {
        Self {
            alive: true,
            pumping: false,
            woken_during_pump: false,
            pending: None,
            next_token: 1,
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.alive = false;
        self.pending = None;
    }

    fn alloc_token(&mut self) -> u64 {
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        token
    }

    /// Handles a `Waker::wake` / `wake_by_ref` notification.
    ///
    /// While a pump is running, records `woken_during_pump = true` instead of queuing a microtask
    /// mid-pump so `PumpNext::Macrotask` cannot be bypassed and no spurious closures are allocated.
    pub(crate) fn on_wake(&mut self) -> Option<(ScheduleKind, u64)> {
        if !self.alive {
            return None;
        }
        if self.pumping {
            self.woken_during_pump = true;
            return None;
        }
        if self.pending.is_some() {
            return None;
        }
        let token = self.alloc_token();
        self.pending = Some((ScheduleKind::Microtask, token));
        Some((ScheduleKind::Microtask, token))
    }

    #[cfg(test)]
    pub(crate) fn is_pumping(&self) -> bool {
        self.pumping
    }

    /// Clears `woken_during_pump` immediately before a `poll_work` pass that is about to drain all
    /// ready tasks, avoiding a redundant trailing microtask once the pump settles cleanly.
    #[inline]
    pub(crate) fn clear_mid_pump_wake(&mut self) {
        self.woken_during_pump = false;
    }

    /// Enters a scheduled microtask or macrotask callback. Returns `false` if the host has shut
    /// down, `token` was superseded (e.g. a queued microtask upgraded to a macrotask paint yield),
    /// or a pump is already active (in which case a matching active token clears `pending` and
    /// folds into `woken_during_pump` so pump exit reschedules instead of wedging).
    pub(crate) fn on_scheduled_tick_enter(&mut self, token: u64) -> bool {
        if !self.alive {
            return false;
        }
        if self.pending.map(|(_, t)| t) != Some(token) {
            return false;
        }
        self.pending = None;
        if self.pumping {
            self.woken_during_pump = true;
            return false;
        }
        self.pumping = true;
        self.woken_during_pump = false;
        true
    }

    /// Enters a synchronous direct pump (initial boot or DOM event callback), distinguishing
    /// active re-entrancy (`AlreadyPumping`) from a torn-down surface (`Shutdown`).
    pub(crate) fn on_direct_pump_enter(&mut self) -> Result<(), DirectPumpBlocked> {
        if !self.alive {
            return Err(DirectPumpBlocked::Shutdown);
        }
        if self.pumping {
            return Err(DirectPumpBlocked::AlreadyPumping);
        }
        self.pumping = true;
        self.woken_during_pump = false;
        Ok(())
    }

    /// Exits an active pump pass and returns the single follow-up schedule to dispatch, if any.
    pub(crate) fn on_pump_exit(&mut self, next: PumpNext) -> Option<(ScheduleKind, u64)> {
        self.pumping = false;
        let woken = core::mem::take(&mut self.woken_during_pump);
        if !self.alive {
            return None;
        }
        let desired = match next {
            PumpNext::Macrotask => Some(ScheduleKind::Macrotask),
            PumpNext::Microtask => Some(ScheduleKind::Microtask),
            PumpNext::Done => woken.then_some(ScheduleKind::Microtask),
        };
        match desired {
            None => {
                // Settled pump (`PumpNext::Done` with no mid-pump wakes) drained all ready work,
                // so invalidate any obsolete pre-existing pending microtask/macrotask token.
                self.pending = None;
                None
            }
            Some(ScheduleKind::Macrotask) => {
                if matches!(self.pending, Some((ScheduleKind::Macrotask, _))) {
                    return None;
                }
                let token = self.alloc_token();
                self.pending = Some((ScheduleKind::Macrotask, token));
                Some((ScheduleKind::Macrotask, token))
            }
            Some(ScheduleKind::Microtask) => {
                if self.pending.is_some() {
                    return None;
                }
                let token = self.alloc_token();
                self.pending = Some((ScheduleKind::Microtask, token));
                Some((ScheduleKind::Microtask, token))
            }
        }
    }
}
