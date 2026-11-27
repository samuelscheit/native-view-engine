//! Tests for coalesced `SkiaHostServices::request_frame_wake` and direct `DioxusDesktopApp`
//! background-thread async task wakeup (`DioxusSurface::poll_work` -> `DesktopApp::poll_async_work`).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use nve_abi::{ColorRgba, METHOD_WINDOW_REQUEST_FRAME, SERVICE_WINDOW};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::Viewport;
use nve_host_services::HostServiceProvider;

use super::{DesktopApp, DesktopPreviewOptions, DesktopSession, DesktopWindowHost, DioxusDesktopApp};
use crate::host_services::SkiaHostServices;

#[test]
fn request_frame_wake_coalesces_bursts_and_preserves_pre_sink_wakes() {
    let services = SkiaHostServices::new();
    let wake_calls = Arc::new(AtomicUsize::new(0));

    // Fire 3 wakes BEFORE attaching a sink -> attaching sink fires exactly once.
    services.request_frame_wake();
    services.request_frame_wake();
    services.request_frame_wake();

    let calls_cb = Arc::clone(&wake_calls);
    services.attach_wake_sink(Arc::new(move || {
        calls_cb.fetch_add(1, Ordering::SeqCst);
    }));
    assert_eq!(wake_calls.load(Ordering::SeqCst), 1);

    // Additional wakes while `pending_app_wake` is still armed coalesce to 0 extra calls!
    services.request_frame_wake();
    let _ = services.invoke_raw(SERVICE_WINDOW, METHOD_WINDOW_REQUEST_FRAME, &[]);
    assert_eq!(wake_calls.load(Ordering::SeqCst), 1);

    // Consuming the pending flag re-arms coalesced delivery for the next wake.
    assert!(services.take_pending_app_wake());
    assert!(!services.take_pending_app_wake());
    services.request_frame_wake();
    assert_eq!(wake_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn dioxus_desktop_app_wakes_on_background_thread_completion_and_renders_update() {
    #[derive(Clone, Default)]
    struct Gate {
        ready: Arc<AtomicBool>,
        waker: Arc<Mutex<Option<Waker>>>,
    }

    #[component]
    fn AsyncDesktopRoot() -> Element {
        let gate = dioxus_core::consume_context::<Gate>();
        let mut label = use_signal(|| "PENDING".to_owned());
        use_hook(move || {
            spawn(async move {
                core::future::poll_fn(|cx| {
                    if gate.ready.load(Ordering::Acquire) {
                        Poll::Ready(())
                    } else {
                        *gate.waker.lock().unwrap() = Some(cx.waker().clone());
                        if gate.ready.load(Ordering::Acquire) {
                            Poll::Ready(())
                        } else {
                            Poll::Pending
                        }
                    }
                })
                .await;
                label.set("RESOLVED_FROM_BG_THREAD".to_owned());
            });
        });
        rsx! { text { "{label}" } }
    }

    let gate = Gate::default();
    let surface = DioxusSurface::new(1, AsyncDesktopRoot).with_context(gate.clone());
    let mut app = DioxusDesktopApp::new(surface);

    let wake_hits = Arc::new(AtomicUsize::new(0));
    let hits_cb = Arc::clone(&wake_hits);
    app.install_wake_callback(Arc::new(move || {
        hits_cb.fetch_add(1, Ordering::SeqCst);
    }));

    let mut session = DesktopSession::new(
        app,
        1,
        Viewport {
            width: 320.0,
            height: 200.0,
            scale_factor: 1.0,
        },
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().unwrap();
    // Initial poll starts the spawned future and arms `DioxusDesktopApp`'s waker.
    while session.app_mut().poll_async_work() {
        session.render_and_rasterize().unwrap();
    }
    let before = wake_hits.load(Ordering::SeqCst);

    // Resolve the future on a separate OS thread with ZERO user input events!
    let bg_gate = gate.clone();
    std::thread::spawn(move || {
        bg_gate.ready.store(true, Ordering::Release);
        if let Some(w) = bg_gate.waker.lock().unwrap().take() {
            w.wake();
        }
    })
    .join()
    .unwrap();

    assert!(
        wake_hits.load(Ordering::SeqCst) > before,
        "resolving an async future on a background thread MUST fire DioxusDesktopApp's wake callback!"
    );
    assert!(
        session.app_mut().poll_async_work(),
        "poll_async_work must report dirty work after background completion"
    );
    let initial_bytes = session.last_frame_bytes().to_vec();
    session.render_and_rasterize().unwrap();
    assert_ne!(
        session.last_frame_bytes(),
        initial_bytes.as_slice(),
        "post-wake render must emit updated frame bytes"
    );
    assert!(session.app().surface().store().live_nodes().next().is_some());
    assert!(session.app_mut().surface_mut().store().live_nodes().next().is_some());
    assert!(!session.app_mut().poll_async_work());

    let host_surface = DioxusSurface::new(1, AsyncDesktopRoot).with_context(gate);
    let host = DesktopWindowHost::new_dioxus(host_surface, DesktopPreviewOptions::default())
        .expect("new_dioxus must construct primary session");
    assert!(host.windows.is_empty());
}
