//! Starter typed zero-JSON [`NativeViewSpec`] implementations:
//! `BlurSurface`, `WebView`, `VideoPlayer`, and `WebElement`.

use std::borrow::Cow;

use super::codec::{NativeCommandDecoder, NativePropDecoder, NativePropEncoder};
use super::wire::NativeViewCodecError;
use super::handle::{
    cmd_encoder, DecodeNativeCommand, DecodeNativeProps, EncodeNativeCommand, EncodeNativeProps,
    NativeViewSpec, NoCommand,
};

// =============================================================================
// 1. BlurSurface
// =============================================================================

/// Platform-native backdrop blur material style (AppKit `NSVisualEffectView`, UIKit
/// `UIBlurEffect`, WinUI Acrylic/Mica, CSS `backdrop-filter`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlurMaterial {
    UltraThin = 0,
    Thin = 1,
    #[default]
    Regular = 2,
    Thick = 3,
    Chrome = 4,
    SystemMaterial = 5,
}

impl BlurMaterial {
    pub fn from_u32(v: u32) -> Result<Self, NativeViewCodecError> {
        match v {
            0 => Ok(Self::UltraThin),
            1 => Ok(Self::Thin),
            2 => Ok(Self::Regular),
            3 => Ok(Self::Thick),
            4 => Ok(Self::Chrome),
            5 => Ok(Self::SystemMaterial),
            other => Err(NativeViewCodecError::InvalidEnum("BlurMaterial", other)),
        }
    }
}

/// Typed props for [`BlurSurface`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurSurfaceProps {
    pub material: BlurMaterial,
    pub intensity: f32,
    /// Optional packed `0xRRGGBBAA` tint overlay.
    pub tint: Option<u32>,
}

impl Default for BlurSurfaceProps {
    fn default() -> Self {
        Self {
            material: BlurMaterial::Regular,
            intensity: 1.0,
            tint: None,
        }
    }
}

impl EncodeNativeProps for BlurSurfaceProps {
    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        enc.u32_field(1, self.material as u32)?
            .f32_field(2, self.intensity)?
            .opt_u32_field(3, self.tint)?;
        Ok(())
    }
}

impl DecodeNativeProps for BlurSurfaceProps {
    fn decode_props(dec: &NativePropDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        let material = BlurMaterial::from_u32(dec.require_u32(1)?)?;
        let intensity = dec.require_f32(2)?;
        let tint = dec.require_opt_u32(3)?;
        Ok(Self {
            material,
            intensity,
            tint,
        })
    }
}

/// Native backdrop blur surface escape hatch (`KIND = "nve.blur_surface"`).
pub struct BlurSurface;

impl NativeViewSpec for BlurSurface {
    const KIND: &'static str = "nve.blur_surface";
    type Props = BlurSurfaceProps;
    type Command = NoCommand;
}

// =============================================================================
// 2. WebView
// =============================================================================

/// Typed props for [`WebView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebViewProps {
    pub url: String,
    pub allows_back_forward: bool,
}

impl WebViewProps {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            allows_back_forward: true,
        }
    }
}

impl EncodeNativeProps for WebViewProps {
    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        enc.str_field(1, &self.url)?
            .bool_field(2, self.allows_back_forward)?;
        Ok(())
    }
}

impl DecodeNativeProps for WebViewProps {
    fn decode_props(dec: &NativePropDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        Ok(Self {
            url: dec.require_str(1)?.to_owned(),
            allows_back_forward: dec.require_bool(2)?,
        })
    }
}

/// Typed imperative commands for [`WebView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebViewCommand {
    Navigate(String),
    Reload,
    GoBack,
    GoForward,
    EvaluateScript(String),
}

impl WebViewCommand {
    const OP_NAVIGATE: u16 = 1;
    const OP_RELOAD: u16 = 2;
    const OP_GO_BACK: u16 = 3;
    const OP_GO_FORWARD: u16 = 4;
    const OP_EVALUATE_SCRIPT: u16 = 5;
}

impl EncodeNativeCommand for WebViewCommand {
    fn command_name(&self) -> &'static str {
        match self {
            Self::Navigate(_) => "navigate",
            Self::Reload => "reload",
            Self::GoBack => "go_back",
            Self::GoForward => "go_forward",
            Self::EvaluateScript(_) => "evaluate_script",
        }
    }

    fn encode_command(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        match self {
            Self::Navigate(url) => {
                let mut enc = cmd_encoder(Self::OP_NAVIGATE);
                enc.arg_str(url)?;
                Ok(enc.finish())
            }
            Self::Reload => Ok(cmd_encoder(Self::OP_RELOAD).finish()),
            Self::GoBack => Ok(cmd_encoder(Self::OP_GO_BACK).finish()),
            Self::GoForward => Ok(cmd_encoder(Self::OP_GO_FORWARD).finish()),
            Self::EvaluateScript(script) => {
                let mut enc = cmd_encoder(Self::OP_EVALUATE_SCRIPT);
                enc.arg_str(script)?;
                Ok(enc.finish())
            }
        }
    }
}

impl DecodeNativeCommand for WebViewCommand {
    fn decode_command(dec: &NativeCommandDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        match dec.opcode() {
            Self::OP_NAVIGATE => {
                dec.expect_arg_count(1)?;
                Ok(Self::Navigate(dec.require_str(0)?.to_owned()))
            }
            Self::OP_RELOAD => {
                dec.expect_arg_count(0)?;
                Ok(Self::Reload)
            }
            Self::OP_GO_BACK => {
                dec.expect_arg_count(0)?;
                Ok(Self::GoBack)
            }
            Self::OP_GO_FORWARD => {
                dec.expect_arg_count(0)?;
                Ok(Self::GoForward)
            }
            Self::OP_EVALUATE_SCRIPT => {
                dec.expect_arg_count(1)?;
                Ok(Self::EvaluateScript(dec.require_str(0)?.to_owned()))
            }
            other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
        }
    }
}

/// Native embedded WebView escape hatch (`KIND = "nve.web_view"`).
pub struct WebView;

impl NativeViewSpec for WebView {
    const KIND: &'static str = "nve.web_view";
    type Props = WebViewProps;
    type Command = WebViewCommand;
}

// =============================================================================
// 3. VideoPlayer
// =============================================================================

/// Typed props for [`VideoPlayer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPlayerProps {
    pub source_uri: String,
    pub autoplay: bool,
    pub muted: bool,
    pub looping: bool,
    pub controls: bool,
}

impl VideoPlayerProps {
    pub fn new(source_uri: impl Into<String>) -> Self {
        Self {
            source_uri: source_uri.into(),
            autoplay: false,
            muted: false,
            looping: false,
            controls: true,
        }
    }
}

impl EncodeNativeProps for VideoPlayerProps {
    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        enc.str_field(1, &self.source_uri)?
            .bool_field(2, self.autoplay)?
            .bool_field(3, self.muted)?
            .bool_field(4, self.looping)?
            .bool_field(5, self.controls)?;
        Ok(())
    }
}

impl DecodeNativeProps for VideoPlayerProps {
    fn decode_props(dec: &NativePropDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        Ok(Self {
            source_uri: dec.require_str(1)?.to_owned(),
            autoplay: dec.require_bool(2)?,
            muted: dec.require_bool(3)?,
            looping: dec.require_bool(4)?,
            controls: dec.require_bool(5)?,
        })
    }
}

/// Typed imperative commands for [`VideoPlayer`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoPlayerCommand {
    Play,
    Pause,
    SeekMs(u64),
    SetVolume(f32),
}

impl VideoPlayerCommand {
    const OP_PLAY: u16 = 1;
    const OP_PAUSE: u16 = 2;
    const OP_SEEK_MS: u16 = 3;
    const OP_SET_VOLUME: u16 = 4;
}

impl EncodeNativeCommand for VideoPlayerCommand {
    fn command_name(&self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::SeekMs(_) => "seek_ms",
            Self::SetVolume(_) => "set_volume",
        }
    }

    fn encode_command(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        match self {
            Self::Play => Ok(cmd_encoder(Self::OP_PLAY).finish()),
            Self::Pause => Ok(cmd_encoder(Self::OP_PAUSE).finish()),
            Self::SeekMs(ms) => {
                let mut enc = cmd_encoder(Self::OP_SEEK_MS);
                enc.arg_u64(*ms)?;
                Ok(enc.finish())
            }
            Self::SetVolume(vol) => {
                let mut enc = cmd_encoder(Self::OP_SET_VOLUME);
                enc.arg_f32(*vol)?;
                Ok(enc.finish())
            }
        }
    }
}

impl DecodeNativeCommand for VideoPlayerCommand {
    fn decode_command(dec: &NativeCommandDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        match dec.opcode() {
            Self::OP_PLAY => {
                dec.expect_arg_count(0)?;
                Ok(Self::Play)
            }
            Self::OP_PAUSE => {
                dec.expect_arg_count(0)?;
                Ok(Self::Pause)
            }
            Self::OP_SEEK_MS => {
                dec.expect_arg_count(1)?;
                Ok(Self::SeekMs(dec.require_u64(0)?))
            }
            Self::OP_SET_VOLUME => {
                dec.expect_arg_count(1)?;
                Ok(Self::SetVolume(dec.require_f32(0)?))
            }
            other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
        }
    }
}

/// Native hardware-accelerated video player escape hatch (`KIND = "nve.video_player"`).
pub struct VideoPlayer;

impl NativeViewSpec for VideoPlayer {
    const KIND: &'static str = "nve.video_player";
    type Props = VideoPlayerProps;
    type Command = VideoPlayerCommand;
}

// =============================================================================
// 4. WebElement (`"web.<tag>"` passthrough on Web DOM hosts)
// =============================================================================

/// Typed props for [`WebElement`], mapping `tag` (e.g. `"video"`, `"svg"`, `"iframe"`,
/// `"canvas"`, or a custom Web Component tag) to `"web.<tag>"` on Web DOM hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebElementProps {
    pub tag: &'static str,
    pub attrs: Vec<(String, String)>,
}

impl WebElementProps {
    pub const fn new(tag: &'static str) -> Self {
        Self {
            tag,
            attrs: Vec::new(),
        }
    }

    pub fn with_attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attrs.push((name.into(), value.into()));
        self
    }

    /// Returns the concrete `"web.<tag>"` kind string for registry lookup and node creation.
    pub fn resolved_kind(&self) -> String {
        format!("web.{}", self.tag)
    }
}

impl EncodeNativeProps for WebElementProps {
    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        enc.str_field(1, self.tag)?;
        let count = u32::try_from(self.attrs.len())
            .map_err(|_| NativeViewCodecError::TooManyFields(self.attrs.len()))?;
        enc.u32_field(2, count)?;
        // Pack attrs as a single binary byte slice (`(klen:u32, key, vlen:u32, val)*`) — zero JSON.
        let mut attr_buf = Vec::new();
        for (k, v) in &self.attrs {
            let kb = k.as_bytes();
            let vb = v.as_bytes();
            let klen =
                u32::try_from(kb.len()).map_err(|_| NativeViewCodecError::PayloadTooLarge)?;
            let vlen =
                u32::try_from(vb.len()).map_err(|_| NativeViewCodecError::PayloadTooLarge)?;
            attr_buf.extend_from_slice(&klen.to_le_bytes());
            attr_buf.extend_from_slice(kb);
            attr_buf.extend_from_slice(&vlen.to_le_bytes());
            attr_buf.extend_from_slice(vb);
        }
        enc.bytes_field(3, &attr_buf)?;
        Ok(())
    }
}

/// Decoded owned representation of [`WebElementProps`] (host-side decoding of `NVNP` bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedWebElementProps {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
}

impl DecodeNativeProps for DecodedWebElementProps {
    fn decode_props(dec: &NativePropDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        let tag = dec.require_str(1)?.to_owned();
        let count = dec.require_u32(2)? as usize;
        let raw = dec.require(3)?.as_bytes()?;
        let mut pos = 0usize;
        if count > raw.len() / 8 {
            return Err(NativeViewCodecError::Truncated);
        }
        let mut attrs = Vec::with_capacity(count);
        for _ in 0..count {
            if pos + 4 > raw.len() {
                return Err(NativeViewCodecError::Truncated);
            }
            let klen = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            let kend = pos
                .checked_add(klen)
                .ok_or(NativeViewCodecError::Truncated)?;
            if kend > raw.len() {
                return Err(NativeViewCodecError::Truncated);
            }
            let key = std::str::from_utf8(&raw[pos..kend])
                .map_err(|_| NativeViewCodecError::InvalidUtf8)?
                .to_owned();
            pos = kend;

            if pos + 4 > raw.len() {
                return Err(NativeViewCodecError::Truncated);
            }
            let vlen = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            let vend = pos
                .checked_add(vlen)
                .ok_or(NativeViewCodecError::Truncated)?;
            if vend > raw.len() {
                return Err(NativeViewCodecError::Truncated);
            }
            let val = std::str::from_utf8(&raw[pos..vend])
                .map_err(|_| NativeViewCodecError::InvalidUtf8)?
                .to_owned();
            pos = vend;

            attrs.push((key, val));
        }
        if pos != raw.len() {
            return Err(NativeViewCodecError::TrailingBytes(raw.len() - pos));
        }
        Ok(Self { tag, attrs })
    }
}

/// Web DOM element passthrough escape hatch (`KIND = "web.*"`, resolves to `"web.<tag>"`).
pub struct WebElement;

impl NativeViewSpec for WebElement {
    const KIND: &'static str = "web.*";
    type Props = WebElementProps;
    type Command = NoCommand;

    #[inline]
    fn resolved_kind(props: &Self::Props) -> Cow<'static, str> {
        Cow::Owned(props.resolved_kind())
    }
}
