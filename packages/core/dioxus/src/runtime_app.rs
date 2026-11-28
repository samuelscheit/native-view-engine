//! Direct zero-serialization [`nve_engine::RuntimeApp`] adapter over [`DioxusSurface`] with
//! single-bridge coalesced async task wakeup via [`nve_engine::WakeBridge`].

use std::sync::Arc;
use std::task::Waker;

use nve_abi::{FrameId, ValidatedEvents};
use nve_engine::{AppFrame, CapabilityUnsupported, HotPatchMsg, RuntimeApp, WakeBridge, WakeToken};
use nve_store::NodeStore;

use crate::DioxusSurface;

/// Hosts an in-process [`DioxusSurface`] directly inside any `SurfaceRuntime` with zero `NVE2`
/// wire serialization and single-bridge coalesced cross-thread async task wakeups.
pub struct DioxusRuntimeApp {
    // Declared FIRST so dropping `DioxusRuntimeApp` disables zombie wakers BEFORE `surface` drops.
    _wake_token: WakeToken,
    waker: Waker,
    wake: Arc<WakeBridge>,
    surface: DioxusSurface,
    mounted: bool,
}

impl DioxusRuntimeApp {
    pub fn new(surface: DioxusSurface) -> Self {
        let wake = Arc::new(WakeBridge::new());
        let (waker, wake_token) = wake.detachable_waker();
        Self {
            _wake_token: wake_token,
            waker,
            wake,
            surface,
            mounted: false,
        }
    }

    #[inline]
    pub fn surface(&self) -> &DioxusSurface {
        &self.surface
    }

    #[inline]
    pub fn surface_mut(&mut self) -> &mut DioxusSurface {
        &mut self.surface
    }

    #[inline]
    pub fn wake_bridge(&self) -> &Arc<WakeBridge> {
        &self.wake
    }
}

impl RuntimeApp for DioxusRuntimeApp {
    fn render_frame(&mut self, frame_id: FrameId) -> AppFrame<'_> {
        let _ = self.surface.reconcile(frame_id);
        let first_mount = !self.mounted;
        self.mounted = true;
        if !first_mount && self.surface.last_changes().is_empty() {
            return AppFrame::Clean;
        }
        AppFrame::Direct {
            changes: self.surface.last_changes(),
        }
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        if events.is_empty() {
            return false;
        }
        self.surface.dispatch_events(events)
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(self.surface.store())
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(self.surface.store_mut())
    }

    fn poll_async_work(&mut self) -> bool {
        self.surface.poll_work(&self.waker)
    }

    fn install_wake_bridge(&mut self, wake: Arc<WakeBridge>) -> bool {
        let had_pending = self.wake.take_pending();
        let (waker, wake_token) = wake.detachable_waker();
        // Dropping the previous `_wake_token` immediately disables zombie wakers from prior apps.
        self._wake_token = wake_token;
        self.waker = waker;
        self.wake = wake;
        had_pending
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, CapabilityUnsupported> {
        Ok(self.surface.apply_hot_patch(patch))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Waker};

    use dioxus_core::Element;
    use nve_abi::{SurfaceId, NVE_RUNTIME_SCHEMA_HASH};
    use nve_engine::Viewport;
    use nve_runtime::{RuntimeBuilder, RuntimeError};

    use super::DioxusRuntimeApp;
    use crate::DioxusSurface;
    use crate::prelude::*;

    fn sample_app() -> Element {
        rsx! {
            view {
                style: "width: 240px; height: 96px; flex-direction: column;",
                text { "Zero-copy DioxusRuntimeApp" }
            }
        }
    }

    #[derive(Clone, Default)]
    struct OneShotState {
        inner: Arc<Mutex<(Option<String>, Option<Waker>)>>,
    }

    impl OneShotState {
        fn send(&self, value: &str) {
            let waker = {
                let mut g = self.inner.lock().expect("oneshot lock");
                g.0 = Some(value.to_owned());
                g.1.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        }
    }

    struct OneShotFut(OneShotState);

    impl Future for OneShotFut {
        type Output = String;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let mut g = self.0.inner.lock().expect("oneshot lock");
            if let Some(val) = g.0.take() {
                Poll::Ready(val)
            } else {
                g.1 = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    fn async_app() -> Element {
        let ch = consume_context::<OneShotState>();
        let mut label = use_signal(|| "loading".to_owned());
        dioxus_core::use_hook(move || {
            dioxus_core::spawn(async move {
                let val = OneShotFut(ch).await;
                label.set(val);
            });
        });
        rsx! {
            view {
                style: "width: 240px; height: 96px;",
                text { "{label}" }
            }
        }
    }

    fn read_text_slab(guard: &nve_runtime::SceneReadGuard<'_>) -> String {
        let view = guard.view();
        assert!(!view.text_slab.is_null());
        // SAFETY: `guard` holds the reader lock for `text_slab_len` bytes.
        let slab = unsafe {
            std::slice::from_raw_parts(view.text_slab, view.text_slab_len as usize)
        };
        std::str::from_utf8(slab).expect("UTF-8 text slab").to_owned()
    }

    #[test]
    fn dioxus_runtime_app_drives_surface_runtime_zero_copy() {
        let surface = DioxusSurface::new(SurfaceId::ROOT, sample_app);
        let mut rt = RuntimeBuilder::new()
            .surface_id(SurfaceId::ROOT)
            .viewport(Viewport {
                width: 400.0,
                height: 300.0,
                scale_factor: 1.0,
            })
            .app(Box::new(DioxusRuntimeApp::new(surface)))
            .build();

        assert!(rt.tick(1.0 / 60.0).expect("initial direct tick"));
        // Post-mount clean tick returns `false` (no redundant re-publish).
        assert!(!rt.tick(1.0 / 60.0).expect("clean second tick"));
        let guard = rt.acquire_scene().expect("acquire scene");
        let view = guard.view();
        assert_eq!(view.schema_hash, NVE_RUNTIME_SCHEMA_HASH);
        assert_eq!(view.frame_id, 1);
        assert!(view.node_capacity >= 3);
        assert!(!view.rows.is_null());
        assert!(!view.rects.is_null());
        assert!(view.text_slab_len > 0);
        assert!(read_text_slab(&guard).contains("Zero-copy DioxusRuntimeApp"));
    }

    #[test]
    fn dioxus_runtime_app_multi_tick_async_rerender_and_scene_busy_preserves_wakes() {
        let ch = OneShotState::default();
        let surface = DioxusSurface::new_with_context(SurfaceId::ROOT, async_app, ch.clone());
        let mut rt = RuntimeBuilder::new()
            .surface_id(SurfaceId::ROOT)
            .viewport(Viewport {
                width: 400.0,
                height: 300.0,
                scale_factor: 1.0,
            })
            .app(Box::new(DioxusRuntimeApp::new(surface)))
            .build();

        let wakes = Arc::new(AtomicUsize::new(0));
        let w = Arc::clone(&wakes);
        rt.set_wake_callback(Arc::new(move || {
            w.fetch_add(1, Ordering::Release);
        }));

        // Tick 1: mounts initial "loading" text AND post-render polls/arms the spawned future.
        assert!(rt.tick(1.0 / 60.0).expect("tick 1"));
        {
            let guard = rt.acquire_scene().expect("acquire 1");
            assert!(read_text_slab(&guard).contains("loading"));
            assert_eq!(guard.view().frame_id, 1);

            // Resolve the future on a background thread while the scene reader lock is held.
            let sender = ch.clone();
            std::thread::spawn(move || sender.send("async-resolved-42"))
                .join()
                .expect("worker thread");
            assert_eq!(
                wakes.load(Ordering::Acquire),
                1,
                "post-render waker must fire host callback from worker thread"
            );
            assert!(rt.wake_bridge().is_pending());

            // Ticking while `guard` is held returns `SceneBusy` and MUST NOT clear `wake` pending!
            assert_eq!(rt.tick_busy_probe(), Err(RuntimeError::SceneBusy));
            assert!(
                rt.wake_bridge().is_pending(),
                "SceneBusy tick must preserve pending wakeup for retry"
            );
        }

        // Tick 2 after releasing reader guard: consumes pending wake, updates signal, renders new text.
        assert!(rt.tick(1.0 / 60.0).expect("tick 2"));
        assert!(!rt.wake_bridge().is_pending());
        {
            let guard = rt.acquire_scene().expect("acquire 2");
            assert_eq!(guard.view().frame_id, 2);
            assert!(read_text_slab(&guard).contains("async-resolved-42"));
        }

        // Tick 3: steady-state clean tick returns false and keeps frame_id == 2.
        assert!(!rt.tick(1.0 / 60.0).expect("tick 3 clean"));
        assert_eq!(rt.acquire_scene().expect("acquire 3").view().frame_id, 2);
    }

    trait BusyProbeExt {
        fn tick_busy_probe(&self) -> Result<bool, RuntimeError>;
    }

    impl BusyProbeExt for nve_runtime::SurfaceRuntime {
        fn tick_busy_probe(&self) -> Result<bool, RuntimeError> {
            // Exercises the C ABI path (shared `&self`) while a `SceneReadGuard` is alive on stack.
            let api_ptr = nve_runtime::nve_runtime_get_interface(nve_abi::NVE_RUNTIME_ABI_VERSION);
            // SAFETY: Static vtable returned for matching ABI version.
            let api = unsafe { *api_ptr };
            let mut changed = 0u32;
            let handle = (self as *const nve_runtime::SurfaceRuntime)
                .cast_mut()
                .cast::<std::ffi::c_void>();
            // SAFETY: `self` is a live `SurfaceRuntime` on the owner thread.
            let rc = unsafe { (api.tick)(handle, 1.0 / 60.0, &mut changed) };
            match nve_abi::NveRuntimeStatus::from_i32(rc) {
                nve_abi::NveRuntimeStatus::Ok => Ok(changed != 0),
                nve_abi::NveRuntimeStatus::Busy => Err(RuntimeError::SceneBusy),
                _ => Err(RuntimeError::InvalidArg("unexpected status")),
            }
        }
    }
}
