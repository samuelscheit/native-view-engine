//! **Aether Workspace** + **NVE Studio** — flagship local-first workspace, messaging, and
//! live self-editing meta-workbench built on `native-view-engine`.
//!
//! Exercises every single NVE primitive (`View`, `Text`, `Pressable`, `ScrollView`, `TextInput`,
//! `Image`, `SkiaCanvas`) and exports the modular `nve_get_interface` C ABI via
//! `nve_engine::export_nve_app!` with zero `unsafe`, full Tier-1 hot-patching, and Tier-2 `cdylib`
//! hot-swap state preservation (preserving every task, message, theme, X-Ray setting, `<Island>`
//! sandbox state, Live Sync telemetry, and deterministic time-travel snapshot).

pub mod assets;
pub mod persist;
pub mod state;
pub mod theme;
pub mod ui;
mod ui_board;
mod ui_channels;
pub mod ui_docs;
pub mod ui_islands;
mod ui_islands_view;
mod ui_metrics;
pub mod ui_palette;
pub mod ui_stream;
pub mod ui_studio;
pub mod ui_workbench;
pub mod virtual_list;

use std::path::Path;
use std::sync::{Arc, Mutex};

use nve_abi::{
    decode_events, encode_events, BinValueView, EventPayload, FrameId, NativeEvent,
    NativeEventKind, NveGestureDisposition, NveIntrinsicSize, NvePropsRef, NveSizeConstraints,
    SurfaceId, ROOT_NODE_ID,
};
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};

pub use assets::{avatar_for_author, build_metrics_sparkline};
pub use persist::{
    PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats, WorkspaceStore,
    DEFAULT_WAL_COMPACT_MAX_BYTES, DEFAULT_WAL_COMPACT_MAX_RECORDS, SNAPSHOT_FILE_NAME,
    STUDIO_PERSIST_ENV_VAR, STUDIO_SNAPSHOT_SCHEMA_VERSION, WAL_FILE_NAME,
};
pub use state::{
    default_seed_docs, AccentTheme, ActiveView, BoardFilter, ChannelKind, ChatMessage,
    CornerRadiusMode, DensityMode, DocBlock, DocBlockKind, HostSurfaceSpec, LayoutEngineMode,
    Priority, Stream100kState, StudioConfig, SyncTelemetryState, SyncTransportMode, TaskItem,
    TaskStatus, WorkbenchPreset, WorkbenchState, WorkspaceDoc, WorkspaceSnapshot, WorkspaceState,
    HOST_SURFACES, MAX_HISTORY, MAX_TELEMETRY_SAMPLES, MOBILE_BREAKPOINT_PT,
    WORKSPACE_STATE_BIN_MAGIC, WORKSPACE_STATE_BIN_VERSION,
};
pub use ui::StudioWorkspaceApp;
pub use ui_islands::{
    GestureSimMode, IslandCardState, IslandHostKind, IslandSandboxRuntime, IslandsSnapshot,
    TOPIC_WORKSPACE_SYNC,
};
pub use ui_palette::{
    build_palette_items, dispatch_keydown, filter_palette_items, fuzzy_score,
    parse_keydown_payload, CommandPaletteAction, CommandPaletteItem, CommandPaletteState,
    KeyDownEvent,
};
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

fn parse_accent(s: &str) -> Option<AccentTheme> {
    match s.to_ascii_lowercase().as_str() {
        "cyan" => Some(AccentTheme::Cyan),
        "violet" => Some(AccentTheme::Violet),
        "green" => Some(AccentTheme::Green),
        "amber" => Some(AccentTheme::Amber),
        _ => None,
    }
}

fn parse_layout_engine(s: &str) -> Option<LayoutEngineMode> {
    match s.to_ascii_lowercase().as_str() {
        "taffy" => Some(LayoutEngineMode::Taffy),
        "yoga" => Some(LayoutEngineMode::Yoga),
        _ => None,
    }
}

fn parse_active_view(s: &str) -> Option<ActiveView> {
    match s.to_ascii_lowercase().as_str() {
        "board" => Some(ActiveView::Board),
        "engineering" => Some(ActiveView::ChannelEngineering),
        "design" => Some(ActiveView::ChannelDesign),
        "docs" | "doc" => Some(ActiveView::Docs),
        "workbench" | "wb" | "preview" => Some(ActiveView::Workbench),
        "stream" | "stream100k" | "100k" => Some(ActiveView::Stream100k),
        "islands" | "island" => Some(ActiveView::Islands),
        "metrics" => Some(ActiveView::Metrics),
        _ => None,
    }
}

#[derive(Default)]
struct PropUpdates<'a> {
    studio_open: Option<bool>,
    accent: Option<&'a str>,
    layout_engine: Option<&'a str>,
    view: Option<&'a str>,
    viewport_width: Option<f64>,
    viewport_height: Option<f64>,
    stream_scroll_y: Option<u64>,
    sync_transport: Option<&'a str>,
    sync_peers: Option<u16>,
    sync_kbps: Option<f64>,
    sync_compression: Option<f64>,
    sync_rtt_ms: Option<f64>,
}

fn apply_prop_updates(guard: &mut WorkspaceState, upd: PropUpdates<'_>) -> bool {
    let mut changed = false;
    if let Some(open) = upd.studio_open
        && guard.current.studio.panel_open != open
    {
        guard.toggle_studio_panel();
        changed = true;
    }
    if let Some(accent) = upd.accent.and_then(parse_accent) {
        guard.set_accent(accent);
        changed = true;
    }
    if let Some(mode) = upd.layout_engine.and_then(parse_layout_engine) {
        guard.set_layout_engine(mode);
        changed = true;
    }
    if let Some(v) = upd.view.and_then(parse_active_view) {
        guard.select_view(v);
        changed = true;
    }
    if upd.viewport_width.is_some() || upd.viewport_height.is_some() {
        let next_w = upd
            .viewport_width
            .map(|v| v as f32)
            .unwrap_or(guard.current.viewport_width as f32);
        let next_h = upd
            .viewport_height
            .map(|v| v as f32)
            .unwrap_or(guard.current.viewport_height as f32);
        if guard.set_viewport_size(next_w, next_h) {
            changed = true;
        }
    }
    if let Some(scroll_y) = upd.stream_scroll_y {
        guard.set_stream_scroll_offset(scroll_y.min(u32::MAX as u64) as u32);
        changed = true;
    }
    if upd.sync_transport.is_some()
        || upd.sync_peers.is_some()
        || upd.sync_kbps.is_some()
        || upd.sync_compression.is_some()
        || upd.sync_rtt_ms.is_some()
    {
        let mut tel = guard.sync_telemetry.clone();
        if let Some(mode) = upd.sync_transport.and_then(SyncTransportMode::parse) {
            tel.transport = mode;
        }
        if let Some(peers) = upd.sync_peers {
            tel.connected_peers = peers;
        }
        if let Some(kbps) = upd.sync_kbps.filter(|v| v.is_finite() && *v >= 0.0) {
            tel.kbps_tenths = (kbps * 10.0).round() as u32;
        }
        if let Some(ratio) = upd.sync_compression.filter(|v| v.is_finite() && *v >= 0.0) {
            tel.compression_tenths = ((ratio * 10.0).round() as u32).min(u16::MAX as u32) as u16;
        }
        if let Some(rtt) = upd.sync_rtt_ms.filter(|v| v.is_finite() && *v >= 0.0) {
            tel.rtt_ms_tenths = (rtt * 10.0).round() as u32;
        }
        if guard.update_sync_telemetry(tel) {
            changed = true;
        }
    }
    changed
}

/// Safe `NveApp` wrapper exported via `nve_get_interface` for native shells & Tier-2 hot-swap.
pub struct StudioWorkspaceModule {
    surface_id: SurfaceId,
    store: Arc<Mutex<WorkspaceState>>,
    surface: DioxusSurface,
    mounted: bool,
    last_reported_layout_engine: LayoutEngineMode,
    last_state_buf: Vec<u8>,
    last_emitted_events_buf: Vec<u8>,
    persist_store: Option<WorkspaceStore>,
    last_persisted_frame_count: u64,
}

impl StudioWorkspaceModule {
    /// Creates a `StudioWorkspaceModule` backed by an explicit local persistence directory.
    pub fn create_with_persist_dir(
        surface_id: SurfaceId,
        dir: impl AsRef<Path>,
    ) -> Result<Self, PersistError> {
        Self::create_with_persist_config(surface_id, dir, PersistConfig::default())
    }

    /// Creates a `StudioWorkspaceModule` backed by an explicit local persistence directory and config.
    pub fn create_with_persist_config(
        surface_id: SurfaceId,
        dir: impl AsRef<Path>,
        config: PersistConfig,
    ) -> Result<Self, PersistError> {
        let persist = WorkspaceStore::open_with_config(dir, config)?;
        let initial = persist.initial_state().clone();
        let initial_frame_count = initial.frame_count;
        let layout_engine = initial.current.studio.layout_engine;
        Ok(Self {
            surface_id,
            store: Arc::new(Mutex::new(initial)),
            surface: create_surface(surface_id),
            mounted: false,
            last_reported_layout_engine: layout_engine,
            last_state_buf: Vec::new(),
            last_emitted_events_buf: Vec::new(),
            persist_store: Some(persist),
            last_persisted_frame_count: initial_frame_count,
        })
    }

    pub fn persist_store(&self) -> Option<&WorkspaceStore> {
        self.persist_store.as_ref()
    }

    pub fn persist_now(&mut self) -> Option<Result<PersistOutcome, PersistError>> {
        self.persist_store.as_ref()?;
        let snapshot = self.state_snapshot();
        self.last_persisted_frame_count = snapshot.frame_count;
        self.persist_store.as_mut().map(|store| store.save(&snapshot))
    }

    fn persist_if_dirty(&mut self) {
        if self.persist_store.is_none() {
            return;
        }
        let snapshot = self.state_snapshot();
        if snapshot.frame_count != self.last_persisted_frame_count
            && let Some(persist) = self.persist_store.as_mut()
            && persist.save(&snapshot).is_ok()
        {
            self.last_persisted_frame_count = snapshot.frame_count;
        }
    }

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

    /// Applies an already-decoded [`HotPatchMsg`] directly with zero serialization.
    pub fn apply_hot_patch_msg(&mut self, msg: &HotPatchMsg) -> bool {
        self.surface.apply_hot_patch(msg)
    }

    /// Decodes a wire hot-patch frame (`NVP1` binary or UTF-8 JSON) and queues it on the surface.
    pub fn apply_hot_patch_wire(&mut self, raw: &[u8]) -> bool {
        let Ok(msg) = HotPatchMsg::from_wire(raw) else {
            return false;
        };
        self.surface.apply_hot_patch(&msg)
    }
}

impl NveApp for StudioWorkspaceModule {
    fn create(surface_id: SurfaceId) -> Self {
        let persist_store = WorkspaceStore::open_from_env().and_then(Result::ok);
        let initial = persist_store
            .as_ref()
            .map(|p| p.initial_state().clone())
            .unwrap_or_default();
        let initial_frame_count = initial.frame_count;
        let layout_engine = initial.current.studio.layout_engine;
        let store = Arc::new(Mutex::new(initial));
        Self {
            surface_id,
            store,
            surface: create_surface(surface_id),
            mounted: false,
            last_reported_layout_engine: layout_engine,
            last_state_buf: Vec::new(),
            last_emitted_events_buf: Vec::new(),
            persist_store,
            last_persisted_frame_count: initial_frame_count,
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        if !self.mounted {
            ui::set_pending_store(Arc::clone(&self.store));
            self.mounted = true;
            let slice = self.surface.render_frame_slice(frame_id);
            ui::clear_pending_store();
            slice
        } else {
            self.surface.render_frame_slice(frame_id)
        }
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
                    if let Ok(mut guard) = self.store.lock()
                        && guard.set_viewport_size(*width, *height)
                    {
                        handled_host_event = true;
                    }
                }
                (NativeEventKind::Scroll, EventPayload::Scroll { y, .. }) => {
                    if let Ok(mut guard) = self.store.lock()
                        && guard.current.active_view == ActiveView::Stream100k
                    {
                        let target_y = if y.is_finite() && *y >= 0.0 {
                            *y as u32
                        } else {
                            0
                        };
                        guard.set_stream_scroll_offset(target_y);
                        handled_host_event = true;
                    }
                }
                (NativeEventKind::Custom, EventPayload::Custom { name, data })
                    if name == "keydown" =>
                {
                    if let Some(kd) = parse_keydown_payload(data)
                        && let Ok(mut guard) = self.store.lock()
                        && dispatch_keydown(&mut guard, &kd.key, kd.has_primary_modifier())
                    {
                        handled_host_event = true;
                    }
                }
                (NativeEventKind::Custom, EventPayload::Custom { name, .. })
                    if name == "submit" =>
                {
                    if let Ok(mut guard) = self.store.lock() {
                        let submitted = if guard.palette.open {
                            guard.execute_palette_selected()
                        } else {
                            match guard.current.active_view {
                                ActiveView::Board => guard.submit_new_task_draft(),
                                ActiveView::ChannelEngineering => {
                                    guard.submit_channel_draft(ChannelKind::Engineering)
                                }
                                ActiveView::ChannelDesign => {
                                    guard.submit_channel_draft(ChannelKind::Design)
                                }
                                ActiveView::Docs => {
                                    if guard.current.doc_editing_block_id.is_some() {
                                        guard.commit_doc_block_edit()
                                    } else {
                                        guard.submit_new_doc_block_draft()
                                    }
                                }
                                _ => false,
                            }
                        };
                        if submitted {
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
        if handled_host_event || handled_ui {
            self.persist_if_dirty();
        }
        handled_host_event || handled_ui
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> NveIntrinsicSize {
        let min_w = constraints.min_w.clamp(320.0, 1280.0);
        let max_w = if constraints.max_w.is_finite() && constraints.max_w > 0.0 {
            constraints.max_w.min(1280.0)
        } else {
            960.0
        };
        let w = if max_w < min_w {
            max_w
        } else {
            max_w.clamp(min_w, 1280.0)
        };

        let min_h = constraints.min_h.clamp(240.0, 840.0);
        let max_h = if constraints.max_h.is_finite() && constraints.max_h > 0.0 {
            constraints.max_h.min(840.0)
        } else {
            640.0
        };
        let h = if max_h < min_h {
            max_h
        } else {
            max_h.clamp(min_h, 840.0)
        };

        NveIntrinsicSize {
            width: w,
            height: h,
        }
    }

    fn hit_test(&mut self, _x: f32, _y: f32, _pointer_phase: u8) -> NveGestureDisposition {
        NveGestureDisposition::Claim
    }

    fn set_props_direct(&mut self, props: NvePropsRef<'_>) -> bool {
        let upd = PropUpdates {
            studio_open: props.get("studioOpen").and_then(|v| v.as_bool()),
            accent: props.get("accent").and_then(|v| v.as_str()),
            layout_engine: props.get("layoutEngine").and_then(|v| v.as_str()),
            view: props.get("view").and_then(|v| v.as_str()),
            viewport_width: props.get("viewportWidth").and_then(|v| v.as_number()),
            viewport_height: props.get("viewportHeight").and_then(|v| v.as_number()),
            stream_scroll_y: props
                .get("streamScrollY")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .filter(|&n| n >= 0)
                .map(|n| n as u64),
            sync_transport: props.get("syncTransport").and_then(|v| v.as_str()),
            sync_peers: props
                .get("syncPeers")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .and_then(|n| u16::try_from(n).ok()),
            sync_kbps: props.get("syncKbps").and_then(|v| v.as_number()),
            sync_compression: props.get("syncCompression").and_then(|v| v.as_number()),
            sync_rtt_ms: props.get("syncRttMs").and_then(|v| v.as_number()),
        };
        let changed = self
            .store
            .lock()
            .map(|mut guard| apply_prop_updates(&mut guard, upd))
            .unwrap_or(false);
        if changed {
            self.reset_surface_with_current_store();
            self.persist_if_dirty();
        }
        changed
    }

    fn set_props(&mut self, props_bytes: &[u8]) -> bool {
        // Fast path: zero-JSON binary `NveBinValue` map.
        if let Ok(view) = BinValueView::parse(props_bytes)
            && view.as_map().is_some()
        {
            let upd = PropUpdates {
                studio_open: view.get("studioOpen").and_then(|v| v.as_bool()),
                accent: view.get("accent").and_then(|v| v.as_str()),
                layout_engine: view.get("layoutEngine").and_then(|v| v.as_str()),
                view: view.get("view").and_then(|v| v.as_str()),
                viewport_width: view
                    .get("viewportWidth")
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64))),
                viewport_height: view
                    .get("viewportHeight")
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64))),
                stream_scroll_y: view
                    .get("streamScrollY")
                    .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                    .filter(|&n| n >= 0)
                    .map(|n| n as u64),
                sync_transport: view.get("syncTransport").and_then(|v| v.as_str()),
                sync_peers: view
                    .get("syncPeers")
                    .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                    .and_then(|n| u16::try_from(n).ok()),
                sync_kbps: view
                    .get("syncKbps")
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64))),
                sync_compression: view
                    .get("syncCompression")
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64))),
                sync_rtt_ms: view
                    .get("syncRttMs")
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64))),
            };
            let changed = self
                .store
                .lock()
                .map(|mut guard| apply_prop_updates(&mut guard, upd))
                .unwrap_or(false);
            if changed {
                self.reset_surface_with_current_store();
                self.persist_if_dirty();
            }
            return changed;
        }

        // Fallback for UTF-8 JSON props from external hosts / dynamic plugin tests.
        let Ok(val) = serde_json::from_slice::<serde_json::Value>(props_bytes) else {
            return false;
        };
        let upd = PropUpdates {
            studio_open: val.get("studioOpen").and_then(|v| v.as_bool()),
            accent: val.get("accent").and_then(|v| v.as_str()),
            layout_engine: val.get("layoutEngine").and_then(|v| v.as_str()),
            view: val.get("view").and_then(|v| v.as_str()),
            viewport_width: val.get("viewportWidth").and_then(|v| v.as_f64()),
            viewport_height: val.get("viewportHeight").and_then(|v| v.as_f64()),
            stream_scroll_y: val.get("streamScrollY").and_then(|v| v.as_u64()),
            sync_transport: val.get("syncTransport").and_then(|v| v.as_str()),
            sync_peers: val
                .get("syncPeers")
                .and_then(|v| v.as_u64())
                .and_then(|n| u16::try_from(n).ok()),
            sync_kbps: val.get("syncKbps").and_then(|v| v.as_f64()),
            sync_compression: val.get("syncCompression").and_then(|v| v.as_f64()),
            sync_rtt_ms: val.get("syncRttMs").and_then(|v| v.as_f64()),
        };
        let changed = self
            .store
            .lock()
            .map(|mut guard| apply_prop_updates(&mut guard, upd))
            .unwrap_or(false);
        if changed {
            self.reset_surface_with_current_store();
            self.persist_if_dirty();
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

    fn apply_hot_patch(&mut self, patch_wire: &str) -> bool {
        self.apply_hot_patch_wire(patch_wire.as_bytes())
    }

    fn snapshot_state(&mut self) -> &[u8] {
        self.last_state_buf = self
            .store
            .lock()
            .map(|g| g.to_bytes())
            .unwrap_or_default();
        &self.last_state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        if let Some(restored) = WorkspaceState::from_bytes(state) {
            if let Ok(mut guard) = self.store.lock() {
                *guard = restored;
            }
            self.reset_surface_with_current_store();
        }
    }
}

// Keep the C ABI export in its own private module so `rustc` places `nve_get_interface` in a
// dedicated CGU. That prevents MSVC `link.exe` LNK2005 duplicate-symbol clashes when workspace
// crates (such as `nve-web-shell` tests under `cargo test --workspace`) link multiple example
// `rlib`s simultaneously while `cdylib` builds still export `nve_get_interface`.
#[cfg(all(feature = "cdylib-export", not(target_arch = "wasm32")))]
mod cdylib_export {
    nve_engine::export_nve_app!(super::StudioWorkspaceModule);
}
