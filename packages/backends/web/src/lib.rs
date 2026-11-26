//! Web DOM / WASM RenderBackend consuming the exact same `nve-abi` `CommandBuffer`.
//!
//! - **Option 5 Hybrid CSS**: static `.nve` base reset (`NVE_BASE_RESET_CSS`), bounded
//!   content-addressed structural stylesheet cache (`.s<base36(fnv1a64)>`), zero-serialization
//!   `@media` responsive rules (`.nve-m-<8hex>`), volatile/continuous inline lane, `.nve-tl`
//!   leaf-text inheritance, and deferred once-per-frame style flushing.
//! - **Keyed Minimal-Diff `SetChildren`**: O(N) head/tail-trimmed + LIS DOM reconciler preserving
//!   element identity, focus, `<input>` cursor/IME, and `ScrollView` scroll position.
//! - **Semantic HTML5 SSR**: `render_ssr()` and `render_ssr_document()` mapping semantic ARIA/link
//!   props onto real HTML5 tags with deterministic CSS rules and opt-in `<script data-nve-replay>`.
//! - **Zero-Flicker Hydration & Root Event Delegation**: `hydrate_into()` adopts SSR DOM nodes
//!   in-place with a single root delegated listener set, 3-way `StateReconciler` control
//!   preservation, atomic pre-hydration replay, and structured `HydrationMismatch` reports.

pub mod canvas;
mod css;
mod dom;
mod event_replay;
mod events;
mod frame;
mod hot_patch;
mod html;
#[cfg(feature = "hydrate")]
mod hydrate;
mod input_attrs;
mod layout_ext_css;
mod media_css;
#[cfg(any(
    feature = "nv-blur",
    feature = "nv-webview",
    feature = "nv-video",
    feature = "nv-web-element",
    feature = "camera",
))]
pub mod native_view;
mod paint_svg;
mod raw_style;
mod reconcile;
mod semantic_attrs;
#[cfg(feature = "ssr")]
mod snapshot;
#[cfg(feature = "ssr")]
mod ssg;
#[cfg(feature = "ssr")]
mod ssr;
#[cfg(feature = "ssr")]
pub mod streaming;
#[cfg(feature = "ssr")]
mod streaming_live;
mod state_css;
mod style_hash;
mod stylesheet;
#[cfg(target_arch = "wasm32")]
mod stylesheet_wasm;
mod transition_css;
mod tree;
mod visual_viewport;

#[cfg(test)]
mod css_format_tests;
#[cfg(all(test, feature = "ssr"))]
mod correctness_tests;
#[cfg(test)]
mod native_view_tests;
#[cfg(all(test, feature = "ssr"))]
mod snapshot_tests;
#[cfg(test)]
mod ssr_hydrate_tests;
#[cfg(all(test, feature = "ssr"))]
mod streaming_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transition_tests;

pub use css::{
    canonical_f32_bits, component_to_html_tag, is_semantic_tag_prop, layout_to_css,
    layout_to_css_into, resolve_semantic_tag, style_to_css, style_to_css_into, write_compact_color,
    write_compact_edges, MirrorElement, SemanticTag, NVE_BASE_RESET_CSS,
};
pub use dom::{
    is_truthy_raw_prop, map_a11y_or_input_attr, raw_prop_route, MappedHtmlAttr, RawPropRoute,
};
pub use event_replay::{
    drain_valid_pre_hydration_events, pre_hydration_event_to_native,
    pre_hydration_replay_script_tag, CapturedPreHydrationEvent, MAX_REPLAY_AGE_MS,
    MAX_REPLAY_QUEUE_LEN, PRE_HYDRATION_REPLAY_SCRIPT_TAG,
};
pub use events::{
    resolve_delegated_node, should_intercept_anchor_click, ImeCompositionGuard, InputSyncTracker,
    NodeEventMeta,
};
pub use visual_viewport::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, KeyboardInsets,
    KEYBOARD_INSETS_EVENT_NAME, VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};
#[cfg(feature = "hydrate")]
pub use hydrate::{HydrationMismatch, HydrationReport};
pub use layout_ext_css::{write_compact_layout_ext_into, write_compact_transform_into};
pub use media_css::{
    compile_node_media_rules, format_media_class, write_responsive_patch_decls,
    write_style_patch_decls, CompiledMediaRule,
};
pub use paint_svg::write_compact_background_fill_into;
pub use state_css::{
    compile_node_state_rules, compile_node_state_rules_with_hover_guard, format_cursor_rule,
    format_state_class, CompiledStateRule,
};
#[cfg(any(
    feature = "nv-blur",
    feature = "nv-webview",
    feature = "nv-video",
    feature = "nv-web-element",
    feature = "camera",
))]
pub use native_view::{
    DecodedNativeView, NativeCommand, NativeCommandDecoder, NativePropDecoder, NativeProps,
    NativeValueRef, NativeViewCodecError, NativeViewNodeState, TypedNativeViewCommand,
    TypedNativeViewProps,
};
#[cfg(feature = "nv-blur")]
pub use native_view::{BlurMaterial, BlurSurfaceProps};
#[cfg(feature = "nv-video")]
pub use native_view::{VideoPlayerCommand, VideoPlayerProps};
#[cfg(feature = "nv-web-element")]
pub use native_view::WebElementProps;
#[cfg(feature = "nv-webview")]
pub use native_view::{WebViewCommand, WebViewProps};
#[cfg(feature = "camera")]
pub use native_view::{
    CameraNodeState, CameraPreviewCommand, CameraPreviewFacing, CameraPreviewProps,
    StabilizationMode, TorchMode,
};
pub use raw_style::{
    is_image_fit_prop, is_raw_style_prop, is_safe_css_value, raw_prop_to_object_fit, RawStyleProps,
};
pub use reconcile::{plan_keyed_child_ops, ChildDiffOp};
pub use semantic_attrs::{
    canonical_html_attr_name, is_tag_only_raw_prop, should_close_dialog_on_backdrop_click,
};
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
    inject_ssg_into_shell_html, inject_ssg_with_scripts_into_shell_html, prerender_nve_app_frame0,
    WebSsgDocumentOptions,
};
#[cfg(feature = "ssr")]
pub use ssr::{SsrDocumentOptions, SsrOutput};
#[cfg(feature = "ssr")]
pub use streaming::{
    decode_form_urlencoded, decode_form_urlencoded_map, encode_http_chunk,
    sanitize_stream_slot_id, scoped_slot_hiding_css, slot_placeholder_html, stream_slot_name,
    streaming_http_headers, FormActionResponse, LiveUpdateChunk, StreamingDocumentOptions,
    StreamingError, StreamingMode, StreamingSsrSession, ZeroJsLiveConfig, ZeroJsLiveSession,
    DEFAULT_MAX_LIVE_UPDATES, HTTP_CHUNK_TERMINATOR, NVE_LIVE_SINK_IFRAME_HTML,
    NVE_LIVE_SINK_IFRAME_NAME, NVE_STREAM_KEEPALIVE_COMMENT, NVE_STREAM_SLOT_PREFIX,
};
pub use style_hash::{
    format_structural_class, hash_structural_style, is_inheriting_leaf_text,
    is_volatile_dimension, is_volatile_opacity, VOLATILE_DIMENSION_LIMIT,
};
pub use stylesheet::{
    resolve_node_style, CachedRule, ResolvedNodeStyle, StyleSheetCache,
    INCREMENTAL_RULE_BATCH_THRESHOLD, MAX_DYNAMIC_RULES,
};
pub use transition_css::{
    compile_node_transitions, format_transition_class, CompiledTransitionRule,
};

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as _;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nve_abi::hash::HashMap;
#[cfg(feature = "hydrate")]
use nve_abi::hash::HashSet;
#[cfg(target_arch = "wasm32")]
use nve_abi::Dimension;
use nve_abi::{ComponentKind, EventMask, NativeEvent, NodeId, SurfaceId, ROOT_NODE_ID};
use nve_engine::Viewport;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::events::RootEventDelegator;
use crate::visual_viewport::KeyboardInsetsCallback;
#[cfg(target_arch = "wasm32")]
use crate::visual_viewport::VisualViewportTracker;
#[cfg(feature = "hydrate")]
use crate::hydrate::HydratedControlState;
#[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
use crate::hydrate::NativeSsrNode;

pub(crate) type RenderCallback = Rc<dyn Fn()>;

/// Native-testable Web DOM backend (mirrors exact DOM ops; mounts real `web_sys::Element`s on `wasm32`).
pub struct WebBackend {
    pub(crate) surface_id: Rc<Cell<SurfaceId>>,
    pub(crate) viewport: Viewport,
    pub(crate) frame_counter: u64,
    pub(crate) nodes: HashMap<NodeId, MirrorElement>,
    #[cfg(any(
        feature = "nv-blur",
        feature = "nv-webview",
        feature = "nv-video",
        feature = "nv-web-element",
        feature = "camera",
    ))]
    pub(crate) native_views: HashMap<NodeId, NativeViewNodeState>,
    pub(crate) parent_of: HashMap<NodeId, NodeId>,
    pub(crate) stylesheet: StyleSheetCache,
    pub(crate) events: Rc<RefCell<Vec<NativeEvent>>>,
    pub(crate) on_event: Rc<RefCell<Option<RenderCallback>>>,
    pub(crate) node_event_meta: Rc<RefCell<HashMap<NodeId, NodeEventMeta>>>,
    pub(crate) keyboard_insets: Rc<Cell<KeyboardInsets>>,
    pub(crate) on_keyboard_insets: Rc<RefCell<Option<KeyboardInsetsCallback>>>,
    #[cfg(feature = "hydrate")]
    pub(crate) hydrating: bool,
    #[cfg(feature = "hydrate")]
    pub(crate) hydrated_dom_tags: HashMap<NodeId, String>,
    #[cfg(feature = "hydrate")]
    pub(crate) hydrated_inline_nodes: HashSet<NodeId>,
    #[cfg(feature = "hydrate")]
    pub(crate) hydrated_controls: HashMap<NodeId, HydratedControlState>,
    #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
    pub(crate) hydrated_children: HashMap<NodeId, Vec<NodeId>>,
    #[cfg(feature = "hydrate")]
    pub(crate) pending_pre_hydration_events: Vec<CapturedPreHydrationEvent>,
    #[cfg(feature = "hydrate")]
    pub(crate) pending_responsive_dirty: Vec<NodeId>,
    #[cfg(feature = "hydrate")]
    pub(crate) hydration_report: HydrationReport,
    #[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
    pub(crate) native_ssr_nodes: HashMap<NodeId, NativeSsrNode>,
    #[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
    pub(crate) next_native_instance_id: u64,
    #[cfg(target_arch = "wasm32")]
    pub(crate) attr_scratch: String,
    #[cfg(target_arch = "wasm32")]
    pub(crate) dom_nodes: HashMap<NodeId, web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) ime_guards: Rc<RefCell<HashMap<NodeId, ImeCompositionGuard>>>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) input_sync: Rc<RefCell<InputSyncTracker>>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) root_delegator: Option<RootEventDelegator>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) window_keydown: Option<Closure<dyn FnMut(web_sys::KeyboardEvent)>>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) visual_viewport_tracker: Option<VisualViewportTracker>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for WebBackend {
    fn drop(&mut self) {
        if let Some(window_keydown) = self.window_keydown.take()
            && let Some(window) = web_sys::window()
        {
            let _ = window.remove_event_listener_with_callback(
                "keydown",
                window_keydown.as_ref().unchecked_ref(),
            );
        }
        // Detach `visualViewport` listeners and all delegated container listeners cleanly.
        self.visual_viewport_tracker = None;
        self.root_delegator = None;
        self.dom_nodes.clear();
    }
}

impl Default for WebBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebBackend {
    pub fn new() -> Self {
        let mut nodes = HashMap::default();
        nodes.insert(ROOT_NODE_ID, MirrorElement::new(ComponentKind::Root));
        let mut meta = HashMap::default();
        meta.insert(
            ROOT_NODE_ID,
            NodeEventMeta {
                kind: ComponentKind::Root,
                mask: EventMask::NONE,
            },
        );
        Self {
            surface_id: Rc::new(Cell::new(1)),
            viewport: Viewport::default(),
            frame_counter: 0,
            nodes,
            #[cfg(any(
                feature = "nv-blur",
                feature = "nv-webview",
                feature = "nv-video",
                feature = "nv-web-element",
                feature = "camera",
            ))]
            native_views: HashMap::default(),
            parent_of: HashMap::default(),
            stylesheet: StyleSheetCache::new(),
            events: Rc::new(RefCell::new(Vec::new())),
            on_event: Rc::new(RefCell::new(None)),
            node_event_meta: Rc::new(RefCell::new(meta)),
            keyboard_insets: Rc::new(Cell::new(KeyboardInsets::HIDDEN)),
            on_keyboard_insets: Rc::new(RefCell::new(None)),
            #[cfg(feature = "hydrate")]
            hydrating: false,
            #[cfg(feature = "hydrate")]
            hydrated_dom_tags: HashMap::default(),
            #[cfg(feature = "hydrate")]
            hydrated_inline_nodes: HashSet::default(),
            #[cfg(feature = "hydrate")]
            hydrated_controls: HashMap::default(),
            #[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
            hydrated_children: HashMap::default(),
            #[cfg(feature = "hydrate")]
            pending_pre_hydration_events: Vec::new(),
            #[cfg(feature = "hydrate")]
            pending_responsive_dirty: Vec::new(),
            #[cfg(feature = "hydrate")]
            hydration_report: HydrationReport::default(),
            #[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
            native_ssr_nodes: HashMap::default(),
            #[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
            next_native_instance_id: 1,
            #[cfg(target_arch = "wasm32")]
            attr_scratch: String::with_capacity(64),
            #[cfg(target_arch = "wasm32")]
            dom_nodes: HashMap::default(),
            #[cfg(target_arch = "wasm32")]
            ime_guards: Rc::new(RefCell::new(HashMap::default())),
            #[cfg(target_arch = "wasm32")]
            input_sync: Rc::new(RefCell::new(InputSyncTracker::new())),
            #[cfg(target_arch = "wasm32")]
            root_delegator: None,
            #[cfg(target_arch = "wasm32")]
            window_keydown: None,
            #[cfg(target_arch = "wasm32")]
            visual_viewport_tracker: None,
        }
    }

    #[cfg(not(any(
        feature = "nv-blur",
        feature = "nv-webview",
        feature = "nv-video",
        feature = "nv-web-element",
        feature = "camera",
    )))]
    #[inline]
    pub(crate) fn effective_html_tag<'a>(&'a self, _node_id: NodeId, el: &'a MirrorElement) -> &'a str {
        el.tag
    }

    /// Returns the most recently configured [`Viewport`] for this backend.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Returns a read-only handle to the Option 5 Hybrid [`StyleSheetCache`].
    pub fn stylesheet(&self) -> &StyleSheetCache {
        &self.stylesheet
    }

    /// Mounts `ROOT_NODE_ID` into an existing browser DOM container (e.g. `#app`),
    /// adopts the static `.nve` base reset stylesheet, and attaches a single root event delegator
    /// plus a global `window` `keydown` listener.
    #[cfg(target_arch = "wasm32")]
    pub fn mount_into(container: web_sys::Element) -> Self {
        let mut backend = Self::new();
        container.set_text_content(None);
        backend.attr_scratch.clear();
        let _ = write!(&mut backend.attr_scratch, "{ROOT_NODE_ID}");
        let _ = container.set_attribute("data-nve-id", &backend.attr_scratch);
        if let Some(root) = backend.nodes.get_mut(&ROOT_NODE_ID) {
            root.layout.width = Dimension::Percent(100.0);
            root.layout.height = Dimension::Percent(100.0);
        }
        backend.dom_nodes.insert(ROOT_NODE_ID, container.clone());
        backend.flush_dirty_styles(&[ROOT_NODE_ID]);
        backend.attach_root_event_delegator(container);
        backend.attach_window_keydown_listener();
        #[cfg(feature = "hydrate")]
        {
            let _ = crate::event_replay::drain_browser_pre_hydration_events();
        }
        backend.attach_visual_viewport_listener();
        backend
    }

    pub fn set_render_callback<F: Fn() + 'static>(&mut self, callback: F) {
        *self.on_event.borrow_mut() = Some(Rc::new(callback));
    }

    pub fn with_render_callback<F: Fn() + 'static>(mut self, callback: F) -> Self {
        self.set_render_callback(callback);
        self
    }

    pub fn event_queue(&self) -> Rc<RefCell<Vec<NativeEvent>>> {
        Rc::clone(&self.events)
    }

    pub fn push_event(&self, event: NativeEvent) {
        self.events.borrow_mut().push(event);
        let cb = self.on_event.borrow().clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    pub fn node(&self, node_id: NodeId) -> Option<&MirrorElement> {
        self.nodes.get(&node_id)
    }

    pub fn iter_nodes(&self) -> impl Iterator<Item = (NodeId, &MirrorElement)> {
        self.nodes.iter().map(|(&id, el)| (id, el))
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn event_mask_of(&self, node_id: NodeId) -> u64 {
        self.nodes
            .get(&node_id)
            .map(|n| n.event_mask)
            .unwrap_or(EventMask::NONE)
    }

    pub(crate) fn remove_node_event_meta(&mut self, node_id: NodeId) {
        self.node_event_meta.borrow_mut().remove(&node_id);
        #[cfg(target_arch = "wasm32")]
        {
            self.ime_guards.borrow_mut().remove(&node_id);
            self.input_sync.borrow_mut().remove(node_id);
        }
    }

    pub(crate) fn queue_leaf_text_children(&self, parent_id: NodeId, dirty_styles: &mut Vec<NodeId>) {
        if let Some(parent) = self.nodes.get(&parent_id)
            && parent.kind == ComponentKind::Text
        {
            for &cid in &parent.children {
                if self
                    .nodes
                    .get(&cid)
                    .is_some_and(|c| c.kind == ComponentKind::Text && c.children.is_empty())
                {
                    dirty_styles.push(cid);
                }
            }
        }
    }
}
