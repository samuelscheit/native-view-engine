//! Camera preview `NativeView` adapter (`nve.camera.preview` / `nve.camera`), gated behind
//! `feature = "camera"` so apps that do not use live camera streams pay zero bytes for codecs
//! or `navigator.mediaDevices.getUserMedia` inline JS glue.

use crate::native_view::{
    push_escaped_attr, NativeCommandDecoder, NativePropDecoder, NativeViewCodecError,
    NativeViewNodeState,
};
#[cfg(target_arch = "wasm32")]
use crate::native_view::sync_bool_attr;

pub const KIND_CAMERA_PREVIEW: &str = "nve.camera.preview";
pub const KIND_CAMERA_ALIAS: &str = "nve.camera";
pub const MAX_CAMERA_FPS: u16 = 240;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraPreviewFacing {
    #[default]
    Back = 1,
    Front = 2,
    External = 3,
}

impl CameraPreviewFacing {
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        match v {
            1 => Ok(Self::Back),
            2 => Ok(Self::Front),
            3 => Ok(Self::External),
            other => Err(NativeViewCodecError::InvalidEnum("CameraPreviewFacing", other)),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Front => "front",
            Self::External => "external",
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub const fn web_facing_mode(self) -> &'static str {
        match self {
            Self::Back => "environment",
            Self::Front => "user",
            Self::External => "",
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TorchMode {
    #[default]
    Off = 0,
    On = 1,
    Auto = 2,
}

impl TorchMode {
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        match v {
            0 => Ok(Self::Off),
            1 => Ok(Self::On),
            2 => Ok(Self::Auto),
            other => Err(NativeViewCodecError::InvalidEnum("TorchMode", other)),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Auto => "auto",
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StabilizationMode {
    #[default]
    Off = 0,
    Standard = 1,
    Cinematic = 2,
}

impl StabilizationMode {
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        match v {
            0 => Ok(Self::Off),
            1 => Ok(Self::Standard),
            2 => Ok(Self::Cinematic),
            other => Err(NativeViewCodecError::InvalidEnum("StabilizationMode", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraPreviewProps {
    pub device_id: Option<String>,
    pub facing: CameraPreviewFacing,
    pub torch: TorchMode,
    pub zoom: f32,
    pub fps: u16,
    pub hdr: bool,
    pub stabilization: StabilizationMode,
    pub enable_frame_processor: bool,
    pub audio_enabled: bool,
    pub active: bool,
}

impl Default for CameraPreviewProps {
    fn default() -> Self {
        Self {
            device_id: None,
            facing: CameraPreviewFacing::Back,
            torch: TorchMode::Off,
            zoom: 1.0,
            fps: 30,
            hdr: false,
            stabilization: StabilizationMode::Off,
            enable_frame_processor: false,
            audio_enabled: false,
            active: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CameraPreviewCommand {
    SwitchFacing(CameraPreviewFacing),
    SetTorch(TorchMode),
    SetZoom(f32),
    FocusPoint { x: f32, y: f32 },
    CapturePhoto { quality: u8, flash: u8 },
    StartRecording { output_path: String },
    StopRecording,
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

pub fn decode_camera_preview_props(
    dec: &NativePropDecoder<'_>,
) -> Result<CameraPreviewProps, NativeViewCodecError> {
    let device_id = dec.require(1)?.as_opt_str()?.map(str::to_owned);
    let facing = CameraPreviewFacing::from_u32(dec.require(2)?.as_u32()?)?;
    let torch = TorchMode::from_u32(dec.require(3)?.as_u32()?)?;
    let zoom = dec.require(4)?.as_f32()?;
    if !zoom.is_finite() || zoom <= 0.0 {
        return Err(NativeViewCodecError::TypeMismatch("finite positive zoom"));
    }
    let fps_raw = dec.require(5)?.as_u32()?;
    if !(1..=u32::from(MAX_CAMERA_FPS)).contains(&fps_raw) {
        return Err(NativeViewCodecError::InvalidEnum("CameraFps", fps_raw));
    }
    Ok(CameraPreviewProps {
        device_id,
        facing,
        torch,
        zoom,
        fps: fps_raw as u16,
        hdr: dec.require(6)?.as_bool()?,
        stabilization: StabilizationMode::from_u32(dec.require(7)?.as_u32()?)?,
        enable_frame_processor: dec.require(8)?.as_bool()?,
        audio_enabled: dec.require(9)?.as_bool()?,
        active: dec.require(10)?.as_bool()?,
    })
}

pub fn decode_camera_preview_command(
    bytes: &[u8],
) -> Result<CameraPreviewCommand, NativeViewCodecError> {
    let dec = NativeCommandDecoder::decode(bytes)?;
    match dec.opcode() {
        1 => {
            dec.expect_arg_count(1)?;
            Ok(CameraPreviewCommand::SwitchFacing(
                CameraPreviewFacing::from_u32(dec.require(0)?.as_u32()?)?,
            ))
        }
        2 => {
            dec.expect_arg_count(1)?;
            Ok(CameraPreviewCommand::SetTorch(TorchMode::from_u32(
                dec.require(0)?.as_u32()?,
            )?))
        }
        3 => {
            dec.expect_arg_count(1)?;
            let zoom = dec.require(0)?.as_f32()?;
            if !zoom.is_finite() || zoom <= 0.0 {
                return Err(NativeViewCodecError::TypeMismatch("finite positive zoom"));
            }
            Ok(CameraPreviewCommand::SetZoom(zoom))
        }
        4 => {
            dec.expect_arg_count(2)?;
            let (x, y) = (dec.require(0)?.as_f32()?, dec.require(1)?.as_f32()?);
            if !x.is_finite() || !(0.0..=1.0).contains(&x) || !y.is_finite() || !(0.0..=1.0).contains(&y)
            {
                return Err(NativeViewCodecError::TypeMismatch(
                    "focus_point in 0.0..=1.0",
                ));
            }
            Ok(CameraPreviewCommand::FocusPoint { x, y })
        }
        5 => {
            dec.expect_arg_count(2)?;
            let (q, f) = (dec.require(0)?.as_u32()?, dec.require(1)?.as_u32()?);
            if q > 100 {
                return Err(NativeViewCodecError::InvalidEnum("PhotoQuality", q));
            }
            if f > 2 {
                return Err(NativeViewCodecError::InvalidEnum("FlashMode", f));
            }
            Ok(CameraPreviewCommand::CapturePhoto {
                quality: q as u8,
                flash: f as u8,
            })
        }
        6 => {
            dec.expect_arg_count(1)?;
            Ok(CameraPreviewCommand::StartRecording {
                output_path: dec.require(0)?.as_str()?.to_owned(),
            })
        }
        7 => {
            dec.expect_arg_count(0)?;
            Ok(CameraPreviewCommand::StopRecording)
        }
        other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
    }
}

pub(crate) fn format_camera_zoom(zoom: f32) -> String {
    let rounded = (zoom * 100.0).round() / 100.0;
    let s = format!("{rounded:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

pub(crate) fn write_camera_html_attrs(cam: &CameraNodeState, out: &mut String) {
    out.push_str(" autoplay playsinline muted");
    push_escaped_attr("data-nve-camera", "1", out);
    push_escaped_attr("data-nve-camera-facing", cam.facing.as_str(), out);
    push_escaped_attr("data-nve-camera-torch", cam.torch.as_str(), out);
    push_escaped_attr("data-nve-camera-zoom", &format_camera_zoom(cam.zoom), out);
    push_escaped_attr("data-nve-camera-fps", &cam.fps.to_string(), out);
    push_escaped_attr(
        "data-nve-camera-active",
        if cam.active { "1" } else { "0" },
        out,
    );
    push_escaped_attr(
        "data-nve-camera-audio",
        if cam.audio_enabled { "1" } else { "0" },
        out,
    );
    if let Some(id) = cam.device_id.as_deref()
        && !id.is_empty()
    {
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
    let _ = el.set_attribute("data-nve-camera-facing", cam.facing.as_str());
    let _ = el.set_attribute("data-nve-camera-torch", cam.torch.as_str());
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
            cam.facing.web_facing_mode(),
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

pub(crate) fn dispatch_camera_command(
    state: &mut NativeViewNodeState,
    #[cfg(target_arch = "wasm32")] dom_el: Option<&web_sys::Element>,
    payload: &[u8],
) -> bool {
    let Ok(cmd) = decode_camera_preview_command(payload) else {
        return false;
    };
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
                let _ = el.set_attribute("data-nve-camera-torch", mode.as_str());
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
