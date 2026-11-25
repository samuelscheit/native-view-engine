//! Flagship NVE Kitchen Sink Component Gallery authored in Dioxus `rsx!` + `nve-ui`.
//!
//! Showcases every `nve-ui` component and NVE engine primitive interactively with live Dark/Light
//! theming, `Compact`/`Comfortable`/`Spacious` `UiDensity`, live `nve-i18n` locale & RTL
//! switching (`en-US`, `de-DE`, `ar-EG`, `ja-JP`, `PseudoLocalizer`), `ReducedMotion`,
//! Canvas draw-list rendering, subtree `UiErrorBoundary` recovery, and versioned
//! `NVE_KITCHEN_SINK` zero-JSON binary state snapshots for 0ms web restore and Tier-2 hot-swapping.

mod i18n_copy;
mod sections;
mod shell;
pub mod state;

use std::sync::{Arc, Mutex};

use nve_abi::{
    decode_events, encode_events, FrameId, NveEventBatchRef, NveGestureDisposition,
    NveIntrinsicSize, NvePropsRef, NveSizeConstraints, SurfaceId,
};
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};

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
    last_frame_buf: Vec<u8>,
    last_state_buf: Vec<u8>,
    last_emitted_events_buf: Vec<u8>,
}

impl KitchenSinkModule {
    /// Returns a cloned snapshot of the current `KitchenSinkState`.
    pub fn state_snapshot(&self) -> KitchenSinkState {
        self.store
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Direct zero-serialization access to the underlying [`DioxusSurface`] so web/native hosts
    /// can synchronize responsive `@media`, `:hover`/`:active`/`:focus-visible`, cursor, and
    /// transition tables directly without wire encoding.
    pub fn surface(&self) -> &DioxusSurface {
        &self.surface
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
            *guard = restored;
        }
        self.surface = create_surface(self.surface_id);
        self.mounted = false;
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
            last_frame_buf: Vec::new(),
            last_state_buf,
            last_emitted_events_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let bytes = if !self.mounted {
            state::set_pending_store(Some(Arc::clone(&self.store)));
            self.mounted = true;
            let (_, _, bytes) = self.surface.render_frame(frame_id);
            state::set_pending_store(None);
            bytes
        } else {
            let (_, _, bytes) = self.surface.render_frame(frame_id);
            bytes
        };
        self.last_frame_buf = bytes;
        &self.last_frame_buf
    }

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        let Ok(decoded) = decode_events(encoded_events) else {
            return false;
        };
        self.surface.dispatch_events(&decoded)
    }

    fn dispatch_events_direct(&mut self, events: NveEventBatchRef<'_>) -> bool {
        self.surface.dispatch_events_direct(events)
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

    fn set_props_direct(&mut self, props: NvePropsRef<'_>) -> bool {
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

    fn drain_emitted_events(&mut self) -> &[u8] {
        self.last_emitted_events_buf = encode_events(&[]);
        &self.last_emitted_events_buf
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
