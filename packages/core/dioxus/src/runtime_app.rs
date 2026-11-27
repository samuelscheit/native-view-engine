//! Direct zero-serialization [`nve_runtime::RuntimeApp`] adapter over [`DioxusSurface`] with
//! coalesced async task wakeup via [`nve_runtime::WakeBridge`].

use std::sync::Arc;
use std::task::Waker;

use nve_abi::{FrameId, NveEventBatchRef};
use nve_engine::HotPatchMsg;
use nve_runtime::{AppFrame, RuntimeApp, WakeBridge, WakeCallback};
use nve_store::NodeStore;

use crate::DioxusSurface;

/// Hosts an in-process [`DioxusSurface`] directly inside [`nve_runtime::SurfaceRuntime`] with
/// zero `NVE2` wire serialization and coalesced cross-thread async task wakeups.
pub struct DioxusRuntimeApp {
    surface: DioxusSurface,
    wake: Arc<WakeBridge>,
    waker: Waker,
    generation: u64,
}

impl DioxusRuntimeApp {
    pub fn new(surface: DioxusSurface) -> Self {
        let wake = Arc::new(WakeBridge::new());
        let waker = wake.waker();
        Self {
            surface,
            wake,
            waker,
            generation: 1,
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
    fn generation(&self) -> u64 {
        self.generation
    }

    fn render_frame(&mut self, frame_id: FrameId) -> AppFrame<'_> {
        let _ = self.surface.reconcile(frame_id);
        AppFrame::Direct {
            store: self.surface.store(),
            changes: self.surface.last_changes(),
        }
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        if events.is_empty() {
            return false;
        }
        self.surface.dispatch_direct_events(events)
    }

    fn direct_store(&self) -> Option<&NodeStore> {
        Some(self.surface.store())
    }

    fn direct_store_mut(&mut self) -> Option<&mut NodeStore> {
        Some(self.surface.store_mut())
    }

    fn poll_async_work(&mut self) -> bool {
        let _ = self.wake.take_pending();
        self.surface.poll_work(&self.waker)
    }

    fn install_wake(&mut self, wake: WakeCallback) {
        self.wake.set_sink(wake);
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        if self.surface.apply_hot_patch(patch) {
            self.generation = self.generation.saturating_add(1);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use dioxus_core::Element;
    use nve_abi::NVE_RUNTIME_SCHEMA_HASH;
    use nve_engine::Viewport;
    use nve_runtime::RuntimeBuilder;

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

    #[test]
    fn dioxus_runtime_app_drives_surface_runtime_zero_copy() {
        let surface = DioxusSurface::new(1, sample_app);
        let mut rt = RuntimeBuilder::new()
            .surface_id(1)
            .viewport(Viewport {
                width: 400.0,
                height: 300.0,
                scale_factor: 1.0,
            })
            .app(Box::new(DioxusRuntimeApp::new(surface)))
            .build();

        assert!(rt.tick(1.0 / 60.0).expect("initial direct tick"));
        let guard = rt.acquire_scene().expect("acquire scene");
        let view = guard.view();
        assert_eq!(view.schema_hash, NVE_RUNTIME_SCHEMA_HASH);
        assert_eq!(view.frame_id, 1);
        assert!(view.node_capacity >= 3);
        assert!(!view.rows.is_null());
        assert!(!view.rects.is_null());
        assert!(view.text_slab_len > 0);
        // SAFETY: `guard` holds the read lock and `view.text_slab` is valid for `text_slab_len`.
        let slab = unsafe {
            std::slice::from_raw_parts(view.text_slab, view.text_slab_len as usize)
        };
        let slab_str = std::str::from_utf8(slab).expect("UTF-8 text slab");
        assert!(slab_str.contains("Zero-copy DioxusRuntimeApp"));
    }
}
