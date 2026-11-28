//! Query invalidation failure non-loop (P0 #1), retry backoff SleepHandle (P1 #2),
//! 3-field Form settle (P1 #3), and external controller swap (P1 #7) regression tests.

use core::task::Waker;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_rpc::Tag64;
use nve_ui::prelude::*;

use crate::{
    find_by_test_id, new_web_backend, pump_settled, render_once, GetItem, NoopFlagWaker,
    RegTransport,
};

#[test]
fn invalidated_query_failure_does_not_loop_and_retry_uses_sleep_backoff() {
    let transport = RegTransport::new("Alpha");
    transport.inner.lock().unwrap().transient_failures_left = 2;
    let recorded_delays = Arc::new(Mutex::new(Vec::<u32>::new()));
    let delays_for_sleep = Arc::clone(&recorded_delays);
    let sleep = SleepHandle::new(move |ms| {
        delays_for_sleep.lock().unwrap().push(ms);
        Box::pin(async {})
    });

    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    #[component]
    fn Root() -> Element {
        let (client, sleep) = dioxus_core::consume_context::<(ErasedQueryClient, SleepHandle)>();
        rsx! {
            QueryClientProvider {
                client: client,
                sleep: Some(sleep),
                View {}
            }
        }
    }

    #[component]
    fn View() -> Element {
        let mut opts = QueryOptions::default().stale_time_ms(60_000);
        opts.max_retries = 2;
        opts.retry_base_delay_ms = 250;
        let q = use_query_with::<GetItem>(1, opts);
        let label = q.data().map(|d| d.name.clone()).unwrap_or_else(|| "NONE".into());
        let has_err = q.error().is_some();
        rsx! { text { "ITEM:{label}:ERR:{has_err}" } }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, Root).with_context((client.clone(), sleep));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html(surface.store()).contains("ITEM:Alpha:ERR:false"));
    assert_eq!(*recorded_delays.lock().unwrap(), vec![250, 500]);
    assert_eq!(transport.inner.lock().unwrap().query_calls, 3);

    // Fail all subsequent queries and invalidate -> MUST stop after initial attempt + 2 retries!
    recorded_delays.lock().unwrap().clear();
    transport.inner.lock().unwrap().fail_queries = true;
    let calls_before = transport.inner.lock().unwrap().query_calls;
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 1)]);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    let calls_after = transport.inner.lock().unwrap().query_calls;
    assert_eq!(
        calls_after - calls_before,
        3,
        "invalidated failing query must stop after initial attempt + 2 retries, not loop forever"
    );
    assert_eq!(*recorded_delays.lock().unwrap(), vec![250, 500]);
    assert!(web.render_html(surface.store()).contains("ITEM:Alpha:ERR:true"));
}

#[test]
fn multi_field_form_settles_without_loop_and_controller_swap_rebinds_cleanly() {
    type CtrlSlot = Rc<RefCell<Option<Signal<Signal<FormController>>>>>;
    let submitted = Arc::new(Mutex::new(Vec::<String>::new()));
    let submitted_ctx = Arc::clone(&submitted);
    let ctrl_slot: CtrlSlot = Rc::new(RefCell::new(None));

    #[component]
    fn MultiFormRoot() -> Element {
        let submitted = dioxus_core::consume_context::<Arc<Mutex<Vec<String>>>>();
        let slot = dioxus_core::consume_context::<CtrlSlot>();
        let mut c1 = use_signal(FormController::new);
        use_hook(move || {
            c1.write().set_text("a", "first-a");
        });
        let active = use_signal(move || c1);
        use_hook(move || {
            *slot.borrow_mut() = Some(active);
        });
        let cur = active();
        rsx! {
            ThemeProvider {
                theme: Theme::LIGHT,
                Form {
                    test_id: "multi-form".to_owned(),
                    controller: Some(cur),
                    on_submit: move |payload: FormSubmitPayload| {
                        submitted
                            .lock()
                            .unwrap()
                            .push(payload.values.get("a").cloned().unwrap_or_default());
                    },
                    FormField { name: "a".to_owned(), test_id: "f-a".to_owned() }
                    FormField { name: "b".to_owned(), test_id: "f-b".to_owned() }
                    FormField { name: "c".to_owned(), test_id: "f-c".to_owned() }
                    FormSubmitButton { test_id: "multi-submit".to_owned(), "Go" }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(SurfaceId::ROOT, MultiFormRoot)
        .with_context(submitted_ctx)
        .with_context(Rc::clone(&ctrl_slot));
    let mut web = new_web_backend();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let ssr1 = web.render_ssr(surface.store()).html;
    assert!(ssr1.contains("value=\"first-a\""), "{ssr1}");
    assert_eq!(
        pump_settled(&mut surface, &mut web, &waker, &mut frame),
        0,
        "3-field Form must settle with 0 spurious async re-renders!"
    );

    let form_id = find_by_test_id(surface.store(), "multi-form");
    let mut submit_ev = nve_abi::EventBatchBuf::new();
    submit_ev.push_submit(SurfaceId::ROOT, form_id, nve_abi::TimestampUs::ZERO, "");
    assert!(surface.dispatch_events(submit_ev.as_validated()));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    assert_eq!(*submitted.lock().unwrap(), vec!["first-a".to_owned()]);

    // Swap `controller` prop -> FormField and FormSubmitContext must reflect the new controller!
    let mut active_sig = ctrl_slot.borrow().unwrap();
    surface.in_root_scope(|| {
        let mut fresh = Signal::new(FormController::new());
        fresh.write().set_text("a", "swapped-a");
        active_sig.set(fresh);
    });
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let ssr2 = web.render_ssr(surface.store()).html;
    assert!(
        ssr2.contains("value=\"swapped-a\""),
        "swapped controller value must render: {ssr2}"
    );
    assert!(surface.dispatch_events(submit_ev.as_validated()));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    assert_eq!(
        *submitted.lock().unwrap(),
        vec!["first-a".to_owned(), "swapped-a".to_owned()]
    );
}
