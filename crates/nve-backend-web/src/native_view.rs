//! Host-side `NativeView` mount adapters for `WebBackend`: zero-JSON `NVNP` prop & `NVNC`
//! command decoding, allowlisted `"web.<tag>"` DOM passthrough, CSS `BlurSurface`,
//! sandboxed `WebView` `<iframe>`, and `<video>` `VideoPlayer` for live DOM and Semantic SSR.

use nve_abi::num::write_f32;
use nve_abi::{ColorRgba, ComponentKind, NodeId, RawPropValue};
use crate::css::MirrorElement;
use crate::html::write_attr;
use crate::WebBackend;

pub const NATIVE_PROPS_MAGIC: [u8; 4] = *b"NVNP";
pub const NATIVE_PROPS_VERSION: u8 = 1;
pub const NATIVE_PROPS_HEADER_SIZE: usize = 7;
pub const NATIVE_CMD_MAGIC: [u8; 4] = *b"NVNC";
pub const NATIVE_CMD_VERSION: u8 = 1;
pub const NATIVE_CMD_HEADER_SIZE: usize = 8;
pub const MAX_NATIVE_PROP_FIELDS: usize = 1024;
pub const MAX_NATIVE_CMD_ARGS: usize = 64;
pub const NATIVE_VIEW_KIND_PROP_KEY: &str = "nve.native_kind";
pub const NATIVE_VIEW_KIND_PROP_ALIAS: &str = "nve.native.kind";
pub const NATIVE_VIEW_PROPS_PROP_KEY: &str = "nve.native_props";
pub const NATIVE_VIEW_PROPS_PROP_ALIAS: &str = "nve.native.props";
pub const KIND_BLUR_SURFACE: &str = "nve.blur_surface";
pub const KIND_BLUR_ALIAS: &str = "nve.blur";
pub const KIND_WEB_VIEW: &str = "nve.web_view";
pub const KIND_WEB_VIEW_ALIAS: &str = "nve.webview";
pub const KIND_VIDEO_PLAYER: &str = "nve.video_player";
pub const KIND_VIDEO_ALIAS: &str = "nve.video";
pub const KIND_CAMERA_PREVIEW: &str = "nve.camera.preview";
pub const KIND_CAMERA_ALIAS: &str = "nve.camera";
pub const MAX_CAMERA_FPS: u16 = 240;
const WEBVIEW_SANDBOX_ATTR: &str = "allow-scripts allow-same-origin allow-forms allow-popups";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeViewCodecError {
    Truncated, InvalidMagic([u8; 4]), UnsupportedVersion(u8), InvalidTag(u8), InvalidBool(u8),
    InvalidEnum(&'static str, u32), InvalidUtf8, MissingField(u16), MissingArg(usize),
    UnexpectedArgCount { expected: usize, actual: usize }, TypeMismatch(&'static str),
    TrailingBytes(usize), UnknownCommandOpcode(u16), TooManyFields(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeValueRef<'a> {
    Null, Bool(bool), U32(u32), I64(i64), U64(u64), F32(f32), F64(f64), Str(&'a str), Bytes(&'a [u8]),
}

macro_rules! impl_val_as {
    ($($fn:ident => $var:ident($ty:ty), $lbl:literal;)*) => {$(
        pub fn $fn(self) -> Result<$ty, NativeViewCodecError> {
            if let Self::$var(v) = self { Ok(v) } else { Err(NativeViewCodecError::TypeMismatch($lbl)) }
        }
    )*};
}

impl<'a> NativeValueRef<'a> {
    impl_val_as! {
        as_bool => Bool(bool), "bool"; as_u32 => U32(u32), "u32"; as_u64 => U64(u64), "u64";
        as_f32 => F32(f32), "f32"; as_str => Str(&'a str), "str"; as_bytes => Bytes(&'a [u8]), "bytes";
    }
    pub fn as_opt_u32(self) -> Result<Option<u32>, NativeViewCodecError> {
        match self { Self::Null => Ok(None), Self::U32(v) => Ok(Some(v)), _ => Err(NativeViewCodecError::TypeMismatch("Option<u32>")) }
    }
    pub fn as_opt_str(self) -> Result<Option<&'a str>, NativeViewCodecError> {
        match self { Self::Null => Ok(None), Self::Str(s) => Ok(Some(s)), _ => Err(NativeViewCodecError::TypeMismatch("Option<str>")) }
    }
}

#[inline]
fn take_slice<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], NativeViewCodecError> {
    let end = pos.checked_add(len).ok_or(NativeViewCodecError::Truncated)?;
    let slice = bytes.get(*pos..end).ok_or(NativeViewCodecError::Truncated)?;
    *pos = end;
    Ok(slice)
}

#[inline]
fn take_arr<const N: usize>(bytes: &[u8], pos: &mut usize) -> Result<[u8; N], NativeViewCodecError> {
    Ok(take_slice(bytes, pos, N)?.try_into().unwrap())
}

#[inline]
fn read_len_str<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a str, NativeViewCodecError> {
    let len = u32::from_le_bytes(take_arr(bytes, pos)?) as usize;
    core::str::from_utf8(take_slice(bytes, pos, len)?).map_err(|_| NativeViewCodecError::InvalidUtf8)
}

fn read_value<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<NativeValueRef<'a>, NativeViewCodecError> {
    let tag = *bytes.get(*pos).ok_or(NativeViewCodecError::Truncated)?;
    *pos += 1;
    match tag {
        0 => Ok(NativeValueRef::Null),
        1 => match *bytes.get(*pos).ok_or(NativeViewCodecError::Truncated)? {
            0 => { *pos += 1; Ok(NativeValueRef::Bool(false)) }
            1 => { *pos += 1; Ok(NativeValueRef::Bool(true)) }
            other => Err(NativeViewCodecError::InvalidBool(other)),
        },
        2 => Ok(NativeValueRef::U32(u32::from_le_bytes(take_arr(bytes, pos)?))),
        3 => Ok(NativeValueRef::I64(i64::from_le_bytes(take_arr(bytes, pos)?))),
        4 => Ok(NativeValueRef::U64(u64::from_le_bytes(take_arr(bytes, pos)?))),
        5 => Ok(NativeValueRef::F32(f32::from_le_bytes(take_arr(bytes, pos)?))),
        6 => Ok(NativeValueRef::F64(f64::from_le_bytes(take_arr(bytes, pos)?))),
        7 => Ok(NativeValueRef::Str(read_len_str(bytes, pos)?)),
        8 => { let len = u32::from_le_bytes(take_arr(bytes, pos)?) as usize; Ok(NativeValueRef::Bytes(take_slice(bytes, pos, len)?)) }
        other => Err(NativeViewCodecError::InvalidTag(other)),
    }
}

/// Validated zero-copy decoder for binary `NVNP` native view prop buffers.
#[derive(Debug, Clone)]
pub struct NativePropDecoder<'a> { fields: Vec<(u16, NativeValueRef<'a>)> }

impl<'a> NativePropDecoder<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, NativeViewCodecError> {
        if bytes.len() < NATIVE_PROPS_HEADER_SIZE { return Err(NativeViewCodecError::Truncated); }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != NATIVE_PROPS_MAGIC { return Err(NativeViewCodecError::InvalidMagic(magic)); }
        if bytes[4] != NATIVE_PROPS_VERSION { return Err(NativeViewCodecError::UnsupportedVersion(bytes[4])); }
        let count = usize::from(u16::from_le_bytes([bytes[5], bytes[6]]));
        if count > MAX_NATIVE_PROP_FIELDS { return Err(NativeViewCodecError::TooManyFields(count)); }
        let mut pos = NATIVE_PROPS_HEADER_SIZE;
        if count > (bytes.len() - pos) / 3 { return Err(NativeViewCodecError::Truncated); }
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            let id = u16::from_le_bytes(take_arr(bytes, &mut pos)?);
            fields.push((id, read_value(bytes, &mut pos)?));
        }
        if pos != bytes.len() { return Err(NativeViewCodecError::TrailingBytes(bytes.len() - pos)); }
        Ok(Self { fields })
    }
    pub fn get(&self, id: u16) -> Option<NativeValueRef<'a>> { self.fields.iter().find(|(k, _)| *k == id).map(|(_, v)| *v) }
    pub fn require(&self, id: u16) -> Result<NativeValueRef<'a>, NativeViewCodecError> { self.get(id).ok_or(NativeViewCodecError::MissingField(id)) }
}

/// Validated zero-copy decoder for binary `NVNC` imperative command payloads.
#[derive(Debug, Clone)]
pub struct NativeCommandDecoder<'a> { opcode: u16, args: Vec<NativeValueRef<'a>> }

impl<'a> NativeCommandDecoder<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, NativeViewCodecError> {
        if bytes.len() < NATIVE_CMD_HEADER_SIZE { return Err(NativeViewCodecError::Truncated); }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != NATIVE_CMD_MAGIC { return Err(NativeViewCodecError::InvalidMagic(magic)); }
        if bytes[4] != NATIVE_CMD_VERSION { return Err(NativeViewCodecError::UnsupportedVersion(bytes[4])); }
        let opcode = u16::from_le_bytes([bytes[5], bytes[6]]);
        let arg_count = usize::from(bytes[7]);
        if arg_count > MAX_NATIVE_CMD_ARGS { return Err(NativeViewCodecError::TooManyFields(arg_count)); }
        let mut pos = NATIVE_CMD_HEADER_SIZE;
        if arg_count > bytes.len() - pos { return Err(NativeViewCodecError::Truncated); }
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count { args.push(read_value(bytes, &mut pos)?); }
        if pos != bytes.len() { return Err(NativeViewCodecError::TrailingBytes(bytes.len() - pos)); }
        Ok(Self { opcode, args })
    }
    #[inline]
    pub fn opcode(&self) -> u16 { self.opcode }
    pub fn expect_arg_count(&self, expected: usize) -> Result<(), NativeViewCodecError> {
        if self.args.len() != expected { return Err(NativeViewCodecError::UnexpectedArgCount { expected, actual: self.args.len() }); }
        Ok(())
    }
    pub fn require(&self, idx: usize) -> Result<NativeValueRef<'a>, NativeViewCodecError> { self.args.get(idx).copied().ok_or(NativeViewCodecError::MissingArg(idx)) }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlurMaterial { UltraThin = 0, Thin = 1, Regular = 2, Thick = 3, Chrome = 4, SystemMaterial = 5 }

impl BlurMaterial {
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        match v {
            0 => Ok(Self::UltraThin), 1 => Ok(Self::Thin), 2 => Ok(Self::Regular),
            3 => Ok(Self::Thick), 4 => Ok(Self::Chrome), 5 => Ok(Self::SystemMaterial),
            other => Err(NativeViewCodecError::InvalidEnum("BlurMaterial", other)),
        }
    }
    fn css_params(self) -> (f32, &'static str) {
        match self {
            Self::UltraThin => (8.0, "0.08"), Self::Thin => (12.0, "0.14"), Self::Regular => (20.0, "0.22"),
            Self::Thick => (32.0, "0.34"), Self::Chrome => (24.0, "0.28"), Self::SystemMaterial => (16.0, "0.18"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurSurfaceProps { pub material: BlurMaterial, pub intensity: f32, pub tint: Option<u32> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebViewProps { pub url: String, pub allows_back_forward: bool }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebViewCommand { Navigate(String), Reload, GoBack, GoForward, EvaluateScript(String) }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPlayerProps { pub source_uri: String, pub autoplay: bool, pub muted: bool, pub looping: bool, pub controls: bool }
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoPlayerCommand { Play, Pause, SeekMs(u64), SetVolume(f32) }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebElementProps { pub tag: String, pub attrs: Vec<(String, String)> }
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraPreviewFacing { #[default] Back = 1, Front = 2, External = 3 }

impl CameraPreviewFacing {
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        match v { 1 => Ok(Self::Back), 2 => Ok(Self::Front), 3 => Ok(Self::External), other => Err(NativeViewCodecError::InvalidEnum("CameraPreviewFacing", other)) }
    }
    pub const fn as_str(self) -> &'static str { match self { Self::Back => "back", Self::Front => "front", Self::External => "external" } }
    #[cfg(target_arch = "wasm32")]
    pub const fn web_facing_mode(self) -> &'static str { match self { Self::Back => "environment", Self::Front => "user", Self::External => "" } }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TorchMode { #[default] Off = 0, On = 1, Auto = 2 }

impl TorchMode {
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        match v { 0 => Ok(Self::Off), 1 => Ok(Self::On), 2 => Ok(Self::Auto), other => Err(NativeViewCodecError::InvalidEnum("TorchMode", other)) }
    }
    pub const fn as_str(self) -> &'static str { match self { Self::Off => "off", Self::On => "on", Self::Auto => "auto" } }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StabilizationMode { #[default] Off = 0, Standard = 1, Cinematic = 2 }

impl StabilizationMode {
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        match v { 0 => Ok(Self::Off), 1 => Ok(Self::Standard), 2 => Ok(Self::Cinematic), other => Err(NativeViewCodecError::InvalidEnum("StabilizationMode", other)) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraPreviewProps {
    pub device_id: Option<String>, pub facing: CameraPreviewFacing, pub torch: TorchMode,
    pub zoom: f32, pub fps: u16, pub hdr: bool, pub stabilization: StabilizationMode,
    pub enable_frame_processor: bool, pub audio_enabled: bool, pub active: bool,
}

impl Default for CameraPreviewProps {
    fn default() -> Self {
        Self {
            device_id: None, facing: CameraPreviewFacing::Back, torch: TorchMode::Off, zoom: 1.0,
            fps: 30, hdr: false, stabilization: StabilizationMode::Off,
            enable_frame_processor: false, audio_enabled: false, active: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CameraPreviewCommand {
    SwitchFacing(CameraPreviewFacing), SetTorch(TorchMode), SetZoom(f32),
    FocusPoint { x: f32, y: f32 }, CapturePhoto { quality: u8, flash: u8 },
    StartRecording { output_path: String }, StopRecording,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CameraNodeState {
    pub props: CameraPreviewProps, pub recording: bool, pub last_recording_path: Option<String>,
    pub last_focus_point: Option<(f32, f32)>, pub photo_capture_count: u32,
    pub last_photo_quality: Option<u8>, pub last_photo_flash: Option<u8>,
}

impl core::ops::Deref for CameraNodeState {
    type Target = CameraPreviewProps;
    fn deref(&self) -> &Self::Target { &self.props }
}

impl core::ops::DerefMut for CameraNodeState {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.props }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub(crate) struct CameraStreamGuard { el: web_sys::Element }

#[cfg(target_arch = "wasm32")]
impl Drop for CameraStreamGuard {
    fn drop(&mut self) { nve_camera_stop(&self.el); }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedNativeView {
    BlurSurface(BlurSurfaceProps), WebView(WebViewProps),
    VideoPlayer(VideoPlayerProps), CameraPreview(CameraPreviewProps), WebElement(WebElementProps),
}

/// Host-side state tracked per mounted `ComponentKind::Custom` native view node.
#[derive(Debug, Clone)]
pub struct NativeViewNodeState {
    pub kind: String, pub resolved_tag: String, pub static_tag: Option<&'static str>,
    pub decoded: Option<DecodedNativeView>, pub extra_css: String, pub current_url: Option<String>,
    pub reload_count: u32, pub go_back_count: u32, pub go_forward_count: u32,
    pub playing: bool, pub position_ms: u64, pub volume: f32, pub camera: CameraNodeState,
    #[cfg(target_arch = "wasm32")] pub(crate) applied_attrs: Vec<String>,
    #[cfg(target_arch = "wasm32")] pub(crate) camera_guard: Option<std::rc::Rc<CameraStreamGuard>>,
}

impl NativeViewNodeState {
    fn new(kind: String) -> Self {
        Self {
            kind, resolved_tag: "div".to_owned(), static_tag: Some("div"), decoded: None,
            extra_css: String::new(), current_url: None, reload_count: 0, go_back_count: 0,
            go_forward_count: 0, playing: false, position_ms: 0, volume: 1.0, camera: CameraNodeState::default(),
            #[cfg(target_arch = "wasm32")] applied_attrs: Vec::new(),
            #[cfg(target_arch = "wasm32")] camera_guard: None,
        }
    }
    fn set_tag(&mut self, tag: String, static_tag: Option<&'static str>) { self.resolved_tag = tag; self.static_tag = static_tag; }
}

#[inline]
pub fn is_native_view_prop(key: &str) -> bool {
    matches!(key, NATIVE_VIEW_KIND_PROP_KEY | NATIVE_VIEW_KIND_PROP_ALIAS | NATIVE_VIEW_PROPS_PROP_KEY | NATIVE_VIEW_PROPS_PROP_ALIAS)
}

#[inline]
pub fn is_native_kind_prop(key: &str) -> bool { matches!(key, NATIVE_VIEW_KIND_PROP_KEY | NATIVE_VIEW_KIND_PROP_ALIAS) }

pub fn allowlisted_builtin_web_tag(tag: &str) -> Option<&'static str> {
    match tag {
        "video" => Some("video"), "audio" => Some("audio"), "svg" => Some("svg"), "iframe" => Some("iframe"),
        "canvas" => Some("canvas"), "input" => Some("input"), "select" => Some("select"), "textarea" => Some("textarea"),
        "progress" => Some("progress"), "meter" => Some("meter"), "details" => Some("details"), "summary" => Some("summary"),
        _ => None,
    }
}

/// Validates `"web.<tag>"` against the safe HTML5 allowlist or W3C custom-element rules.
pub fn validate_web_element_tag(tag: &str) -> Option<(String, Option<&'static str>)> {
    if let Some(st) = allowlisted_builtin_web_tag(tag) { return Some((st.to_owned(), Some(st))); }
    let bytes = tag.as_bytes();
    if tag.len() < 3 || tag.len() > 64 || !tag.contains('-') || tag.ends_with('-') || !bytes[0].is_ascii_lowercase()
        || !bytes.iter().all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_'))
        || matches!(tag, "annotation-xml" | "color-profile" | "font-face" | "font-face-src"
            | "font-face-uri" | "font-face-format" | "font-face-name" | "missing-glyph") {
        return None;
    }
    Some((tag.to_owned(), None))
}

/// Rejects `javascript:`, `vbscript:`, and `data:` URI schemes (even with embedded control/whitespace).
pub fn is_safe_web_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() { return false; }
    let norm: String = trimmed.chars().filter(|c| !c.is_ascii_control() && !c.is_ascii_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
    if norm.starts_with("javascript:") || norm.starts_with("vbscript:") || norm.starts_with("data:") { return false; }
    if norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank" { return true; }
    let sep = norm.find(['/', '?', '#']).unwrap_or(norm.len());
    !norm[..sep].contains(':')
}

/// Validates `WebElement` attributes, rejecting `on*` event handlers, `srcdoc`, `data-nve-id`, and unsafe URIs.
pub fn is_safe_web_attr(name: &str, value: &str) -> bool {
    let bytes = name.as_bytes();
    if name.is_empty() || name.len() > 64 || !bytes[0].is_ascii_alphabetic()
        || !bytes.iter().all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':')) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("on") || lower == "srcdoc" || lower == "data-nve-id" { return false; }
    if matches!(lower.as_str(), "src" | "href" | "xlink:href" | "action" | "formaction" | "poster" | "data") {
        return is_safe_web_url(value);
    }
    true
}

pub fn decode_blur_surface_props(dec: &NativePropDecoder<'_>) -> Result<BlurSurfaceProps, NativeViewCodecError> {
    Ok(BlurSurfaceProps {
        material: BlurMaterial::from_u32(dec.require(1)?.as_u32()?)?,
        intensity: dec.require(2)?.as_f32()?, tint: dec.require(3)?.as_opt_u32()?,
    })
}

pub fn decode_web_view_props(dec: &NativePropDecoder<'_>) -> Result<WebViewProps, NativeViewCodecError> {
    Ok(WebViewProps { url: dec.require(1)?.as_str()?.to_owned(), allows_back_forward: dec.require(2)?.as_bool()? })
}

pub fn decode_web_view_command(bytes: &[u8]) -> Result<WebViewCommand, NativeViewCodecError> {
    let dec = NativeCommandDecoder::decode(bytes)?;
    match dec.opcode() {
        1 => { dec.expect_arg_count(1)?; Ok(WebViewCommand::Navigate(dec.require(0)?.as_str()?.to_owned())) }
        2 => { dec.expect_arg_count(0)?; Ok(WebViewCommand::Reload) }
        3 => { dec.expect_arg_count(0)?; Ok(WebViewCommand::GoBack) }
        4 => { dec.expect_arg_count(0)?; Ok(WebViewCommand::GoForward) }
        5 => { dec.expect_arg_count(1)?; Ok(WebViewCommand::EvaluateScript(dec.require(0)?.as_str()?.to_owned())) }
        other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
    }
}

pub fn decode_video_player_props(dec: &NativePropDecoder<'_>) -> Result<VideoPlayerProps, NativeViewCodecError> {
    Ok(VideoPlayerProps {
        source_uri: dec.require(1)?.as_str()?.to_owned(), autoplay: dec.require(2)?.as_bool()?,
        muted: dec.require(3)?.as_bool()?, looping: dec.require(4)?.as_bool()?, controls: dec.require(5)?.as_bool()?,
    })
}

pub fn decode_video_player_command(bytes: &[u8]) -> Result<VideoPlayerCommand, NativeViewCodecError> {
    let dec = NativeCommandDecoder::decode(bytes)?;
    match dec.opcode() {
        1 => { dec.expect_arg_count(0)?; Ok(VideoPlayerCommand::Play) }
        2 => { dec.expect_arg_count(0)?; Ok(VideoPlayerCommand::Pause) }
        3 => { dec.expect_arg_count(1)?; Ok(VideoPlayerCommand::SeekMs(dec.require(0)?.as_u64()?)) }
        4 => { dec.expect_arg_count(1)?; Ok(VideoPlayerCommand::SetVolume(dec.require(0)?.as_f32()?)) }
        other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
    }
}

pub fn decode_camera_preview_props(dec: &NativePropDecoder<'_>) -> Result<CameraPreviewProps, NativeViewCodecError> {
    let device_id = dec.require(1)?.as_opt_str()?.map(str::to_owned);
    let facing = CameraPreviewFacing::from_u32(dec.require(2)?.as_u32()?)?;
    let torch = TorchMode::from_u32(dec.require(3)?.as_u32()?)?;
    let zoom = dec.require(4)?.as_f32()?;
    if !zoom.is_finite() || zoom <= 0.0 { return Err(NativeViewCodecError::TypeMismatch("finite positive zoom")); }
    let fps_raw = dec.require(5)?.as_u32()?;
    if !(1..=u32::from(MAX_CAMERA_FPS)).contains(&fps_raw) { return Err(NativeViewCodecError::InvalidEnum("CameraFps", fps_raw)); }
    Ok(CameraPreviewProps {
        device_id, facing, torch, zoom, fps: fps_raw as u16,
        hdr: dec.require(6)?.as_bool()?,
        stabilization: StabilizationMode::from_u32(dec.require(7)?.as_u32()?)?,
        enable_frame_processor: dec.require(8)?.as_bool()?,
        audio_enabled: dec.require(9)?.as_bool()?,
        active: dec.require(10)?.as_bool()?,
    })
}

pub fn decode_camera_preview_command(bytes: &[u8]) -> Result<CameraPreviewCommand, NativeViewCodecError> {
    let dec = NativeCommandDecoder::decode(bytes)?;
    match dec.opcode() {
        1 => { dec.expect_arg_count(1)?; Ok(CameraPreviewCommand::SwitchFacing(CameraPreviewFacing::from_u32(dec.require(0)?.as_u32()?)?)) }
        2 => { dec.expect_arg_count(1)?; Ok(CameraPreviewCommand::SetTorch(TorchMode::from_u32(dec.require(0)?.as_u32()?)?)) }
        3 => {
            dec.expect_arg_count(1)?;
            let zoom = dec.require(0)?.as_f32()?;
            if !zoom.is_finite() || zoom <= 0.0 { return Err(NativeViewCodecError::TypeMismatch("finite positive zoom")); }
            Ok(CameraPreviewCommand::SetZoom(zoom))
        }
        4 => {
            dec.expect_arg_count(2)?;
            let (x, y) = (dec.require(0)?.as_f32()?, dec.require(1)?.as_f32()?);
            if !x.is_finite() || !(0.0..=1.0).contains(&x) || !y.is_finite() || !(0.0..=1.0).contains(&y) {
                return Err(NativeViewCodecError::TypeMismatch("focus_point in 0.0..=1.0"));
            }
            Ok(CameraPreviewCommand::FocusPoint { x, y })
        }
        5 => {
            dec.expect_arg_count(2)?;
            let (q, f) = (dec.require(0)?.as_u32()?, dec.require(1)?.as_u32()?);
            if q > 100 { return Err(NativeViewCodecError::InvalidEnum("PhotoQuality", q)); }
            if f > 2 { return Err(NativeViewCodecError::InvalidEnum("FlashMode", f)); }
            Ok(CameraPreviewCommand::CapturePhoto { quality: q as u8, flash: f as u8 })
        }
        6 => { dec.expect_arg_count(1)?; Ok(CameraPreviewCommand::StartRecording { output_path: dec.require(0)?.as_str()?.to_owned() }) }
        7 => { dec.expect_arg_count(0)?; Ok(CameraPreviewCommand::StopRecording) }
        other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
    }
}

pub fn decode_web_element_props(dec: &NativePropDecoder<'_>) -> Result<WebElementProps, NativeViewCodecError> {
    let tag = dec.require(1)?.as_str()?.to_owned();
    let count = dec.require(2)?.as_u32()? as usize;
    let raw = dec.require(3)?.as_bytes()?;
    let mut pos = 0usize;
    if count > raw.len() / 8 { return Err(NativeViewCodecError::Truncated); }
    let mut attrs = Vec::with_capacity(count);
    for _ in 0..count {
        let (key, val) = (read_len_str(raw, &mut pos)?, read_len_str(raw, &mut pos)?);
        if is_safe_web_attr(key, val) { attrs.push((key.to_owned(), val.to_owned())); }
    }
    if pos != raw.len() { return Err(NativeViewCodecError::TrailingBytes(raw.len() - pos)); }
    Ok(WebElementProps { tag, attrs })
}

fn format_blur_css(props: BlurSurfaceProps) -> String {
    let (base_px, default_alpha) = props.material.css_params();
    let intensity = if props.intensity.is_finite() { props.intensity.clamp(0.0, 1.0) } else { 1.0 };
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
        KIND_BLUR_SURFACE | KIND_BLUR_ALIAS => {
            state.set_tag("div".to_owned(), Some("div"));
            if state.extra_css.is_empty() {
                state.extra_css = format_blur_css(BlurSurfaceProps { material: BlurMaterial::Regular, intensity: 1.0, tint: None });
            }
        }
        KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => state.set_tag("iframe".to_owned(), Some("iframe")),
        KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS | KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => {
            state.set_tag("video".to_owned(), Some("video"));
        }
        other => match other.strip_prefix("web.").and_then(validate_web_element_tag) {
            Some((tag, st)) => state.set_tag(tag, st),
            None => state.set_tag("div".to_owned(), Some("div")),
        },
    }
}

fn format_camera_zoom(zoom: f32) -> String {
    let rounded = (zoom * 100.0).round() / 100.0;
    let s = format!("{rounded:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn apply_nvnp_to_state(state: &mut NativeViewNodeState, bytes: &[u8]) -> bool {
    let Ok(dec) = NativePropDecoder::decode(bytes) else { return false; };
    let prev_tag = state.resolved_tag.clone();
    match state.kind.as_str() {
        KIND_BLUR_SURFACE | KIND_BLUR_ALIAS => if let Ok(p) = decode_blur_surface_props(&dec) {
            state.extra_css = format_blur_css(p); state.decoded = Some(DecodedNativeView::BlurSurface(p));
            state.set_tag("div".to_owned(), Some("div"));
        },
        KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => if let Ok(p) = decode_web_view_props(&dec) {
            state.current_url = is_safe_web_url(&p.url).then(|| p.url.clone()); state.decoded = Some(DecodedNativeView::WebView(p));
            state.set_tag("iframe".to_owned(), Some("iframe")); state.extra_css.clear();
        },
        KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => if let Ok(p) = decode_video_player_props(&dec) {
            state.playing = p.autoplay; state.decoded = Some(DecodedNativeView::VideoPlayer(p));
            state.set_tag("video".to_owned(), Some("video")); state.extra_css.clear();
        },
        KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => if let Ok(p) = decode_camera_preview_props(&dec) {
            state.camera.props = p.clone();
            state.decoded = Some(DecodedNativeView::CameraPreview(p));
            state.set_tag("video".to_owned(), Some("video")); state.extra_css.clear();
        },
        other if other.starts_with("web.") => if let Ok(p) = decode_web_element_props(&dec) {
            match validate_web_element_tag(&p.tag) {
                Some((tag, st)) => { state.set_tag(tag, st); state.decoded = Some(DecodedNativeView::WebElement(p)); }
                None => { state.set_tag("div".to_owned(), Some("div")); state.decoded = None; }
            }
            state.extra_css.clear();
        },
        _ => {}
    }
    state.resolved_tag != prev_tag
}

fn write_camera_html_attrs(cam: &CameraNodeState, out: &mut String) {
    out.push_str(" autoplay playsinline muted");
    write_attr("data-nve-camera", "1", out);
    write_attr("data-nve-camera-facing", cam.facing.as_str(), out);
    write_attr("data-nve-camera-torch", cam.torch.as_str(), out);
    write_attr("data-nve-camera-zoom", &format_camera_zoom(cam.zoom), out);
    write_attr("data-nve-camera-fps", &cam.fps.to_string(), out);
    write_attr("data-nve-camera-active", if cam.active { "1" } else { "0" }, out);
    write_attr("data-nve-camera-audio", if cam.audio_enabled { "1" } else { "0" }, out);
    if let Some(id) = cam.device_id.as_deref() && !id.is_empty() { write_attr("data-nve-camera-device", id, out); }
    if cam.recording { write_attr("data-nve-camera-recording", "1", out); }
}

pub(crate) fn write_native_view_html_attrs(state: &NativeViewNodeState, out: &mut String) {
    write_attr("data-nve-native-kind", &state.kind, out);
    match &state.decoded {
        Some(DecodedNativeView::BlurSurface(_)) => {}
        Some(DecodedNativeView::WebView(_)) => {
            write_attr("sandbox", WEBVIEW_SANDBOX_ATTR, out);
            write_attr("referrerpolicy", "no-referrer", out);
            if let Some(url) = state.current_url.as_deref() && is_safe_web_url(url) { write_attr("src", url, out); }
        }
        Some(DecodedNativeView::VideoPlayer(p)) => {
            if is_safe_web_url(&p.source_uri) { write_attr("src", &p.source_uri, out); }
            for (flag, name) in [(p.autoplay, " autoplay"), (p.muted, " muted"), (p.looping, " loop"), (p.controls, " controls")] {
                if flag { out.push_str(name); }
            }
            out.push_str(" playsinline");
        }
        Some(DecodedNativeView::CameraPreview(_)) => write_camera_html_attrs(&state.camera, out),
        Some(DecodedNativeView::WebElement(p)) => {
            for (k, v) in &p.attrs { if is_safe_web_attr(k, v) { write_attr(k, v, out); } }
        }
        None if matches!(state.kind.as_str(), KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS) => {
            write_attr("sandbox", WEBVIEW_SANDBOX_ATTR, out);
            write_attr("referrerpolicy", "no-referrer", out);
        }
        None if matches!(state.kind.as_str(), KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS) => {
            write_camera_html_attrs(&state.camera, out);
        }
        None => {}
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_video_play(el) { try { if (el && typeof el.play === 'function') { const p = el.play(); if (p && typeof p.catch === 'function') p.catch(() => {}); } } catch (_) {} }
export function nve_video_pause(el) { try { if (el && typeof el.pause === 'function') el.pause(); } catch (_) {} }
export function nve_video_seek_ms(el, ms) { try { if (el && 'currentTime' in el) el.currentTime = ms / 1000.0; } catch (_) {} }
export function nve_video_set_volume(el, vol) { try { if (el && 'volume' in el) el.volume = Math.max(0, Math.min(1, vol)); } catch (_) {} }
export function nve_video_set_muted(el, muted) { try { if (el && 'muted' in el) el.muted = !!muted; } catch (_) {} }
export function nve_iframe_reload(el) { try { if (el && el.contentWindow && el.contentWindow.location) { el.contentWindow.location.reload(); return; } } catch (_) {} try { if (el && el.getAttribute('src')) el.setAttribute('src', el.getAttribute('src')); } catch (_) {} }
export function nve_iframe_go_back(el) { try { if (el && el.contentWindow && el.contentWindow.history) el.contentWindow.history.back(); } catch (_) {} }
export function nve_iframe_go_forward(el) { try { if (el && el.contentWindow && el.contentWindow.history) el.contentWindow.history.forward(); } catch (_) {} }
function nve_cam_stop_stream(s) { try { if (s && typeof s.getTracks === 'function') s.getTracks().forEach((t) => { try { t.stop(); } catch (_) {} }); } catch (_) {} }
export function nve_camera_stop(el) { try { if (!el) return; el.__nveCamGen = (el.__nveCamGen || 0) + 1; if (el.__nveRecorder && el.__nveRecorder.state !== 'inactive') { try { el.__nveRecorder.stop(); } catch (_) {} } el.__nveRecorder = null; if (el.srcObject) { nve_cam_stop_stream(el.srcObject); el.srcObject = null; } el.__nveCamKey = ''; } catch (_) {} }
export function nve_camera_set_zoom(el, zoom) { try { if (!el) return; const z = Number.isFinite(zoom) && zoom > 0 ? zoom : 1; const vt = el.srcObject && typeof el.srcObject.getVideoTracks === 'function' ? el.srcObject.getVideoTracks()[0] : null; if (vt && typeof vt.applyConstraints === 'function') { try { vt.applyConstraints({ advanced: [{ zoom: z }] }).catch(() => {}); } catch (_) {} } if (el.style) el.style.transform = z > 1 ? ('scale(' + z + ')') : ''; } catch (_) {} }
export function nve_camera_set_torch(el, mode) { try { const vt = el && el.srcObject && typeof el.srcObject.getVideoTracks === 'function' ? el.srcObject.getVideoTracks()[0] : null; if (vt && typeof vt.applyConstraints === 'function') { try { vt.applyConstraints({ advanced: [{ torch: mode === 1 }] }).catch(() => {}); } catch (_) {} } } catch (_) {} }
export function nve_camera_attach(el, facingMode, deviceId, fps, audio, zoom, torch) { try { if (!el || typeof navigator === 'undefined' || !navigator.mediaDevices || typeof navigator.mediaDevices.getUserMedia !== 'function') return; const key = [facingMode, deviceId, fps, audio ? 1 : 0].join('|'); if (el.__nveCamKey === key && el.srcObject) { nve_camera_set_zoom(el, zoom); nve_camera_set_torch(el, torch); return; } nve_camera_stop(el); const gen = el.__nveCamGen || 1; el.__nveCamKey = key; const video = { frameRate: { ideal: fps } }; if (deviceId) video.deviceId = { exact: deviceId }; else if (facingMode) video.facingMode = { ideal: facingMode }; navigator.mediaDevices.getUserMedia({ video, audio: !!audio }).then((stream) => { if (el.__nveCamGen !== gen) { nve_cam_stop_stream(stream); return; } el.srcObject = stream; el.muted = true; el.playsInline = true; const p = typeof el.play === 'function' ? el.play() : null; if (p && typeof p.catch === 'function') p.catch(() => {}); nve_camera_set_zoom(el, zoom); nve_camera_set_torch(el, torch); }).catch(() => { if (el.__nveCamGen === gen) el.__nveCamKey = ''; }); } catch (_) {} }
export function nve_camera_capture_photo(el, quality) { try { if (!el || typeof document === 'undefined') return; const w = el.videoWidth || el.clientWidth || 0, h = el.videoHeight || el.clientHeight || 0; if (w <= 0 || h <= 0) return; const c = document.createElement('canvas'); c.width = w; c.height = h; const ctx = c.getContext('2d'); if (!ctx) return; ctx.drawImage(el, 0, 0, w, h); el.__nveLastPhotoDataUrl = c.toDataURL('image/jpeg', Math.max(0.01, Math.min(1, quality / 100.0))); el.setAttribute('data-nve-camera-last-photo', '1'); } catch (_) {} }
export function nve_camera_start_recording(el) { try { if (!el || !el.srcObject || typeof MediaRecorder === 'undefined' || (el.__nveRecorder && el.__nveRecorder.state !== 'inactive')) return; const chunks = [], rec = new MediaRecorder(el.srcObject); rec.ondataavailable = (ev) => { if (ev.data && ev.data.size > 0) chunks.push(ev.data); }; el.__nveRecChunks = chunks; el.__nveRecorder = rec; rec.start(); } catch (_) {} }
export function nve_camera_stop_recording(el) { try { if (el && el.__nveRecorder && el.__nveRecorder.state !== 'inactive') el.__nveRecorder.stop(); if (el) el.__nveRecorder = null; } catch (_) {} }
")]
extern "C" {
    fn nve_video_play(el: &web_sys::Element);
    fn nve_video_pause(el: &web_sys::Element);
    fn nve_video_seek_ms(el: &web_sys::Element, ms: f64);
    fn nve_video_set_volume(el: &web_sys::Element, vol: f32);
    fn nve_video_set_muted(el: &web_sys::Element, muted: bool);
    fn nve_iframe_reload(el: &web_sys::Element);
    fn nve_iframe_go_back(el: &web_sys::Element);
    fn nve_iframe_go_forward(el: &web_sys::Element);
    fn nve_camera_attach(el: &web_sys::Element, facing_mode: &str, device_id: &str, fps: u16, audio: bool, zoom: f32, torch: u8);
    fn nve_camera_stop(el: &web_sys::Element);
    fn nve_camera_set_zoom(el: &web_sys::Element, zoom: f32);
    fn nve_camera_set_torch(el: &web_sys::Element, mode: u8);
    fn nve_camera_capture_photo(el: &web_sys::Element, quality: u8);
    fn nve_camera_start_recording(el: &web_sys::Element);
    fn nve_camera_stop_recording(el: &web_sys::Element);
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn create_dom_element_for_tag(doc: &web_sys::Document, tag: &str) -> Option<web_sys::Element> {
    if tag == "svg" { doc.create_element_ns(Some("http://www.w3.org/2000/svg"), "svg").ok() } else { doc.create_element(tag).ok() }
}

#[cfg(target_arch = "wasm32")]
fn sync_bool_attr(el: &web_sys::Element, name: &str, on: bool) {
    if on && !el.has_attribute(name) { let _ = el.set_attribute(name, ""); }
    else if !on && el.has_attribute(name) { let _ = el.remove_attribute(name); }
}

#[cfg(target_arch = "wasm32")]
fn sync_opt_src(el: &web_sys::Element, url: Option<&str>) {
    if let Some(u) = url && is_safe_web_url(u) {
        if el.get_attribute("src").as_deref() != Some(u) { let _ = el.set_attribute("src", u); }
    } else if el.has_attribute("src") { let _ = el.remove_attribute("src"); }
}

#[cfg(target_arch = "wasm32")]
fn sync_camera_dom(el: &web_sys::Element, state: &mut NativeViewNodeState) {
    let cam = &state.camera;
    for n in ["autoplay", "playsinline", "muted"] { sync_bool_attr(el, n, true); }
    nve_video_set_muted(el, true);
    let _ = el.set_attribute("data-nve-camera", "1");
    let _ = el.set_attribute("data-nve-camera-facing", cam.facing.as_str());
    let _ = el.set_attribute("data-nve-camera-torch", cam.torch.as_str());
    let _ = el.set_attribute("data-nve-camera-zoom", &format_camera_zoom(cam.zoom));
    let _ = el.set_attribute("data-nve-camera-fps", &cam.fps.to_string());
    let _ = el.set_attribute("data-nve-camera-active", if cam.active { "1" } else { "0" });
    let _ = el.set_attribute("data-nve-camera-audio", if cam.audio_enabled { "1" } else { "0" });
    match cam.device_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => { let _ = el.set_attribute("data-nve-camera-device", id); }
        None => { let _ = el.remove_attribute("data-nve-camera-device"); }
    }
    if cam.recording { let _ = el.set_attribute("data-nve-camera-recording", "1"); } else { let _ = el.remove_attribute("data-nve-camera-recording"); }
    if cam.active {
        if !state.camera_guard.as_ref().is_some_and(|g| g.el.is_same_node(Some(el))) {
            state.camera_guard = Some(std::rc::Rc::new(CameraStreamGuard { el: el.clone() }));
        }
        nve_camera_attach(el, cam.facing.web_facing_mode(), cam.device_id.as_deref().unwrap_or(""), cam.fps, cam.audio_enabled, cam.zoom, cam.torch as u8);
    } else {
        state.camera_guard = None;
        nve_camera_stop(el);
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn sync_native_view_dom_attrs(el: &web_sys::Element, state: &mut NativeViewNodeState) {
    let _ = el.set_attribute("data-nve-native-kind", &state.kind);
    match &state.decoded {
        Some(DecodedNativeView::BlurSurface(_)) => {}
        Some(DecodedNativeView::WebView(_)) => {
            let _ = el.set_attribute("sandbox", WEBVIEW_SANDBOX_ATTR);
            let _ = el.set_attribute("referrerpolicy", "no-referrer");
            sync_opt_src(el, state.current_url.as_deref());
        }
        Some(DecodedNativeView::VideoPlayer(p)) => {
            sync_opt_src(el, Some(p.source_uri.as_str()));
            for (n, on) in [("autoplay", p.autoplay), ("muted", p.muted), ("loop", p.looping), ("controls", p.controls), ("playsinline", true)] { sync_bool_attr(el, n, on); }
            nve_video_set_muted(el, p.muted);
        }
        Some(DecodedNativeView::CameraPreview(_)) => sync_camera_dom(el, state),
        Some(DecodedNativeView::WebElement(p)) => {
            for old in state.applied_attrs.drain(..) { if !p.attrs.iter().any(|(k, _)| k == &old) { let _ = el.remove_attribute(&old); } }
            for (k, v) in &p.attrs {
                if is_safe_web_attr(k, v) { if el.get_attribute(k).as_deref() != Some(v.as_str()) { let _ = el.set_attribute(k, v); } state.applied_attrs.push(k.clone()); }
            }
        }
        None if matches!(state.kind.as_str(), KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS) => sync_camera_dom(el, state),
        None => {}
    }
}

impl WebBackend {
    pub fn native_view(&self, node_id: NodeId) -> Option<&NativeViewNodeState> { self.native_views.get(&node_id) }

    #[inline]
    pub(crate) fn effective_html_tag<'a>(&'a self, node_id: NodeId, el: &'a MirrorElement) -> &'a str {
        if el.kind == ComponentKind::Custom && let Some(nv) = self.native_views.get(&node_id) { return nv.resolved_tag.as_str(); }
        el.tag
    }

    pub(crate) fn init_custom_native_view(&mut self, node_id: NodeId, custom_name: &str) {
        let mut state = NativeViewNodeState::new(custom_name.to_owned());
        resolve_kind_defaults(&mut state);
        if let Some(el) = self.nodes.get_mut(&node_id) && let Some(st) = state.static_tag { el.tag = st; }
        self.native_views.insert(node_id, state);
    }

    pub(crate) fn apply_native_view_prop(&mut self, node_id: NodeId, key: &str, value: &RawPropValue, dirty_styles: &mut Vec<NodeId>) {
        if !self.nodes.get(&node_id).is_some_and(|n| n.kind == ComponentKind::Custom) { return; }
        let mut tag_changed = false;
        if is_native_kind_prop(key) {
            if let RawPropValue::String(kind_str) = value {
                let state = self.native_views.entry(node_id).or_insert_with(|| NativeViewNodeState::new(kind_str.clone()));
                let prev = state.resolved_tag.clone();
                state.kind = kind_str.clone();
                resolve_kind_defaults(state);
                if let Some(RawPropValue::Bytes(raw)) = self.nodes.get(&node_id).and_then(|n| {
                    n.raw_props.get(NATIVE_VIEW_PROPS_PROP_KEY).or_else(|| n.raw_props.get(NATIVE_VIEW_PROPS_PROP_ALIAS))
                }) { let _ = apply_nvnp_to_state(state, raw); }
                tag_changed = state.resolved_tag != prev;
            }
        } else if let RawPropValue::Bytes(raw) = value {
            let fallback = self.nodes.get(&node_id).and_then(|n| {
                n.raw_props.get(NATIVE_VIEW_KIND_PROP_KEY).or_else(|| n.raw_props.get(NATIVE_VIEW_KIND_PROP_ALIAS))
            }).and_then(|v| if let RawPropValue::String(s) = v { Some(s.clone()) } else { None }).unwrap_or_default();
            let state = self.native_views.entry(node_id).or_insert_with(|| NativeViewNodeState::new(fallback));
            tag_changed = apply_nvnp_to_state(state, raw);
        }
        if let Some(state) = self.native_views.get(&node_id) && let Some(el) = self.nodes.get_mut(&node_id) && let Some(st) = state.static_tag {
            el.tag = st;
        }
        dirty_styles.push(node_id);
        #[cfg(target_arch = "wasm32")]
        {
            if tag_changed && !self.hydrating { self.upgrade_dom_element_tag(node_id); }
            if let (Some(dom_el), Some(state)) = (self.dom_nodes.get(&node_id), self.native_views.get_mut(&node_id)) {
                sync_native_view_dom_attrs(dom_el, state);
            }
        }
        #[cfg(not(target_arch = "wasm32"))] let _ = tag_changed;
    }

    pub(crate) fn dispatch_native_view_command(&mut self, node_id: NodeId, _command_name: &str, payload: &[u8]) -> bool {
        let Some(state) = self.native_views.get_mut(&node_id) else { return false; };
        match state.kind.as_str() {
            KIND_WEB_VIEW | KIND_WEB_VIEW_ALIAS => {
                let Ok(cmd) = decode_web_view_command(payload) else { return false; };
                let allows_bf = !matches!(&state.decoded, Some(DecodedNativeView::WebView(p)) if !p.allows_back_forward);
                match cmd {
                    WebViewCommand::Navigate(url) if is_safe_web_url(&url) => {
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { let _ = el.set_attribute("src", &url); }
                        state.current_url = Some(url); true
                    }
                    WebViewCommand::Reload => {
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { nve_iframe_reload(el); }
                        state.reload_count = state.reload_count.saturating_add(1); true
                    }
                    WebViewCommand::GoBack if allows_bf => {
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { nve_iframe_go_back(el); }
                        state.go_back_count = state.go_back_count.saturating_add(1); true
                    }
                    WebViewCommand::GoForward if allows_bf => {
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { nve_iframe_go_forward(el); }
                        state.go_forward_count = state.go_forward_count.saturating_add(1); true
                    }
                    _ => false,
                }
            }
            KIND_VIDEO_PLAYER | KIND_VIDEO_ALIAS => {
                let Ok(cmd) = decode_video_player_command(payload) else { return false; };
                match cmd {
                    VideoPlayerCommand::Play => {
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { nve_video_play(el); }
                        state.playing = true; true
                    }
                    VideoPlayerCommand::Pause => {
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { nve_video_pause(el); }
                        state.playing = false; true
                    }
                    VideoPlayerCommand::SeekMs(ms) => {
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { nve_video_seek_ms(el, ms as f64); }
                        state.position_ms = ms; true
                    }
                    VideoPlayerCommand::SetVolume(vol) if vol.is_finite() => {
                        state.volume = vol.clamp(0.0, 1.0);
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { nve_video_set_volume(el, state.volume); }
                        true
                    }
                    _ => false,
                }
            }
            KIND_CAMERA_PREVIEW | KIND_CAMERA_ALIAS => {
                let Ok(cmd) = decode_camera_preview_command(payload) else { return false; };
                match cmd {
                    CameraPreviewCommand::SwitchFacing(facing) => {
                        state.camera.facing = facing;
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { sync_camera_dom(el, state); }
                        true
                    }
                    CameraPreviewCommand::SetTorch(mode) => {
                        state.camera.torch = mode;
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) {
                            let _ = el.set_attribute("data-nve-camera-torch", mode.as_str());
                            nve_camera_set_torch(el, mode as u8);
                        }
                        true
                    }
                    CameraPreviewCommand::SetZoom(zoom) => {
                        state.camera.zoom = zoom;
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) {
                            let _ = el.set_attribute("data-nve-camera-zoom", &format_camera_zoom(zoom));
                            nve_camera_set_zoom(el, zoom);
                        }
                        true
                    }
                    CameraPreviewCommand::FocusPoint { x, y } => { state.camera.last_focus_point = Some((x, y)); true }
                    CameraPreviewCommand::CapturePhoto { quality, flash } => {
                        state.camera.photo_capture_count = state.camera.photo_capture_count.saturating_add(1);
                        state.camera.last_photo_quality = Some(quality);
                        state.camera.last_photo_flash = Some(flash);
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) { nve_camera_capture_photo(el, quality); }
                        true
                    }
                    CameraPreviewCommand::StartRecording { output_path } => {
                        state.camera.recording = true;
                        state.camera.last_recording_path = Some(output_path);
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) {
                            let _ = el.set_attribute("data-nve-camera-recording", "1");
                            nve_camera_start_recording(el);
                        }
                        true
                    }
                    CameraPreviewCommand::StopRecording => {
                        state.camera.recording = false;
                        #[cfg(target_arch = "wasm32")] if let Some(el) = self.dom_nodes.get(&node_id) {
                            let _ = el.remove_attribute("data-nve-camera-recording");
                            nve_camera_stop_recording(el);
                        }
                        true
                    }
                }
            }
            _ => false,
        }
    }
}
