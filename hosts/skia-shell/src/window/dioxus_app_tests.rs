//! Tests for coalesced `SkiaHostServices::request_frame_wake` and direct `DioxusDesktopApp`
//! background-thread async task wakeup (`DioxusSurface::poll_work` -> `DesktopApp::poll_async_work`).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use nve_abi::{ColorRgba, SurfaceId, METHOD_WINDOW_REQUEST_FRAME, SERVICE_WINDOW};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, Viewport};
use nve_host_services::HostServiceProvider;

use super::{
    DesktopApp, DesktopHostError, DesktopPreviewOptions, DesktopSession, DesktopWindowHost,
    DioxusDesktopApp,
};
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
    let surface = DioxusSurface::new(SurfaceId::ROOT, AsyncDesktopRoot).with_context(gate.clone());
    let mut app = DioxusDesktopApp::new(surface);

    let wake_hits = Arc::new(AtomicUsize::new(0));
    let hits_cb = Arc::clone(&wake_hits);
    app.install_wake_callback(Arc::new(move || {
        hits_cb.fetch_add(1, Ordering::SeqCst);
    }));

    let mut session = DesktopSession::new(
        app,
        SurfaceId::ROOT,
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

    let host_surface = DioxusSurface::new(SurfaceId::ROOT, AsyncDesktopRoot).with_context(gate);
    let host = DesktopWindowHost::new_dioxus(host_surface, DesktopPreviewOptions::default())
        .expect("new_dioxus must construct primary session");
    assert!(host.windows.is_empty());
}

#[test]
fn dioxus_desktop_app_reload_returns_err_and_preserves_replica_and_nve2_sync() {
    use std::cell::RefCell;
    use std::rc::Rc;

    type SignalSlot = Rc<RefCell<Option<Signal<u32>>>>;

    #[component]
    fn CounterRoot() -> Element {
        let count = use_signal(|| 1u32);
        let bump = dioxus_core::consume_context::<SignalSlot>();
        use_hook(move || {
            *bump.borrow_mut() = Some(count);
        });
        rsx! {
            view {
                text { "COUNT:{count()}" }
            }
        }
    }

    let sig_slot: SignalSlot = Rc::new(RefCell::new(None));
    let surface = DioxusSurface::new(SurfaceId::ROOT, CounterRoot).with_context(Rc::clone(&sig_slot));
    let mut session = DesktopSession::new(
        DioxusDesktopApp::new(surface),
        SurfaceId::ROOT,
        Viewport {
            width: 320.0,
            height: 200.0,
            scale_factor: 1.0,
        },
        ColorRgba::BLACK,
        false,
    );
    session.render_and_rasterize().unwrap();
    let live_before = session.replica.store().live_nodes().count();
    let bytes_before = session.last_frame_bytes().to_vec();
    assert!(live_before > 1, "initial frame must populate replica nodes");
    assert!(!bytes_before.is_empty());

    let reload_err = session
        .handle_hot_patch(HotPatchMsg::DylibReload {
            generation: 2,
            dylib_path: "nonexistent_app.dylib".into(),
            build_ms: 12,
        })
        .expect_err("in-process DioxusDesktopApp must reject dylib reload");
    assert!(
        matches!(reload_err, DesktopHostError::Reload(_)),
        "expected DesktopHostError::Reload, got {reload_err:?}"
    );
    assert_eq!(
        session.replica.store().live_nodes().count(),
        live_before,
        "failed reload must preserve replica store instead of blanking it"
    );
    assert_eq!(
        session.last_frame_bytes(),
        bytes_before.as_slice(),
        "failed reload must preserve last_frame_bytes"
    );

    // Subsequent incremental NVE2 delta must still apply cleanly onto the preserved replica.
    let mut sig = sig_slot.borrow().expect("root signal installed");
    session.app_mut().surface_mut().in_root_scope(|| sig.set(2));
    session
        .render_and_rasterize()
        .expect("incremental NVE2 frame after rejected reload must remain synchronized");
    assert_eq!(session.replica.store().live_nodes().count(), live_before);
}
