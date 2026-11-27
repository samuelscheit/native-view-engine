//! **Aether Workspace** + **NVE Studio** — flagship local-first workspace, messaging, and
//! live self-editing meta-workbench built on `native-view-engine`.
//!
//! Exercises every single NVE primitive (`View`, `Text`, `Pressable`, `ScrollView`, `TextInput`,
//! `Image`, `SkiaCanvas`) and exports the modular `nve_get_interface` C ABI via
//! `nve_engine::export_nve_app!` with zero `unsafe`, full Tier-1 hot-patching, and Tier-2 `cdylib`
//! hot-swap state preservation (preserving every task, message, theme, X-Ray setting, `<Island>`
//! sandbox state, Live Sync telemetry, and deterministic time-travel snapshot).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod assets;
pub mod persist;
mod props;
pub mod routes;
pub mod sdk;
pub mod state;
pub mod theme;
pub mod traits;

/// Studio's web table plugins (`nve-web-html` + canvas), shared by every web host and prerenderer.
#[cfg(feature = "web")]
pub fn register_web_systems(registry: &mut nve_backend_web::WebTableRegistry) {
    nve_web_html_web::register_web_html_systems(registry);
    registry.register(nve_canvas_web::CanvasWebSystem);
}
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
    NveHostCallbacks, NveIntrinsicSize, NvePropsRef, NveSizeConstraints, NveValue, ROOT_NODE_ID,
    SurfaceId,
};
use nve_dioxus::DioxusSurface;
use nve_engine::{HotPatchMsg, NveApp};
use nve_system_env::{SystemEnvEvent, ThemeSnapshot};

pub use assets::{avatar_for_author, build_metrics_sparkline};
pub use persist::{
    DEFAULT_COMPACT_INTERVAL, PersistConfig, PersistError, PersistOutcome, PersistRecoveryStats,
    STATE_ARENA_FILE_NAME, STUDIO_PERSIST_ENV_VAR, STUDIO_SNAPSHOT_SCHEMA_VERSION,
    USER_FRAME0_FILE_NAME, WorkspaceStore,
};
pub use routes::{
    STUDIO_DEEP_LINK_HOST, STUDIO_DEEP_LINK_SCHEME, STUDIO_UNIVERSAL_LINK_HOST, StudioRoute,
    StudioRouter,
};
pub use sdk::{HostEventSyncReport, StudioHostServices, StudioSystemEnv};
pub use state::{
    AccentTheme, ActiveView, BoardFilter, CRDT_DIGEST_MAGIC, CRDT_DIGEST_MIN_LEN,
    CRDT_DIGEST_VERSION, ChannelKind, ChatMessage, CornerRadiusMode, CrdtMergeSummary,
    CrdtPeerNode, CrdtPeerTransport, CrdtPumpReport, CrdtSyncMetrics, CrdtTransportError,
    CrdtWireFrame, DensityMode, DocBlock, DocBlockKind, HOST_SURFACES, HostSurfaceSpec,
    InMemoryMeshHub, InMemoryMeshTransport, LayoutEngineMode, MAX_HISTORY, MAX_TELEMETRY_SAMPLES,
    MOBILE_BREAKPOINT_PT, MeshFaultConfig, Priority, Stream100kState, StudioConfig, StudioCrdtDoc,
    SyncTelemetryState, SyncTransportMode, TOPIC_CRDT_NVC1, TaskItem, TaskStatus,
    WORKSPACE_STATE_BIN_MAGIC, WORKSPACE_STATE_BIN_VERSION, WorkbenchPreset, WorkbenchState,
    WorkspaceDoc, WorkspaceSnapshot, WorkspaceState, default_seed_docs,
    version_vector_has_unseen_ops,
};
#[cfg(not(target_arch = "wasm32"))]
pub use state::{LoopbackTcpCrdtCluster, LoopbackTcpCrdtTransport};
pub use traits::{
    ResolvedStudioLayout, StudioAccentTrait, StudioCompactModeTrait, StudioTraitBox,
    StudioTraitPreviewChip, apply_studio_traits, build_studio_trait_store, resolve_studio_layout,
    sdk_color_scheme_to_trait, sdk_locale_to_trait, studio_accent_override,
    studio_compact_override, studio_density_to_trait,
};
pub use ui::StudioWorkspaceApp;
pub use ui_islands::{
    GestureSimMode, IslandCardState, IslandHostKind, IslandSandboxRuntime, IslandsSnapshot,
    TOPIC_WORKSPACE_SYNC,
};
pub use ui_palette::{
    CommandPaletteAction, CommandPaletteItem, CommandPaletteState, KeyDownEvent,
    build_palette_items, dispatch_keydown, filter_palette_items, fuzzy_score,
};
pub use virtual_list::{
    DEFAULT_OVERSCAN, DEFAULT_STREAM_ROW_COUNT, RowHeights, StreamVirtualEngine, VirtualRowMeta,
    VirtualSlice, stream_row_heights, uniform_row_height,
};

use props::{PropUpdates, apply_prop_updates};

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
        Ok(Self::assemble(
            surface_id,
            Arc::new(StudioHostServices::native()),
            Some(persist),
        ))
    }

    /// A module over `host_services` (e.g. [`StudioHostServices::new`] for deterministic headless
    /// runs), persisting where [`STUDIO_PERSIST_ENV_VAR`] points when it is set.
    pub fn with_services(surface_id: SurfaceId, host_services: Arc<StudioHostServices>) -> Self {
        let persist_store = WorkspaceStore::open_from_env().and_then(Result::ok);
        Self::assemble(surface_id, host_services, persist_store)
    }

    fn assemble(
        surface_id: SurfaceId,
        host_services: Arc<StudioHostServices>,
        persist_store: Option<WorkspaceStore>,
    ) -> Self {
        let initial = persist_store
            .as_ref()
            .map(|p| p.initial_state().clone())
            .unwrap_or_default();
        Self {
            surface_id,
            last_reported_layout_engine: initial.current.studio.layout_engine,
            last_persisted_frame_count: initial.frame_count,
            store: Arc::new(Mutex::new(initial)),
            host_services,
            surface: create_surface(surface_id),
            mounted: false,
            last_state_buf: Vec::new(),
            last_emitted_events: Vec::new(),
            persist_store,
        }
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
        self.persist_store
            .as_mut()
            .map(|store| store.save(&snapshot))
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

    /// Mutates the authoritative state. Once the surface is mounted this goes through its
    /// [`ui::store::StudioStore`], which republishes only the changed domains, so host events,
    /// props, and theme changes re-render just their subscribers and never rebuild the surface.
    fn update_state<R>(&mut self, f: impl FnOnce(&mut WorkspaceState) -> R) -> R {
        if self.mounted
            && let Some(store) = self
                .surface
                .in_root_scope(ui::store::StudioStore::from_root)
        {
            return self.surface.in_root_scope(|| store.update(f));
        }
        let mut guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// Direct zero-serialization access to the underlying [`DioxusSurface`].
    pub fn surface(&self) -> &DioxusSurface {
        &self.surface
    }

    pub fn surface_mut(&mut self) -> &mut DioxusSurface {
        &mut self.surface
    }

    /// Reconciles and commits the workspace directly into `backend` via [`nve_engine::StoreBackend`]
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
            ui::set_pending_store(Arc::clone(&self.store));
            ui::set_pending_host_services(Arc::clone(&self.host_services));
            self.mounted = true;
            let res = self.surface.commit_to_timed(frame_id, backend);
            ui::clear_pending_store();
            ui::clear_pending_host_services();
            res
        } else {
            self.surface.commit_to_timed(frame_id, backend)
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
        let changed = report.env_changed()
            && self.update_state(|state| {
                let mut next = state.system_env.clone();
                if let Some(theme) = report.updated_theme {
                    next.theme = theme;
                }
                if let Some(locale) = report.updated_locale {
                    next.locale = locale;
                }
                state.apply_system_env(next)
            });
        changed || report.processed_events > 0
    }

    /// Restores workspace state from binary `NWS1` bytes; returns `false` if corrupt or
    /// version-mismatched. The whole state is replaced, so the surface remounts from it.
    pub fn try_restore_state(&mut self, state: &[u8]) -> bool {
        let Some(restored) = WorkspaceState::from_bytes(state) else {
            return false;
        };
        *self.store.lock().unwrap_or_else(|e| e.into_inner()) = restored;
        self.surface = create_surface(self.surface_id);
        self.mounted = false;
        true
    }
}

impl NveApp for StudioWorkspaceModule {
    /// The native entry point: seeded from the OS appearance and locale until a host binds.
    fn create(surface_id: SurfaceId) -> Self {
        Self::with_services(surface_id, Arc::new(StudioHostServices::native()))
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        if !self.mounted {
            ui::set_pending_store(Arc::clone(&self.store));
            ui::set_pending_host_services(Arc::clone(&self.host_services));
            self.mounted = true;
            let slice = self.surface.render_nve2_slice(frame_id);
            ui::clear_pending_store();
            ui::clear_pending_host_services();
            slice
        } else {
            self.surface.render_nve2_slice(frame_id)
        }
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        let mut handled_host_event = false;
        for ev in events.iter() {
            handled_host_event |= match (ev.kind(), ev.payload_tag()) {
                (Some(NativeEventKind::Layout), 4)
                    if ev.node_id() == ROOT_NODE_ID && ev.width() > 0.0 =>
                {
                    self.update_state(|state| {
                        let prev_stream_h = state.current.stream.viewport_height;
                        let mask = state.update_viewport_with_mask(ev.width(), ev.height());
                        mask.any_breakpoint_changed()
                            || (state.current.active_view == ActiveView::Stream100k
                                && state.current.stream.viewport_height != prev_stream_h)
                    })
                }
                (Some(NativeEventKind::Scroll), 2) => self.update_state(|state| {
                    let streaming = state.current.active_view == ActiveView::Stream100k;
                    if streaming {
                        let y = ev.y();
                        state.set_stream_scroll_offset(if y.is_finite() && y >= 0.0 {
                            y as u32
                        } else {
                            0
                        });
                    }
                    streaming
                }),
                (Some(NativeEventKind::KeyDown), 6) if !ev.key().is_empty() => {
                    self.update_state(|state| {
                        dispatch_keydown(state, ev.key(), ev.has_primary_modifier())
                    })
                }
                (Some(NativeEventKind::Submit), 0 | 3) => self.update_state(submit_active_composer),
                _ => false,
            };
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
        let changed = self.update_state(|state| apply_prop_updates(state, upd));
        if changed {
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
        self.last_state_buf = self.store.lock().map(|g| g.to_bytes()).unwrap_or_default();
        &self.last_state_buf
    }

    fn restore_state(&mut self, state: &[u8]) {
        let _ = self.try_restore_state(state);
    }

    fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) {
        self.host_services.bind_host(callbacks);
        let services = Arc::clone(&self.host_services);
        self.update_state(|state| state.sync_system_env(&services));
    }

    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        if let Ok(theme) = ThemeSnapshot::from_repr_c_bytes(batch) {
            return self.dispatch_system_env_events(&[SystemEnvEvent::ThemeChanged(theme)]);
        }
        let report = self.host_services.drain_system_env_events();
        self.apply_env_report(report)
    }
}

/// Host `Submit` (Return key): runs the open palette's selection, else submits the active view's
/// composer. Returns `true` when something was submitted.
fn submit_active_composer(state: &mut WorkspaceState) -> bool {
    if state.palette.open {
        return state.execute_palette_selected();
    }
    match state.current.active_view {
        ActiveView::Board => state.submit_new_task_draft(),
        ActiveView::ChannelEngineering => state.submit_channel_draft(ChannelKind::Engineering),
        ActiveView::ChannelDesign => state.submit_channel_draft(ChannelKind::Design),
        ActiveView::Docs if state.current.doc_editing_block_id.is_some() => {
            state.commit_doc_block_edit()
        }
        ActiveView::Docs => state.submit_new_doc_block_draft(),
        _ => false,
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
