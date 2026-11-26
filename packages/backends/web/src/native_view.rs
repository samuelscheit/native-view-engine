//! Host-side `NativeView` mount adapters for `WebBackend`: zero-serialization in-process typed
//! props/commands (`TypedNativeViewProps`, `NativeCommand`, `TypedNativeViewCommand`) + canonical
//! `NVNP`/`NVNC` FFI decoding via `nve_components::native_view`, allowlisted `"web.<tag>"` DOM
//! passthrough, CSS `BlurSurface`, sandboxed `WebView` `<iframe>`, `<video>` `VideoPlayer`, and
//! opt-in `CameraPreview`.

#[cfg(feature = "camera")]
#[path = "native_view_camera.rs"]
mod camera;
#[path = "native_view_dom.rs"]
mod dom;
#[path = "native_view_security.rs"]
mod security;

#[cfg(feature = "camera")]
pub use camera::*;
#[cfg(all(target_arch = "wasm32", feature = "camera"))]
pub(crate) use dom::sync_bool_attr;
#[cfg(target_arch = "wasm32")]
pub(crate) use dom::sync_native_view_dom_attrs;
pub(crate) use dom::write_native_view_html_attrs;
#[cfg(feature = "nv-video")]
use dom::apply_video_player_command;
#[cfg(feature = "nv-webview")]
use dom::apply_web_view_command;
use dom::dispatch_command_source;
pub use nve_components::native_view::{
    DecodeNativeCommand, DecodeNativeProps, DecodedWebElementProps as WebElementProps,
    NativeCommand, NativeCommandDecoder, NativeCommandSource, NativePropDecoder, NativePropSource,
    NativeProps, NativeValueRef, NativeViewCodecError, TypedNativeViewCommand,
    TypedNativeViewProps, MAX_NATIVE_CMD_ARGS, MAX_NATIVE_PROP_FIELDS, NATIVE_CMD_HEADER_SIZE,
    NATIVE_CMD_MAGIC, NATIVE_CMD_VERSION, NATIVE_PROPS_HEADER_SIZE, NATIVE_PROPS_MAGIC,
    NATIVE_PROPS_VERSION, NATIVE_VIEW_KIND_PROP_ALIAS, NATIVE_VIEW_KIND_PROP_KEY,
    NATIVE_VIEW_PROPS_PROP_ALIAS, NATIVE_VIEW_PROPS_PROP_KEY,
};
#[cfg(feature = "nv-blur")]
pub use nve_components::native_view::{BlurMaterial, BlurSurfaceProps};
#[cfg(feature = "nv-video")]
pub use nve_components::native_view::{VideoPlayerCommand, VideoPlayerProps};
#[cfg(feature = "nv-webview")]
pub use nve_components::native_view::{WebViewCommand, WebViewProps};
pub use security::*;

#[cfg(feature = "nv-blur")]
use nve_abi::num::write_f32;
#[cfg(feature = "nv-blur")]
use nve_abi::ColorRgba;
use nve_abi::{ComponentKind, NodeId, RawPropValue};

use crate::css::MirrorElement;
use crate::WebBackend;

#[cfg(feature = "nv-blur")]
pub const KIND_BLUR_SURFACE: &str = "nve.blur_surface";
#[cfg(feature = "nv-blur")]
pub const KIND_BLUR_ALIAS: &str = "nve.blur";
#[cfg(feature = "nv-webview")]
pub const KIND_WEB_VIEW: &str = "nve.web_view";
#[cfg(feature = "nv-webview")]
pub const KIND_WEB_VIEW_ALIAS: &str = "nve.webview";
#[cfg(feature = "nv-video")]
pub const KIND_VIDEO_PLAYER: &str = "nve.video_player";
#[cfg(feature = "nv-video")]
pub const KIND_VIDEO_ALIAS: &str = "nve.video";
#[cfg(feature = "nv-webview")]
const WEBVIEW_SANDBOX_ATTR: &str = "allow-scripts allow-same-origin allow-forms allow-popups";

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedNativeView {
    #[cfg(feature = "nv-blur")]
    BlurSurface(BlurSurfaceProps),
    #[cfg(feature = "nv-webview")]
    WebView(WebViewProps),
    #[cfg(feature = "nv-video")]
    VideoPlayer(VideoPlayerProps),
    #[cfg(feature = "camera")]
    CameraPreview(CameraPreviewProps),
    #[cfg(feature = "nv-web-element")]
    WebElement(WebElementProps),
}

/// Host-side state tracked per mounted `ComponentKind::Custom` native view node.
#[derive(Debug, Clone)]
pub struct NativeViewNodeState {
    pub kind: String,
    pub resolved_tag: String,
    pub static_tag: Option<&'static str>,
    pub decoded: Option<DecodedNativeView>,
    pub extra_css: String,
    #[cfg(feature = "nv-webview")]
    pub current_url: Option<String>,
    #[cfg(feature = "nv-webview")]
    pub reload_count: u32,
    #[cfg(feature = "nv-webview")]
    pub go_back_count: u32,
    #[cfg(feature = "nv-webview")]
    pub go_forward_count: u32,
    #[cfg(feature = "nv-video")]
    pub playing: bool,
    #[cfg(feature = "nv-video")]
    pub position_ms: u64,
    #[cfg(feature = "nv-video")]
    pub volume: f32,
    #[cfg(feature = "camera")]
    pub camera: CameraNodeState,
    #[cfg(all(target_arch = "wasm32", feature = "nv-web-element"))]
    pub(crate) applied_attrs: Vec<String>,
    #[cfg(all(target_arch = "wasm32", feature = "camera"))]
    pub(crate) camera_guard: Option<std::rc::Rc<CameraStreamGuard>>,
}

impl NativeViewNodeState {
    fn new(kind: String) -> Self {
        Self {
            kind,
            resolved_tag: "div".to_owned(),
            static_tag: Some("div"),
            decoded: None,
            extra_css: String::new(),
            #[cfg(feature = "nv-webview")]
            current_url: None,
            #[cfg(feature = "nv-webview")]
            reload_count: 0,
            #[cfg(feature = "nv-webview")]
            go_back_count: 0,
            #[cfg(feature = "nv-webview")]
            go_forward_count: 0,
            #[cfg(feature = "nv-video")]
            playing: false,
            #[cfg(feature = "nv-video")]
            position_ms: 0,
            #[cfg(feature = "nv-video")]
            volume: 1.0,
            #[cfg(feature = "camera")]
            camera: CameraNodeState::default(),
            #[cfg(all(target_arch = "wasm32", feature = "nv-web-element"))]
            applied_attrs: Vec::new(),
            #[cfg(all(target_arch = "wasm32", feature = "camera"))]
            camera_guard: None,
        }
    }

    pub(crate) fn set_tag(&mut self, tag: String, static_tag: Option<&'static str>) {
        self.resolved_tag = tag;
        self.static_tag = static_tag;
    }
}

#[cfg(feature = "nv-blur")]
fn format_blur_css(props: BlurSurfaceProps) -> String {
    let (base_px, default_alpha) = blur_material_css_params(props.material);
    let intensity = if props.intensity.is_finite() {
        props.intensity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let blur_px = ((base_px * intensity) * 100.0).round() / 100.0;
    let mut css = String::with_capacity(96);
    for prop in ["backdrop-filter:blur(", "-webkit-backdrop-filter:blur("] {
        css.push_str(prop);
        write_f32(&mut css, blur_px);
        css.push_str("px);");
    }
    css.push_str("background-color:");
    match props.tint {
        Some(rgba) => {
            let [r, g, b, a] = rgba.to_be_bytes();
            crate::css::write_rgba(ColorRgba { r, g, b, a }, &mut css);
        }
        None => {
            css.push_str("rgba(255,255,255,");
            css.push_str(default_alpha);
            css.push(')');
        }
    }
    css.push(';');
    css
}

fn resolve_kind_defaults(state: &mut NativeViewNodeState) {
    match state.kind.as_str() {
        #[cfg(feature = "nv-blur")]
        KIND_BLUR_SURFACE | KIND_BLUR_ALIAS => {
            state.set_tag("div".to_owned(), Some("div"));
            if state.extra_css.is_empty() {
                state.extra_css = format_blur_css(BlurSurfaceProps::default());
            }
        }
        #[cfg(feature = "nv-webview")]
        KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => {
            state.set_tag("iframe".to_owned(), Some("iframe"));
        }
        #[cfg(feature = "nv-video")]
        KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => {
            state.set_tag("video".to_owned(), Some("video"));
        }
        #[cfg(feature = "camera")]
        KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => {
            state.set_tag("video".to_owned(), Some("video"));
        }
        #[cfg(feature = "nv-web-element")]
        other => match other.strip_prefix("web.").and_then(validate_web_element_tag) {
            Some((tag, st)) => state.set_tag(tag, st),
            None => state.set_tag("div".to_owned(), Some("div")),
        },
        #[cfg(not(feature = "nv-web-element"))]
        _ => state.set_tag("div".to_owned(), Some("div")),
    }
}

#[cfg(feature = "nv-web-element")]
fn apply_web_element_props(state: &mut NativeViewNodeState, mut p: WebElementProps) {
    p.attrs.retain(|(k, v)| is_safe_web_attr(k, v));
    match validate_web_element_tag(&p.tag) {
        Some((tag, st)) => {
            state.set_tag(tag, st);
            state.decoded = Some(DecodedNativeView::WebElement(p));
        }
        None => {
            state.set_tag("div".to_owned(), Some("div"));
            state.decoded = None;
        }
    }
    state.extra_css.clear();
}

fn apply_prop_source<S: NativePropSource>(state: &mut NativeViewNodeState, src: &S) -> bool {
    let prev_tag = state.resolved_tag.clone();
    match state.kind.as_str() {
        #[cfg(feature = "nv-blur")]
        KIND_BLUR_SURFACE | KIND_BLUR_ALIAS => {
            if let Ok(p) = BlurSurfaceProps::read_props(src) {
                state.extra_css = format_blur_css(p);
                state.decoded = Some(DecodedNativeView::BlurSurface(p));
                state.set_tag("div".to_owned(), Some("div"));
            }
        }
        #[cfg(feature = "nv-webview")]
        KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => {
            if let Ok(p) = WebViewProps::read_props(src) {
                state.current_url = is_safe_web_url(&p.url).then(|| p.url.clone());
                state.decoded = Some(DecodedNativeView::WebView(p));
                state.set_tag("iframe".to_owned(), Some("iframe"));
                state.extra_css.clear();
            }
        }
        #[cfg(feature = "nv-video")]
        KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => {
            if let Ok(p) = VideoPlayerProps::read_props(src) {
                state.playing = p.autoplay;
                state.decoded = Some(DecodedNativeView::VideoPlayer(p));
                state.set_tag("video".to_owned(), Some("video"));
                state.extra_css.clear();
            }
        }
        #[cfg(feature = "camera")]
        KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => {
            if let Ok(p) = CameraPreviewProps::read_props(src) {
                apply_camera_props(state, p);
            }
        }
        #[cfg(feature = "nv-web-element")]
        other if other.starts_with("web.") => {
            if let Ok(p) = WebElementProps::read_props(src) {
                apply_web_element_props(state, p);
            }
        }
        _ => {}
    }
    state.resolved_tag != prev_tag
}

/// Applies zero-serialization [`TypedNativeViewProps`] directly via fast downcast (falling back
/// to the typed [`NativeProps`] field slice without any binary decoding).
fn apply_typed_props_to_state(
    state: &mut NativeViewNodeState,
    props: &TypedNativeViewProps,
) -> bool {
    let prev_tag = state.resolved_tag.clone();
    match state.kind.as_str() {
        #[cfg(feature = "nv-blur")]
        KIND_BLUR_SURFACE | KIND_BLUR_ALIAS => {
            if let Some(&p) = props.downcast_ref::<BlurSurfaceProps>() {
                state.extra_css = format_blur_css(p);
                state.decoded = Some(DecodedNativeView::BlurSurface(p));
                state.set_tag("div".to_owned(), Some("div"));
                return state.resolved_tag != prev_tag;
            }
        }
        #[cfg(feature = "nv-webview")]
        KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => {
            if let Some(p) = props.downcast_ref::<WebViewProps>() {
                state.current_url = is_safe_web_url(&p.url).then(|| p.url.clone());
                state.decoded = Some(DecodedNativeView::WebView(p.clone()));
                state.set_tag("iframe".to_owned(), Some("iframe"));
                state.extra_css.clear();
                return state.resolved_tag != prev_tag;
            }
        }
        #[cfg(feature = "nv-video")]
        KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => {
            if let Some(p) = props.downcast_ref::<VideoPlayerProps>() {
                state.playing = p.autoplay;
                state.decoded = Some(DecodedNativeView::VideoPlayer(p.clone()));
                state.set_tag("video".to_owned(), Some("video"));
                state.extra_css.clear();
                return state.resolved_tag != prev_tag;
            }
        }
        #[cfg(feature = "camera")]
        KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => {
            if let Some(p) = props.downcast_ref::<CameraPreviewProps>() {
                if p.validate().is_ok() {
                    apply_camera_props(state, p.clone());
                }
                return state.resolved_tag != prev_tag;
            }
        }
        #[cfg(feature = "nv-web-element")]
        other if other.starts_with("web.") => {
            if let Some(p) = props.downcast_ref::<nve_components::native_view::WebElementProps>() {
                apply_web_element_props(state, p.to_decoded());
                return state.resolved_tag != prev_tag;
            }
            if let Some(p) = props.downcast_ref::<WebElementProps>() {
                apply_web_element_props(state, p.clone());
                return state.resolved_tag != prev_tag;
            }
        }
        _ => {}
    }
    apply_prop_source(state, props.props())
}

fn apply_nvnp_to_state(state: &mut NativeViewNodeState, bytes: &[u8]) -> bool {
    let Ok(dec) = NativePropDecoder::decode(bytes) else {
        return false;
    };
    apply_prop_source(state, &dec)
}

#[cfg(feature = "camera")]
pub(crate) use crate::html::write_attr as push_escaped_attr;

impl WebBackend {
    pub fn native_view(&self, node_id: NodeId) -> Option<&NativeViewNodeState> {
        self.native_views.get(&node_id)
    }

    #[inline]
    pub(crate) fn effective_html_tag<'a>(&'a self, node_id: NodeId, el: &'a MirrorElement) -> &'a str {
        if el.kind == ComponentKind::Custom
            && let Some(nv) = self.native_views.get(&node_id)
        {
            return nv.resolved_tag.as_str();
        }
        el.tag
    }

    pub(crate) fn init_custom_native_view(&mut self, node_id: NodeId, custom_name: &str) {
        let mut state = NativeViewNodeState::new(custom_name.to_owned());
        resolve_kind_defaults(&mut state);
        if let Some(el) = self.nodes.get_mut(&node_id)
            && let Some(st) = state.static_tag
        {
            el.tag = st;
        }
        self.native_views.insert(node_id, state);
    }

    fn finish_native_view_prop_update(
        &mut self,
        node_id: NodeId,
        tag_changed: bool,
        dirty_styles: &mut Vec<NodeId>,
    ) {
        if let Some(state) = self.native_views.get(&node_id)
            && let Some(el) = self.nodes.get_mut(&node_id)
            && let Some(st) = state.static_tag
        {
            el.tag = st;
        }
        dirty_styles.push(node_id);
        #[cfg(target_arch = "wasm32")]
        {
            #[cfg(feature = "hydrate")]
            let skip_upgrade = self.hydrating;
            #[cfg(not(feature = "hydrate"))]
            let skip_upgrade = false;
            if tag_changed && !skip_upgrade {
                self.upgrade_dom_element_tag(node_id);
            }
            if let (Some(dom_el), Some(state)) =
                (self.dom_nodes.get(&node_id), self.native_views.get_mut(&node_id))
            {
                sync_native_view_dom_attrs(dom_el, state);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = tag_changed;
    }

    /// Applies zero-serialization [`TypedNativeViewProps`] inside an active `RenderSink` frame,
    /// recording dirty styles for the single end-of-frame flush.
    pub(crate) fn apply_typed_native_view_props_in_frame(
        &mut self,
        node_id: NodeId,
        props: &TypedNativeViewProps,
        dirty_styles: &mut Vec<NodeId>,
    ) {
        if !self
            .nodes
            .get(&node_id)
            .is_some_and(|n| n.kind == ComponentKind::Custom)
        {
            return;
        }
        let state = self
            .native_views
            .entry(node_id)
            .or_insert_with(|| NativeViewNodeState::new(props.kind().to_owned()));
        let prev = state.resolved_tag.clone();
        if state.kind != props.kind() {
            state.kind = props.kind().to_owned();
            resolve_kind_defaults(state);
        }
        let _ = apply_typed_props_to_state(state, props);
        let tag_changed = state.resolved_tag != prev;
        self.finish_native_view_prop_update(node_id, tag_changed, dirty_styles);
    }

    /// Applies zero-serialization [`TypedNativeViewProps`] directly to `node_id` without any
    /// `NVNP` binary encoding or decoding.
    pub fn set_native_view_props(&mut self, node_id: NodeId, props: &TypedNativeViewProps) {
        let mut dirty = Vec::with_capacity(1);
        self.apply_typed_native_view_props_in_frame(node_id, props, &mut dirty);
        if !dirty.is_empty() {
            self.flush_or_queue_responsive_dirty(dirty);
        }
    }

    /// Dispatches a zero-serialization [`NativeCommand`] directly to `node_id` (zero `NVNC` bytes).
    pub fn dispatch_native_command(&mut self, node_id: NodeId, cmd: &NativeCommand) -> bool {
        let Some(state) = self.native_views.get_mut(&node_id) else {
            return false;
        };
        dispatch_command_source(
            state,
            #[cfg(target_arch = "wasm32")]
            self.dom_nodes.get(&node_id),
            cmd,
        )
    }

    /// Dispatches a zero-serialization [`TypedNativeViewCommand`] directly to `node_id`, taking
    /// the 0-conversion concrete downcast fast path when available.
    pub fn dispatch_typed_native_command(
        &mut self,
        node_id: NodeId,
        cmd: &TypedNativeViewCommand,
    ) -> bool {
        let Some(state) = self.native_views.get_mut(&node_id) else {
            return false;
        };
        match state.kind.as_str() {
            #[cfg(feature = "nv-webview")]
            KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => {
                if let Some(c) = cmd.downcast_ref::<WebViewCommand>() {
                    return apply_web_view_command(
                        state,
                        #[cfg(target_arch = "wasm32")]
                        self.dom_nodes.get(&node_id),
                        c.clone(),
                    );
                }
            }
            #[cfg(feature = "nv-video")]
            KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => {
                if let Some(&c) = cmd.downcast_ref::<VideoPlayerCommand>() {
                    return apply_video_player_command(
                        state,
                        #[cfg(target_arch = "wasm32")]
                        self.dom_nodes.get(&node_id),
                        c,
                    );
                }
            }
            #[cfg(feature = "camera")]
            KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => {
                if let Some(c) = cmd.downcast_ref::<CameraPreviewCommand>() {
                    if c.validate().is_err() {
                        return false;
                    }
                    return dispatch_camera_typed_command(
                        state,
                        #[cfg(target_arch = "wasm32")]
                        self.dom_nodes.get(&node_id),
                        c.clone(),
                    );
                }
            }
            _ => {}
        }
        dispatch_command_source(
            state,
            #[cfg(target_arch = "wasm32")]
            self.dom_nodes.get(&node_id),
            cmd.command(),
        )
    }

    pub(crate) fn apply_native_view_prop(
        &mut self,
        node_id: NodeId,
        key: &str,
        value: &RawPropValue,
        dirty_styles: &mut Vec<NodeId>,
    ) {
        if !self
            .nodes
            .get(&node_id)
            .is_some_and(|n| n.kind == ComponentKind::Custom)
        {
            return;
        }
        let mut tag_changed = false;
        if is_native_kind_prop(key) {
            if let RawPropValue::String(kind_str) = value {
                let state = self
                    .native_views
                    .entry(node_id)
                    .or_insert_with(|| NativeViewNodeState::new(kind_str.clone()));
                let prev = state.resolved_tag.clone();
                state.kind = kind_str.clone();
                resolve_kind_defaults(state);
                if let Some(RawPropValue::Bytes(raw)) = self.nodes.get(&node_id).and_then(|n| {
                    n.raw_props
                        .get(NATIVE_VIEW_PROPS_PROP_KEY)
                        .or_else(|| n.raw_props.get(NATIVE_VIEW_PROPS_PROP_ALIAS))
                }) {
                    let _ = apply_nvnp_to_state(state, raw);
                }
                tag_changed = state.resolved_tag != prev;
            }
        } else if let RawPropValue::Bytes(raw) = value {
            let fallback = self
                .nodes
                .get(&node_id)
                .and_then(|n| {
                    n.raw_props
                        .get(NATIVE_VIEW_KIND_PROP_KEY)
                        .or_else(|| n.raw_props.get(NATIVE_VIEW_KIND_PROP_ALIAS))
                })
                .and_then(|v| {
                    if let RawPropValue::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            let state = self
                .native_views
                .entry(node_id)
                .or_insert_with(|| NativeViewNodeState::new(fallback));
            tag_changed = apply_nvnp_to_state(state, raw);
        }
        self.finish_native_view_prop_update(node_id, tag_changed, dirty_styles);
    }

    pub(crate) fn dispatch_native_view_command(
        &mut self,
        node_id: NodeId,
        _command_name: &str,
        payload: &[u8],
    ) -> bool {
        let Ok(dec) = NativeCommandDecoder::decode(payload) else {
            return false;
        };
        let Some(state) = self.native_views.get_mut(&node_id) else {
            return false;
        };
        dispatch_command_source(
            state,
            #[cfg(target_arch = "wasm32")]
            self.dom_nodes.get(&node_id),
            &dec,
        )
    }
}
