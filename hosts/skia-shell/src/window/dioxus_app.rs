//! Direct in-process [`DesktopApp`] adapter delegating to [`nve_dioxus::DioxusRuntimeApp`] and
//! [`nve_runtime::WakeBridge`] for coalesced cross-thread async task wakeup in desktop windows.

use std::path::Path;
use std::sync::Arc;

use nve_abi::{FrameId, NveEventBatchRef, SurfaceId};
use nve_dioxus::{DioxusRuntimeApp, DioxusSurface};
use nve_engine::{HotPatchMsg, Viewport};
use nve_runtime::{RuntimeApp, WakeBridge};

use super::session::DesktopSession;
use super::window_host::DesktopWindowHost;
use super::{DesktopApp, DesktopHostError, DesktopPreviewOptions};

/// Hosts an in-process [`DioxusSurface`] inside [`DesktopSession`] / [`DesktopWindowHost`] via
/// [`DioxusRuntimeApp`] and the shared [`WakeBridge`] (eliminating host-local wake bridges).
pub struct DioxusDesktopApp {
    // Declared FIRST so dropping `DioxusDesktopApp` invalidates `WakeToken` before `wake` drops.
    app: DioxusRuntimeApp,
    wake: Arc<WakeBridge>,
    generation: u64,
}

impl DioxusDesktopApp {
    pub fn new(surface: DioxusSurface) -> Self {
        let wake = Arc::new(WakeBridge::new());
        let mut app = DioxusRuntimeApp::new(surface);
        if app.install_wake_bridge(Arc::clone(&wake)) {
            wake.trigger();
        }
        Self {
            app,
            wake,
            generation: 1,
        }
    }

    pub fn surface(&self) -> &DioxusSurface {
        self.app.surface()
    }

    pub fn surface_mut(&mut self) -> &mut DioxusSurface {
        self.app.surface_mut()
    }
}

impl Drop for DioxusDesktopApp {
    fn drop(&mut self) {
        self.wake.clear_sink();
    }
}

impl DesktopApp for DioxusDesktopApp {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        self.app.surface_mut().render_nve2_slice(frame_id)
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        self.app.dispatch_events(events)
    }

    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        let Ok(msg) = HotPatchMsg::from_bytes(patch_nvp1) else {
            return false;
        };
        if self.app.apply_hot_patch(&msg) {
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
        Err(DesktopHostError::Reload(
            "in-process DioxusDesktopApp does not support dylib reload".into(),
        ))
    }

    fn install_wake_callback(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.wake.set_sink(wake);
    }

    fn poll_async_work(&mut self) -> bool {
        let _ = self.wake.take_pending();
        self.app.poll_async_work()
    }
}

impl DesktopWindowHost<DioxusDesktopApp> {
    /// Creates a live [`DesktopWindowHost`] directly wrapping an in-process [`DioxusSurface`]
    /// with zero `cdylib` boundary and native async task wakeup via [`DioxusRuntimeApp`].
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
        let mut session = DesktopSession::new_with_layout(
            DioxusDesktopApp::new(surface),
            options.surface_id,
            initial_viewport,
            options.clear_color,
            options.show_hud,
            options.layout_engine,
        )?;
        let _ = session.notify_viewport_resize();
        session.render_and_rasterize()?;
        Ok(Self::from_primary_session(options, session))
    }
}
