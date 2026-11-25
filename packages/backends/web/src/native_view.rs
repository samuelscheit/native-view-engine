//! Host-side `NativeView` mount adapters for `WebBackend`: zero-JSON `NVNP` prop & `NVNC`
//! command decoding, allowlisted `"web.<tag>"` DOM passthrough, CSS `BlurSurface`,
//! sandboxed `WebView` `<iframe>`, `<video>` `VideoPlayer`, and opt-in `CameraPreview`.

#[cfg(feature = "camera")]
#[path = "native_view_camera.rs"]
mod camera;
#[path = "native_view_codec.rs"]
mod codec;

#[cfg(feature = "camera")]
pub use camera::*;
pub use codec::*;

#[cfg(feature = "nv-blur")]
use nve_abi::num::write_f32;
#[cfg(feature = "nv-blur")]
use nve_abi::ColorRgba;
use nve_abi::{ComponentKind, NodeId, RawPropValue};

use crate::css::MirrorElement;
use crate::html::write_attr;
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

    fn set_tag(&mut self, tag: String, static_tag: Option<&'static str>) {
        self.resolved_tag = tag;
        self.static_tag = static_tag;
    }
}

#[cfg(feature = "nv-blur")]
fn format_blur_css(props: BlurSurfaceProps) -> String {
    let (base_px, default_alpha) = props.material.css_params();
    let intensity = if props.intensity.is_finite() {
        props.intensity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let blur_px = ((base_px * intensity) * 100.0).round() / 100.0;
    // `nve_abi::num` CSS number format (no `core::fmt` float printing in wasm).
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
                state.extra_css = format_blur_css(BlurSurfaceProps {
                    material: BlurMaterial::Regular,
                    intensity: 1.0,
                    tint: None,
                });
            }
        }
        #[cfg(feature = "nv-webview")]
        KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => {
            state.set_tag("iframe".to_owned(), Some("iframe"))
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

fn apply_nvnp_to_state(state: &mut NativeViewNodeState, bytes: &[u8]) -> bool {
    let prev_tag = state.resolved_tag.clone();
    #[cfg(feature = "camera")]
    if matches!(state.kind.as_str(), KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS) {
        if let Ok(p) = decode_camera_preview_props(bytes) {
            state.camera.props = p.clone();
            state.decoded = Some(DecodedNativeView::CameraPreview(p));
            state.set_tag("video".to_owned(), Some("video"));
            state.extra_css.clear();
        }
        return state.resolved_tag != prev_tag;
    }
    let Ok(dec) = NativePropDecoder::decode(bytes) else {
        return false;
    };
    let _ = &dec;
    match state.kind.as_str() {
        #[cfg(feature = "nv-blur")]
        KIND_BLUR_SURFACE | KIND_BLUR_ALIAS => {
            if let Ok(p) = decode_blur_surface_props(&dec) {
                state.extra_css = format_blur_css(p);
                state.decoded = Some(DecodedNativeView::BlurSurface(p));
                state.set_tag("div".to_owned(), Some("div"));
            }
        }
        #[cfg(feature = "nv-webview")]
        KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => {
            if let Ok(p) = decode_web_view_props(&dec) {
                state.current_url = is_safe_web_url(&p.url).then(|| p.url.clone());
                state.decoded = Some(DecodedNativeView::WebView(p));
                state.set_tag("iframe".to_owned(), Some("iframe"));
                state.extra_css.clear();
            }
        }
        #[cfg(feature = "nv-video")]
        KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => {
            if let Ok(p) = decode_video_player_props(&dec) {
                state.playing = p.autoplay;
                state.decoded = Some(DecodedNativeView::VideoPlayer(p));
                state.set_tag("video".to_owned(), Some("video"));
                state.extra_css.clear();
            }
        }
        #[cfg(feature = "nv-web-element")]
        other if other.starts_with("web.") => {
            if let Ok(p) = decode_web_element_props(&dec) {
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
        }
        _ => {}
    }
    state.resolved_tag != prev_tag
}

#[cfg(feature = "camera")]
pub(crate) use crate::html::write_attr as push_escaped_attr;

pub(crate) fn write_native_view_html_attrs(state: &NativeViewNodeState, out: &mut String) {
    write_attr("data-nve-native-kind", &state.kind, out);
    match &state.decoded {
        #[cfg(feature = "nv-blur")]
        Some(DecodedNativeView::BlurSurface(_)) => {}
        #[cfg(feature = "nv-webview")]
        Some(DecodedNativeView::WebView(_)) => {
            write_attr("sandbox", WEBVIEW_SANDBOX_ATTR, out);
            write_attr("referrerpolicy", "no-referrer", out);
            if let Some(url) = state.current_url.as_deref()
                && is_safe_web_url(url)
            {
                write_attr("src", url, out);
            }
        }
        #[cfg(feature = "nv-video")]
        Some(DecodedNativeView::VideoPlayer(p)) => {
            if is_safe_web_url(&p.source_uri) {
                write_attr("src", &p.source_uri, out);
            }
            for (flag, name) in [
                (p.autoplay, " autoplay"),
                (p.muted, " muted"),
                (p.looping, " loop"),
                (p.controls, " controls"),
            ] {
                if flag {
                    out.push_str(name);
                }
            }
            out.push_str(" playsinline");
        }
        #[cfg(feature = "camera")]
        Some(DecodedNativeView::CameraPreview(_)) => write_camera_html_attrs(&state.camera, out),
        #[cfg(feature = "nv-web-element")]
        Some(DecodedNativeView::WebElement(p)) => {
            for (k, v) in &p.attrs {
                if is_safe_web_attr(k, v) {
                    write_attr(k, v, out);
                }
            }
        }
        #[cfg(feature = "nv-webview")]
        None if matches!(state.kind.as_str(), KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS) => {
            write_attr("sandbox", WEBVIEW_SANDBOX_ATTR, out);
            write_attr("referrerpolicy", "no-referrer", out);
        }
        #[cfg(feature = "camera")]
        None if matches!(state.kind.as_str(), KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS) => {
            write_camera_html_attrs(&state.camera, out);
        }
        None => {}
    }
}

#[cfg(all(target_arch = "wasm32", feature = "nv-video"))]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_video_play(el) { try { if (el && typeof el.play === 'function') { const p = el.play(); if (p && typeof p.catch === 'function') p.catch(() => {}); } } catch (_) {} }
export function nve_video_pause(el) { try { if (el && typeof el.pause === 'function') el.pause(); } catch (_) {} }
export function nve_video_seek_ms(el, ms) { try { if (el && 'currentTime' in el) el.currentTime = ms / 1000.0; } catch (_) {} }
export function nve_video_set_volume(el, vol) { try { if (el && 'volume' in el) el.volume = Math.max(0, Math.min(1, vol)); } catch (_) {} }
export function nve_video_set_muted(el, muted) { try { if (el && 'muted' in el) el.muted = !!muted; } catch (_) {} }
")]
extern "C" {
    fn nve_video_play(el: &web_sys::Element);
    fn nve_video_pause(el: &web_sys::Element);
    fn nve_video_seek_ms(el: &web_sys::Element, ms: f64);
    fn nve_video_set_volume(el: &web_sys::Element, vol: f32);
    fn nve_video_set_muted(el: &web_sys::Element, muted: bool);
}

#[cfg(all(target_arch = "wasm32", feature = "nv-webview"))]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_iframe_reload(el) { try { if (el && el.contentWindow && el.contentWindow.location) { el.contentWindow.location.reload(); return; } } catch (_) {} try { if (el && el.getAttribute('src')) el.setAttribute('src', el.getAttribute('src')); } catch (_) {} }
export function nve_iframe_go_back(el) { try { if (el && el.contentWindow && el.contentWindow.history) el.contentWindow.history.back(); } catch (_) {} }
export function nve_iframe_go_forward(el) { try { if (el && el.contentWindow && el.contentWindow.history) el.contentWindow.history.forward(); } catch (_) {} }
")]
extern "C" {
    fn nve_iframe_reload(el: &web_sys::Element);
    fn nve_iframe_go_back(el: &web_sys::Element);
    fn nve_iframe_go_forward(el: &web_sys::Element);
}

#[cfg(all(target_arch = "wasm32", any(feature = "nv-video", feature = "camera")))]
pub(crate) fn sync_bool_attr(el: &web_sys::Element, name: &str, on: bool) {
    if on && !el.has_attribute(name) {
        let _ = el.set_attribute(name, "");
    } else if !on && el.has_attribute(name) {
        let _ = el.remove_attribute(name);
    }
}

#[cfg(all(target_arch = "wasm32", any(feature = "nv-webview", feature = "nv-video")))]
fn sync_opt_src(el: &web_sys::Element, url: Option<&str>) {
    if let Some(u) = url
        && is_safe_web_url(u)
    {
        if el.get_attribute("src").as_deref() != Some(u) {
            let _ = el.set_attribute("src", u);
        }
    } else if el.has_attribute("src") {
        let _ = el.remove_attribute("src");
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn sync_native_view_dom_attrs(el: &web_sys::Element, state: &mut NativeViewNodeState) {
    let _ = el.set_attribute("data-nve-native-kind", &state.kind);
    match &state.decoded {
        #[cfg(feature = "nv-blur")]
        Some(DecodedNativeView::BlurSurface(_)) => {}
        #[cfg(feature = "nv-webview")]
        Some(DecodedNativeView::WebView(_)) => {
            let _ = el.set_attribute("sandbox", WEBVIEW_SANDBOX_ATTR);
            let _ = el.set_attribute("referrerpolicy", "no-referrer");
            sync_opt_src(el, state.current_url.as_deref());
        }
        #[cfg(feature = "nv-video")]
        Some(DecodedNativeView::VideoPlayer(p)) => {
            sync_opt_src(el, Some(p.source_uri.as_str()));
            for (n, on) in [
                ("autoplay", p.autoplay),
                ("muted", p.muted),
                ("loop", p.looping),
                ("controls", p.controls),
                ("playsinline", true),
            ] {
                sync_bool_attr(el, n, on);
            }
            nve_video_set_muted(el, p.muted);
        }
        #[cfg(feature = "camera")]
        Some(DecodedNativeView::CameraPreview(_)) => sync_camera_dom(el, state),
        #[cfg(feature = "nv-web-element")]
        Some(DecodedNativeView::WebElement(p)) => {
            for old in state.applied_attrs.drain(..) {
                if !p.attrs.iter().any(|(k, _)| k == &old) {
                    let _ = el.remove_attribute(&old);
                }
            }
            for (k, v) in &p.attrs {
                if is_safe_web_attr(k, v) {
                    if el.get_attribute(k).as_deref() != Some(v.as_str()) {
                        let _ = el.set_attribute(k, v);
                    }
                    state.applied_attrs.push(k.clone());
                }
            }
        }
        #[cfg(feature = "camera")]
        None if matches!(state.kind.as_str(), KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS) => {
            sync_camera_dom(el, state)
        }
        None => {}
    }
}

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

    pub(crate) fn dispatch_native_view_command(
        &mut self,
        node_id: NodeId,
        _command_name: &str,
        payload: &[u8],
    ) -> bool {
        let _ = payload;
        let Some(state) = self.native_views.get_mut(&node_id) else {
            return false;
        };
        match state.kind.as_str() {
            #[cfg(feature = "nv-webview")]
            KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => {
                let Ok(cmd) = decode_web_view_command(payload) else {
                    return false;
                };
                let allows_bf = !matches!(
                    &state.decoded,
                    Some(DecodedNativeView::WebView(p)) if !p.allows_back_forward
                );
                match cmd {
                    WebViewCommand::Navigate(url) if is_safe_web_url(&url) => {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(el) = self.dom_nodes.get(&node_id) {
                            let _ = el.set_attribute("src", &url);
                        }
                        state.current_url = Some(url);
                        true
                    }
                    WebViewCommand::Reload => {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(el) = self.dom_nodes.get(&node_id) {
                            nve_iframe_reload(el);
                        }
                        state.reload_count = state.reload_count.saturating_add(1);
                        true
                    }
                    WebViewCommand::GoBack if allows_bf => {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(el) = self.dom_nodes.get(&node_id) {
                            nve_iframe_go_back(el);
                        }
                        state.go_back_count = state.go_back_count.saturating_add(1);
                        true
                    }
                    WebViewCommand::GoForward if allows_bf => {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(el) = self.dom_nodes.get(&node_id) {
                            nve_iframe_go_forward(el);
                        }
                        state.go_forward_count = state.go_forward_count.saturating_add(1);
                        true
                    }
                    _ => false,
                }
            }
            #[cfg(feature = "nv-video")]
            KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => {
                let Ok(cmd) = decode_video_player_command(payload) else {
                    return false;
                };
                match cmd {
                    VideoPlayerCommand::Play => {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(el) = self.dom_nodes.get(&node_id) {
                            nve_video_play(el);
                        }
                        state.playing = true;
                        true
                    }
                    VideoPlayerCommand::Pause => {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(el) = self.dom_nodes.get(&node_id) {
                            nve_video_pause(el);
                        }
                        state.playing = false;
                        true
                    }
                    VideoPlayerCommand::SeekMs(ms) => {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(el) = self.dom_nodes.get(&node_id) {
                            nve_video_seek_ms(el, ms as f64);
                        }
                        state.position_ms = ms;
                        true
                    }
                    VideoPlayerCommand::SetVolume(vol) if vol.is_finite() => {
                        state.volume = vol.clamp(0.0, 1.0);
                        #[cfg(target_arch = "wasm32")]
                        if let Some(el) = self.dom_nodes.get(&node_id) {
                            nve_video_set_volume(el, state.volume);
                        }
                        true
                    }
                    _ => false,
                }
            }
            #[cfg(feature = "camera")]
            KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => dispatch_camera_command(
                state,
                #[cfg(target_arch = "wasm32")]
                self.dom_nodes.get(&node_id),
                payload,
            ),
            _ => false,
        }
    }
}
