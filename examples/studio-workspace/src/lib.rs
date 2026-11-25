//! **Aether Workspace** + **NVE Studio** — flagship local-first workspace, messaging, and
//! live self-editing meta-workbench built on `native-view-engine`.
//!
//! Exports the modular `nve_get_interface` C ABI via `nve_engine::export_nve_app!` with zero
//! `unsafe`, full Tier-1 hot-patching, and Tier-2 `cdylib` hot-swap state preservation
//! (preserving every task, message, theme, X-Ray setting, and deterministic time-travel snapshot).

pub mod state;
pub mod theme;
pub mod ui;
pub mod ui_stream;
pub mod ui_studio;
pub mod virtual_list;

use std::sync::{Arc, Mutex};

use nve_abi::{
    decode_events, encode_events, EventPayload, FrameId, NativeEvent, NativeEventKind,
    NveGestureDisposition, NveIntrinsicSize, NveSizeConstraints, SurfaceId, ROOT_NODE_ID,
};
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};

pub use state::{
    AccentTheme, ActiveView, BoardFilter, ChannelKind, ChatMessage, CornerRadiusMode, DensityMode,
    LayoutEngineMode, Priority, Stream100kState, StudioConfig, TaskItem, TaskStatus,
    WorkspaceSnapshot, WorkspaceState, MAX_HISTORY, MOBILE_BREAKPOINT_PT,
};
pub use ui::StudioWorkspaceApp;
pub use virtual_list::{
    FenwickHeightTree, StreamVirtualEngine, VirtualListIndex, VirtualRowMeta, VirtualWindow,
    DEFAULT_OVERSCAN, DEFAULT_STREAM_ROW_COUNT,
};

/// Creates a standalone `DioxusSurface` initialized with default `WorkspaceState`.
pub fn create_surface(surface_id: SurfaceId) -> DioxusSurface {
    DioxusSurface::new(surface_id, StudioWorkspaceApp)
}

/// Creates a standalone `DioxusSurface` initialized for a specific viewport size (e.g. `393x852`
/// mobile portrait or `960x640` desktop) so responsive layout activates on Frame 1.
pub fn create_surface_for_viewport(
    surface_id: SurfaceId,
    width: f32,
    height: f32,
) -> DioxusSurface {
    let mut initial = WorkspaceState::default();
    initial.set_viewport_size(width, height);
    create_surface_with_store(surface_id, Arc::new(Mutex::new(initial)))
}

/// Creates a `DioxusSurface` bound to a caller-supplied shared `WorkspaceState` store.
pub fn create_surface_with_store(
    surface_id: SurfaceId,
    store: Arc<Mutex<WorkspaceState>>,
) -> DioxusSurface {
    ui::set_pending_store(store);
    let mut surface = DioxusSurface::new(surface_id, StudioWorkspaceApp);
    // Do not force render yet; caller invokes `render_frame(1)` which consumes `PENDING_STORE`.
    let _ = &mut surface;
    surface
}

/// Safe `NveApp` wrapper exported via `nve_get_interface` for native shells & Tier-2 hot-swap.
pub struct StudioWorkspaceModule {
    surface_id: SurfaceId,
    store: Arc<Mutex<WorkspaceState>>,
    surface: DioxusSurface,
    mounted: bool,
    last_reported_layout_engine: LayoutEngineMode,
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
            last_reported_layout_engine: LayoutEngineMode::Taffy,
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
        let mut handled_host_event = false;
        for ev in &decoded {
            match (ev.kind, &ev.payload) {
                (NativeEventKind::Layout, EventPayload::Layout { width, height, .. })
                    if ev.node_id == ROOT_NODE_ID && *width > 0.0 =>
                {
                    if let Ok(mut guard) = self.store.lock() {
                        if guard.set_viewport_size(*width, *height) {
                            handled_host_event = true;
                        }
                    }
                }
                (NativeEventKind::Scroll, EventPayload::Scroll { y, .. }) => {
                    if let Ok(mut guard) = self.store.lock() {
                        if guard.current.active_view == ActiveView::Stream100k {
                            let target_y = if y.is_finite() && *y >= 0.0 {
                                *y as u32
                            } else {
                                0
                            };
                            guard.set_stream_scroll_offset(target_y);
                            handled_host_event = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if handled_host_event {
            self.reset_surface_with_current_store();
        }
        let handled_ui = self.surface.dispatch_events(&decoded);
        handled_host_event || handled_ui
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> NveIntrinsicSize {
        let min_w = constraints.min_w.max(320.0).min(1280.0);
        let max_w = if constraints.max_w.is_finite() && constraints.max_w > 0.0 {
            constraints.max_w.min(1280.0)
        } else {
            960.0
        };
        let w = if max_w < min_w { max_w } else { max_w.clamp(min_w, 1280.0) };

        let min_h = constraints.min_h.max(240.0).min(840.0);
        let max_h = if constraints.max_h.is_finite() && constraints.max_h > 0.0 {
            constraints.max_h.min(840.0)
        } else {
            640.0
        };
        let h = if max_h < min_h { max_h } else { max_h.clamp(min_h, 840.0) };

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
            if let Some(engine) = val.get("layoutEngine").and_then(|v| v.as_str()) {
                let parsed = match engine.to_ascii_lowercase().as_str() {
                    "taffy" => Some(LayoutEngineMode::Taffy),
                    "yoga" => Some(LayoutEngineMode::Yoga),
                    _ => None,
                };
                if let Some(mode) = parsed {
                    guard.set_layout_engine(mode);
                    changed = true;
                }
            }
            if let Some(view) = val.get("view").and_then(|v| v.as_str()) {
                let parsed = match view.to_ascii_lowercase().as_str() {
                    "board" => Some(ActiveView::Board),
                    "engineering" => Some(ActiveView::ChannelEngineering),
                    "design" => Some(ActiveView::ChannelDesign),
                    "stream" | "stream100k" | "100k" => Some(ActiveView::Stream100k),
                    "metrics" => Some(ActiveView::Metrics),
                    _ => None,
                };
                if let Some(v) = parsed {
                    guard.select_view(v);
                    changed = true;
                }
            }
            let vp_w = val.get("viewportWidth").and_then(|v| v.as_f64());
            let vp_h = val.get("viewportHeight").and_then(|v| v.as_f64());
            if vp_w.is_some() || vp_h.is_some() {
                let next_w = vp_w.map(|v| v as f32).unwrap_or(guard.current.viewport_width as f32);
                let next_h = vp_h
                    .map(|v| v as f32)
                    .unwrap_or(guard.current.viewport_height as f32);
                if guard.set_viewport_size(next_w, next_h) {
                    changed = true;
                }
            }
            if let Some(scroll_y) = val.get("streamScrollY").and_then(|v| v.as_u64()) {
                guard.set_stream_scroll_offset(scroll_y.min(u32::MAX as u64) as u32);
                changed = true;
            }
        }
        if changed {
            self.reset_surface_with_current_store();
        }
        changed
    }

    fn drain_emitted_events(&mut self) -> &[u8] {
        let current_engine = self
            .store
            .lock()
            .map(|g| g.current.studio.layout_engine)
            .unwrap_or(self.last_reported_layout_engine);
        if current_engine != self.last_reported_layout_engine {
            self.last_reported_layout_engine = current_engine;
            let ev = NativeEvent {
                surface_id: self.surface_id,
                node_id: ROOT_NODE_ID,
                timestamp_us: 0,
                kind: NativeEventKind::Custom,
                payload: EventPayload::Custom {
                    name: "layout_engine_changed".into(),
                    data: current_engine.as_str().as_bytes().to_vec(),
                },
            };
            self.last_emitted_events_buf = encode_events(&[ev]);
        } else {
            self.last_emitted_events_buf = encode_events(&[]);
        }
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

#[cfg(all(feature = "cdylib-export", not(target_arch = "wasm32")))]
nve_engine::export_nve_app!(StudioWorkspaceModule);
