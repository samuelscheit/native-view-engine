//! Target-independent coalesced async wake scheduler for `hosts/web-shell`.
//!
//! Absorbs wakes fired during an active `pump_async_work` pass into a single deferred follow-up
//! decision and guarantees that a `MAX_PUMPS` macrotask paint yield (`setTimeout(0)`) always wins
//! over microtasks, invalidating any earlier stale microtask token.

/// Scheduling tier for the next async pump tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduleKind {
    Microtask,
    Macrotask,
}

/// Action requested by `pump_surface_async_work` / `run_direct_pump` upon exiting a pump pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PumpNext {
    Done,
    Microtask,
    Macrotask,
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

    /// Enters a scheduled microtask or macrotask callback. Returns `false` if the host has shut
    /// down, a pump is already active, or `token` was superseded (e.g. a queued microtask upgraded
    /// to a macrotask paint yield).
    pub(crate) fn on_scheduled_tick_enter(&mut self, token: u64) -> bool {
        if !self.alive || self.pumping {
            return false;
        }
        if self.pending.map(|(_, t)| t) != Some(token) {
            return false;
        }
        self.pending = None;
        self.pumping = true;
        self.woken_during_pump = false;
        true
    }

    /// Enters a synchronous direct pump (initial boot or DOM event callback).
    pub(crate) fn on_direct_pump_enter(&mut self) -> bool {
        if !self.alive || self.pumping {
            return false;
        }
        self.pumping = true;
        self.woken_during_pump = false;
        true
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
            None => None,
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Poll, Wake, Waker};

    use nve_backend_web::WebBackend;
    use nve_dioxus::prelude::*;
    use nve_dioxus::DioxusSurface;
    use nve_engine::Viewport;

    use super::{PumpNext, ScheduleKind, WakeScheduler};
    use crate::session::WebAppSession;

    #[derive(Default)]
    struct Queues {
        microtasks: VecDeque<u64>,
        macrotasks: VecDeque<u64>,
    }

    struct TestFrameWaker {
        sched: Mutex<WakeScheduler>,
        queues: Mutex<Queues>,
    }

    impl TestFrameWaker {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                sched: Mutex::new(WakeScheduler::new()),
                queues: Mutex::new(Queues::default()),
            })
        }

        fn enqueue(&self, item: Option<(ScheduleKind, u64)>) {
            if let Some((kind, tok)) = item {
                let mut q = self.queues.lock().unwrap();
                match kind {
                    ScheduleKind::Microtask => q.microtasks.push_back(tok),
                    ScheduleKind::Macrotask => q.macrotasks.push_back(tok),
                }
            }
        }

        fn run_tick(self: &Arc<Self>, token: u64, session: &mut WebAppSession) -> bool {
            if !self.sched.lock().unwrap().on_scheduled_tick_enter(token) {
                return false;
            }
            let waker = Waker::from(Arc::clone(self));
            let outcome = session.pump_async_work(&waker).unwrap();
            let next = if outcome.hit_cap {
                PumpNext::Macrotask
            } else {
                PumpNext::Done
            };
            let follow = self.sched.lock().unwrap().on_pump_exit(next);
            self.enqueue(follow);
            true
        }
    }

    impl Wake for TestFrameWaker {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            let item = self.sched.lock().unwrap().on_wake();
            self.enqueue(item);
        }
    }

    #[test]
    fn cap_hitting_burst_with_mid_pump_wakes_schedules_only_macrotask() {
        #[derive(Clone, Default)]
        struct StepGate {
            ready: Arc<AtomicBool>,
            waker: Rc<RefCell<Option<Waker>>>,
        }

        impl StepGate {
            fn trigger(&self) {
                self.ready.store(true, Ordering::Release);
                if let Some(w) = self.waker.borrow_mut().take() {
                    w.wake();
                }
            }
        }

        #[component]
        fn BurstApp() -> Element {
            let gate = dioxus_core::consume_context::<StepGate>();
            let mut count = use_signal(|| 0u32);
            let gate_for_task = gate.clone();
            use_hook(move || {
                spawn(async move {
                    for step in 1..=24u32 {
                        core::future::poll_fn(|cx| {
                            if gate_for_task.ready.swap(false, Ordering::AcqRel) {
                                Poll::Ready(())
                            } else {
                                *gate_for_task.waker.borrow_mut() = Some(cx.waker().clone());
                                Poll::Pending
                            }
                        })
                        .await;
                        count.set(step);
                    }
                });
            });
            let cur = count();
            if cur > 0 && cur < 24 {
                gate.trigger();
            }
            rsx! { text { "BURST:{cur}" } }
        }

        let gate = StepGate::default();
        let surface = DioxusSurface::new(1, BurstApp).with_context(gate.clone());
        let mut session = WebAppSession::new(1, surface, WebBackend::new(), Viewport::default());
        session.render_initial().unwrap();

        let fw = TestFrameWaker::new();
        // Initial boot pump arms the task's waker cleanly.
        assert!(fw.sched.lock().unwrap().on_direct_pump_enter());
        let init_out = session.pump_async_work(&Waker::from(Arc::clone(&fw))).unwrap();
        assert!(!init_out.hit_cap);
        let follow = fw.sched.lock().unwrap().on_pump_exit(PumpNext::Done);
        fw.enqueue(follow);
        assert!(fw.queues.lock().unwrap().microtasks.is_empty());
        assert!(fw.queues.lock().unwrap().macrotasks.is_empty());

        // Trigger the 24-step burst outside a pump -> schedules 1 microtask.
        gate.trigger();
        let first_tok = fw.queues.lock().unwrap().microtasks.pop_front().unwrap();
        assert!(fw.queues.lock().unwrap().microtasks.is_empty());
        assert!(fw.queues.lock().unwrap().macrotasks.is_empty());

        // Running that microtask hits MAX_PUMPS (8) WHILE every step wakes `fw`:
        // MUST schedule 1 macrotask (paint yield) and 0 microtasks!
        assert!(fw.run_tick(first_tok, &mut session));
        {
            let q = fw.queues.lock().unwrap();
            assert_eq!(
                q.microtasks.len(),
                0,
                "mid-pump wakes must not bypass macrotask paint yield with a microtask"
            );
            assert_eq!(
                q.macrotasks.len(),
                1,
                "MAX_PUMPS cap hit must schedule a macrotask continuation"
            );
        }
        assert!(!session.backend().render_html().contains("BURST:24"));

        // Drain macrotasks until settled -> all 24 steps complete without microtask starvation.
        for _ in 0..8 {
            let next_tok = {
                let mut q = fw.queues.lock().unwrap();
                assert!(
                    q.microtasks.is_empty()
                        || session.backend().render_html().contains("BURST:24"),
                    "intermediate cap-hitting ticks must yield via macrotasks, not microtasks"
                );
                q.macrotasks.pop_front().or_else(|| q.microtasks.pop_front())
            };
            let Some(tok) = next_tok else {
                break;
            };
            assert!(fw.run_tick(tok, &mut session));
        }
        assert!(session.backend().render_html().contains("BURST:24"));
        assert!(fw.queues.lock().unwrap().microtasks.is_empty());
        assert!(fw.queues.lock().unwrap().macrotasks.is_empty());
    }

    #[test]
    fn wake_scheduler_upgrades_stale_microtask_handles_done_wakes_and_shutdown() {
        let mut s = WakeScheduler::new();

        // 1. External wake queues microtask; duplicate wakes coalesce.
        let (k1, tok1) = s.on_wake().expect("first wake schedules microtask");
        assert_eq!(k1, ScheduleKind::Microtask);
        assert_eq!(s.on_wake(), None);

        // 2. Direct pump hits MAX_PUMPS while microtask `tok1` is still queued -> upgrades to
        //    macrotask `tok2`, and stale `tok1` is skipped cleanly.
        assert!(s.on_direct_pump_enter());
        assert_eq!(s.on_wake(), None);
        let (k2, tok2) = s
            .on_pump_exit(PumpNext::Macrotask)
            .expect("cap hit must upgrade pending microtask to macrotask");
        assert_eq!(k2, ScheduleKind::Macrotask);
        assert!(!s.on_scheduled_tick_enter(tok1), "stale microtask must be skipped");
        assert!(s.on_scheduled_tick_enter(tok2), "upgraded macrotask must enter");

        // 3. Wake during a non-cap tick (e.g. `yield_once()`) schedules 1 microtask on Done.
        assert_eq!(s.on_wake(), None);
        let (k3, tok3) = s
            .on_pump_exit(PumpNext::Done)
            .expect("mid-pump wake on Done must schedule one microtask");
        assert_eq!(k3, ScheduleKind::Microtask);
        assert!(s.on_scheduled_tick_enter(tok3));
        assert_eq!(s.on_pump_exit(PumpNext::Done), None);

        // 4. Re-entrancy (`PumpNext::Microtask`) schedules microtask; shutdown cancels all future ticks.
        assert!(s.on_direct_pump_enter());
        let (k4, tok4) = s.on_pump_exit(PumpNext::Microtask).unwrap();
        assert_eq!(k4, ScheduleKind::Microtask);
        s.shutdown();
        assert!(!s.on_scheduled_tick_enter(tok4));
        assert_eq!(s.on_wake(), None);
        assert!(!s.on_direct_pump_enter());
    }
}
