//! Cross-target Web shell session (`WebAppSession`), Wizer Frame-0 pre-initialization,
//! hot-reload dispatch (`HotReloadAction`), and zero-copy `NveSceneTable` mirroring.

use std::cell::RefCell;

use nve_abi::{
    component_kind_to_scene_u8, ComponentKind, Dimension, EventMask, FrameId, NodeId,
    NveRenderNode, NveSceneRef, NveSceneTable, NveSceneView, SceneNodeInput, SurfaceId,
    NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE, NVE_NODE_LEAF_TEXT,
};
use nve_backend_web::{MirrorElement, WebBackend};
use nve_dioxus::DioxusSurface;
use nve_engine::{BackendError, FrameTiming, HotPatchMsg, RenderBackend, SurfaceRunner, Viewport};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

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
        let mut studio = nve_studio_workspace::create_surface(PREINIT_SURFACE_ID);
        let (_, _, bytes) = studio.render_frame(1);
        bytes
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

/// Cross-target session binding a `DioxusSurface` to a `SurfaceRunner<WebBackend>` and a
/// zero-copy [`NveSceneTable`] snapshot mirror for direct WASM linear-memory inspection.
///
/// Runs identically in browser WASM (`web-dom-wasm`) and in host unit tests (`web-dom-mirror`).
pub struct WebAppSession {
    surface: DioxusSurface,
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
        Self::new_with_preinit(surface_id, surface, None, backend, viewport)
    }

    fn new_with_preinit(
        surface_id: SurfaceId,
        surface: DioxusSurface,
        preinit_frame0: Option<Vec<u8>>,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        let scene_table = NveSceneTable::new();
        let scene_view_box = Box::new(scene_table.as_view());
        Self {
            surface,
            runner: SurfaceRunner::new(surface_id, viewport, backend),
            preinit_frame0,
            needs_vdom_hydrate: false,
            scene_table,
            scene_view_box,
            scene_sort_scratch: Vec::new(),
            scene_frame_id: 0,
        }
    }

    /// Boots the flagship `nve-studio-workspace` (Aether Workspace + NVE Studio) Dioxus surface.
    ///
    /// When [`wizer_initialize`] pre-baked Frame 0 for `surface_id == PREINIT_SURFACE_ID`,
    /// mounts the pre-baked Frame-0 `CommandBuffer` immediately (0 us reconcile/encode) and
    /// defers VDOM hydration until the first interactive event.
    pub fn new_studio_workspace(
        surface_id: SurfaceId,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        let preinit = if surface_id == PREINIT_SURFACE_ID {
            take_preinit_frame0_command_buffer_for(PreinitApp::StudioWorkspace)
        } else {
            None
        };
        Self::new_with_preinit(
            surface_id,
            nve_studio_workspace::create_surface(surface_id),
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
        Self::new_with_preinit(
            surface_id,
            nve_example_counter::create_surface(surface_id),
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
            let _ = self.surface.render_frame(1);
            self.needs_vdom_hydrate = false;
        }
    }

    /// Drains pending DOM events, dispatches them to Dioxus, and commits an incremental frame if handled.
    pub fn pump_events(&mut self) -> Result<Option<FrameTiming>, BackendError> {
        let events = self.runner.backend_mut().drain_events();
        if events.is_empty() {
            return Ok(None);
        }
        self.ensure_vdom_hydrated();
        if !self.surface.dispatch_events(&events) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }

    /// Parses and applies one JSON-serialized `HotPatchMsg` from `nve dev`.
    pub fn handle_hot_patch_json(&mut self, raw: &str) -> Result<HotReloadAction, String> {
        let msg = HotPatchMsg::from_json(raw).map_err(|e| format!("invalid HotPatchMsg: {e}"))?;
        self.handle_hot_patch(msg)
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
                let _ = self.surface.apply_hot_patch(msg);
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
        let (reconcile_us, encode_us, bytes) = self.surface.render_frame(frame_id);
        let timing = self
            .runner
            .submit_recorded_frame(frame_id, reconcile_us, encode_us, &bytes)?;
        self.sync_scene_table(frame_id);
        Ok(timing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{
        EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId, NveSceneRef, RawPropValue,
        NVE_SCENE_SCHEMA_HASH, ROOT_NODE_ID,
    };

    fn find_pressable_nodes(backend: &WebBackend) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = (1..=64)
            .filter(|&id| (backend.event_mask_of(id) & EventMask::PRESS) != 0)
            .collect();
        ids.sort_unstable();
        ids
    }

    fn find_node_by_test_id(backend: &WebBackend, target: &str) -> Option<NodeId> {
        fn walk(backend: &WebBackend, id: NodeId, target: &str) -> Option<NodeId> {
            let el = backend.node(id)?;
            if matches!(
                el.raw_props.get("testID"),
                Some(RawPropValue::String(s)) if s == target
            ) {
                return Some(id);
            }
            for &child in &el.children {
                if let Some(found) = walk(backend, child, target) {
                    return Some(found);
                }
            }
            None
        }
        walk(backend, ROOT_NODE_ID, target)
    }

    fn press_test_id(session: &mut WebAppSession, test_id: &str, timestamp_us: u64) -> FrameTiming {
        let node_id = find_node_by_test_id(session.backend(), test_id)
            .unwrap_or_else(|| panic!("expected mounted node with test_id='{test_id}'"));
        assert_ne!(
            session.backend().event_mask_of(node_id) & EventMask::PRESS,
            0,
            "node '{test_id}' (id={node_id}) should have EventMask::PRESS"
        );
        session.backend().push_event(NativeEvent {
            surface_id: 1,
            node_id,
            timestamp_us,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 16.0, y: 12.0 },
        });
        session
            .pump_events()
            .unwrap()
            .unwrap_or_else(|| panic!("pressing '{test_id}' should trigger an incremental frame"))
    }

    fn assert_valid_dom_html(html: &str) {
        assert!(
            html.starts_with("<div data-nve-id=\"1\""),
            "DOM root should be node 1 <div>"
        );
        assert!(html.ends_with("</div>"), "DOM HTML should close root </div>");
        for tag in ["div", "button", "span"] {
            let opens = html.matches(&format!("<{tag} ")).count();
            let closes = html.matches(&format!("</{tag}>")).count();
            assert_eq!(
                opens, closes,
                "unbalanced <{tag}> tags in rendered DOM HTML (opens={opens}, closes={closes})"
            );
        }
    }

    fn assert_scene_view_matches_backend(session: &WebAppSession) {
        let view = session.scene_view();
        let ptr_view = unsafe { *session.scene_view_ptr() };
        assert_eq!(view.schema_hash, NVE_SCENE_SCHEMA_HASH);
        assert_eq!(ptr_view.schema_hash, NVE_SCENE_SCHEMA_HASH);
        assert_eq!(ptr_view.frame_id, view.frame_id);
        assert_eq!(ptr_view.node_count, view.node_count);
        assert_eq!(ptr_view.nodes, view.nodes);

        let scene = unsafe { NveSceneRef::from_raw(ptr_view) }
            .expect("scene_view_ptr must yield a valid NveSceneRef");
        let backend = session.backend();
        assert_eq!(scene.nodes().len(), backend.node_count());

        for rnode in scene.nodes() {
            let el = backend
                .node(rnode.node_id)
                .unwrap_or_else(|| panic!("scene node {} missing in WebBackend", rnode.node_id));
            assert_eq!(rnode.kind, component_kind_to_scene_u8(el.kind as u16));
            assert_eq!(
                rnode.is_leaf_text(),
                el.kind == ComponentKind::Text && el.children.is_empty()
            );
            assert_eq!(
                rnode.is_interactive(),
                el.kind == ComponentKind::Pressable || el.event_mask != EventMask::NONE
            );
            assert_eq!(rnode.clips_bounds(), el.kind == ComponentKind::ScrollView);
            assert_eq!(
                rnode.opacity_u8,
                NveRenderNode::encode_opacity_u8(el.style.opacity)
            );
            assert_eq!(rnode.bg_rgba, el.style.background.to_u32());
            assert_eq!(rnode.text_rgba, el.style.text_color.to_u32());
            assert_eq!(rnode.border_rgba, el.style.border_color.to_u32());
            assert_eq!(rnode.border_radius.to_bits(), el.style.border_radius.to_bits());
            assert_eq!(rnode.border_width.to_bits(), el.style.border_width.to_bits());
            assert_eq!(rnode.font_size.to_bits(), el.style.font_size.to_bits());
            assert_eq!(rnode.z_index, el.style.z_index);
            assert_eq!(
                scene.node_text(rnode).unwrap(),
                el.text.as_deref().unwrap_or("")
            );
            assert_eq!(scene.node_children(rnode).unwrap(), el.children.as_slice());
        }
    }

    #[test]
    fn counter_app_mounts_clicks_incrementally_and_resets() {
        let mut session = WebAppSession::new_counter(1, WebBackend::new(), Viewport::default());
        let init = session.render_initial().unwrap();
        assert!(init.op_count > 0);
        assert!(init.mutation_count > 0);

        let html0 = session.backend().render_html();
        assert!(html0.contains("NATIVE VIEW ENGINE"));
        assert!(html0.contains("Count: 0"));
        assert!(html0.contains("Increment +"));
        assert!(html0.contains("Reset"));
        assert_scene_view_matches_backend(&session);

        let buttons = find_pressable_nodes(session.backend());
        assert_eq!(buttons.len(), 2, "expected Increment and Reset buttons");
        let increment_id = buttons[0];
        let reset_id = buttons[1];

        // Click Increment twice.
        for expected in 1..=2 {
            session.backend().push_event(NativeEvent {
                surface_id: 1,
                node_id: increment_id,
                timestamp_us: expected as u64 * 1000,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point { x: 80.0, y: 24.0 },
            });
            let timing = session.pump_events().unwrap().expect("expected frame");
            assert!(
                timing.mutation_count <= 2,
                "incremental click should only patch changed text, got {} mutations",
                timing.mutation_count
            );
            let html = session.backend().render_html();
            assert!(html.contains(&format!("Count: {expected}")));
            assert_scene_view_matches_backend(&session);
            // Incremental NveSceneView update marks only changed nodes dirty.
            let view = session.scene_view();
            assert!(view.dirty_count >= 1 && view.dirty_count < view.node_count);
            assert_eq!(view.removed_count, 0);
        }

        // Click Reset -> returns to Count: 0.
        session.backend().push_event(NativeEvent {
            surface_id: 1,
            node_id: reset_id,
            timestamp_us: 3000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 60.0, y: 24.0 },
        });
        session.pump_events().unwrap().expect("expected reset frame");
        assert!(session.backend().render_html().contains("Count: 0"));
        assert_scene_view_matches_backend(&session);
    }

    #[test]
    fn handles_hot_patch_messages_for_tier1_tier2_and_status() {
        let mut session = WebAppSession::new_counter(1, WebBackend::new(), Viewport::default());
        session.render_initial().unwrap();
        assert_scene_view_matches_backend(&session);

        // Tier-1 TemplatePatch applies live to DOM mirror and refreshes NveSceneView.
        let tpl = HotPatchMsg::TemplatePatch {
            generation: 3,
            template_id: "CounterApp".into(),
            attr_updates: vec![("border-color".into(), "#38bdf8".into())],
            text_updates: vec![(0, "NVE LIVE PATCHED".into())],
            target_test_id: None,
            node_path: vec![],
            text_replacements: vec![],
        };
        let act1 = session.handle_hot_patch_json(&tpl.to_json()).unwrap();
        match act1 {
            HotReloadAction::TemplatePatched {
                generation,
                template_id,
                mutations,
            } => {
                assert_eq!(generation, 3);
                assert_eq!(template_id, "CounterApp");
                assert!(mutations > 0);
            }
            other => panic!("unexpected action: {other:?}"),
        }
        assert!(session.backend().render_html().contains("NVE LIVE PATCHED"));
        assert_scene_view_matches_backend(&session);
        let scene_ref = session.scene_ref();
        let patched_in_scene = scene_ref
            .nodes()
            .iter()
            .any(|node| scene_ref.node_text(node).ok() == Some("NVE LIVE PATCHED"));
        assert!(patched_in_scene);

        // Tier-2 DylibReload returns clean ReloadPage directive.
        let reload = HotPatchMsg::DylibReload {
            generation: 4,
            dylib_path: "target/debug/libnve_example_counter.dylib".into(),
            build_ms: 215,
        };
        let act2 = session.handle_hot_patch_json(&reload.to_json()).unwrap();
        assert_eq!(
            act2,
            HotReloadAction::ReloadPage {
                generation: 4,
                dylib_path: "target/debug/libnve_example_counter.dylib".into(),
                build_ms: 215,
            }
        );

        // DevStatus heartbeat.
        let status = HotPatchMsg::DevStatus {
            message: "watching examples/counter/src".into(),
            latency_ms: 12,
        };
        let act3 = session.handle_hot_patch_json(&status.to_json()).unwrap();
        assert_eq!(
            act3,
            HotReloadAction::DevStatus {
                message: "watching examples/counter/src".into(),
                latency_ms: 12,
            }
        );

        // Malformed JSON returns descriptive error without panicking.
        assert!(session.handle_hot_patch_json("{not valid json").is_err());
    }

    #[test]
    fn studio_workspace_mounts_interacts_and_time_travels() {
        let viewport = Viewport {
            width: 1280.0,
            height: 800.0,
            scale_factor: 1.0,
        };
        let mut session = WebAppSession::new_studio_workspace(1, WebBackend::new(), viewport);

        // 1. Initial mount renders the 3-column Aether Workspace + NVE Studio DOM.
        let init = session.render_initial().unwrap();
        assert!(init.op_count > 0);
        assert!(init.mutation_count > 0);
        assert!(session.backend().node_count() > 20);
        assert_scene_view_matches_backend(&session);

        let html0 = session.backend().render_html();
        assert_valid_dom_html(&html0);
        assert!(html0.contains("AETHER"));
        assert!(html0.contains("Issue Board"));
        assert!(html0.contains("NVE STUDIO"));
        assert!(html0.contains("AET-101"));
        assert!(html0.contains("Zero-copy EventRing ABI"));
        assert!(html0.contains("In Progress"));
        assert!(html0.contains("Done (1)"));
        assert!(html0.contains("Snapshot 1 / 1"));
        assert!(html0.contains("Workspace Boot"));

        // 2. Click status pill on AET-101 -> cycles In Progress -> Done.
        let t_status = press_test_id(&mut session, "task-status-1", 1000);
        assert!(t_status.mutation_count > 0);
        let html_after_status = session.backend().render_html();
        assert_valid_dom_html(&html_after_status);
        assert!(html_after_status.contains("Status -> Done"));
        assert!(html_after_status.contains("Done (2)"));
        assert!(html_after_status.contains("Snapshot 2 / 2"));
        assert_scene_view_matches_backend(&session);

        // 3. Switch views: Board -> # engineering -> Live Metrics (verifies removals + parity).
        press_test_id(&mut session, "nav-engineering", 2000);
        assert!(
            session.scene_view().removed_count > 0,
            "switching views should report removed nodes in NveSceneView"
        );
        assert_scene_view_matches_backend(&session);
        let html_eng = session.backend().render_html();
        assert_valid_dom_html(&html_eng);
        assert!(html_eng.contains("# engineering"));
        assert!(html_eng.contains("Single ABI drives all 4 hosts!"));
        assert!(html_eng.contains("QUICK REPLY"));
        assert!(html_eng.contains("Snapshot 3 / 3"));

        press_test_id(&mut session, "nav-metrics", 3000);
        assert_scene_view_matches_backend(&session);
        let html_metrics = session.backend().render_html();
        assert_valid_dom_html(&html_metrics);
        assert!(html_metrics.contains("Live Metrics"));
        assert!(html_metrics.contains("Node Batch #01"));
        assert!(html_metrics.contains("Snapshot 4 / 4"));

        // 4. Live self-edit in docked NVE Studio panel (Accent -> Violet).
        press_test_id(&mut session, "accent-violet", 4000);
        assert_scene_view_matches_backend(&session);
        let html_violet = session.backend().render_html();
        assert_valid_dom_html(&html_violet);
        assert!(html_violet.contains("Accent -> Violet"));
        assert!(html_violet.contains("rgba(167,139,250,1.000)"));
        assert!(html_violet.contains("Snapshot 5 / 5"));

        // 5. Deterministic time-travel: Undo -> First (#1) -> Live.
        press_test_id(&mut session, "tt-undo", 5000);
        let html_undo = session.backend().render_html();
        assert_valid_dom_html(&html_undo);
        assert!(html_undo.contains("Snapshot 4 / 5"));
        assert!(html_undo.contains("Live Metrics"));

        press_test_id(&mut session, "tt-first", 6000);
        let html_first = session.backend().render_html();
        assert_valid_dom_html(&html_first);
        assert!(html_first.contains("Snapshot 1 / 5"));
        assert!(html_first.contains("Workspace Boot"));
        assert!(html_first.contains("Issue Board"));
        assert!(html_first.contains("Done (1)"));

        press_test_id(&mut session, "tt-live", 7000);
        assert_scene_view_matches_backend(&session);
        let html_live = session.backend().render_html();
        assert_valid_dom_html(&html_live);
        assert!(html_live.contains("Snapshot 5 / 5"));
        assert!(html_live.contains("Accent -> Violet"));
        assert!(html_live.contains("Live Metrics"));
    }

    #[test]
    fn wizer_initialize_populates_frame0_buffer_idempotently_and_hydrates_interactively() {
        // Fresh test thread starts uninitialized.
        assert!(!is_preinitialized());
        assert!(take_preinit_frame0_command_buffer().is_none());

        // First call pre-bakes both StudioWorkspace and Counter Frame-0 CommandBuffers.
        wizer_initialize();
        assert!(is_preinitialized());

        // Second call is idempotent and preserves the pre-baked buffers.
        wizer_initialize();
        assert!(is_preinitialized());

        // Direct consumer take_preinit_frame0_command_buffer() returns valid CommandBuffer once.
        let studio_frame0 =
            take_preinit_frame0_command_buffer().expect("expected pre-baked Studio Frame 0");
        let (header, ops) = nve_abi::CommandDecoder::new(&studio_frame0)
            .expect("pre-baked Frame-0 header must be valid")
            .decode_all()
            .expect("pre-baked Frame-0 buffer must decode cleanly");
        assert_eq!(header.surface_id, PREINIT_SURFACE_ID);
        assert_eq!(header.frame_id, 1);
        assert!(!ops.is_empty());
        assert!(take_preinit_frame0_command_buffer().is_none());

        // CounterApp session adopts the pre-baked surface + Frame-0 buffer for 0 us initial reconcile
        // and continues handling live clicks seamlessly.
        let mut counter_session =
            WebAppSession::new_counter(PREINIT_SURFACE_ID, WebBackend::new(), Viewport::default());
        assert!(counter_session.has_preinit_frame0());
        let init = counter_session.render_initial().unwrap();
        assert_eq!(init.reconcile_us, 0);
        assert_eq!(init.encode_us, 0);
        assert!(init.op_count > 0);
        assert!(counter_session.backend().render_html().contains("Count: 0"));
        assert_scene_view_matches_backend(&counter_session);

        let buttons = find_pressable_nodes(counter_session.backend());
        assert_eq!(buttons.len(), 2);
        counter_session.backend().push_event(NativeEvent {
            surface_id: PREINIT_SURFACE_ID,
            node_id: buttons[0],
            timestamp_us: 1000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 40.0, y: 20.0 },
        });
        counter_session
            .pump_events()
            .unwrap()
            .expect("post-wizer click should trigger incremental frame");
        assert!(counter_session.backend().render_html().contains("Count: 1"));
        assert_scene_view_matches_backend(&counter_session);
    }

    #[test]
    fn scene_view_is_valid_before_initial_render_and_tracks_pointer_stability() {
        let mut session = WebAppSession::new_counter(1, WebBackend::new(), Viewport::default());
        let ptr_before = session.scene_view_ptr();
        let empty_view = unsafe { *ptr_before };
        assert_eq!(empty_view.schema_hash, NVE_SCENE_SCHEMA_HASH);
        assert_eq!(empty_view.node_count, 0);
        let empty_ref = unsafe { NveSceneRef::from_raw(empty_view) }.unwrap();
        assert!(empty_ref.nodes().is_empty());

        session.render_initial().unwrap();
        // Heap-pinned `Box<NveSceneView>` preserves its address across frames.
        assert_eq!(session.scene_view_ptr(), ptr_before);
        let mounted_view = unsafe { *ptr_before };
        assert_eq!(mounted_view.frame_id, 1);
        assert_eq!(mounted_view.node_count as usize, session.backend().node_count());
        assert_scene_view_matches_backend(&session);
    }
}
