//! Direct in-process [`DesktopApp`] adapter for [`DioxusSurface`] with coalesced async task
//! wakeup (`DioxusSurface::poll_work`) for native Skia desktop windows.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};

use nve_abi::{FrameId, NveEventBatchRef, SurfaceId};
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, Viewport};

use super::session::DesktopSession;
use super::window_host::DesktopWindowHost;
use super::{DesktopApp, DesktopHostError, DesktopPreviewOptions};

struct DioxusWakeBridge {
    pending: AtomicBool,
    sink: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl DioxusWakeBridge {
    fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            sink: Mutex::new(None),
        }
    }

    fn set_sink(&self, cb: Arc<dyn Fn() + Send + Sync>) {
        let fire_now = {
            let mut guard = self.sink.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(Arc::clone(&cb));
            self.pending.load(Ordering::Acquire)
        };
        if fire_now {
            cb();
        }
    }
}

impl Wake for DioxusWakeBridge {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            let sink = self.sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(cb) = sink {
                cb();
            }
        }
    }
}

/// Hosts an in-process [`DioxusSurface`] directly inside [`DesktopSession`] /
/// [`DesktopWindowHost`] with zero `cdylib` serialization and coalesced cross-thread async wakes.
pub struct DioxusDesktopApp {
    surface: DioxusSurface,
    wake: Arc<DioxusWakeBridge>,
    waker: Waker,
    generation: u64,
}

impl DioxusDesktopApp {
    pub fn new(surface: DioxusSurface) -> Self {
        let wake = Arc::new(DioxusWakeBridge::new());
        let waker = Waker::from(Arc::clone(&wake));
        Self {
            surface,
            wake,
            waker,
            generation: 1,
        }
    }

    pub fn surface(&self) -> &DioxusSurface {
        &self.surface
    }

    pub fn surface_mut(&mut self) -> &mut DioxusSurface {
        &mut self.surface
    }
}

impl DesktopApp for DioxusDesktopApp {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        self.surface.render_frame_slice(frame_id)
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        if events.is_empty() {
            return false;
        }
        self.surface.dispatch_direct_events(events)
    }

    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        let Ok(msg) = HotPatchMsg::from_bytes(patch_nvp1) else {
            return false;
        };
        if self.surface.apply_hot_patch(&msg) {
            self.generation = self.generation.saturating_add(1);
            true
        } else {
            false
        }
    }

    fn reload(
        &mut self,
        _new_dylib_path: &Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        Ok(self.generation)
    }

    fn install_wake_callback(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.wake.set_sink(wake);
    }

    fn poll_async_work(&mut self) -> bool {
        self.wake.pending.store(false, Ordering::Release);
        self.surface.poll_work(&self.waker)
    }
}

impl DesktopWindowHost<DioxusDesktopApp> {
    /// Creates a live [`DesktopWindowHost`] directly wrapping an in-process [`DioxusSurface`]
    /// with zero `cdylib` boundary and native async task wakeup via [`DesktopApp::poll_async_work`].
    pub fn new_dioxus(
        surface: DioxusSurface,
        options: DesktopPreviewOptions,
    ) -> Result<Self, DesktopHostError> {
        let scale = if options.scale_factor.is_finite() && options.scale_factor > 0.0 {
            options.scale_factor
        } else {
            1.0
        };
        let initial_viewport = Viewport {
            width: options.width,
            height: options.height,
            scale_factor: scale,
        };
        let mut session = DesktopSession::new(
            DioxusDesktopApp::new(surface),
            options.surface_id,
            initial_viewport,
            options.clear_color,
            options.show_hud,
        );
        let _ = session.notify_viewport_resize();
        session.render_and_rasterize()?;
        Ok(Self::from_primary_session(options, session))
    }
}
