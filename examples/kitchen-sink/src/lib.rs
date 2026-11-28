//! Flagship NVE Kitchen Sink Component Gallery authored in Dioxus `rsx!` + `nve-ui`.
//!
//! Showcases every `nve-ui` component and NVE engine primitive interactively with live Dark/Light
//! theming, `Compact`/`Comfortable`/`Spacious` `UiDensity`, live `nve-i18n` locale & RTL
//! switching (`en-US`, `de-DE`, `ar-EG`, `ja-JP`, `PseudoLocalizer`), `ReducedMotion`,
//! Canvas draw-list rendering, subtree `UiErrorBoundary` recovery, and versioned
//! `NVE_KITCHEN_SINK` binary state snapshots for 0ms web restore and Tier-2 hot-swapping.

mod i18n_copy;
mod sections;
mod shell;
pub mod state;

/// The Kitchen Sink's web table plugins (`nve-web-html` + canvas), shared by every web host and
/// prerenderer.
#[cfg(feature = "web")]
pub fn register_web_systems(registry: &mut nve_backend_web::WebTableRegistry) {
    nve_web_html_web::register_web_html_systems(registry);
    registry.register(nve_canvas_web::CanvasWebSystem);
}

use std::sync::{Arc, Mutex};

use nve_abi::{
    ColorScheme, FrameId, NveGestureDisposition, NveHostCallbacks, NveIntrinsicSize, NvePropsRef,
    NveSizeConstraints, SurfaceId, ValidatedEvents, SERVICE_SYSTEM_ENV,
};
use nve_dioxus::prelude::{Signal, WritableExt};
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};
use nve_host_services::{FfiHostServices, HostChannelEvent, HostPushEvent};
use nve_system_env::{ThemeSnapshot, EVENT_THEME_CHANGED};
use nve_ui::Appearance;

pub use shell::KitchenSinkApp;
pub use state::{
    decode_kitchen_sink_state, encode_kitchen_sink_state, DensityPreset, GallerySection,
    KitchenSinkState, LocalePreset, SharedKitchenSinkStore, SpringPreset, ThemeMode,
    KITCHEN_SINK_STATE_MAGIC, KITCHEN_SINK_STATE_VERSION,
};

/// Creates a standalone `DioxusSurface` rendering `KitchenSinkApp` with default state.
pub fn create_surface(surface_id: SurfaceId) -> DioxusSurface {
    DioxusSurface::new(surface_id, KitchenSinkApp)
}

/// Creates a `DioxusSurface` bound to an explicit shared `KitchenSinkState` store.
pub fn create_surface_with_store(
    surface_id: SurfaceId,
    store: SharedKitchenSinkStore,
) -> DioxusSurface {
    state::set_pending_store(Some(store));
    let surface = DioxusSurface::new(surface_id, KitchenSinkApp);
    state::set_pending_store(None);
    surface
}

/// Host-agnostic `NveApp` implementation for `nve-example-kitchen-sink`.
pub struct KitchenSinkModule {
    surface_id: SurfaceId,
    store: SharedKitchenSinkStore,
    surface: DioxusSurface,
    mounted: bool,
    last_state_buf: Vec<u8>,
    host_services: FfiHostServices,
}

impl KitchenSinkModule {
    /// Returns a cloned snapshot of the current `KitchenSinkState`.
    pub fn state_snapshot(&self) -> KitchenSinkState {
        self.store
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Direct zero-serialization access to the underlying [`DioxusSurface`].
    pub fn surface(&self) -> &DioxusSurface {
        &self.surface
    }

    pub fn surface_mut(&mut self) -> &mut DioxusSurface {
        &mut self.surface
    }

    /// Reconciles and commits directly into `backend` via [`nve_engine::StoreBackend`]
    /// with zero wire encode/decode overhead.
    pub fn commit_to<B: nve_engine::StoreBackend>(
        &mut self,
        frame_id: FrameId,
        backend: &mut B,
    ) -> Result<nve_engine::BackendApplyStats, nve_engine::BackendError> {
        let (_, stats) = self.commit_to_timed(frame_id, backend)?;
        Ok(stats)
    }

    /// Timed variant of [`Self::commit_to`] returning `(reconcile_us, stats)`.
    pub fn commit_to_timed<B: nve_engine::StoreBackend>(
        &mut self,
        frame_id: FrameId,
        backend: &mut B,
    ) -> Result<(u64, nve_engine::BackendApplyStats), nve_engine::BackendError> {
        if !self.mounted {
            state::set_pending_store(Some(Arc::clone(&self.store)));
            self.mounted = true;
            let res = self.surface.commit_to_timed(frame_id, backend);
            state::set_pending_store(None);
            res
        } else {
            self.surface.commit_to_timed(frame_id, backend)
        }
    }

    /// Returns the `NVE_IFACE_HOST = 5` FFI host services facade bound to this module.
    #[inline]
    pub fn host_services(&self) -> &FfiHostServices {
        &self.host_services
    }

    /// Returns the interactive counter value inside the Overview gallery.
    #[inline]
    pub fn count(&self) -> i32 {
        self.store.lock().map(|g| g.counter).unwrap_or(0)
    }

    /// Attempts to restore Kitchen Sink state from binary bytes; returns `false` on corrupt data.
    pub fn try_restore_state(&mut self, bytes: &[u8]) -> bool {
        let Some(restored) = decode_kitchen_sink_state(bytes) else {
            return false;
        };
        if let Ok(mut guard) = self.store.lock() {
            // The OS appearance is host environment, not snapshot state: keep the live one.
            *guard = KitchenSinkState { os: guard.os, ..restored };
        }
        self.surface = create_surface(self.surface_id);
        self.mounted = false;
        true
    }

    /// Applies the host OS appearance (followed by the `Auto` theme mode) in place: the mounted
    /// tree's state signal is written through the root scope, so the surface is never rebuilt.
    pub fn set_os_appearance(&mut self, os: Appearance) -> bool {
        match self.store.lock() {
            Ok(mut guard) if guard.os != os => guard.os = os,
            _ => return false,
        }
        if self.mounted
            && let Some(mut live) =
                self.surface.in_root_scope(dioxus_core::try_consume_context::<Signal<KitchenSinkState>>)
        {
            live.write().os = os;
        }
        true
    }
}

impl NveApp for KitchenSinkModule {
    fn create(surface_id: SurfaceId) -> Self {
        let initial = KitchenSinkState::default();
        let last_state_buf = encode_kitchen_sink_state(&initial);
        Self {
            surface_id,
            store: Arc::new(Mutex::new(initial)),
            surface: create_surface(surface_id),
            mounted: false,
            last_state_buf,
            host_services: FfiHostServices::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        if !self.mounted {
            state::set_pending_store(Some(Arc::clone(&self.store)));
            self.mounted = true;
            let slice = self.surface.render_nve2_slice(frame_id);
            state::set_pending_store(None);
            slice
        } else {
            self.surface.render_nve2_slice(frame_id)
        }
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        self.surface.dispatch_events(events)
    }

    fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
        self.host_services.bind(callbacks);
    }

    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        let Ok(stats) = self.host_services.ingest_batch(batch) else {
            return false;
        };
        // Drain push and channel records consumed by this app so repeated host pushes never
        // accumulate unboundedly; apply canonical `SERVICE_SYSTEM_ENV` theme pushes in place.
        let pushes = self
            .host_services
            .typed_events()
            .drain_typed::<HostPushEvent>();
        let _ = self
            .host_services
            .typed_events()
            .drain_typed::<HostChannelEvent>();
        let mut appearance_changed = false;
        for ev in pushes {
            if ev.service_id == SERVICE_SYSTEM_ENV
                && ev.event_id == EVENT_THEME_CHANGED
                && let Ok(theme) = ThemeSnapshot::from_repr_c_bytes(&ev.payload)
            {
                appearance_changed |= self.set_os_appearance(Appearance {
                    scheme: theme.color_scheme.unwrap_or(ColorScheme::Dark),
                    contrast: theme.contrast,
                });
            }
        }
        appearance_changed || stats.non_cancelled_responses() > 0
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> NveIntrinsicSize {
        let w = constraints.max_w.clamp(constraints.min_w.max(360.0), 1280.0);
        let h = constraints.max_h.clamp(constraints.min_h.max(480.0), 900.0);
        NveIntrinsicSize {
            width: w,
            height: h,
        }
    }

    fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        NveGestureDisposition::Claim
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
        if let Some(c) = props
            .get("initialCount")
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        {
            if let Ok(mut guard) = self.store.lock() {
                guard.counter = c as i32;
            }
            self.surface = create_surface(self.surface_id);
            self.mounted = false;
            return true;
        }
        false
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        self.surface.apply_hot_patch(patch)
    }

    fn snapshot_state(&mut self) -> &[u8] {
        let snap = self.state_snapshot();
        self.last_state_buf = encode_kitchen_sink_state(&snap);
        &self.last_state_buf
    }

    fn restore_state(&mut self, bytes: &[u8]) {
        let _ = self.try_restore_state(bytes);
    }
}

// Keep the C ABI export in its own private module so `rustc` places `nve_get_interface` in a
// dedicated CGU. That prevents MSVC `link.exe` LNK2005 duplicate-symbol clashes when workspace
// crates (such as `nve-web-shell` tests under `cargo test --workspace`) link multiple example
// `rlib`s simultaneously while `cdylib` builds still export `nve_get_interface`.
#[cfg(all(feature = "cdylib-export", not(target_arch = "wasm32")))]
mod cdylib_export {
    nve_engine::export_nve_app!(super::KitchenSinkModule);
}
