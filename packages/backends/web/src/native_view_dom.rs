//! HTML/SSR attribute emission, live DOM attribute sync, and imperative command dispatch
//! for `WebBackend` native views (`WebView`, `VideoPlayer`, `CameraPreview`, `WebElement`).

use nve_components::native_view::{DecodeNativeCommand, NativeCommandSource};
#[cfg(feature = "nv-video")]
use nve_components::native_view::VideoPlayerCommand;
#[cfg(feature = "nv-webview")]
use nve_components::native_view::WebViewCommand;

use crate::html::write_attr;
#[cfg(feature = "camera")]
use super::camera::{
    dispatch_camera_typed_command, write_camera_html_attrs, CameraPreviewCommand,
    KIND_CAMERA_ALIAS, KIND_CAMERA_PREVIEW,
};
#[cfg(all(target_arch = "wasm32", feature = "camera"))]
use super::camera::sync_camera_dom;
#[cfg(feature = "nv-web-element")]
use super::security::is_safe_web_attr;
#[cfg(any(feature = "nv-webview", feature = "nv-video"))]
use super::security::is_safe_web_url;
use super::{DecodedNativeView, NativeViewNodeState};
#[cfg(feature = "nv-video")]
use super::{KIND_VIDEO_ALIAS, KIND_VIDEO_PLAYER};
#[cfg(feature = "nv-webview")]
use super::{KIND_WEB_VIEW, KIND_WEB_VIEW_ALIAS, WEBVIEW_SANDBOX_ATTR};

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
            sync_camera_dom(el, state);
        }
        None => {}
    }
}

#[cfg(feature = "nv-webview")]
pub(super) fn apply_web_view_command(
    state: &mut NativeViewNodeState,
    #[cfg(target_arch = "wasm32")] dom_el: Option<&web_sys::Element>,
    cmd: WebViewCommand,
) -> bool {
    let allows_bf = !matches!(
        &state.decoded,
        Some(DecodedNativeView::WebView(p)) if !p.allows_back_forward
    );
    match cmd {
        WebViewCommand::Navigate(url) if is_safe_web_url(&url) => {
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                let _ = el.set_attribute("src", &url);
            }
            state.current_url = Some(url);
            true
        }
        WebViewCommand::Reload => {
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                nve_iframe_reload(el);
            }
            state.reload_count = state.reload_count.saturating_add(1);
            true
        }
        WebViewCommand::GoBack if allows_bf => {
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                nve_iframe_go_back(el);
            }
            state.go_back_count = state.go_back_count.saturating_add(1);
            true
        }
        WebViewCommand::GoForward if allows_bf => {
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                nve_iframe_go_forward(el);
            }
            state.go_forward_count = state.go_forward_count.saturating_add(1);
            true
        }
        _ => false,
    }
}

#[cfg(feature = "nv-video")]
pub(super) fn apply_video_player_command(
    state: &mut NativeViewNodeState,
    #[cfg(target_arch = "wasm32")] dom_el: Option<&web_sys::Element>,
    cmd: VideoPlayerCommand,
) -> bool {
    match cmd {
        VideoPlayerCommand::Play => {
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                nve_video_play(el);
            }
            state.playing = true;
            true
        }
        VideoPlayerCommand::Pause => {
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                nve_video_pause(el);
            }
            state.playing = false;
            true
        }
        VideoPlayerCommand::SeekMs(ms) => {
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                nve_video_seek_ms(el, ms as f64);
            }
            state.position_ms = ms;
            true
        }
        VideoPlayerCommand::SetVolume(vol) if vol.is_finite() => {
            state.volume = vol.clamp(0.0, 1.0);
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                nve_video_set_volume(el, state.volume);
            }
            true
        }
        _ => false,
    }
}

pub(super) fn dispatch_command_source<S: NativeCommandSource>(
    state: &mut NativeViewNodeState,
    #[cfg(target_arch = "wasm32")] dom_el: Option<&web_sys::Element>,
    src: &S,
) -> bool {
    match state.kind.as_str() {
        #[cfg(feature = "nv-webview")]
        KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => WebViewCommand::read_command(src)
            .map(|cmd| {
                apply_web_view_command(
                    state,
                    #[cfg(target_arch = "wasm32")]
                    dom_el,
                    cmd,
                )
            })
            .unwrap_or(false),
        #[cfg(feature = "nv-video")]
        KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => VideoPlayerCommand::read_command(src)
            .map(|cmd| {
                apply_video_player_command(
                    state,
                    #[cfg(target_arch = "wasm32")]
                    dom_el,
                    cmd,
                )
            })
            .unwrap_or(false),
        #[cfg(feature = "camera")]
        KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => CameraPreviewCommand::read_command(src)
            .map(|cmd| {
                dispatch_camera_typed_command(
                    state,
                    #[cfg(target_arch = "wasm32")]
                    dom_el,
                    cmd,
                )
            })
            .unwrap_or(false),
        _ => false,
    }
}
