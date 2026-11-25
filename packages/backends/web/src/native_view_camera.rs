//! Camera preview `NativeView` adapter (`nve.camera.preview` / `nve.camera`), gated behind
//! `feature = "camera"` so apps that do not use live camera streams pay zero bytes for codecs
//! or `navigator.mediaDevices.getUserMedia` inline JS glue.
//!
//! Reuses canonical types and single-source codecs directly from `nve-camera` / `nve-components`.

pub use nve_camera::{
    CameraFacing, CameraFacing as CameraPreviewFacing, CameraPreviewCommand, CameraPreviewProps,
    StabilizationMode, TorchMode, CAMERA_PREVIEW_KIND, MAX_CAMERA_FPS,
};

#[cfg(target_arch = "wasm32")]
use crate::native_view::sync_bool_attr;
use crate::native_view::{
    push_escaped_attr, DecodedNativeView, NativeViewCodecError, NativeViewNodeState,
};

pub const KIND_CAMERA_PREVIEW: &str = CAMERA_PREVIEW_KIND;
pub const KIND_CAMERA_ALIAS: &str = "nve.camera";

pub const fn facing_as_str(facing: CameraFacing) -> &'static str {
    match facing {
        CameraFacing::Back => "back",
        CameraFacing::Front => "front",
        CameraFacing::External => "external",
        CameraFacing::Unknown => "unknown",
    }
}

pub const fn torch_as_str(mode: TorchMode) -> &'static str {
    match mode {
        TorchMode::Off => "off",
        TorchMode::On => "on",
        TorchMode::Auto => "auto",
    }
}

#[cfg(target_arch = "wasm32")]
pub const fn web_facing_mode(facing: CameraFacing) -> &'static str {
    match facing {
        CameraFacing::Back => "environment",
        CameraFacing::Front => "user",
        CameraFacing::External | CameraFacing::Unknown => "",
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CameraNodeState {
    pub props: CameraPreviewProps,
    pub recording: bool,
    pub last_recording_path: Option<String>,
    pub last_focus_point: Option<(f32, f32)>,
    pub photo_capture_count: u32,
    pub last_photo_quality: Option<u8>,
    pub last_photo_flash: Option<u8>,
}

impl core::ops::Deref for CameraNodeState {
    type Target = CameraPreviewProps;
    fn deref(&self) -> &Self::Target {
        &self.props
    }
}

impl core::ops::DerefMut for CameraNodeState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.props
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub(crate) struct CameraStreamGuard {
    el: web_sys::Element,
}

#[cfg(target_arch = "wasm32")]
impl Drop for CameraStreamGuard {
    fn drop(&mut self) {
        nve_camera_stop(&self.el);
    }
}

#[inline]
pub fn decode_camera_preview_props(bytes: &[u8]) -> Result<CameraPreviewProps, NativeViewCodecError> {
    CameraPreviewProps::from_prop_bytes(bytes)
}

#[inline]
pub fn decode_camera_preview_command(
    bytes: &[u8],
) -> Result<CameraPreviewCommand, NativeViewCodecError> {
    CameraPreviewCommand::from_command_bytes(bytes)
}

pub(crate) fn apply_camera_props(state: &mut NativeViewNodeState, props: CameraPreviewProps) {
    state.camera.props = props.clone();
    state.decoded = Some(DecodedNativeView::CameraPreview(props));
    state.set_tag("video".to_owned(), Some("video"));
    state.extra_css.clear();
}

pub(crate) fn format_camera_zoom(zoom: f32) -> String {
    let rounded = (zoom * 100.0).round() / 100.0;
    format!("{rounded:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

pub(crate) fn write_camera_html_attrs(cam: &CameraNodeState, out: &mut String) {
    out.push_str(" autoplay playsinline muted");
    push_escaped_attr("data-nve-camera", "1", out);
    push_escaped_attr("data-nve-camera-facing", facing_as_str(cam.facing), out);
    push_escaped_attr("data-nve-camera-torch", torch_as_str(cam.torch), out);
    push_escaped_attr("data-nve-camera-zoom", &format_camera_zoom(cam.zoom), out);
    push_escaped_attr("data-nve-camera-fps", &cam.fps.to_string(), out);
    push_escaped_attr("data-nve-camera-active", if cam.active { "1" } else { "0" }, out);
    push_escaped_attr("data-nve-camera-audio", if cam.audio_enabled { "1" } else { "0" }, out);
    if let Some(id) = cam.device_id.as_deref().filter(|s| !s.is_empty()) {
        push_escaped_attr("data-nve-camera-device", id, out);
    }
    if cam.recording {
        push_escaped_attr("data-nve-camera-recording", "1", out);
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
function nve_cam_stop_stream(s) { try { if (s && typeof s.getTracks === 'function') s.getTracks().forEach((t) => { try { t.stop(); } catch (_) {} }); } catch (_) {} }
export function nve_camera_set_muted(el) { try { if (el && 'muted' in el) el.muted = true; } catch (_) {} }
export function nve_camera_stop(el) { try { if (!el) return; el.__nveCamGen = (el.__nveCamGen || 0) + 1; if (el.__nveRecorder && el.__nveRecorder.state !== 'inactive') { try { el.__nveRecorder.stop(); } catch (_) {} } el.__nveRecorder = null; if (el.srcObject) { nve_cam_stop_stream(el.srcObject); el.srcObject = null; } el.__nveCamKey = ''; } catch (_) {} }
export function nve_camera_set_zoom(el, zoom) { try { if (!el) return; const z = Number.isFinite(zoom) && zoom > 0 ? zoom : 1; const vt = el.srcObject && typeof el.srcObject.getVideoTracks === 'function' ? el.srcObject.getVideoTracks()[0] : null; if (vt && typeof vt.applyConstraints === 'function') { try { vt.applyConstraints({ advanced: [{ zoom: z }] }).catch(() => {}); } catch (_) {} } if (el.style) el.style.transform = z > 1 ? ('scale(' + z + ')') : ''; } catch (_) {} }
export function nve_camera_set_torch(el, mode) { try { const vt = el && el.srcObject && typeof el.srcObject.getVideoTracks === 'function' ? el.srcObject.getVideoTracks()[0] : null; if (vt && typeof vt.applyConstraints === 'function') { try { vt.applyConstraints({ advanced: [{ torch: mode === 1 }] }).catch(() => {}); } catch (_) {} } } catch (_) {} }
export function nve_camera_attach(el, facingMode, deviceId, fps, audio, zoom, torch) { try { if (!el || typeof navigator === 'undefined' || !navigator.mediaDevices || typeof navigator.mediaDevices.getUserMedia !== 'function') return; const key = [facingMode, deviceId, fps, audio ? 1 : 0].join('|'); if (el.__nveCamKey === key && el.srcObject) { nve_camera_set_zoom(el, zoom); nve_camera_set_torch(el, torch); return; } nve_camera_stop(el); const gen = el.__nveCamGen || 1; el.__nveCamKey = key; const video = { frameRate: { ideal: fps } }; if (deviceId) video.deviceId = { exact: deviceId }; else if (facingMode) video.facingMode = { ideal: facingMode }; navigator.mediaDevices.getUserMedia({ video, audio: !!audio }).then((stream) => { if (el.__nveCamGen !== gen) { nve_cam_stop_stream(stream); return; } el.srcObject = stream; el.muted = true; el.playsInline = true; const p = typeof el.play === 'function' ? el.play() : null; if (p && typeof p.catch === 'function') p.catch(() => {}); nve_camera_set_zoom(el, zoom); nve_camera_set_torch(el, torch); }).catch(() => { if (el.__nveCamGen === gen) el.__nveCamKey = ''; }); } catch (_) {} }
export function nve_camera_capture_photo(el, quality) { try { if (!el || typeof document === 'undefined') return; const w = el.videoWidth || el.clientWidth || 0, h = el.videoHeight || el.clientHeight || 0; if (w <= 0 || h <= 0) return; const c = document.createElement('canvas'); c.width = w; c.height = h; const ctx = c.getContext('2d'); if (!ctx) return; ctx.drawImage(el, 0, 0, w, h); el.__nveLastPhotoDataUrl = c.toDataURL('image/jpeg', Math.max(0.01, Math.min(1, quality / 100.0))); el.setAttribute('data-nve-camera-last-photo', '1'); } catch (_) {} }
export function nve_camera_start_recording(el) { try { if (!el || !el.srcObject || typeof MediaRecorder === 'undefined' || (el.__nveRecorder && el.__nveRecorder.state !== 'inactive')) return; const chunks = [], rec = new MediaRecorder(el.srcObject); rec.ondataavailable = (ev) => { if (ev.data && ev.data.size > 0) chunks.push(ev.data); }; el.__nveRecChunks = chunks; el.__nveRecorder = rec; rec.start(); } catch (_) {} }
export function nve_camera_stop_recording(el) { try { if (el && el.__nveRecorder && el.__nveRecorder.state !== 'inactive') el.__nveRecorder.stop(); if (el) el.__nveRecorder = null; } catch (_) {} }
")]
extern "C" {
    fn nve_camera_set_muted(el: &web_sys::Element);
    fn nve_camera_attach(
        el: &web_sys::Element,
        facing_mode: &str,
        device_id: &str,
        fps: u16,
        audio: bool,
        zoom: f32,
        torch: u8,
    );
    fn nve_camera_stop(el: &web_sys::Element);
    fn nve_camera_set_zoom(el: &web_sys::Element, zoom: f32);
    fn nve_camera_set_torch(el: &web_sys::Element, mode: u8);
    fn nve_camera_capture_photo(el: &web_sys::Element, quality: u8);
    fn nve_camera_start_recording(el: &web_sys::Element);
    fn nve_camera_stop_recording(el: &web_sys::Element);
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn sync_camera_dom(el: &web_sys::Element, state: &mut NativeViewNodeState) {
    let cam = &state.camera;
    for n in ["autoplay", "playsinline", "muted"] {
        sync_bool_attr(el, n, true);
    }
    nve_camera_set_muted(el);
    let _ = el.set_attribute("data-nve-camera", "1");
    let _ = el.set_attribute("data-nve-camera-facing", facing_as_str(cam.facing));
    let _ = el.set_attribute("data-nve-camera-torch", torch_as_str(cam.torch));
    let _ = el.set_attribute("data-nve-camera-zoom", &format_camera_zoom(cam.zoom));
    let _ = el.set_attribute("data-nve-camera-fps", &cam.fps.to_string());
    let _ = el.set_attribute("data-nve-camera-active", if cam.active { "1" } else { "0" });
    let _ = el.set_attribute(
        "data-nve-camera-audio",
        if cam.audio_enabled { "1" } else { "0" },
    );
    match cam.device_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            let _ = el.set_attribute("data-nve-camera-device", id);
        }
        None => {
            let _ = el.remove_attribute("data-nve-camera-device");
        }
    }
    if cam.recording {
        let _ = el.set_attribute("data-nve-camera-recording", "1");
    } else {
        let _ = el.remove_attribute("data-nve-camera-recording");
    }
    if cam.active {
        if !state
            .camera_guard
            .as_ref()
            .is_some_and(|g| g.el.is_same_node(Some(el)))
        {
            state.camera_guard = Some(std::rc::Rc::new(CameraStreamGuard { el: el.clone() }));
        }
        nve_camera_attach(
            el,
            web_facing_mode(cam.facing),
            cam.device_id.as_deref().unwrap_or(""),
            cam.fps,
            cam.audio_enabled,
            cam.zoom,
            cam.torch as u8,
        );
    } else {
        state.camera_guard = None;
        nve_camera_stop(el);
    }
}

pub(crate) fn dispatch_camera_typed_command(
    state: &mut NativeViewNodeState,
    #[cfg(target_arch = "wasm32")] dom_el: Option<&web_sys::Element>,
    cmd: CameraPreviewCommand,
) -> bool {
    match cmd {
        CameraPreviewCommand::SwitchFacing(facing) => {
            state.camera.facing = facing;
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                sync_camera_dom(el, state);
            }
            true
        }
        CameraPreviewCommand::SetTorch(mode) => {
            state.camera.torch = mode;
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                let _ = el.set_attribute("data-nve-camera-torch", torch_as_str(mode));
                nve_camera_set_torch(el, mode as u8);
            }
            true
        }
        CameraPreviewCommand::SetZoom(zoom) => {
            state.camera.zoom = zoom;
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                let _ = el.set_attribute("data-nve-camera-zoom", &format_camera_zoom(zoom));
                nve_camera_set_zoom(el, zoom);
            }
            true
        }
        CameraPreviewCommand::FocusPoint { x, y } => {
            state.camera.last_focus_point = Some((x, y));
            true
        }
        CameraPreviewCommand::CapturePhoto { quality, flash } => {
            state.camera.photo_capture_count = state.camera.photo_capture_count.saturating_add(1);
            state.camera.last_photo_quality = Some(quality);
            state.camera.last_photo_flash = Some(flash);
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                nve_camera_capture_photo(el, quality);
            }
            true
        }
        CameraPreviewCommand::StartRecording { output_path } => {
            state.camera.recording = true;
            state.camera.last_recording_path = Some(output_path);
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                let _ = el.set_attribute("data-nve-camera-recording", "1");
                nve_camera_start_recording(el);
            }
            true
        }
        CameraPreviewCommand::StopRecording => {
            state.camera.recording = false;
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = dom_el {
                let _ = el.remove_attribute("data-nve-camera-recording");
                nve_camera_stop_recording(el);
            }
            true
        }
    }
}
