//! Unified Web DOM / WASM / SSR / Hydration [`StoreBackend`] over `NodeStore` word tables and
//! [`WebTableRegistry`].
//!
//! `nve-backend-web` has **zero dependencies** on optional capability modules (`svg`, `canvas`,
//! `blur`, `webview`, `video`, `camera`, `web-element`, `web-html`). Every capability registers a
//! [`WebTableSystem`] onto [`WebBackend::register_table_system`], covering live `wasm32` DOM,
//! SSR, streaming SSR, and hydration uniformly.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod css;
pub mod event_replay;
pub mod events;
#[cfg(target_arch = "wasm32")]
mod events_dom;
mod hot_patch;
pub mod html;
#[cfg(feature = "hydrate")]
pub mod hydrate;
pub mod input_attrs;
pub mod layout_ext_css;
pub mod media_css;
pub mod patch_css;
pub mod sanitize;
#[cfg(feature = "ssr")]
pub mod snapshot;
#[cfg(feature = "ssr")]
pub mod ssg;
#[cfg(feature = "ssr")]
pub mod ssr;
pub mod state_css;
#[cfg(feature = "ssr")]
pub mod streaming;
#[cfg(feature = "ssr")]
pub mod streaming_live;
pub mod stylesheet;
#[cfg(target_arch = "wasm32")]
mod stylesheet_wasm;
pub mod table_system;
pub mod transition_css;
pub mod visual_viewport;
#[cfg(target_arch = "wasm32")]
mod wasm_dom;

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Optional host callback invoked when a live browser event is pushed.
pub type RenderCallback = Rc<dyn Fn()>;

use nve_abi::hash::HashMap;
use nve_abi::{EventBatchBuf, NodeId, SurfaceId};
#[cfg(feature = "hydrate")]
use nve_abi::{InteractionEvent, TextInputEvent, TimestampUs};
use nve_components::keyboard::KeyboardInsets;
#[cfg(feature = "hydrate")]
use nve_components::tables::{Interaction, TextInput};
use nve_engine::{BackendApplyStats, BackendError, StoreBackend, Viewport};
use nve_store::{ChangeSet, NodeStore};

pub use css::{canonical_f32_bits, write_compositor_css, write_store_node_css, NVE_BASE_RESET_CSS};
pub use event_replay::{
    drain_valid_pre_hydration_events, pre_hydration_replay_script_tag, push_pre_hydration_event,
    CapturedPreHydrationEvent, PRE_HYDRATION_REPLAY_SCRIPT_TAG,
};
pub use events::{
    decode_form_urlencoded_map, is_hard_nav_anchor, should_intercept_anchor_click,
    ImeCompositionGuard, InputSyncTracker, WebEventState, NODE_KIND_INPUT, NODE_KIND_LINK,
};
#[cfg(feature = "hydrate")]
pub use hydrate::{HydrationMismatch, HydrationReport};
#[cfg(all(feature = "ssr", target_arch = "wasm32"))]
pub use snapshot::BrowserLocalStorage;
#[cfg(feature = "ssr")]
pub use snapshot::{
    decode_state_envelope, encode_state_envelope, query_has_reset_param,
    snapshot_and_replay_script_tags, snapshot_bootstrap_script_tag, snapshot_storage_key,
    state_storage_key, MemorySnapshotStorage, SnapshotError, SnapshotStorage, WebSnapshotPayload,
    WebSnapshotStore, DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_MAX_STATE_BYTES,
    SNAPSHOT_BOOTSTRAP_MAX_BYTES, SNAPSHOT_ENVELOPE_MAGIC, SNAPSHOT_KEY_PREFIX,
    STATE_ENVELOPE_MAGIC, STATE_KEY_PREFIX, WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG,
};
#[cfg(feature = "ssr")]
pub use ssg::{
    inject_ssg_into_shell_html, inject_ssg_with_scripts_into_shell_html,
    prerender_nve_app_frame0, WebSsgDocumentOptions,
};
#[cfg(feature = "ssr")]
pub use ssr::{SsrDocumentOptions, SsrOutput};
#[cfg(feature = "ssr")]
pub use streaming::{
    encode_http_chunk, sanitize_stream_slot_id, scoped_slot_hiding_css, slot_placeholder_html,
    stream_slot_name, streaming_http_headers, StreamingDocumentOptions, StreamingError,
    StreamingMode, StreamingSsrSession,
};
pub use stylesheet::{ResolvedNodeStyle, StyleSheetCache, MAX_DYNAMIC_RULES};
pub use table_system::{
    is_safe_custom_tag, is_safe_web_attr, is_safe_web_url, RealizedWebNode, WebStoreRenderer,
    WebSystemCommitCtx, WebTableRegistry, WebTableSystem,
};
pub use visual_viewport::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, KEYBOARD_INSETS_EVENT_NAME,
    VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};

/// Unified Web DOM / WASM / SSR / Hydration [`StoreBackend`].
pub struct WebBackend {
    pub(crate) surface_id: Rc<Cell<SurfaceId>>,
    pub(crate) viewport: Viewport,
    pub(crate) renderer: WebStoreRenderer,
    pub(crate) stylesheet: StyleSheetCache,
    pub(crate) patch_css: HashMap<NodeId, String>,
    pub(crate) pending_events: Rc<RefCell<EventBatchBuf>>,
    pub(crate) event_state: Rc<RefCell<WebEventState>>,
    pub(crate) input_sync: Rc<RefCell<InputSyncTracker>>,
    pub(crate) ime_guard: Rc<ImeCompositionGuard>,
    pub(crate) keyboard_insets: Rc<Cell<KeyboardInsets>>,
    pub(crate) on_keyboard_insets: Rc<RefCell<Option<visual_viewport::KeyboardInsetsCallback>>>,
    pub(crate) on_event: Rc<RefCell<Option<RenderCallback>>>,
    #[cfg(feature = "hydrate")]
    pub(crate) hydration_report: HydrationReport,
    #[cfg(feature = "hydrate")]
    pub(crate) hydrated_control_values: HashMap<NodeId, String>,
    #[cfg(feature = "hydrate")]
    pub(crate) pre_hydration_events: Vec<CapturedPreHydrationEvent>,
    #[cfg(feature = "hydrate")]
    pub(crate) pending_ssr_nodes: HashMap<NodeId, String>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) mount_root: Option<web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) live_elements: HashMap<NodeId, web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) unverified_ssr_ids: nve_abi::hash::HashSet<NodeId>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) delegated_events: Option<events::RootEventDelegator>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) visual_viewport_tracker: Option<visual_viewport::VisualViewportTracker>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) created_scratch: nve_abi::hash::HashSet<NodeId>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) retagged_scratch: Vec<NodeId>,
}

impl Default for WebBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebBackend {
    pub fn new() -> Self {
        Self {
            surface_id: Rc::new(Cell::new(SurfaceId::ROOT)),
            viewport: Viewport::default(),
            renderer: WebStoreRenderer::new(),
            stylesheet: StyleSheetCache::new(),
            patch_css: HashMap::default(),
            pending_events: Rc::new(RefCell::new(EventBatchBuf::new())),
            event_state: Rc::new(RefCell::new(WebEventState::new())),
            input_sync: Rc::new(RefCell::new(InputSyncTracker::new())),
            ime_guard: Rc::new(ImeCompositionGuard::new()),
            keyboard_insets: Rc::new(Cell::new(KeyboardInsets::default())),
            on_keyboard_insets: Rc::new(RefCell::new(None)),
            on_event: Rc::new(RefCell::new(None)),
            #[cfg(feature = "hydrate")]
            hydration_report: HydrationReport::default(),
            #[cfg(feature = "hydrate")]
            hydrated_control_values: HashMap::default(),
            #[cfg(feature = "hydrate")]
            pre_hydration_events: Vec::new(),
            #[cfg(feature = "hydrate")]
            pending_ssr_nodes: HashMap::default(),
            #[cfg(target_arch = "wasm32")]
            mount_root: None,
            #[cfg(target_arch = "wasm32")]
            live_elements: HashMap::default(),
            #[cfg(target_arch = "wasm32")]
            unverified_ssr_ids: nve_abi::hash::HashSet::default(),
            #[cfg(target_arch = "wasm32")]
            delegated_events: None,
            #[cfg(target_arch = "wasm32")]
            visual_viewport_tracker: None,
            #[cfg(target_arch = "wasm32")]
            created_scratch: nve_abi::hash::HashSet::default(),
            #[cfg(target_arch = "wasm32")]
            retagged_scratch: Vec::new(),
        }
    }

    #[inline]
    pub fn register_table_system<S: WebTableSystem>(&mut self, system: S) -> &mut Self {
        self.renderer.registry_mut().register(system);
        self
    }

    #[inline]
    pub fn table_registry(&self) -> &WebTableRegistry {
        self.renderer.registry()
    }

    #[inline]
    pub fn table_registry_mut(&mut self) -> &mut WebTableRegistry {
        self.renderer.registry_mut()
    }

    #[inline]
    pub fn table_events(&self) -> &EventBatchBuf {
        &self.renderer.events
    }

    #[inline]
    pub fn table_events_mut(&mut self) -> &mut EventBatchBuf {
        &mut self.renderer.events
    }

    /// Registers a callback invoked when asynchronous browser events are queued.
    pub fn set_on_event<F: Fn() + 'static>(&mut self, callback: F) {
        *self.on_event.borrow_mut() = Some(Rc::new(callback));
    }

    #[inline]
    pub fn renderer(&self) -> &WebStoreRenderer {
        &self.renderer
    }

    #[inline]
    pub fn renderer_mut(&mut self) -> &mut WebStoreRenderer {
        &mut self.renderer
    }

    #[inline]
    pub fn stylesheet(&self) -> &StyleSheetCache {
        &self.stylesheet
    }

    #[inline]
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    #[inline]
    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id.get()
    }

    #[inline]
    pub fn ime_guard(&self) -> &ImeCompositionGuard {
        &self.ime_guard
    }

    /// Mutably accesses the pending [`EventBatchBuf`] queue (for hosts/tests injecting events).
    #[inline]
    pub fn with_pending_events<R>(&mut self, f: impl FnOnce(&mut EventBatchBuf) -> R) -> R {
        f(&mut self.pending_events.borrow_mut())
    }

    #[inline]
    pub fn composite_fast_lane_hits(&self) -> u64 {
        self.renderer.composite_fast_lane_hits
    }

    #[inline]
    pub fn full_style_rebuilds(&self) -> u64 {
        self.renderer.full_style_rebuilds
    }

    fn warm_stylesheet(&mut self, store: &NodeStore, changes: &ChangeSet) {
        for &id in &changes.created {
            if store.is_live(id) {
                let _ = self.stylesheet.resolve_node(store, id);
            }
        }
        for (&id, &fx) in changes.touched.iter().zip(changes.effects.iter()) {
            if store.is_live(id) && fx != nve_abi::table::effect::COMPOSITE {
                let _ = self.stylesheet.resolve_node(store, id);
            }
        }
    }

    fn sync_event_state(&mut self, store: &NodeStore, changes: &ChangeSet) {
        for &removed in &changes.removed {
            self.input_sync.borrow_mut().remove(removed);
        }
        let mut state = self.event_state.borrow_mut();
        state.sync(store, changes, |_, _, _| {});
        #[cfg(target_arch = "wasm32")]
        if let Some(ref mut delegator) = self.delegated_events {
            delegator.sync_listeners(&state);
        }
    }
}

impl StoreBackend for WebBackend {
    fn name(&self) -> &'static str {
        "web"
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        self.surface_id.set(surface_id);
        self.viewport = viewport;
    }

    fn commit(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
    ) -> Result<BackendApplyStats, BackendError> {
        for &removed in &changes.removed {
            self.patch_css.remove(&removed);
        }

        #[cfg(not(target_arch = "wasm32"))]
        let stats = self.renderer.commit(store, changes)?;
        #[cfg(target_arch = "wasm32")]
        let mut stats = {
            let m = self.renderer.commit_core(store, changes);
            BackendApplyStats {
                op_count: m,
                mutation_count: m,
                ..Default::default()
            }
        };

        self.sync_event_state(store, changes);

        #[cfg(feature = "hydrate")]
        if !self.pre_hydration_events.is_empty() {
            let drained = std::mem::take(&mut self.pre_hydration_events);
            let valid = drain_valid_pre_hydration_events(drained, |id| store.is_live(id));
            let state = self.event_state.borrow();
            let mut evs = self.pending_events.borrow_mut();
            let sid = self.surface_id.get();
            for captured in &valid {
                let target_id = resolve_replay_target(store, &state, captured);
                push_pre_hydration_event(captured, sid, TimestampUs(0), target_id, &mut evs);
                self.hydration_report.replayed_events += 1;
            }
        }

        #[cfg(feature = "hydrate")]
        if !self.pending_ssr_nodes.is_empty() {
            let ssr_nodes = std::mem::take(&mut self.pending_ssr_nodes);
            for (node_id, expected_tag) in ssr_nodes {
                if !store.is_live(node_id) {
                    self.hydration_report.replaced_nodes += 1;
                    self.hydration_report.mismatches.push(HydrationMismatch {
                        node_id,
                        reason: format!("SSR <{expected_tag}> missing from live NodeStore"),
                    });
                    continue;
                }
                let (actual_tag, _) = self.renderer.registry().resolve_tag(store, node_id);
                if actual_tag.eq_ignore_ascii_case(&expected_tag) {
                    self.hydration_report.adopted_nodes += 1;
                } else {
                    self.hydration_report.replaced_nodes += 1;
                    self.hydration_report.mismatches.push(HydrationMismatch {
                        node_id,
                        reason: format!("SSR <{expected_tag}> != live <{actual_tag}>"),
                    });
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            if self.mount_root.is_some() {
                let plugin_muts = self.commit_live_dom(store, changes);
                stats.op_count += plugin_muts;
                stats.mutation_count += plugin_muts;
            } else {
                let plugin_muts = self.renderer.commit_plugins(store, changes, None, &[]);
                stats.op_count += plugin_muts;
                stats.mutation_count += plugin_muts;
                self.warm_stylesheet(store, changes);
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            // Keep stylesheet rules warm for newly created / non-composite-touched nodes.
            self.warm_stylesheet(store, changes);
        }

        Ok(stats)
    }

    fn drain_events(&mut self, out: &mut EventBatchBuf) {
        {
            let mut pending = self.pending_events.borrow_mut();
            if !pending.is_empty() {
                out.extend_from_validated(pending.as_validated());
                pending.clear();
            }
        }
        if !self.renderer.events.is_empty() {
            out.extend_from_validated(self.renderer.events.as_validated());
            self.renderer.events.clear();
        }
    }

    fn reset(&mut self) {
        self.renderer.reset();
        self.stylesheet.clear();
        self.patch_css.clear();
        self.pending_events.borrow_mut().clear();
        {
            let mut state = self.event_state.borrow_mut();
            state.reset();
            #[cfg(target_arch = "wasm32")]
            if let Some(ref mut delegator) = self.delegated_events {
                delegator.sync_listeners(&state);
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.clear_live_dom();
    }
}

#[cfg(feature = "hydrate")]
fn resolve_replay_target(
    store: &NodeStore,
    state: &WebEventState,
    ev: &CapturedPreHydrationEvent,
) -> NodeId {
    let mut cur = Some(ev.node_id());
    while let Some(nid) = cur {
        let matched = match ev {
            CapturedPreHydrationEvent::Press { .. } => {
                state.listens::<Interaction>(nid, InteractionEvent::Press)
            }
            CapturedPreHydrationEvent::KeyDown { .. } => {
                state.listens::<Interaction>(nid, InteractionEvent::KeyDown)
            }
            CapturedPreHydrationEvent::Submit { .. } => {
                state.is_input(nid) || state.listens::<TextInput>(nid, TextInputEvent::Submit)
            }
        };
        if matched {
            return nid;
        }
        cur = store.parent(nid);
    }
    ev.node_id()
}
