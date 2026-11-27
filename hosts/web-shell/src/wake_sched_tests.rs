use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Wake, Waker};

use nve_abi::{EventPayload, NativeEvent, NativeEventKind};
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::Viewport;

use super::{DirectPumpBlocked, PumpNext, ScheduleKind, WakeScheduler};
use crate::session::{AsyncPumpOutcome, WebAppSession};

#[derive(Default)]
struct Queues {
    microtasks: VecDeque<u64>,
    macrotasks: VecDeque<u64>,
}

struct TestFrameWaker {
    sched: Mutex<WakeScheduler>,
    queues: Mutex<Queues>,
    mid_pump_wakes: AtomicUsize,
    before_poll_calls: AtomicUsize,
}

impl TestFrameWaker {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            sched: Mutex::new(WakeScheduler::new()),
            queues: Mutex::new(Queues::default()),
            mid_pump_wakes: AtomicUsize::new(0),
            before_poll_calls: AtomicUsize::new(0),
        })
    }

    fn enqueue(&self, item: Option<(ScheduleKind, u64)>) {
        if let Some((kind, tok)) = item {
            let mut q = self.queues.lock().unwrap_or_else(|e| e.into_inner());
            match kind {
                ScheduleKind::Microtask => q.microtasks.push_back(tok),
                ScheduleKind::Macrotask => q.macrotasks.push_back(tok),
            }
        }
    }

    fn clear_mid_pump_wake(&self) {
        self.before_poll_calls.fetch_add(1, Ordering::AcqRel);
        self.sched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear_mid_pump_wake();
    }

    fn run_tick_outcome(
        self: &Arc<Self>,
        token: u64,
        session: &mut WebAppSession,
    ) -> Option<AsyncPumpOutcome> {
        if !self
            .sched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .on_scheduled_tick_enter(token)
        {
            return None;
        }
        let waker = Waker::from(Arc::clone(self));
        let outcome = session
            .pump_async_work_with(&waker, || self.clear_mid_pump_wake())
            .expect("pump_async_work_with");
        let follow = self
            .sched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .on_pump_exit(PumpNext::from_hit_cap(outcome.hit_cap));
        self.enqueue(follow);
        Some(outcome)
    }

    fn run_tick(self: &Arc<Self>, token: u64, session: &mut WebAppSession) -> bool {
        self.run_tick_outcome(token, session).is_some()
    }
}

impl Wake for TestFrameWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let item = {
            let mut sched = self.sched.lock().unwrap_or_else(|e| e.into_inner());
            if sched.is_pumping() {
                self.mid_pump_wakes.fetch_add(1, Ordering::AcqRel);
            }
            sched.on_wake()
        };
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
        let outer_waker = dioxus_core::consume_context::<Waker>();
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
            // Exercise real mid-pump wakes directly against `TestFrameWaker` during 0..MAX_PUMPS.
            outer_waker.wake_by_ref();
            gate.trigger();
        }
        rsx! { text { "BURST:{cur}" } }
    }

    let fw = TestFrameWaker::new();
    let gate = StepGate::default();
    let surface = DioxusSurface::new(1, BurstApp)
        .with_context(gate.clone())
        .with_context(Waker::from(Arc::clone(&fw)));
    let mut session = WebAppSession::new(1, surface, WebBackend::new(), Viewport::default());
    session.render_initial().unwrap();

    assert_eq!(fw.sched.lock().unwrap().on_direct_pump_enter(), Ok(()));
    let init_out = session
        .pump_async_work_with(&Waker::from(Arc::clone(&fw)), || fw.clear_mid_pump_wake())
        .unwrap();
    assert!(!init_out.hit_cap);
    let follow = fw.sched.lock().unwrap().on_pump_exit(PumpNext::Done);
    fw.enqueue(follow);
    assert!(fw.queues.lock().unwrap().microtasks.is_empty());
    assert!(fw.queues.lock().unwrap().macrotasks.is_empty());

    // Trigger 24-step burst outside a pump -> schedules 1 microtask.
    gate.trigger();
    let first_tok = fw.queues.lock().unwrap().microtasks.pop_front().unwrap();
    assert!(fw.queues.lock().unwrap().microtasks.is_empty());
    assert!(fw.queues.lock().unwrap().macrotasks.is_empty());

    // Running that microtask hits MAX_PUMPS (8) WHILE every step wakes `fw`:
    // MUST absorb >= 8 mid-pump wakes and schedule 1 macrotask (paint yield) + 0 microtasks!
    assert!(fw.run_tick(first_tok, &mut session));
    assert!(
        fw.mid_pump_wakes.load(Ordering::Acquire) >= 8,
        "test must exercise mid-pump wakes on TestFrameWaker (saw {})",
        fw.mid_pump_wakes.load(Ordering::Acquire)
    );
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
    assert!(!session.render_html().contains("BURST:24"));

    for _ in 0..8 {
        let next_tok = {
            let mut q = fw.queues.lock().unwrap();
            assert!(
                q.microtasks.is_empty() || session.render_html().contains("BURST:24"),
                "intermediate cap-hitting ticks must yield via macrotasks, not microtasks"
            );
            q.macrotasks.pop_front().or_else(|| q.microtasks.pop_front())
        };
        let Some(tok) = next_tok else {
            break;
        };
        assert!(fw.run_tick(tok, &mut session));
    }
    assert!(session.render_html().contains("BURST:24"));
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
    //    macrotask `tok2`, and stale `tok1` is skipped cleanly. Also verifies AlreadyPumping.
    assert_eq!(s.on_direct_pump_enter(), Ok(()));
    assert_eq!(
        s.on_direct_pump_enter(),
        Err(DirectPumpBlocked::AlreadyPumping)
    );
    assert_eq!(s.on_wake(), None);
    let (k2, tok2) = s
        .on_pump_exit(PumpNext::Macrotask)
        .expect("cap hit must upgrade pending microtask to macrotask");
    assert_eq!(k2, ScheduleKind::Macrotask);
    assert!(!s.on_scheduled_tick_enter(tok1), "stale microtask must be skipped");
    assert!(s.on_scheduled_tick_enter(tok2), "upgraded macrotask must enter");

    // 3. Clearing an absorbed mid-pump wake before settling yields None, while a wake fired
    //    AFTER `clear_mid_pump_wake()` schedules 1 microtask.
    assert_eq!(s.on_wake(), None);
    s.clear_mid_pump_wake();
    assert_eq!(s.on_pump_exit(PumpNext::Done), None);

    assert_eq!(s.on_direct_pump_enter(), Ok(()));
    s.clear_mid_pump_wake();
    assert_eq!(s.on_wake(), None);
    let (k3, tok3) = s
        .on_pump_exit(PumpNext::Done)
        .expect("wake fired after clear_mid_pump_wake must schedule one microtask");
    assert_eq!(k3, ScheduleKind::Microtask);
    assert!(s.on_scheduled_tick_enter(tok3));
    assert_eq!(s.on_pump_exit(PumpNext::Done), None);

    // 4. Settled direct pump (`Done` with no mid-pump wakes) clears stale pending microtask.
    let (km, stale_micro) = s.on_wake().unwrap();
    assert_eq!(km, ScheduleKind::Microtask);
    assert_eq!(s.on_direct_pump_enter(), Ok(()));
    assert_eq!(s.on_pump_exit(PumpNext::Done), None);
    assert!(!s.on_scheduled_tick_enter(stale_micro));
    let (k_fresh, tok_fresh) = s.on_wake().expect("cleared pending allows immediate wake");
    assert_eq!(k_fresh, ScheduleKind::Microtask);

    // 5. Re-entrant scheduled tick arriving while `pumping == true` clears `pending` and folds
    //    into `woken_during_pump` so exit reschedules instead of wedging.
    assert_eq!(s.on_direct_pump_enter(), Ok(()));
    assert!(!s.on_scheduled_tick_enter(tok_fresh));
    let (k_re, tok_re) = s
        .on_pump_exit(PumpNext::Done)
        .expect("re-entrant tick arriving mid-pump must reschedule instead of wedging");
    assert_eq!(k_re, ScheduleKind::Microtask);
    assert!(s.on_scheduled_tick_enter(tok_re));
    assert_eq!(s.on_pump_exit(PumpNext::Done), None);

    // 6. Re-entrancy (`PumpNext::Microtask`) schedules microtask; shutdown reports Shutdown.
    assert_eq!(s.on_direct_pump_enter(), Ok(()));
    let (k4, tok4) = s.on_pump_exit(PumpNext::Microtask).unwrap();
    assert_eq!(k4, ScheduleKind::Microtask);
    s.shutdown();
    assert!(!s.on_scheduled_tick_enter(tok4));
    assert_eq!(s.on_wake(), None);
    assert_eq!(s.on_direct_pump_enter(), Err(DirectPumpBlocked::Shutdown));
}

#[test]
fn pump_async_work_with_clears_prior_wakes_per_iteration_and_preserves_final_poll_wake() {
    #[derive(Clone, Default)]
    struct PollGate {
        ready: Arc<AtomicBool>,
        task_waker: Rc<RefCell<Option<Waker>>>,
    }

    #[derive(Clone, Default)]
    struct PumpGates {
        render_wake: PollGate,
        final_poll: PollGate,
    }

    #[component]
    fn PumpOrderApp() -> Element {
        let outer_waker = dioxus_core::consume_context::<Waker>();
        let gates = dioxus_core::consume_context::<PumpGates>();
        let mut clicks = use_signal(|| 0u32);
        let mut async_done = use_signal(|| 0u32);
        let mut render_wake_step = use_signal(|| 0u32);
        let rw_wake_fired = use_hook(|| Rc::new(core::cell::Cell::new(false)));
        let mut final_poll_done = use_signal(|| false);

        // Fire a one-shot wake on `outer_waker` INSIDE `pump_async_work_with` iteration 0's
        // `render_and_submit()` when `render_wake_step` transitions to 1 (one-shot so sub-test #3
        // stays completely decoupled from sub-test #2). Iteration 1 MUST call `on_before_poll`
        // before the settling `poll_work` so this render-pass wake schedules 0 trailing microtasks!
        let rw = render_wake_step();
        if rw == 1 && !rw_wake_fired.replace(true) {
            outer_waker.wake_by_ref();
        }

        let rw_gate = gates.render_wake.clone();
        let outer_for_final = outer_waker.clone();
        let gate_for_final = gates.final_poll.clone();
        rsx! {
            view {
                text { "CLICKS:{clicks()}:ASYNC:{async_done()}:RW:{rw}:FINAL:{final_poll_done()}" }
                pressable {
                    test_id: "spawn-btn",
                    onpress: move |_| {
                        clicks += 1;
                        spawn(async move {
                            async_done += 1;
                        });
                    },
                    "Spawn"
                }
                pressable {
                    test_id: "render-wake-btn",
                    onpress: move |_| {
                        let gate = rw_gate.clone();
                        spawn(async move {
                            // Park behind an external gate so `render_wake_step.set(1)` cannot
                            // run inside `pump_events()` and is guaranteed to execute inside
                            // `pump_async_work_with` iteration 0's `poll_work()`.
                            core::future::poll_fn(|cx| {
                                if gate.ready.load(Ordering::Acquire) {
                                    Poll::Ready(())
                                } else {
                                    *gate.task_waker.borrow_mut() = Some(cx.waker().clone());
                                    Poll::Pending
                                }
                            })
                            .await;
                            render_wake_step.set(1);
                        });
                    },
                    "RenderWake"
                }
                pressable {
                    test_id: "final-poll-wake-btn",
                    onpress: move |_| {
                        let outer = outer_for_final.clone();
                        let gate = gate_for_final.clone();
                        spawn(async move {
                            let mut polls = 0u32;
                            core::future::poll_fn(|cx| {
                                polls += 1;
                                if gate.ready.load(Ordering::Acquire) {
                                    Poll::Ready(())
                                } else {
                                    *gate.task_waker.borrow_mut() = Some(cx.waker().clone());
                                    // Poll 1 runs synchronously inside `dioxus_core::spawn()`;
                                    // poll 2 runs INSIDE the settling `poll_work()` call (returning
                                    // `false` because no scope is dirty yet), waking `outer`!
                                    if polls == 2 {
                                        outer.wake_by_ref();
                                    }
                                    Poll::Pending
                                }
                            })
                            .await;
                            final_poll_done.set(true);
                        });
                    },
                    "FinalPollWake"
                }
            }
        }
    }

    let fw = TestFrameWaker::new();
    let gates = PumpGates::default();
    let surface = DioxusSurface::new(1, PumpOrderApp)
        .with_context(Waker::from(Arc::clone(&fw)))
        .with_context(gates.clone());
    let mut session = WebAppSession::new(1, surface, WebBackend::new(), Viewport::default());
    let waker = Waker::from(Arc::clone(&fw));

    let run_direct =
        |session: &mut WebAppSession, fw: &Arc<TestFrameWaker>, is_boot: bool| -> AsyncPumpOutcome {
            assert_eq!(fw.sched.lock().unwrap().on_direct_pump_enter(), Ok(()));
            if is_boot {
                session.render_initial().unwrap();
            }
            let _ = session.pump_events().unwrap();
            let out = session
                .pump_async_work_with(&waker, || fw.clear_mid_pump_wake())
                .unwrap();
            let follow = fw
                .sched
                .lock()
                .unwrap()
                .on_pump_exit(PumpNext::from_hit_cap(out.hit_cap));
            fw.enqueue(follow);
            out
        };

    let find = |session: &WebAppSession, tid: &str| {
        crate::tests::find_node_by_test_id(session.store(), tid)
            .unwrap_or_else(|| panic!("missing node test_id={tid}"))
    };

    run_direct(&mut session, &fw, true);
    assert!(fw.queues.lock().unwrap().microtasks.is_empty());

    // 1. Task-spawning click drained synchronously -> 0 trailing microtasks despite mid-pump wake.
    let spawn_btn = find(&session, "spawn-btn");
    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: spawn_btn,
        timestamp_us: 1,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    });
    run_direct(&mut session, &fw, false);
    assert!(session.render_html().contains("CLICKS:1:ASYNC:1"));
    assert!(fw.mid_pump_wakes.load(Ordering::Acquire) >= 1);
    assert!(
        fw.queues.lock().unwrap().microtasks.is_empty(),
        "synchronously drained click must queue 0 trailing microtasks"
    );

    // 2. Multi-iteration wake fired inside iteration 0's `render_and_submit()` -> iteration 1's
    //    `on_before_poll` clears it before settling, so 0 trailing microtasks are queued!
    //    (Fails right here if `on_before_poll` is hoisted once before the loop.)
    let rw_btn = find(&session, "render-wake-btn");
    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: rw_btn,
        timestamp_us: 2,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    });
    run_direct(&mut session, &fw, false);
    assert!(session.render_html().contains("RW:0"));
    assert!(fw.queues.lock().unwrap().microtasks.is_empty());

    let wakes_before_rw = fw.mid_pump_wakes.load(Ordering::Acquire);
    let polls_before_rw = fw.before_poll_calls.load(Ordering::Acquire);
    gates.render_wake.ready.store(true, Ordering::Release);
    gates
        .render_wake
        .task_waker
        .borrow_mut()
        .take()
        .expect("render-wake task must be parked on gate")
        .wake();
    let rw_tok = fw
        .queues
        .lock()
        .unwrap()
        .microtasks
        .pop_front()
        .expect("releasing render-wake gate must schedule a microtask");
    let rw_out = fw
        .run_tick_outcome(rw_tok, &mut session)
        .expect("render-wake microtask must execute");
    assert!(
        rw_out.timing.is_some(),
        "sub-test #2 must render inside pump_async_work_with iteration 0, not pump_events"
    );
    assert_eq!(
        fw.before_poll_calls.load(Ordering::Acquire) - polls_before_rw,
        2,
        "sub-test #2 must execute iteration 0 (render) plus settling iteration 1"
    );
    assert!(session.render_html().contains("RW:1"));
    assert!(
        fw.mid_pump_wakes.load(Ordering::Acquire) > wakes_before_rw,
        "iteration 0 render_and_submit must fire a mid-pump wake"
    );
    assert!(
        fw.queues.lock().unwrap().microtasks.is_empty(),
        "per-iteration on_before_poll must clear iteration-0 render wakes before settling"
    );

    // 3. Wake fired DURING the final settling `poll_work()` (returning `false`) MUST survive and
    //    schedule exactly 1 follow-up microtask! (Fails if `on_before_poll` is moved after poll.)
    let fp_btn = find(&session, "final-poll-wake-btn");
    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: fp_btn,
        timestamp_us: 3,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    });
    run_direct(&mut session, &fw, false);
    assert!(fw.queues.lock().unwrap().microtasks.is_empty());

    // Wake the parked task outside any pump -> queues microtask `tok1`. Running `tok1` executes
    // poll #2 inside `poll_work()` (which wakes `outer` and returns `false`), so `on_pump_exit`
    // MUST preserve that final-poll wake and schedule follow-up microtask `tok2`!
    if let Some(tw) = gates.final_poll.task_waker.borrow_mut().take() {
        tw.wake();
    }
    let tok1 = fw
        .queues
        .lock()
        .unwrap()
        .microtasks
        .pop_front()
        .expect("external task wake must schedule tok1");
    assert!(fw.run_tick(tok1, &mut session));
    assert_eq!(
        fw.queues.lock().unwrap().microtasks.len(),
        1,
        "wake fired during final settling poll_work() must schedule exactly 1 microtask"
    );
    let tok2 = fw.queues.lock().unwrap().microtasks.pop_front().unwrap();
    gates.final_poll.ready.store(true, Ordering::Release);
    if let Some(tw) = gates.final_poll.task_waker.borrow_mut().take() {
        tw.wake();
    }
    assert!(fw.run_tick(tok2, &mut session));
    assert!(session.render_html().contains("FINAL:true"));
    assert!(
        fw.queues.lock().unwrap().microtasks.is_empty(),
        "settling final tick must leave zero trailing microtasks"
    );
}
