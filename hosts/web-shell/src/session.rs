//! Cross-target Web shell session (`WebAppSession`), hot-reload dispatch (`HotReloadAction`,
//! feature `dev`), canonical 0ms snapshot capture, and zero-copy `NveSceneTable` mirroring
//! (feature `scene-view`).

#[cfg(any(feature = "app-studio", feature = "app-counter"))]
use nve_abi::encode_events;
use nve_abi::{
    EventPayload, FrameId, NativeEvent, NativeEventKind, SurfaceId, ROOT_NODE_ID,
};
#[cfg(feature = "scene-view")]
use crate::scene::SceneMirror;
use nve_backend_web::{encode_keydown_payload, WebBackend, WebSnapshotPayload};
use nve_dioxus::DioxusSurface;
#[cfg(feature = "dev")]
use nve_engine::HotPatchMsg;
#[cfg(any(feature = "app-studio", feature = "app-counter"))]
use nve_engine::NveApp;
use nve_engine::{fnv1a_64, BackendError, FrameTiming, RenderBackend, SurfaceRunner, Viewport};
#[cfg(feature = "app-counter")]
use nve_example_counter::{CounterModule, COUNTER_STATE_BIN_VERSION};
#[cfg(feature = "app-studio")]
use nve_studio_workspace::{StudioWorkspaceModule, STUDIO_SNAPSHOT_SCHEMA_VERSION};

#[cfg(all(test, feature = "app-studio", feature = "app-counter", feature = "dev", feature = "scene-view"))]
#[path = "tests.rs"]
mod tests;

/// Surface ID of the first (primary) app mounted into the page.
pub const DEFAULT_SURFACE_ID: SurfaceId = 1;

/// Outcome of handling one `nve dev` WebSocket `HotPatchMsg`.
#[cfg(feature = "dev")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotReloadAction {
    /// Tier-1 template/style patch applied live without reloading the page.
    TemplatePatched {
        generation: u64,
        template_id: String,
        mutations: u32,
    },
    /// Tier-2 dylib/WASM rebuild finished; browser shell should log and reload cleanly.
    ReloadPage {
        generation: u64,
        dylib_path: String,
        build_ms: u64,
    },
    /// Status heartbeat from `nve dev`.
    DevStatus { message: String, latency_ms: u64 },
}

enum SessionApp {
    Dioxus(Box<DioxusSurface>),
    #[cfg(feature = "app-studio")]
    Studio(Box<StudioWorkspaceModule>),
    #[cfg(feature = "app-counter")]
    Counter(Box<CounterModule>),
}

impl SessionApp {
    fn app_slug(&self) -> Option<&'static str> {
        match self {
            Self::Dioxus(_) => None,
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => Some("studio"),
            #[cfg(feature = "app-counter")]
            Self::Counter(_) => Some("counter"),
        }
    }

    fn schema_version(&self) -> u32 {
        match self {
            Self::Dioxus(_) => 1,
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => STUDIO_SNAPSHOT_SCHEMA_VERSION,
            #[cfg(feature = "app-counter")]
            Self::Counter(_) => COUNTER_STATE_BIN_VERSION,
        }
    }

    fn render_frame(&mut self, frame_id: FrameId) -> (u64, u64, Vec<u8>) {
        match self {
            Self::Dioxus(surface) => surface.render_frame(frame_id),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => {
                #[cfg(not(target_arch = "wasm32"))]
                let t0 = std::time::Instant::now();
                let bytes = module.render(frame_id).to_vec();
                #[cfg(not(target_arch = "wasm32"))]
                let elapsed = t0.elapsed().as_micros() as u64;
                #[cfg(target_arch = "wasm32")]
                let elapsed = 0u64;
                (elapsed, 0, bytes)
            }
            #[cfg(feature = "app-counter")]
            Self::Counter(module) => {
                #[cfg(not(target_arch = "wasm32"))]
                let t0 = std::time::Instant::now();
                let bytes = module.render(frame_id).to_vec();
                #[cfg(not(target_arch = "wasm32"))]
                let elapsed = t0.elapsed().as_micros() as u64;
                #[cfg(target_arch = "wasm32")]
                let elapsed = 0u64;
                (elapsed, 0, bytes)
            }
        }
    }

    fn dispatch_events(&mut self, events: &[NativeEvent]) -> bool {
        match self {
            Self::Dioxus(surface) => surface.dispatch_events(events),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.dispatch_events(&encode_events(events)),
            #[cfg(feature = "app-counter")]
            Self::Counter(module) => module.dispatch_events(&encode_events(events)),
        }
    }

    #[cfg(feature = "dev")]
    fn apply_hot_patch(&mut self, msg: &HotPatchMsg) -> bool {
        match self {
            Self::Dioxus(surface) => surface.apply_hot_patch(msg),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.apply_hot_patch(msg),
            #[cfg(feature = "app-counter")]
            Self::Counter(module) => module.apply_hot_patch(msg),
        }
    }

    fn snapshot_state(&mut self) -> Option<Vec<u8>> {
        match self {
            Self::Dioxus(_) => None,
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => Some(module.snapshot_state().to_vec()),
            #[cfg(feature = "app-counter")]
            Self::Counter(module) => Some(module.snapshot_state().to_vec()),
        }
    }

    fn try_restore_state(&mut self, bytes: &[u8]) -> bool {
        match self {
            Self::Dioxus(_) => false,
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.try_restore_state(bytes),
            #[cfg(feature = "app-counter")]
            Self::Counter(module) => module.try_restore_state(bytes),
        }
    }
}

/// Cross-target session binding an NVE app (`DioxusSurface`, `StudioWorkspaceModule`, or
/// `CounterModule`) to a `SurfaceRunner<WebBackend>` (plus, with `scene-view`, a zero-copy
/// [`SceneMirror`]).
///
/// Runs identically in browser WASM (`web-dom-wasm`) and in host unit tests (`web-dom-mirror`).
pub struct WebAppSession {
    app: SessionApp,
    runner: SurfaceRunner<WebBackend>,
    #[cfg(feature = "scene-view")]
    scene: SceneMirror,
    #[cfg(feature = "dev")]
    scene_frame_id: FrameId,
}

impl WebAppSession {
    pub fn new(
        surface_id: SurfaceId,
        surface: DioxusSurface,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        Self::with_app(surface_id, SessionApp::Dioxus(Box::new(surface)), backend, viewport)
    }

    fn with_app(
        surface_id: SurfaceId,
        app: SessionApp,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        Self {
            app,
            runner: SurfaceRunner::new(surface_id, viewport, backend),
            #[cfg(feature = "scene-view")]
            scene: SceneMirror::new(),
            #[cfg(feature = "dev")]
            scene_frame_id: 0,
        }
    }

    /// Boots the flagship `nve-studio-workspace` (Aether Workspace + NVE Studio) module, laid out
    /// for `viewport` before the first frame so responsive breakpoints apply to frame 0.
    #[cfg(feature = "app-studio")]
    pub fn new_studio_workspace(
        surface_id: SurfaceId,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        Self::new_studio_workspace_with_state(surface_id, backend, viewport, None).0
    }

    /// Boots `nve-studio-workspace` and optionally restores binary `NWS1` state bytes BEFORE
    /// applying the active viewport layout and rendering Frame 0. Returns `(session, restored)`.
    #[cfg(feature = "app-studio")]
    pub fn new_studio_workspace_with_state(
        surface_id: SurfaceId,
        backend: WebBackend,
        viewport: Viewport,
        state_bytes: Option<&[u8]>,
    ) -> (Self, bool) {
        let mut module = StudioWorkspaceModule::create(surface_id);
        let restored = state_bytes.is_some_and(|bytes| module.try_restore_state(bytes));
        if viewport.width > 0.0 && viewport.height > 0.0 {
            let layout_bytes = encode_events(&[layout_event(surface_id, viewport)]);
            let _ = module.dispatch_events(&layout_bytes);
        }
        (
            Self::with_app(surface_id, SessionApp::Studio(Box::new(module)), backend, viewport),
            restored,
        )
    }

    /// Boots the `nve-example-counter` module on `WebBackend`.
    #[cfg(feature = "app-counter")]
    pub fn new_counter(surface_id: SurfaceId, backend: WebBackend, viewport: Viewport) -> Self {
        Self::new_counter_with_state(surface_id, backend, viewport, None).0
    }

    /// Boots `nve-example-counter` and optionally restores binary `NWC1` state bytes BEFORE Frame 0.
    #[cfg(feature = "app-counter")]
    pub fn new_counter_with_state(
        surface_id: SurfaceId,
        backend: WebBackend,
        viewport: Viewport,
        state_bytes: Option<&[u8]>,
    ) -> (Self, bool) {
        let mut module = CounterModule::create(surface_id);
        let restored = state_bytes.is_some_and(|bytes| module.try_restore_state(bytes));
        (
            Self::with_app(surface_id, SessionApp::Counter(Box::new(module)), backend, viewport),
            restored,
        )
    }

    pub fn app_slug(&self) -> Option<&'static str> {
        self.app.app_slug()
    }

    pub fn schema_version(&self) -> u32 {
        self.app.schema_version()
    }

    pub fn backend(&self) -> &WebBackend {
        self.runner.backend()
    }

    pub fn backend_mut(&mut self) -> &mut WebBackend {
        self.runner.backend_mut()
    }

    /// Captures the app's binary state payload (`NWS1` / `NWC1`), if supported.
    pub fn snapshot_state(&mut self) -> Option<Vec<u8>> {
        self.app.snapshot_state()
    }

    /// Restores the app's binary state payload (`NWS1` / `NWC1`), returning `true` on success.
    pub fn try_restore_state(&mut self, bytes: &[u8]) -> bool {
        self.app.try_restore_state(bytes)
    }

    /// Computes a fast FNV-1a64 fingerprint of `(state_bytes, viewport, scroll_offsets)` so
    /// debounced & lifecycle snapshot hooks skip redundant work when state has not changed.
    pub fn snapshot_fingerprint(&mut self, scroll_offsets: &[(u32, u32)]) -> Option<(Vec<u8>, u64)> {
        let state_bytes = self.snapshot_state()?;
        let vp = self.runner.backend().viewport();
        let mut buf = Vec::with_capacity(state_bytes.len() + 12 + scroll_offsets.len() * 8);
        buf.extend_from_slice(&state_bytes);
        buf.extend_from_slice(&vp.width.to_bits().to_le_bytes());
        buf.extend_from_slice(&vp.height.to_bits().to_le_bytes());
        for &(idx, top) in scroll_offsets {
            buf.extend_from_slice(&idx.to_le_bytes());
            buf.extend_from_slice(&top.to_le_bytes());
        }
        let hash = fnv1a_64(&buf);
        Some((state_bytes, hash))
    }

    /// Captures binary state + a **canonical Frame-0 SSR snapshot** whose `data-nve-id`s,
    /// `data-nve-ssr-hash`, and `rule_hashes` match fresh restored Frame-0 WASM boot bit-for-bit
    /// (preventing `NodeId` drift after incremental Dioxus mutations and guaranteeing 0 hydration
    /// mismatches + exact pre-hydration click replay targets).
    pub fn capture_canonical_snapshot(
        &mut self,
        scroll_offsets: Vec<(u32, u32)>,
    ) -> Option<(Vec<u8>, WebSnapshotPayload)> {
        let state_bytes = self.snapshot_state()?;
        let payload = self.render_canonical_snapshot_for_state(&state_bytes, scroll_offsets)?;
        Some((state_bytes, payload))
    }

    /// Renders a canonical Frame-0 [`WebSnapshotPayload`] from pre-captured `state_bytes`.
    pub fn render_canonical_snapshot_for_state(
        &self,
        state_bytes: &[u8],
        scroll_offsets: Vec<(u32, u32)>,
    ) -> Option<WebSnapshotPayload> {
        let surface_id = self.runner.surface_id();
        let viewport = self.runner.backend().viewport();
        let schema_version = self.schema_version();
        let mut canonical = match &self.app {
            SessionApp::Dioxus(_) => return None,
            #[cfg(feature = "app-studio")]
            SessionApp::Studio(_) => {
                let (s, ok) = Self::new_studio_workspace_with_state(
                    surface_id,
                    WebBackend::new(),
                    viewport,
                    Some(state_bytes),
                );
                if !ok {
                    return None;
                }
                s
            }
            #[cfg(feature = "app-counter")]
            SessionApp::Counter(_) => {
                let (s, ok) = Self::new_counter_with_state(
                    surface_id,
                    WebBackend::new(),
                    viewport,
                    Some(state_bytes),
                );
                if !ok {
                    return None;
                }
                s
            }
        };
        canonical.render_initial().ok()?;
        let ssr = canonical.backend().render_ssr();
        Some(WebSnapshotPayload::from_ssr(&ssr, schema_version, scroll_offsets))
    }

    /// The zero-copy scene mirror of this session's `WebBackend` tree.
    #[cfg(feature = "scene-view")]
    pub fn scene(&self) -> &SceneMirror {
        &self.scene
    }

    /// Renders and mounts the initial frame.
    pub fn render_initial(&mut self) -> Result<FrameTiming, BackendError> {
        self.render_and_submit()
    }

    /// Updates `WebBackend`'s viewport and dispatches `NativeEventKind::Layout` on `ROOT_NODE_ID`,
    /// re-rendering immediately if the application changed responsive layout.
    pub fn resize_viewport(
        &mut self,
        viewport: Viewport,
    ) -> Result<Option<FrameTiming>, BackendError> {
        let surface_id = self.runner.surface_id();
        self.runner.backend_mut().set_viewport(surface_id, viewport);
        if !(viewport.width.is_finite()
            && viewport.height.is_finite()
            && viewport.width > 0.0
            && viewport.height > 0.0)
        {
            return Ok(None);
        }
        let timing = if self.app.dispatch_events(&[layout_event(surface_id, viewport)]) {
            Some(self.render_and_submit()?)
        } else {
            None
        };
        #[cfg(target_arch = "wasm32")]
        self.runner.backend().repaint_all_canvases();
        Ok(timing)
    }

    /// Dispatches a `keydown` custom event on `ROOT_NODE_ID` (e.g. `Cmd+K`/`Ctrl+K`, `ArrowUp`,
    /// `ArrowDown`, `Enter`, `Escape`) and commits an incremental frame if handled.
    pub fn dispatch_keydown(
        &mut self,
        key: &str,
        meta: bool,
        ctrl: bool,
    ) -> Result<Option<FrameTiming>, BackendError> {
        let surface_id = self.runner.surface_id();
        let data = encode_keydown_payload(key, meta, ctrl, false, false);
        let ev = NativeEvent {
            surface_id,
            node_id: ROOT_NODE_ID,
            timestamp_us: 0,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "keydown".into(),
                data,
            },
        };
        if !self.app.dispatch_events(&[ev]) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }

    /// Drains pending DOM events, dispatches them to the app, and commits an incremental frame if handled.
    pub fn pump_events(&mut self) -> Result<Option<FrameTiming>, BackendError> {
        let events = self.runner.backend_mut().drain_events();
        if events.is_empty() {
            return Ok(None);
        }
        if !self.app.dispatch_events(&events) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }

    /// Decodes and applies one binary `NVP1` hot-patch frame from `nve dev`.
    #[cfg(feature = "dev")]
    pub fn handle_hot_patch_frame(&mut self, raw: &[u8]) -> Result<HotReloadAction, String> {
        let msg = HotPatchMsg::from_bytes(raw).map_err(|e| format!("invalid NVP1 frame: {e}"))?;
        self.handle_hot_patch(msg)
    }

    /// Applies one decoded `HotPatchMsg` from `nve dev`.
    #[cfg(feature = "dev")]
    pub fn handle_hot_patch(&mut self, msg: HotPatchMsg) -> Result<HotReloadAction, String> {
        match msg {
            ref msg @ HotPatchMsg::TemplatePatch {
                generation,
                ref template_id,
                ..
            } => {
                let template_id = template_id.clone();
                let _ = self.app.apply_hot_patch(msg);
                let mutations = self.runner.backend_mut().apply_template_patch(msg);
                let next_frame = self.scene_frame_id.wrapping_add(1).max(1);
                self.sync_scene(next_frame);
                Ok(HotReloadAction::TemplatePatched {
                    generation,
                    template_id,
                    mutations,
                })
            }
            HotPatchMsg::DylibReload {
                generation,
                dylib_path,
                build_ms,
            } => Ok(HotReloadAction::ReloadPage {
                generation,
                dylib_path,
                build_ms,
            }),
            HotPatchMsg::DevStatus {
                message,
                latency_ms,
            } => Ok(HotReloadAction::DevStatus {
                message,
                latency_ms,
            }),
        }
    }

    #[cfg_attr(not(any(feature = "scene-view", feature = "dev")), allow(unused_variables))]
    fn sync_scene(&mut self, frame_id: FrameId) {
        #[cfg(feature = "dev")]
        {
            self.scene_frame_id = frame_id;
        }
        #[cfg(feature = "scene-view")]
        self.scene.sync(self.runner.backend(), frame_id);
    }

    fn render_and_submit(&mut self) -> Result<FrameTiming, BackendError> {
        let frame_id = self.runner.next_frame_id();
        let (reconcile_us, encode_us, bytes) = self.app.render_frame(frame_id);
        let timing = self
            .runner
            .submit_recorded_frame(frame_id, reconcile_us, encode_us, &bytes)?;
        self.sync_scene(frame_id);
        Ok(timing)
    }
}

fn layout_event(surface_id: SurfaceId, viewport: Viewport) -> NativeEvent {
    NativeEvent {
        surface_id,
        node_id: ROOT_NODE_ID,
        timestamp_us: 0,
        kind: NativeEventKind::Layout,
        payload: EventPayload::Layout {
            x: 0.0,
            y: 0.0,
            width: viewport.width,
            height: viewport.height,
        },
    }
}
