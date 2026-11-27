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
mod hot_patch;
pub mod html;
#[cfg(feature = "hydrate")]
pub mod hydrate;
#[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
mod hydrate_native;
#[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
mod hydrate_native_parse;
#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
mod hydrate_wasm;
pub mod input_attrs;
pub mod layout_ext_css;
pub mod media_css;
pub mod sanitize;
#[cfg(feature = "ssr")]
pub mod snapshot;
#[cfg(feature = "ssr")]
pub mod ssg;
#[cfg(feature = "ssr")]
pub mod ssr;
#[cfg(feature = "ssr")]
pub mod ssr_stream;
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
#[cfg(all(test, feature = "hydrate"))]
mod hydrate_tests;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Optional host callback invoked when a live browser event is pushed.
pub type RenderCallback = Rc<dyn Fn()>;

use nve_abi::hash::HashMap;
use nve_abi::{NativeEvent, NodeId, SurfaceId};
use nve_components::keyboard::KeyboardInsets;
use nve_components::tables::{Interaction, Link, TextInput};
use nve_engine::{BackendApplyStats, BackendError, StoreBackend, Viewport};
use nve_store::{ChangeSet, NodeStore, TableEventBus};

pub use css::{canonical_f32_bits, write_compositor_css, write_store_node_css, NVE_BASE_RESET_CSS};
pub use event_replay::{
    drain_valid_pre_hydration_events, pre_hydration_event_to_native,
    pre_hydration_replay_script_tag, CapturedPreHydrationEvent, PRE_HYDRATION_REPLAY_SCRIPT_TAG,
};
pub use events::{
    is_hard_nav_anchor, resolve_delegated_node,
    should_intercept_anchor_click, ImeCompositionGuard, InputSyncTracker, NodeEventMeta,
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
pub use ssr_stream::{PendingStreamSlot, SsrDeltaChunk, SsrStyleTracker, StreamingShellOutput};
#[cfg(feature = "ssr")]
pub use streaming::{
    sanitize_stream_slot_id, scoped_slot_hiding_css,
    slot_placeholder_html, stream_slot_name, streaming_http_headers, FormActionResponse,
    LiveUpdateChunk, StreamingDocumentOptions, StreamingError, StreamingMode, StreamingSsrSession,
    ZeroJsLiveConfig, ZeroJsLiveSession, DEFAULT_MAX_LIVE_UPDATES, NVE_LIVE_SINK_IFRAME_HTML,
    NVE_LIVE_SINK_IFRAME_NAME, NVE_STREAM_KEEPALIVE_COMMENT, NVE_STREAM_SLOT_PREFIX,
};
pub use stylesheet::{ResolvedNodeStyle, StyleSheetCache, MAX_DYNAMIC_RULES};
pub use table_system::{
    is_safe_custom_tag, is_safe_web_attr, is_safe_web_url, RealizedWebNode, WebStoreRenderer,
    WebSystemCommitCtx, WebTableRegistry, WebTableSystem,
};
#[cfg(target_arch = "wasm32")]
pub use table_system::ClickInterceptor;
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
    pub(crate) pending_events: Rc<RefCell<Vec<NativeEvent>>>,
    pub(crate) node_event_meta: Rc<RefCell<HashMap<NodeId, NodeEventMeta>>>,
    pub(crate) input_sync: Rc<RefCell<InputSyncTracker>>,
    pub(crate) ime_guard: Rc<ImeCompositionGuard>,
    pub(crate) keyboard_insets: Rc<Cell<KeyboardInsets>>,
    pub(crate) on_keyboard_insets: Rc<RefCell<Option<visual_viewport::KeyboardInsetsCallback>>>,
    pub(crate) on_event: Rc<RefCell<Option<RenderCallback>>>,
    #[cfg(feature = "hydrate")]
    pub(crate) hydration_report: HydrationReport,
    #[cfg(feature = "hydrate")]
    pub(crate) pre_hydration_events: Vec<CapturedPreHydrationEvent>,
    #[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
    pub(crate) native_dom: Option<hydrate_native::NativeDom>,
    #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
    pub(crate) hydrate_root: Option<web_sys::Element>,
    #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
    pub(crate) deferred_controls: nve_abi::hash::HashSet<NodeId>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) mount_root: Option<web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) live_elements: HashMap<NodeId, web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) delegated_events: Option<events::RootEventDelegator>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) click_interceptors: Rc<RefCell<table_system::ClickInterceptors>>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) visual_viewport_tracker: Option<visual_viewport::VisualViewportTracker>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) created_scratch: nve_abi::hash::HashSet<nve_store::NodeId>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) retagged_scratch: Vec<nve_store::NodeId>,
}

impl Default for WebBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebBackend {
    pub fn new() -> Self {
        Self {
            surface_id: Rc::new(Cell::new(1)),
            viewport: Viewport::default(),
            renderer: WebStoreRenderer::new(),
            stylesheet: StyleSheetCache::new(),
            patch_css: HashMap::default(),
            pending_events: Rc::new(RefCell::new(Vec::new())),
            node_event_meta: Rc::new(RefCell::new(HashMap::default())),
            input_sync: Rc::new(RefCell::new(InputSyncTracker::new())),
            ime_guard: Rc::new(ImeCompositionGuard::new()),
            keyboard_insets: Rc::new(Cell::new(KeyboardInsets::default())),
            on_keyboard_insets: Rc::new(RefCell::new(None)),
            on_event: Rc::new(RefCell::new(None)),
            #[cfg(feature = "hydrate")]
            hydration_report: HydrationReport::default(),
            #[cfg(feature = "hydrate")]
            pre_hydration_events: Vec::new(),
            #[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
            native_dom: None,
            #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
            hydrate_root: None,
            #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
            deferred_controls: nve_abi::hash::HashSet::default(),
            #[cfg(target_arch = "wasm32")]
            mount_root: None,
            #[cfg(target_arch = "wasm32")]
            live_elements: HashMap::default(),
            #[cfg(target_arch = "wasm32")]
            delegated_events: None,
            #[cfg(target_arch = "wasm32")]
            click_interceptors: Rc::default(),
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
    pub fn table_events(&self) -> &TableEventBus {
        &self.renderer.events
    }

    #[inline]
    pub fn table_events_mut(&mut self) -> &mut TableEventBus {
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

    #[inline]
    pub fn push_event(&mut self, event: NativeEvent) {
        self.pending_events.borrow_mut().push(event);
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

    fn sync_event_meta(&self, store: &NodeStore, changes: &ChangeSet) {
        let mut meta = self.node_event_meta.borrow_mut();
        for &removed in &changes.removed {
            meta.remove(&removed.raw());
            self.input_sync.borrow_mut().remove(removed.raw());
        }
        for &id in changes.created.iter().chain(changes.touched.iter()) {
            if !store.is_live(id) {
                continue;
            }
            let mask = u64::from(
                store
                    .try_row::<Interaction>(id)
                    .map_or(0, |i| i.event_mask()),
            );
            let is_input = store.has::<TextInput>(id);
            let is_link = store.has::<Link>(id);
            if mask != 0 || is_input || is_link {
                meta.insert(
                    id.raw(),
                    NodeEventMeta {
                        is_input,
                        is_link,
                        mask,
                    },
                );
            } else {
                meta.remove(&id.raw());
            }
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
            self.patch_css.remove(&removed.raw());
        }
        #[cfg(feature = "hydrate")]
        let hydration = self.pair_pending_hydration(store);

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

        self.sync_event_meta(store, changes);

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
            #[cfg(feature = "hydrate")]
            self.sync_native_dom(store, changes, hydration.as_ref());
        }

        #[cfg(feature = "hydrate")]
        self.finish_hydration(store, hydration);
        Ok(stats)
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        std::mem::take(&mut *self.pending_events.borrow_mut())
    }

    fn reset(&mut self) {
        self.renderer.reset();
        self.stylesheet.clear();
        self.patch_css.clear();
        self.pending_events.borrow_mut().clear();
        self.node_event_meta.borrow_mut().clear();
        #[cfg(target_arch = "wasm32")]
        self.clear_live_dom();
    }
}
