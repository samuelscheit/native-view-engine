//! Cross-target Web shell session (`WebAppSession`), Wizer Frame-0 pre-initialization,
//! hot-reload dispatch (`HotReloadAction`), and zero-copy `NveSceneTable` mirroring.

use std::cell::RefCell;

use nve_abi::{
    component_kind_to_scene_u8, encode_events, ComponentKind, Dimension, EventMask, EventPayload,
    FrameId, NativeEvent, NativeEventKind, NodeId, NveRenderNode, NveSceneRef, NveSceneTable,
    NveSceneView, SceneNodeInput, SurfaceId, NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE,
    NVE_NODE_LEAF_TEXT, ROOT_NODE_ID,
};
use nve_backend_web::{encode_keydown_payload, MirrorElement, WebBackend};
use nve_dioxus::DioxusSurface;
use nve_engine::{
    BackendError, FrameTiming, HotPatchMsg, NveApp, RenderBackend, SurfaceRunner, Viewport,
};
use nve_studio_workspace::StudioWorkspaceModule;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Default surface ID pre-materialized during `wizer.initialize`.
pub const PREINIT_SURFACE_ID: SurfaceId = 1;

/// Identifies which bundled NVE web demo app's Frame-0 VDOM + `CommandBuffer` was pre-baked
/// into WASM linear memory by [`wizer_initialize`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreinitApp {
    StudioWorkspace,
    Counter,
}

#[derive(Default)]
struct PreinitCache {
    initialized: bool,
    studio_frame0: Option<Vec<u8>>,
    counter_frame0: Option<Vec<u8>>,
}

thread_local! {
    static PREINIT_CACHE: RefCell<PreinitCache> = RefCell::new(PreinitCache::default());
}

/// Build-time Wizer pre-initialization entry point (`wizer.initialize`).
///
/// Materializes the initial Dioxus `VirtualDom`s and bakes the Frame-0 binary `CommandBuffer`s
/// for `StudioWorkspaceApp` and `CounterApp` directly into WASM linear memory (pure Rust, zero
/// browser/WASI calls), dropping temporary VDOM arenas afterward to keep the Wizer snapshot compact.
/// Idempotent: calling more than once leaves the cached buffers intact.
#[cfg_attr(target_arch = "wasm32", unsafe(export_name = "wizer.initialize"))]
pub extern "C" fn wizer_initialize() {
    let already_init = PREINIT_CACHE.with(|cell| cell.borrow().initialized);
    if already_init {
        return;
    }

    let studio_frame0 = {
        let mut studio = StudioWorkspaceModule::create(PREINIT_SURFACE_ID);
        studio.render(1).to_vec()
    };

    let counter_frame0 = {
        let mut counter = nve_example_counter::create_surface(PREINIT_SURFACE_ID);
        let (_, _, bytes) = counter.render_frame(1);
        bytes
    };

    PREINIT_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if !cache.initialized {
            cache.studio_frame0 = Some(studio_frame0);
            cache.counter_frame0 = Some(counter_frame0);
            cache.initialized = true;
        }
    });
}

/// Returns `true` when [`wizer_initialize`] has populated the pre-baked Frame-0 cache.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn is_preinitialized() -> bool {
    PREINIT_CACHE.with(|cell| cell.borrow().initialized)
}

/// Consumes and returns the pre-baked Frame-0 `CommandBuffer` for the default flagship app
/// (`StudioWorkspaceApp`), if available.
pub fn take_preinit_frame0_command_buffer() -> Option<Vec<u8>> {
    take_preinit_frame0_command_buffer_for(PreinitApp::StudioWorkspace)
}

/// Consumes and returns the pre-baked Frame-0 `CommandBuffer` for a specific bundled app.
pub fn take_preinit_frame0_command_buffer_for(app: PreinitApp) -> Option<Vec<u8>> {
    PREINIT_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        match app {
            PreinitApp::StudioWorkspace => cache.studio_frame0.take(),
            PreinitApp::Counter => cache.counter_frame0.take(),
        }
    })
}

/// Outcome of handling one `nve dev` WebSocket `HotPatchMsg`.
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

fn mirror_to_scene_input<'a>(
    backend: &WebBackend,
    node_id: NodeId,
    el: &'a MirrorElement,
) -> SceneNodeInput<'a> {
    let mut flags = 0u8;
    if el.kind == ComponentKind::Text && el.children.is_empty() {
        flags |= NVE_NODE_LEAF_TEXT;
    }
    if el.kind == ComponentKind::Pressable || el.event_mask != EventMask::NONE {
        flags |= NVE_NODE_INTERACTIVE;
    }
    if el.kind == ComponentKind::ScrollView {
        flags |= NVE_NODE_CLIP_BOUNDS;
    }

    let (x, y, width, height) = if let Some(rect) = backend.layout_of(node_id) {
        (rect.x, rect.y, rect.width, rect.height)
    } else {
        let w = match el.layout.width {
            Dimension::Points(v) => v,
            _ => 0.0,
        };
        let h = match el.layout.height {
            Dimension::Points(v) => v,
            _ => 0.0,
        };
        (0.0, 0.0, w, h)
    };

    SceneNodeInput {
        node_id,
        kind: component_kind_to_scene_u8(el.kind as u16),
        flags,
        opacity_u8: NveRenderNode::encode_opacity_u8(el.style.opacity),
        x,
        y,
        width,
        height,
        bg_rgba: el.style.background.to_u32(),
        text_rgba: el.style.text_color.to_u32(),
        border_rgba: el.style.border_color.to_u32(),
        border_radius: el.style.border_radius,
        border_width: el.style.border_width,
        font_size: el.style.font_size,
        z_index: el.style.z_index,
        text: el.text.as_deref().unwrap_or(""),
        children: &el.children,
    }
}

enum SessionApp {
    Dioxus(DioxusSurface),
    Studio(StudioWorkspaceModule),
}

impl SessionApp {
    fn render_frame(&mut self, frame_id: FrameId) -> (u64, u64, Vec<u8>) {
        match self {
            Self::Dioxus(surface) => surface.render_frame(frame_id),
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
        }
    }

    fn dispatch_events(&mut self, events: &[NativeEvent]) -> bool {
        match self {
            Self::Dioxus(surface) => surface.dispatch_events(events),
            Self::Studio(module) => module.dispatch_events(&encode_events(events)),
        }
    }

    fn apply_hot_patch(&mut self, msg: &HotPatchMsg) -> bool {
        match self {
            Self::Dioxus(surface) => surface.apply_hot_patch(msg),
            Self::Studio(module) => module.apply_hot_patch_msg(msg),
        }
    }
}

/// Cross-target session binding an NVE app (`DioxusSurface` or `StudioWorkspaceModule`) to a
/// `SurfaceRunner<WebBackend>` and a zero-copy [`NveSceneTable`] snapshot mirror for direct WASM
/// linear-memory inspection.
///
/// Runs identically in browser WASM (`web-dom-wasm`) and in host unit tests (`web-dom-mirror`).
pub struct WebAppSession {
    app: SessionApp,
    runner: SurfaceRunner<WebBackend>,
    preinit_frame0: Option<Vec<u8>>,
    needs_vdom_hydrate: bool,
    scene_table: NveSceneTable,
    scene_view_box: Box<NveSceneView>,
    scene_sort_scratch: Vec<NodeId>,
    scene_frame_id: FrameId,
}

impl WebAppSession {
    pub fn new(
        surface_id: SurfaceId,
        surface: DioxusSurface,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        Self::new_with_preinit_app(
            surface_id,
            SessionApp::Dioxus(surface),
            None,
            backend,
            viewport,
        )
    }

    fn new_with_preinit_app(
        surface_id: SurfaceId,
        app: SessionApp,
        preinit_frame0: Option<Vec<u8>>,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        let scene_table = NveSceneTable::new();
        let scene_view_box = Box::new(scene_table.as_view());
        Self {
            app,
            runner: SurfaceRunner::new(surface_id, viewport, backend),
            preinit_frame0,
            needs_vdom_hydrate: false,
            scene_table,
            scene_view_box,
            scene_sort_scratch: Vec::new(),
            scene_frame_id: 0,
        }
    }

    /// Boots the flagship `nve-studio-workspace` (Aether Workspace + NVE Studio) module.
    ///
    /// When [`wizer_initialize`] pre-baked Frame 0 for `surface_id == PREINIT_SURFACE_ID` and the
    /// initial viewport is desktop (`>= 620pt`), mounts the pre-baked Frame-0 `CommandBuffer`
    /// immediately (0 us reconcile/encode) and defers VDOM hydration until the first event.
    pub fn new_studio_workspace(
        surface_id: SurfaceId,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        let is_mobile_initial =
            viewport.width > 0.0 && viewport.width < nve_studio_workspace::MOBILE_BREAKPOINT_PT;
        let preinit = if !is_mobile_initial && surface_id == PREINIT_SURFACE_ID {
            take_preinit_frame0_command_buffer_for(PreinitApp::StudioWorkspace)
        } else {
            None
        };
        let mut module = StudioWorkspaceModule::create(surface_id);
        if preinit.is_none() && viewport.width > 0.0 && viewport.height > 0.0 {
            let layout_bytes = encode_events(&[NativeEvent {
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
            }]);
            let _ = module.dispatch_events(&layout_bytes);
        }
        Self::new_with_preinit_app(
            surface_id,
            SessionApp::Studio(module),
            preinit,
            backend,
            viewport,
        )
    }

    /// Boots the `nve-example-counter` Dioxus surface on `WebBackend`.
    ///
    /// Adopts the pre-baked `wizer.initialize` Frame-0 buffer when available for
    /// `surface_id == PREINIT_SURFACE_ID`.
    pub fn new_counter(surface_id: SurfaceId, backend: WebBackend, viewport: Viewport) -> Self {
        let preinit = if surface_id == PREINIT_SURFACE_ID {
            take_preinit_frame0_command_buffer_for(PreinitApp::Counter)
        } else {
            None
        };
        Self::new_with_preinit_app(
            surface_id,
            SessionApp::Dioxus(nve_example_counter::create_surface(surface_id)),
            preinit,
            backend,
            viewport,
        )
    }

    /// Returns `true` if this session holds a pre-baked `wizer.initialize` Frame-0 buffer waiting
    /// to be mounted by [`Self::render_initial`].
    pub fn has_preinit_frame0(&self) -> bool {
        self.preinit_frame0.is_some()
    }

    pub fn backend(&self) -> &WebBackend {
        self.runner.backend()
    }

    pub fn backend_mut(&mut self) -> &mut WebBackend {
        self.runner.backend_mut()
    }

    /// Returns the underlying incremental [`NveSceneTable`] mirroring the live `WebBackend` tree.
    pub fn scene_table(&self) -> &NveSceneTable {
        &self.scene_table
    }

    /// Returns a zero-copy C ABI [`NveSceneView`] pointing directly into this session's scene slabs.
    pub fn scene_view(&self) -> NveSceneView {
        *self.scene_view_box
    }

    /// Returns a stable pointer to this session's [`NveSceneView`] header in linear memory
    /// (suitable for `NveSceneViewReader.fromWasm32Pointer(memory.buffer, ptr)` on `wasm32`).
    pub fn scene_view_ptr(&self) -> *const NveSceneView {
        &*self.scene_view_box as *const NveSceneView
    }

    /// Returns a safe zero-copy borrowed [`NveSceneRef`] over the current scene snapshot.
    pub fn scene_ref(&self) -> NveSceneRef<'_> {
        self.scene_table.as_ref()
    }

    /// Renders and mounts the initial frame (or submits the pre-baked Wizer Frame-0 buffer
    /// directly with 0 us reconcile/encode overhead when available).
    pub fn render_initial(&mut self) -> Result<FrameTiming, BackendError> {
        if let Some(frame0) = self.preinit_frame0.take() {
            self.needs_vdom_hydrate = true;
            let frame_id = self.runner.next_frame_id();
            let timing = self
                .runner
                .submit_recorded_frame(frame_id, 0, 0, &frame0)?;
            self.sync_scene_table(frame_id);
            return Ok(timing);
        }
        self.render_and_submit()
    }

    fn ensure_vdom_hydrated(&mut self) {
        if self.needs_vdom_hydrate {
            // Rebuilds the Dioxus VDOM once to populate the deterministic ElementId <-> NodeId
            // table matching the pre-baked Frame-0 DOM before dispatching live events.
            let _ = self.app.render_frame(1);
            self.needs_vdom_hydrate = false;
        }
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
        self.ensure_vdom_hydrated();
        let ev = NativeEvent {
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
        };
        let timing = if self.app.dispatch_events(&[ev]) {
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
        self.ensure_vdom_hydrated();
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
        self.ensure_vdom_hydrated();
        if !self.app.dispatch_events(&events) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }

    /// Decodes and applies a `HotPatchMsg` from either zero-JSON binary `NVP1` or UTF-8 JSON bytes.
    pub fn handle_hot_patch_wire(&mut self, raw: &[u8]) -> Result<HotReloadAction, String> {
        let msg = HotPatchMsg::from_wire(raw).map_err(|e| format!("invalid HotPatchMsg: {e}"))?;
        self.handle_hot_patch(msg)
    }

    /// Parses and applies one JSON-serialized `HotPatchMsg` from `nve dev`.
    pub fn handle_hot_patch_json(&mut self, raw: &str) -> Result<HotReloadAction, String> {
        self.handle_hot_patch_wire(raw.as_bytes())
    }

    /// Applies one decoded `HotPatchMsg` from `nve dev`.
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
                self.sync_scene_table(next_frame);
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

    fn sync_scene_table(&mut self, frame_id: FrameId) {
        self.scene_frame_id = frame_id;
        let backend = self.runner.backend();
        self.scene_sort_scratch.clear();
        self.scene_sort_scratch
            .extend(backend.iter_nodes().map(|(id, _)| id));
        self.scene_sort_scratch.sort_unstable();

        let sort_ids = &self.scene_sort_scratch;
        let _ = self.scene_table.sync_frame(
            frame_id,
            sort_ids.iter().filter_map(|&node_id| {
                let el = backend.node(node_id)?;
                Some(mirror_to_scene_input(backend, node_id, el))
            }),
        );
        *self.scene_view_box = self.scene_table.as_view();
    }

    fn render_and_submit(&mut self) -> Result<FrameTiming, BackendError> {
        let frame_id = self.runner.next_frame_id();
        let (reconcile_us, encode_us, bytes) = self.app.render_frame(frame_id);
        let timing = self
            .runner
            .submit_recorded_frame(frame_id, reconcile_us, encode_us, &bytes)?;
        self.sync_scene_table(frame_id);
        Ok(timing)
    }
}
