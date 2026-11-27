//! `UseQuery::suspend` keeps suspending while the provider's retry loop is between attempts
//! (transient error committed, backoff sleeping) and only surfaces the error into the enclosing
//! `<QuerySuspense>` `ErrorBoundary` once retries exhaust.

use core::task::{Poll, Waker};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;

use crate::{pump_settled, render_once, GetItem, NoopFlagWaker, RegTransport};

#[derive(Default)]
struct SleepGate {
    released: usize,
    sleeping: usize,
    wakers: Vec<Waker>,
}

/// Backoff sleeps park until [`release_one`] lets exactly one of them finish.
fn gated_sleep(gate: &Arc<Mutex<SleepGate>>) -> SleepHandle {
    let gate = Arc::clone(gate);
    SleepHandle::new(move |_ms| {
        let gate = Arc::clone(&gate);
        let mut ticket = None;
        Box::pin(async move {
            core::future::poll_fn(|cx| {
                let mut g = gate.lock().unwrap();
                let my = *ticket.get_or_insert_with(|| {
                    g.sleeping += 1;
                    g.sleeping
                });
                if g.released >= my {
                    Poll::Ready(())
                } else {
                    g.wakers.push(cx.waker().clone());
                    Poll::Pending
                }
            })
            .await;
        })
    })
}

fn release_one(gate: &Arc<Mutex<SleepGate>>) {
    let wakers = {
        let mut g = gate.lock().unwrap();
        g.released += 1;
        core::mem::take(&mut g.wakers)
    };
    for w in wakers {
        w.wake();
    }
}

#[component]
fn RetryItem() -> Element {
    let q = use_query_with::<GetItem>(
        5,
        QueryOptions::default().stale_time_ms(60_000).max_retries(2),
    );
    let item = q.suspend()?;
    rsx! { text { "OK:{item.name}" } }
}

#[component]
fn RetryApp() -> Element {
    let (client, sleep) = dioxus_core::consume_context::<(ErasedQueryClient, SleepHandle)>();
    rsx! {
        QueryClientProvider {
            client: client,
            sleep: Some(sleep),
            view {
                QuerySuspense {
                    id: Some("retry".into()),
                    fallback: rsx! { text { "LOADING" } },
                    error_fallback: move |report: UiErrorReport, _reset: ErrorBoundaryReset| rsx! {
                        text { "CAUGHT:{report.message}" }
                    },
                    RetryItem {}
                }
            }
        }
    }
}

struct Harness {
    transport: RegTransport,
    gate: Arc<Mutex<SleepGate>>,
    surface: DioxusSurface,
    web: WebBackend,
    frame: u64,
    waker: Waker,
}

impl Harness {
    fn new(transient_failures: usize) -> Self {
        let transport = RegTransport::new("Recovered");
        transport.inner.lock().unwrap().transient_failures_left = transient_failures;
        let client = erased_query_client_with_clock(
            transport.clone(),
            Shared::new(ManualClock::new(1_000)),
            QueryClientConfig::default(),
        );
        let gate = Arc::new(Mutex::new(SleepGate::default()));
        let surface = DioxusSurface::new(1, RetryApp).with_context((client, gated_sleep(&gate)));
        Self {
            transport,
            gate,
            surface,
            web: WebBackend::new(),
            frame: 0,
            waker: Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false)))),
        }
    }

    fn settle(&mut self) -> String {
        let _ = pump_settled(&mut self.surface, &mut self.web, &self.waker, &mut self.frame);
        self.web.render_html()
    }

    fn calls(&self) -> usize {
        self.transport.inner.lock().unwrap().query_calls
    }
}

#[test]
fn transient_error_between_retry_attempts_keeps_suspending_then_resolves() {
    let mut h = Harness::new(1);
    let _ = render_once(&mut h.surface, &mut h.web, &mut h.frame);
    let html = h.settle();

    // Attempt 1 failed and committed a retryable error; the retry loop now sleeps in backoff.
    assert_eq!(h.calls(), 1);
    assert_eq!(h.gate.lock().unwrap().sleeping, 1, "retry loop parked in backoff");
    assert!(
        html.contains("LOADING") && !html.contains("CAUGHT"),
        "transient error must not trip the ErrorBoundary mid-retry: {html}"
    );

    release_one(&h.gate);
    let html = h.settle();
    assert_eq!(h.calls(), 2);
    assert!(
        html.contains("OK:Recovered") && !html.contains("CAUGHT") && !html.contains("LOADING"),
        "retry success resolves the boundary cleanly: {html}"
    );
}

#[test]
fn exhausted_retries_surface_the_final_error_into_the_error_boundary() {
    let mut h = Harness::new(usize::MAX);
    let _ = render_once(&mut h.surface, &mut h.web, &mut h.frame);

    for attempt in 1..=2 {
        let html = h.settle();
        assert_eq!(h.calls(), attempt);
        assert!(
            !html.contains("CAUGHT"),
            "attempt {attempt} still has retries left: {html}"
        );
        release_one(&h.gate);
    }

    let html = h.settle();
    assert_eq!(h.calls(), 3, "1 attempt + max_retries(2)");
    assert!(
        html.contains("CAUGHT:") && html.contains("transient timeout") && !html.contains("OK:"),
        "exhausted retries must reach the ErrorBoundary: {html}"
    );
}
