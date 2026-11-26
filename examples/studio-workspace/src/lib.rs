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
mod props;
pub mod routes;
pub mod sdk;
pub mod state;
pub mod theme;
pub mod traits;
pub mod ui;
mod ui_board;
mod ui_channels;
mod ui_chrome;
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
    FrameId, NativeEventKind, NveDirectEvent, NveEventBatchRef, NveGestureDisposition,
    NveHostCallbacks, NveIntrinsicSize, NvePropsRef, NveSizeConstraints, NveValue, SurfaceId,
    ROOT_NODE_ID,
};
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};
use nve_system_env::{SystemEnvEvent, ThemeSnapshot};

pub use assets::{avatar_for_author, build_metrics_sparkline};
pub use persist::{
    PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats, WorkspaceStore,
    DEFAULT_COMPACT_INTERVAL, STATE_ARENA_FILE_NAME, STUDIO_PERSIST_ENV_VAR,
    STUDIO_SNAPSHOT_SCHEMA_VERSION, USER_FRAME0_FILE_NAME,
};
pub use routes::{
    StudioRoute, StudioRouter, STUDIO_DEEP_LINK_HOST, STUDIO_DEEP_LINK_SCHEME,
    STUDIO_UNIVERSAL_LINK_HOST,
};
pub use sdk::{HostEventSyncReport, StudioHostServices, StudioSystemEnv};
pub use state::{
    default_seed_docs, version_vector_has_unseen_ops, AccentTheme, ActiveView, BoardFilter,
    ChannelKind, ChatMessage, CornerRadiusMode, CrdtMergeSummary, CrdtPeerNode, CrdtPeerTransport,
    CrdtPumpReport, CrdtSyncMetrics, CrdtTransportError, CrdtWireFrame, DensityMode, DocBlock,
    DocBlockKind, HostSurfaceSpec, InMemoryMeshHub, InMemoryMeshTransport, LayoutEngineMode,
    MeshFaultConfig, Priority, Stream100kState, StudioConfig, StudioCrdtDoc, SyncTelemetryState,
    SyncTransportMode, TaskItem, TaskStatus, WorkbenchPreset, WorkbenchState, WorkspaceDoc,
    WorkspaceSnapshot, WorkspaceState, CRDT_DIGEST_MAGIC, CRDT_DIGEST_MIN_LEN,
    CRDT_DIGEST_VERSION, HOST_SURFACES, MAX_HISTORY, MAX_TELEMETRY_SAMPLES, MOBILE_BREAKPOINT_PT,
    TOPIC_CRDT_NVC1, WORKSPACE_STATE_BIN_MAGIC, WORKSPACE_STATE_BIN_VERSION,
};
#[cfg(not(target_arch = "wasm32"))]
pub use state::{LoopbackTcpCrdtCluster, LoopbackTcpCrdtTransport};
pub use traits::{
    apply_studio_traits, build_studio_trait_store, resolve_studio_layout,
    sdk_color_scheme_to_trait, sdk_locale_to_trait, studio_accent_override,
    studio_compact_override, studio_density_to_trait, ResolvedStudioLayout, StudioAccentTrait,
    StudioCompactModeTrait, StudioTraitBox, StudioTraitPreviewChip,
};
pub use ui::StudioWorkspaceApp;
pub use ui_islands::{
    GestureSimMode, IslandCardState, IslandHostKind, IslandSandboxRuntime, IslandsSnapshot,
    TOPIC_WORKSPACE_SYNC,
};
pub use ui_palette::{
    build_palette_items, dispatch_keydown, filter_palette_items, fuzzy_score,
    CommandPaletteAction, CommandPaletteItem, CommandPaletteState, KeyDownEvent,
};
pub use virtual_list::{
    stream_row_heights, FenwickHeightTree, HeightsRevision, RowHeights, StreamVirtualEngine,
    VirtualRowMeta, VirtualSlice, DEFAULT_OVERSCAN, DEFAULT_STREAM_ROW_COUNT,
};

use props::{apply_prop_updates, PropUpdates};

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
    create_surface_with_services(surface_id, store, Arc::new(StudioHostServices::new()))
}

/// Creates a `DioxusSurface` bound to both a caller-supplied `WorkspaceState` store and
/// [`StudioHostServices`] (`nve-clipboard` + `nve-system-env` provider).
pub fn create_surface_with_services(
    surface_id: SurfaceId,
    store: Arc<Mutex<WorkspaceState>>,
    host_services: Arc<StudioHostServices>,
) -> DioxusSurface {
    ui::set_pending_store(store);
    ui::set_pending_host_services(host_services);
    let mut surface = DioxusSurface::new(surface_id, StudioWorkspaceApp);
    // Do not force render yet; caller invokes `render_frame(1)` which consumes pending slots.
    let _ = &mut surface;
    surface
}

/// Safe `NveApp` wrapper exported via `nve_get_interface` for native shells & Tier-2 hot-swap.
pub struct StudioWorkspaceModule {
    surface_id: SurfaceId,
    store: Arc<Mutex<WorkspaceState>>,
    host_services: Arc<StudioHostServices>,
    surface: DioxusSurface,
    mounted: bool,
    last_reported_layout_engine: LayoutEngineMode,
    last_state_buf: Vec<u8>,
    last_emitted_events: Vec<NveDirectEvent>,
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
            host_services: Arc::new(StudioHostServices::new()),
            surface: create_surface(surface_id),
            mounted: false,
            last_reported_layout_engine: layout_engine,
            last_state_buf: Vec::new(),
            last_emitted_events: Vec::new(),
            persist_store: Some(persist),
            last_persisted_frame_count: initial_frame_count,
        })
    }

    /// Returns the dual-mode host service handle (`FfiHostServices` + `MemoryHostServices`).
    pub fn host_services(&self) -> &Arc<StudioHostServices> {
        &self.host_services
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

    /// Direct zero-serialization access to the underlying [`DioxusSurface`].
    pub fn surface(&self) -> &DioxusSurface {
        &self.surface
    }

    /// Reconciles and flushes the workspace directly into `sink` via monomorphized [`nve_engine::RenderSink`]
    /// with zero `CommandBuffer` encode/decode overhead.
    pub fn flush_into<S: nve_engine::RenderSink>(
        &mut self,
        frame_id: FrameId,
        sink: &mut S,
    ) -> Result<nve_engine::BackendApplyStats, nve_engine::BackendError> {
        let (_, stats) = self.flush_into_timed(frame_id, sink)?;
        Ok(stats)
    }

    /// Timed variant of [`Self::flush_into`] returning `(reconcile_us, stats)`.
    pub fn flush_into_timed<S: nve_engine::RenderSink>(
        &mut self,
        frame_id: FrameId,
        sink: &mut S,
    ) -> Result<(u64, nve_engine::BackendApplyStats), nve_engine::BackendError> {
        if !self.mounted {
            ui::set_pending_store(Arc::clone(&self.store));
            ui::set_pending_host_services(Arc::clone(&self.host_services));
            self.mounted = true;
            let res = self.surface.flush_into_timed(frame_id, sink);
            ui::clear_pending_store();
            ui::clear_pending_host_services();
            res
        } else {
            self.surface.flush_into_timed(frame_id, sink)
        }
    }

    pub fn state_snapshot(&self) -> WorkspaceState {
        self.store
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    /// Applies direct typed [`SystemEnvEvent`]s into the studio store with zero byte serialization.
    pub fn dispatch_system_env_events(&mut self, events: &[SystemEnvEvent]) -> bool {
        let report = self.host_services.dispatch_system_env_events(events);
        self.apply_env_report(report)
    }

    fn apply_env_report(&mut self, report: HostEventSyncReport) -> bool {
        if report.env_changed() {
            let changed = self
                .store
                .lock()
                .map(|mut guard| {
                    let mut next = guard.system_env.clone();
                    if let Some(theme) = report.updated_theme {
                        next.theme = theme;
                    }
                    if let Some(locale) = report.updated_locale {
                        next.locale = locale;
                    }
                    guard.apply_system_env(next)
                })
                .unwrap_or(false);
            if changed {
                self.reset_surface_with_current_store();
                return true;
            }
        }
        report.processed_events > 0
    }

    /// Restores workspace state from binary `NWS1` bytes; returns `false` if corrupt or version-mismatched.
    pub fn try_restore_state(&mut self, state: &[u8]) -> bool {
        let Some(restored) = WorkspaceState::from_bytes(state) else {
            return false;
        };
        if let Ok(mut guard) = self.store.lock() {
            *guard = restored;
        }
        self.reset_surface_with_current_store();
        true
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
            host_services: Arc::new(StudioHostServices::new()),
            surface: create_surface(surface_id),
            mounted: false,
            last_reported_layout_engine: layout_engine,
            last_state_buf: Vec::new(),
            last_emitted_events: Vec::new(),
            persist_store,
            last_persisted_frame_count: initial_frame_count,
        }
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        if !self.mounted {
            ui::set_pending_store(Arc::clone(&self.store));
            ui::set_pending_host_services(Arc::clone(&self.host_services));
            self.mounted = true;
            let slice = self.surface.render_frame_slice(frame_id);
            ui::clear_pending_store();
            ui::clear_pending_host_services();
            slice
        } else {
            self.surface.render_frame_slice(frame_id)
        }
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        let mut handled_host_event = false;
        for ev in events.iter() {
            match (ev.kind(), ev.payload_tag()) {
                (Some(NativeEventKind::Layout), 4)
                    if ev.node_id() == ROOT_NODE_ID && ev.width() > 0.0 =>
                {
                    if let Ok(mut guard) = self.store.lock() {
                        let prev_stream_h = guard.current.stream.viewport_height;
                        let mask = guard.update_viewport_with_mask(ev.width(), ev.height());
                        let stream_h_changed = guard.current.active_view == ActiveView::Stream100k
                            && guard.current.stream.viewport_height != prev_stream_h;
                        if mask.any_breakpoint_changed() || stream_h_changed {
                            handled_host_event = true;
                        }
                    }
                }
                (Some(NativeEventKind::Scroll), 2) => {
                    if let Ok(mut guard) = self.store.lock()
                        && guard.current.active_view == ActiveView::Stream100k
                    {
                        let y = ev.y();
                        let target_y = if y.is_finite() && y >= 0.0 {
                            y as u32
                        } else {
                            0
                        };
                        guard.set_stream_scroll_offset(target_y);
                        handled_host_event = true;
                    }
                }
                (Some(NativeEventKind::KeyDown), 6) => {
                    if !ev.key().is_empty()
                        && let Ok(mut guard) = self.store.lock()
                        && dispatch_keydown(&mut guard, ev.key(), ev.has_primary_modifier())
                    {
                        handled_host_event = true;
                    }
                }
                (Some(NativeEventKind::Submit), 0 | 3) => {
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
        let handled_ui = self.surface.dispatch_direct_events(events);
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

    fn set_props(&mut self, props: NvePropsRef<'_>) -> bool {
        let upd = PropUpdates {
            studio_open: props.get("studioOpen").and_then(|v| v.as_bool()),
            accent: props.get("accent").and_then(|v| v.as_str()),
            layout_engine: props.get("layoutEngine").and_then(|v| v.as_str()),
            view: props.get("view").and_then(|v| v.as_str()),
            route: props.get("route").and_then(|v| v.as_str()),
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

    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
        let current_engine = self
            .store
            .lock()
            .map(|g| g.current.studio.layout_engine)
            .unwrap_or(self.last_reported_layout_engine);
        self.last_emitted_events.clear();
        if current_engine != self.last_reported_layout_engine {
            self.last_reported_layout_engine = current_engine;
            self.last_emitted_events.push(NveDirectEvent::custom(
                self.surface_id,
                ROOT_NODE_ID,
                0,
                "layout_engine_changed",
                NveValue::from_str(current_engine.as_str()),
            ));
        }
        &self.last_emitted_events
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        self.surface.apply_hot_patch(patch)
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
        let _ = self.try_restore_state(state);
    }

    fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
        self.host_services.bind_host(callbacks);
        let synced = self
            .store
            .lock()
            .map(|mut guard| guard.sync_system_env(&self.host_services))
            .unwrap_or(false);
        if synced {
            self.reset_surface_with_current_store();
        }
    }

    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        if let Ok(theme) = ThemeSnapshot::from_repr_c_bytes(batch) {
            return self.dispatch_system_env_events(&[SystemEnvEvent::ThemeChanged(theme)]);
        }
        let report = self.host_services.drain_system_env_events();
        self.apply_env_report(report)
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
