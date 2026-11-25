//! **Aether Workspace** + **NVE Studio** — flagship local-first workspace, messaging, and
//! live self-editing meta-workbench built on `native-view-engine`.
//!
//! Exports the modular `nve_get_interface` C ABI via `nve_engine::export_nve_app!` with zero
//! `unsafe`, full Tier-1 hot-patching, and Tier-2 `cdylib` hot-swap state preservation
//! (preserving every task, message, theme, X-Ray setting, and deterministic time-travel snapshot).

pub mod state;
pub mod theme;
pub mod ui;

use std::sync::{Arc, Mutex};

use nve_abi::{
    decode_events, encode_events, FrameId, NveGestureDisposition, NveIntrinsicSize,
    NveSizeConstraints, SurfaceId,
};
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};

pub use state::{
    AccentTheme, ActiveView, BoardFilter, ChannelKind, ChatMessage, CornerRadiusMode, DensityMode,
    Priority, StudioConfig, TaskItem, TaskStatus, WorkspaceSnapshot, WorkspaceState, MAX_HISTORY,
};
pub use ui::StudioWorkspaceApp;

/// Creates a standalone `DioxusSurface` initialized with default `WorkspaceState`.
pub fn create_surface(surface_id: SurfaceId) -> DioxusSurface {
    DioxusSurface::new(surface_id, StudioWorkspaceApp)
}

/// Safe `NveApp` wrapper exported via `nve_get_interface` for native shells & Tier-2 hot-swap.
pub struct StudioWorkspaceModule {
    surface_id: SurfaceId,
    store: Arc<Mutex<WorkspaceState>>,
    surface: DioxusSurface,
    mounted: bool,
    last_frame_buf: Vec<u8>,
    last_state_buf: Vec<u8>,
    last_emitted_events_buf: Vec<u8>,
}

impl StudioWorkspaceModule {
    fn reset_surface_with_current_store(&mut self) {
        self.surface = create_surface(self.surface_id);
        self.mounted = false;
    }

    pub fn state_snapshot(&self) -> WorkspaceState {
        self.store
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }
}

impl NveApp for StudioWorkspaceModule {
    fn create(surface_id: SurfaceId) -> Self {
        let store = Arc::new(Mutex::new(WorkspaceState::default()));
        Self {
            surface_id,
            store,
            surface: create_surface(surface_id),
            mounted: false,
            last_frame_buf: Vec::new(),
            last_state_buf: Vec::new(),
            last_emitted_events_buf: Vec::new(),
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        if !self.mounted {
            ui::set_pending_store(Arc::clone(&self.store));
            let (_, _, bytes) = self.surface.render_frame(frame_id);
            ui::clear_pending_store();
            self.mounted = true;
            self.last_frame_buf = bytes;
        } else {
            let (_, _, bytes) = self.surface.render_frame(frame_id);
            self.last_frame_buf = bytes;
        }
        &self.last_frame_buf
    }

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        let Ok(decoded) = decode_events(encoded_events) else {
            return false;
        };
        self.surface.dispatch_events(&decoded)
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> NveIntrinsicSize {
        let w = constraints.max_w.clamp(constraints.min_w.max(640.0), 1280.0);
        let h = constraints.max_h.clamp(constraints.min_h.max(420.0), 840.0);
        NveIntrinsicSize {
            width: w,
            height: h,
        }
    }

    fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        NveGestureDisposition::Claim
    }

    fn set_props(&mut self, props_json: &[u8]) -> bool {
        let Ok(val) = serde_json::from_slice::<serde_json::Value>(props_json) else {
            return false;
        };
        let mut changed = false;
        if let Ok(mut guard) = self.store.lock() {
            if let Some(open) = val.get("studioOpen").and_then(|v| v.as_bool()) {
                if guard.current.studio.panel_open != open {
                    guard.toggle_studio_panel();
                    changed = true;
                }
            }
            if let Some(accent) = val.get("accent").and_then(|v| v.as_str()) {
                let parsed = match accent.to_ascii_lowercase().as_str() {
                    "cyan" => Some(AccentTheme::Cyan),
                    "violet" => Some(AccentTheme::Violet),
                    "green" => Some(AccentTheme::Green),
                    "amber" => Some(AccentTheme::Amber),
                    _ => None,
                };
                if let Some(theme) = parsed {
                    guard.set_accent(theme);
                    changed = true;
                }
            }
            if let Some(view) = val.get("view").and_then(|v| v.as_str()) {
                let parsed = match view.to_ascii_lowercase().as_str() {
                    "board" => Some(ActiveView::Board),
                    "engineering" => Some(ActiveView::ChannelEngineering),
                    "design" => Some(ActiveView::ChannelDesign),
                    "metrics" => Some(ActiveView::Metrics),
                    _ => None,
                };
                if let Some(v) = parsed {
                    guard.select_view(v);
                    changed = true;
                }
            }
        }
        if changed {
            self.reset_surface_with_current_store();
        }
        changed
    }

    fn drain_emitted_events(&mut self) -> &[u8] {
        self.last_emitted_events_buf = encode_events(&[]);
        &self.last_emitted_events_buf
    }

    fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
        let Ok(msg) = HotPatchMsg::from_json(patch_json) else {
            return false;
        };
        self.surface.apply_hot_patch(&msg)
    }

    fn snapshot_state(&mut self) -> &[u8] {
        self.last_state_buf = self
            .store
            .lock()
            .map(|g| g.to_json_bytes())
            .unwrap_or_default();
        &self.last_state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        if let Some(restored) = WorkspaceState::from_json_bytes(state) {
            if let Ok(mut guard) = self.store.lock() {
                *guard = restored;
            }
            self.reset_surface_with_current_store();
        }
    }
}

nve_engine::export_nve_app!(StudioWorkspaceModule);
