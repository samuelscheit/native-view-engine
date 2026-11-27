//! Cross-target Web shell session (`WebAppSession`), hot-reload dispatch (`HotReloadAction`,
//! feature `dev`), canonical 0ms snapshot capture, and zero-copy `NveSceneTable` mirroring
//! (feature `scene-view`).

#[cfg(feature = "app-studio")]
use nve_abi::NveDirectEvent;
#[cfg(any(feature = "app-studio", feature = "app-kitchen-sink"))]
use nve_abi::{native_events_to_direct, NveEventBatchRef};
use nve_abi::{
    pack_key_modifiers, EventPayload, FrameId, NativeEvent, NativeEventKind, SurfaceId,
    ROOT_NODE_ID,
};
#[cfg(feature = "scene-view")]
use crate::scene::SceneMirror;
use nve_abi::xxh64;
use nve_backend_web::{SsrDocumentOptions, SsrOutput, WebBackend, WebSnapshotPayload};
#[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
use nve_backend_web::WebTableRegistry;
#[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
use nve_dioxus::DioxusSurface;
#[cfg(any(feature = "app-studio", feature = "app-kitchen-sink"))]
use nve_engine::NveApp;
use nve_engine::{BackendApplyStats, BackendError, FrameTiming, StoreBackend, Viewport};
#[cfg(feature = "app-kitchen-sink")]
use nve_example_kitchen_sink::{KitchenSinkModule, KITCHEN_SINK_STATE_VERSION};
#[cfg(feature = "scene-view")]
use nve_store::ChangeSet;
use nve_store::NodeStore;
#[cfg(feature = "app-studio")]
use nve_studio_workspace::{StudioWorkspaceModule, STUDIO_SNAPSHOT_SCHEMA_VERSION};

/// Surface ID of the first (primary) app mounted into the page.
pub const DEFAULT_SURFACE_ID: SurfaceId = 1;

#[cfg(feature = "app-studio")]
fn dispatch_studio_viewport_layout(
    surface_id: SurfaceId,
    module: &mut StudioWorkspaceModule,
    viewport: Viewport,
) {
    if viewport.width > 0.0 && viewport.height > 0.0 {
        let layout_ev = [NveDirectEvent::layout(
            surface_id,
            ROOT_NODE_ID,
            0,
            0.0,
            0.0,
            viewport.width,
            viewport.height,
        )];
        // SAFETY: `layout_ev` is a stack-local slice containing scalar geometry and no pointers.
        let _ = module.dispatch_events(unsafe { NveEventBatchRef::from_slice(&layout_ev) });
    }
}

#[cfg(feature = "dev")]
#[path = "session_hot_reload.rs"]
mod hot_reload;
#[cfg(feature = "dev")]
pub use hot_reload::HotReloadAction;

/// Result of one bounded [`WebAppSession::pump_async_work_with`] tick.
#[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
#[derive(Debug, Clone, Copy)]
pub struct AsyncPumpOutcome {
    /// Timing of the last rendered frame during this pump tick, if any work rendered.
    pub timing: Option<FrameTiming>,
    /// `true` when the per-tick pump cap (`MAX_PUMPS`) was reached while async work may still be
    /// pending; browser hosts must yield to paint and continue via a macrotask (`setTimeout(0)`).
    pub hit_cap: bool,
}

enum SessionApp {
    /// A Dioxus surface plus its app family's web plugin registration.
    #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
    Dioxus(Box<DioxusSurface>, fn(&mut WebTableRegistry)),
    #[cfg(feature = "app-studio")]
    Studio(Box<StudioWorkspaceModule>),
    #[cfg(feature = "app-kitchen-sink")]
    KitchenSink(Box<KitchenSinkModule>),
}

impl SessionApp {
    /// Registers the hosted app family's shared web plugin set (the same function its SSR server
    /// or prerenderer calls), so client and server resolve identical tags.
    fn register_web_systems(&self, backend: &mut WebBackend) {
        let registry = backend.table_registry_mut();
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(_, register) => register(registry),
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => nve_studio_workspace::register_web_systems(registry),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(_) => nve_example_kitchen_sink::register_web_systems(registry),
        }
    }

    fn app_slug(&self) -> Option<&'static str> {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => None,
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => Some("studio"),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(_) => Some("kitchen-sink"),
        }
    }

    fn schema_version(&self) -> u32 {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => 1,
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => STUDIO_SNAPSHOT_SCHEMA_VERSION,
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(_) => KITCHEN_SINK_STATE_VERSION,
        }
    }

    fn commit_to_timed<B: StoreBackend>(
        &mut self,
        frame_id: FrameId,
        backend: &mut B,
    ) -> Result<(u64, BackendApplyStats), BackendError> {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.commit_to_timed(frame_id, backend),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.commit_to_timed(frame_id, backend),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.commit_to_timed(frame_id, backend),
        }
    }

    fn store(&self) -> &NodeStore {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.store(),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.surface().store(),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.surface().store(),
        }
    }

    #[cfg(feature = "dev")]
    fn store_mut(&mut self) -> &mut NodeStore {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.store_mut(),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.surface_mut().store_mut(),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.surface_mut().store_mut(),
        }
    }

    #[cfg(feature = "scene-view")]
    fn last_changes(&self) -> &ChangeSet {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.last_changes(),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.surface().last_changes(),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.surface().last_changes(),
        }
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
    fn poll_work(&mut self, waker: &core::task::Waker) -> bool {
        match self {
            Self::Dioxus(surface, _) => surface.poll_work(waker),
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => false,
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(_) => false,
        }
    }

    fn dispatch_events(&mut self, events: &[NativeEvent]) -> bool {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.dispatch_events(events),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => {
                let mut direct = Vec::with_capacity(events.len());
                native_events_to_direct(events, &mut direct);
                // SAFETY: `direct` borrows directly from `events`, which outlives this call.
                module.dispatch_events(unsafe { NveEventBatchRef::from_slice(&direct) })
            }
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => {
                let mut direct = Vec::with_capacity(events.len());
                native_events_to_direct(events, &mut direct);
                // SAFETY: `direct` borrows directly from `events`, which outlives this call.
                module.dispatch_events(unsafe { NveEventBatchRef::from_slice(&direct) })
            }
        }
    }

    /// Forwards an `NVE_IFACE_HOST` host-event batch (e.g. the OS `ThemeSnapshot`) to the app.
    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(_) => {
                let _ = batch;
                false
            }
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.dispatch_host_events(batch),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.dispatch_host_events(batch),
        }
    }

    fn snapshot_state(&mut self) -> Option<Vec<u8>> {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => None,
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => Some(module.snapshot_state().to_vec()),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => Some(module.snapshot_state().to_vec()),
        }
    }

    #[cfg_attr(not(any(feature = "app-studio", feature = "app-kitchen-sink")), allow(unused_variables))]
    fn try_restore_state(&mut self, bytes: &[u8]) -> bool {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => false,
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.try_restore_state(bytes),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.try_restore_state(bytes),
        }
    }
}

/// Cross-target session binding an NVE app (`DioxusSurface`, `StudioWorkspaceModule`, or
/// `KitchenSinkModule`) directly to a [`WebBackend`] (plus, with `scene-view`, a zero-copy
/// [`SceneMirror`]).
///
/// Runs identically in browser WASM (`web-dom-wasm`) and in host unit tests (`web-dom-mirror`).
pub struct WebAppSession {
    surface_id: SurfaceId,
    frame_id: FrameId,
    app: SessionApp,
    backend: WebBackend,
    #[cfg(feature = "scene-view")]
    scene: SceneMirror,
}

impl WebAppSession {
    /// Hosts a Dioxus `surface`; `register` installs its app family's web plugins (the same
    /// function its SSR server calls), so client and server resolve identical tags.
    #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
    pub fn new(
        surface_id: SurfaceId,
        surface: DioxusSurface,
        register: fn(&mut WebTableRegistry),
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        let app = SessionApp::Dioxus(Box::new(surface), register);
        Self::with_app(surface_id, app, backend, viewport)
    }

    fn with_app(
        surface_id: SurfaceId,
        app: SessionApp,
        mut backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        app.register_web_systems(&mut backend);
        backend.set_viewport(surface_id, viewport);
        Self {
            surface_id,
            frame_id: 0,
            app,
            backend,
            #[cfg(feature = "scene-view")]
            scene: SceneMirror::new(),
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
        // Deterministic defaults: the shell delivers the OS appearance right after Frame 0.
        let services = std::sync::Arc::new(nve_studio_workspace::StudioHostServices::new());
        let mut module = StudioWorkspaceModule::with_services(surface_id, services);
        let restored = state_bytes.is_some_and(|bytes| module.try_restore_state(bytes));
        dispatch_studio_viewport_layout(surface_id, &mut module, viewport);
        (
            Self::with_app(surface_id, SessionApp::Studio(Box::new(module)), backend, viewport),
            restored,
        )
    }

    /// Boots the `nve-example-kitchen-sink` module on `WebBackend`.
    #[cfg(feature = "app-kitchen-sink")]
    pub fn new_kitchen_sink(surface_id: SurfaceId, backend: WebBackend, viewport: Viewport) -> Self {
        Self::new_kitchen_sink_with_state(surface_id, backend, viewport, None).0
    }

    /// Boots `nve-example-kitchen-sink` and optionally restores binary state bytes BEFORE Frame 0.
    #[cfg(feature = "app-kitchen-sink")]
    pub fn new_kitchen_sink_with_state(
        surface_id: SurfaceId,
        backend: WebBackend,
        viewport: Viewport,
        state_bytes: Option<&[u8]>,
    ) -> (Self, bool) {
        let mut module = KitchenSinkModule::create(surface_id);
        let restored = state_bytes.is_some_and(|bytes| module.try_restore_state(bytes));
        (
            Self::with_app(
                surface_id,
                SessionApp::KitchenSink(Box::new(module)),
                backend,
                viewport,
            ),
            restored,
        )
    }

    pub fn app_slug(&self) -> Option<&'static str> {
        self.app.app_slug()
    }

    pub fn schema_version(&self) -> u32 {
        self.app.schema_version()
    }

    /// Applies the active backend viewport layout event before Frame 0 for apps that consume it.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn apply_initial_viewport(&mut self) {
        match &mut self.app {
            #[cfg(feature = "app-studio")]
            SessionApp::Studio(module) => {
                dispatch_studio_viewport_layout(self.surface_id, module, self.backend.viewport());
            }
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            SessionApp::Dioxus(..) => {}
            #[cfg(feature = "app-kitchen-sink")]
            SessionApp::KitchenSink(_) => {}
        }
    }

    /// Replaces the pre-frame backend and installs the live browser viewport once hydration vs.
    /// clean mount has been decided.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn replace_backend_with_viewport(&mut self, mut backend: WebBackend, viewport: Viewport) {
        self.app.register_web_systems(&mut backend);
        backend.set_viewport(self.surface_id, viewport);
        self.backend = backend;
    }

    pub fn backend(&self) -> &WebBackend {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut WebBackend {
        &mut self.backend
    }

    pub fn store(&self) -> &NodeStore {
        self.app.store()
    }

    pub fn node_count(&self) -> usize {
        self.app.store().live_nodes().count()
    }

    pub fn render_html(&self) -> String {
        self.backend.render_html(self.app.store())
    }

    pub fn render_ssr(&self) -> SsrOutput {
        self.backend.render_ssr(self.app.store())
    }

    pub fn render_ssr_document(&self, opts: &SsrDocumentOptions) -> String {
        self.backend.render_ssr_document(self.app.store(), opts)
    }

    /// Captures the app's binary state payload, if supported.
    pub fn snapshot_state(&mut self) -> Option<Vec<u8>> {
        self.app.snapshot_state()
    }

    /// Restores the app's binary state payload, returning `true` on success.
    pub fn try_restore_state(&mut self, bytes: &[u8]) -> bool {
        self.app.try_restore_state(bytes)
    }

    /// Computes a fast XXH64 fingerprint of `(state_bytes, viewport, scroll_offsets)` so
    /// debounced & lifecycle snapshot hooks skip redundant work when state has not changed.
    pub fn snapshot_fingerprint(&mut self, scroll_offsets: &[(u32, u32)]) -> Option<(Vec<u8>, u64)> {
        let state_bytes = self.snapshot_state()?;
        let vp = self.backend.viewport();
        let mut buf = Vec::with_capacity(state_bytes.len() + 12 + scroll_offsets.len() * 8);
        buf.extend_from_slice(&state_bytes);
        buf.extend_from_slice(&vp.width.to_bits().to_le_bytes());
        buf.extend_from_slice(&vp.height.to_bits().to_le_bytes());
        for &(idx, top) in scroll_offsets {
            buf.extend_from_slice(&idx.to_le_bytes());
            buf.extend_from_slice(&top.to_le_bytes());
        }
        let hash = xxh64(&buf, 0);
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
    #[cfg_attr(not(any(feature = "app-studio", feature = "app-kitchen-sink")), allow(unused_variables))]
    pub fn render_canonical_snapshot_for_state(
        &self,
        state_bytes: &[u8],
        scroll_offsets: Vec<(u32, u32)>,
    ) -> Option<WebSnapshotPayload> {
        #[cfg(not(any(feature = "app-studio", feature = "app-kitchen-sink")))]
        {
            None
        }
        #[cfg(any(feature = "app-studio", feature = "app-kitchen-sink"))]
        {
            let surface_id = self.surface_id;
            let viewport = self.backend.viewport();
            let schema_version = self.schema_version();
            let mut canonical = match &self.app {
                #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
                SessionApp::Dioxus(..) => return None,
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
                #[cfg(feature = "app-kitchen-sink")]
                SessionApp::KitchenSink(_) => {
                    let (s, ok) = Self::new_kitchen_sink_with_state(
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
            let ssr = canonical.render_ssr();
            Some(WebSnapshotPayload::from_ssr(&ssr, schema_version, scroll_offsets))
        }
    }

    /// The zero-copy scene mirror of this session's `NodeStore` tree.
    #[cfg(feature = "scene-view")]
    pub fn scene(&self) -> &SceneMirror {
        &self.scene
    }

    /// Safe zero-copy [`nve_abi::NveSceneRef`] over the current scene mirror and `NodeStore` text slab.
    #[cfg(feature = "scene-view")]
    pub fn scene_ref(&self) -> nve_abi::NveSceneRef<'_> {
        self.scene.as_ref_with_store(self.app.store())
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
        let surface_id = self.surface_id;
        self.backend.set_viewport(surface_id, viewport);
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
        let surface_id = self.surface_id;
        let ev = NativeEvent {
            surface_id,
            node_id: ROOT_NODE_ID,
            timestamp_us: 0,
            kind: NativeEventKind::KeyDown,
            payload: EventPayload::Key {
                key: key.to_owned(),
                code: String::new(),
                key_code: 0,
                modifiers: pack_key_modifiers(meta, ctrl, false, false, false),
                is_composing: false,
            },
        };
        if !self.app.dispatch_events(&[ev]) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }

    /// Forwards a host-event batch (e.g. the OS `ThemeSnapshot`); commits a frame on change.
    pub fn dispatch_host_events(&mut self, batch: &[u8]) -> Result<Option<FrameTiming>, BackendError> {
        if !self.app.dispatch_host_events(batch) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }

    /// Drains pending DOM events, dispatches them to the app, and commits an incremental frame if handled.
    pub fn pump_events(&mut self) -> Result<Option<FrameTiming>, BackendError> {
        let events = self.backend.drain_events();
        if events.is_empty() {
            return Ok(None);
        }
        if !self.app.dispatch_events(&events) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }

    /// Polls any ready Dioxus async tasks/effects (such as `nve-query` fetches or mutations) and
    /// renders incremental frames until the scheduler rests with `waker` armed, bounded to avoid
    /// starving browser painting. Invokes `on_before_poll` immediately before each `poll_work`
    /// iteration so wake schedulers can discard already-absorbed wakes from prior event/render
    /// passes without losing wakes fired during the final settling `poll_work`. When the per-tick
    /// pump cap is reached while work remains, `AsyncPumpOutcome::hit_cap` is `true` so hosts
    /// yield to paint and schedule a macrotask continuation instead of looping microtasks.
    #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
    pub fn pump_async_work_with<F: FnMut()>(
        &mut self,
        waker: &core::task::Waker,
        mut on_before_poll: F,
    ) -> Result<AsyncPumpOutcome, BackendError> {
        const MAX_PUMPS: usize = 8;
        let mut last = None;
        let mut hit_cap = true;
        for _ in 0..MAX_PUMPS {
            on_before_poll();
            if !self.app.poll_work(waker) {
                hit_cap = false;
                break;
            }
            last = Some(self.render_and_submit()?);
        }
        Ok(AsyncPumpOutcome {
            timing: last,
            hit_cap,
        })
    }

    #[cfg_attr(not(feature = "scene-view"), allow(unused_variables))]
    fn sync_scene(&mut self, frame_id: FrameId) {
        #[cfg(feature = "scene-view")]
        self.scene
            .sync(self.app.store(), self.app.last_changes(), frame_id);
    }

    fn render_and_submit(&mut self) -> Result<FrameTiming, BackendError> {
        self.frame_id = self.frame_id.wrapping_add(1).max(1);
        let frame_id = self.frame_id;
        let (reconcile_us, stats) = self.app.commit_to_timed(frame_id, &mut self.backend)?;
        self.sync_scene(frame_id);
        Ok(FrameTiming {
            frame_id,
            op_count: stats.op_count,
            mutation_count: stats.mutation_count,
            reconcile_us,
            encode_us: 0,
            decode_us: stats.decode_us,
            layout_us: stats.layout_us,
            diff_us: stats.diff_us,
            mount_us: stats.mount_us,
        })
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
