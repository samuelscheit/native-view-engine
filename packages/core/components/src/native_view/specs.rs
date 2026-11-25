//! Canonical `NativeViewSpec` traits and built-in platform view contracts:
//! `BlurSurface`, `WebView`, `VideoPlayer`, and `WebElement`.
//!
//! Every spec supports BOTH zero-serialization in-process typed passing (`to_native_props` /
//! `from_native_props`, `to_native_command` / `from_native_command`) and cross-language FFI
//! binary `NVNP`/`NVNC` encoding/decoding via one single-source implementation.

use std::borrow::Cow;

use super::codec::{
    NativeCommandDecoder, NativeCommandEncoder, NativeCommandSink, NativeCommandSource,
    NativePropDecoder, NativePropEncoder, NativePropSink, NativePropSource,
};
use super::typed::{NativeCommand, NativeCommandBuilder, NativeProps, NativePropsBuilder};
use super::wire::{read_len_str, NativeViewCodecError};

// =============================================================================
// Core NativeViewSpec & Prop/Command Traits
// =============================================================================

/// Encodes typed native view props into either in-process [`NativeProps`] (zero serialization)
/// or a cross-language FFI `NVNP` binary buffer.
pub trait EncodeNativeProps {
    fn write_props<S: NativePropSink>(&self, sink: &mut S) -> Result<(), NativeViewCodecError>;

    /// Encodes into a cross-language `NVNP` binary encoder.
    #[inline]
    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        self.write_props(enc)
    }

    /// Zero-serialization in-process typed [`NativeProps`] conversion.
    #[inline]
    fn to_native_props(&self) -> Result<NativeProps, NativeViewCodecError> {
        let mut builder = NativePropsBuilder::new();
        self.write_props(&mut builder)?;
        Ok(builder.finish())
    }

    /// Cross-language FFI `NVNP` binary byte encoding.
    #[inline]
    fn to_prop_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        let mut enc = NativePropEncoder::new();
        self.write_props(&mut enc)?;
        Ok(enc.finish())
    }
}

/// Decodes typed native view props from either in-process [`NativeProps`] (zero decoding)
/// or a validated cross-language `NVNP` binary buffer.
pub trait DecodeNativeProps: Sized {
    fn read_props<S: NativePropSource>(src: &S) -> Result<Self, NativeViewCodecError>;

    #[inline]
    fn decode_props(dec: &NativePropDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        Self::read_props(dec)
    }

    #[inline]
    fn from_native_props(props: &NativeProps) -> Result<Self, NativeViewCodecError> {
        Self::read_props(props)
    }

    #[inline]
    fn from_prop_bytes(bytes: &[u8]) -> Result<Self, NativeViewCodecError> {
        let dec = NativePropDecoder::decode(bytes)?;
        Self::read_props(&dec)
    }
}

/// Encodes typed imperative native view commands into either in-process [`NativeCommand`]
/// (zero serialization) or cross-language `NVNC` binary bytes.
pub trait EncodeNativeCommand {
    /// Wire command name carried in `Command::DispatchCommand::command_name`.
    fn command_name(&self) -> &'static str;

    /// Numeric command opcode aligned with cross-language `NVNC` host registries.
    fn command_opcode(&self) -> u16;

    /// Writes typed command arguments into `sink`.
    fn write_command_args<S: NativeCommandSink>(
        &self,
        sink: &mut S,
    ) -> Result<(), NativeViewCodecError>
    where
        Self: Sized;

    /// Zero-serialization in-process typed [`NativeCommand`] conversion.
    fn to_native_command(&self) -> Result<NativeCommand, NativeViewCodecError>
    where
        Self: Sized,
    {
        let mut builder = NativeCommandBuilder::new(self.command_opcode(), self.command_name());
        self.write_command_args(&mut builder)?;
        Ok(builder.finish())
    }

    /// Encodes the command opcode and binary arguments into cross-language `NVNC` bytes.
    fn encode_command(&self) -> Result<Vec<u8>, NativeViewCodecError>
    where
        Self: Sized,
    {
        let mut enc = NativeCommandEncoder::new(self.command_opcode());
        self.write_command_args(&mut enc)?;
        Ok(enc.finish())
    }
}

/// Decodes typed imperative native view commands from either in-process [`NativeCommand`]
/// or a validated cross-language `NVNC` binary payload.
pub trait DecodeNativeCommand: Sized {
    fn read_command<S: NativeCommandSource>(src: &S) -> Result<Self, NativeViewCodecError>;

    #[inline]
    fn decode_command(dec: &NativeCommandDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        Self::read_command(dec)
    }

    #[inline]
    fn from_native_command(cmd: &NativeCommand) -> Result<Self, NativeViewCodecError> {
        Self::read_command(cmd)
    }

    #[inline]
    fn from_command_bytes(bytes: &[u8]) -> Result<Self, NativeViewCodecError> {
        let dec = NativeCommandDecoder::decode(bytes)?;
        Self::read_command(&dec)
    }
}

/// Marker command type for declarative native views that do not expose imperative commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoCommand {}

impl EncodeNativeCommand for NoCommand {
    fn command_name(&self) -> &'static str {
        match *self {}
    }

    fn command_opcode(&self) -> u16 {
        match *self {}
    }

    fn write_command_args<S: NativeCommandSink>(
        &self,
        _sink: &mut S,
    ) -> Result<(), NativeViewCodecError> {
        match *self {}
    }
}

impl DecodeNativeCommand for NoCommand {
    fn read_command<S: NativeCommandSource>(src: &S) -> Result<Self, NativeViewCodecError> {
        Err(NativeViewCodecError::UnknownCommandOpcode(src.opcode()))
    }
}

/// First-class typed contract for a native platform view primitive mounted via `<NativeView>`.
pub trait NativeViewSpec: 'static {
    /// Canonical native view kind string (e.g. `"nve.blur_surface"`, `"nve.web_view"`, `"web.*"`).
    const KIND: &'static str;
    type Props: EncodeNativeProps + Clone + PartialEq + Send + Sync + 'static;
    type Command: EncodeNativeCommand + Send + Sync + 'static;

    /// Resolves the concrete native kind emitted for a specific props instance.
    #[inline]
    fn resolved_kind(_props: &Self::Props) -> Cow<'static, str> {
        Cow::Borrowed(Self::KIND)
    }
}

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
    fn write_props<S: NativePropSink>(&self, sink: &mut S) -> Result<(), NativeViewCodecError> {
        sink.u32_field(1, self.material as u32)?
            .f32_field(2, self.intensity)?
            .opt_u32_field(3, self.tint)?;
        Ok(())
    }

    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        self.write_props(enc)
    }
}

impl DecodeNativeProps for BlurSurfaceProps {
    fn read_props<S: NativePropSource>(src: &S) -> Result<Self, NativeViewCodecError> {
        Ok(Self {
            material: BlurMaterial::from_u32(src.require_u32_field(1)?)?,
            intensity: src.require_f32_field(2)?,
            tint: src.require_opt_u32_field(3)?,
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
    fn write_props<S: NativePropSink>(&self, sink: &mut S) -> Result<(), NativeViewCodecError> {
        sink.str_field(1, &self.url)?
            .bool_field(2, self.allows_back_forward)?;
        Ok(())
    }

    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        self.write_props(enc)
    }
}

impl DecodeNativeProps for WebViewProps {
    fn read_props<S: NativePropSource>(src: &S) -> Result<Self, NativeViewCodecError> {
        Ok(Self {
            url: src.require_str_field(1)?.to_owned(),
            allows_back_forward: src.require_bool_field(2)?,
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
    pub const OP_NAVIGATE: u16 = 1;
    pub const OP_RELOAD: u16 = 2;
    pub const OP_GO_BACK: u16 = 3;
    pub const OP_GO_FORWARD: u16 = 4;
    pub const OP_EVALUATE_SCRIPT: u16 = 5;
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

    fn command_opcode(&self) -> u16 {
        match self {
            Self::Navigate(_) => Self::OP_NAVIGATE,
            Self::Reload => Self::OP_RELOAD,
            Self::GoBack => Self::OP_GO_BACK,
            Self::GoForward => Self::OP_GO_FORWARD,
            Self::EvaluateScript(_) => Self::OP_EVALUATE_SCRIPT,
        }
    }

    fn write_command_args<S: NativeCommandSink>(
        &self,
        sink: &mut S,
    ) -> Result<(), NativeViewCodecError> {
        match self {
            Self::Navigate(url) => {
                sink.arg_str(url)?;
            }
            Self::Reload | Self::GoBack | Self::GoForward => {}
            Self::EvaluateScript(script) => {
                sink.arg_str(script)?;
            }
        }
        Ok(())
    }
}

impl DecodeNativeCommand for WebViewCommand {
    fn read_command<S: NativeCommandSource>(src: &S) -> Result<Self, NativeViewCodecError> {
        match src.opcode() {
            Self::OP_NAVIGATE => {
                src.expect_arg_count(1)?;
                Ok(Self::Navigate(src.require_str_arg(0)?.to_owned()))
            }
            Self::OP_RELOAD => {
                src.expect_arg_count(0)?;
                Ok(Self::Reload)
            }
            Self::OP_GO_BACK => {
                src.expect_arg_count(0)?;
                Ok(Self::GoBack)
            }
            Self::OP_GO_FORWARD => {
                src.expect_arg_count(0)?;
                Ok(Self::GoForward)
            }
            Self::OP_EVALUATE_SCRIPT => {
                src.expect_arg_count(1)?;
                Ok(Self::EvaluateScript(src.require_str_arg(0)?.to_owned()))
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
    fn write_props<S: NativePropSink>(&self, sink: &mut S) -> Result<(), NativeViewCodecError> {
        sink.str_field(1, &self.source_uri)?
            .bool_field(2, self.autoplay)?
            .bool_field(3, self.muted)?
            .bool_field(4, self.looping)?
            .bool_field(5, self.controls)?;
        Ok(())
    }

    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        self.write_props(enc)
    }
}

impl DecodeNativeProps for VideoPlayerProps {
    fn read_props<S: NativePropSource>(src: &S) -> Result<Self, NativeViewCodecError> {
        Ok(Self {
            source_uri: src.require_str_field(1)?.to_owned(),
            autoplay: src.require_bool_field(2)?,
            muted: src.require_bool_field(3)?,
            looping: src.require_bool_field(4)?,
            controls: src.require_bool_field(5)?,
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
    pub const OP_PLAY: u16 = 1;
    pub const OP_PAUSE: u16 = 2;
    pub const OP_SEEK_MS: u16 = 3;
    pub const OP_SET_VOLUME: u16 = 4;
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

    fn command_opcode(&self) -> u16 {
        match self {
            Self::Play => Self::OP_PLAY,
            Self::Pause => Self::OP_PAUSE,
            Self::SeekMs(_) => Self::OP_SEEK_MS,
            Self::SetVolume(_) => Self::OP_SET_VOLUME,
        }
    }

    fn write_command_args<S: NativeCommandSink>(
        &self,
        sink: &mut S,
    ) -> Result<(), NativeViewCodecError> {
        match self {
            Self::Play | Self::Pause => {}
            Self::SeekMs(ms) => {
                sink.arg_u64(*ms)?;
            }
            Self::SetVolume(vol) => {
                sink.arg_f32(*vol)?;
            }
        }
        Ok(())
    }
}

impl DecodeNativeCommand for VideoPlayerCommand {
    fn read_command<S: NativeCommandSource>(src: &S) -> Result<Self, NativeViewCodecError> {
        match src.opcode() {
            Self::OP_PLAY => {
                src.expect_arg_count(0)?;
                Ok(Self::Play)
            }
            Self::OP_PAUSE => {
                src.expect_arg_count(0)?;
                Ok(Self::Pause)
            }
            Self::OP_SEEK_MS => {
                src.expect_arg_count(1)?;
                Ok(Self::SeekMs(src.require_u64_arg(0)?))
            }
            Self::OP_SET_VOLUME => {
                src.expect_arg_count(1)?;
                Ok(Self::SetVolume(src.require_f32_arg(0)?))
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

    /// Converts directly to owned [`DecodedWebElementProps`] without any wire packing.
    pub fn to_decoded(&self) -> DecodedWebElementProps {
        DecodedWebElementProps {
            tag: self.tag.to_owned(),
            attrs: self.attrs.clone(),
        }
    }
}

impl EncodeNativeProps for WebElementProps {
    fn write_props<S: NativePropSink>(&self, sink: &mut S) -> Result<(), NativeViewCodecError> {
        sink.str_field(1, self.tag)?;
        let count = u32::try_from(self.attrs.len())
            .map_err(|_| NativeViewCodecError::TooManyFields(self.attrs.len()))?;
        sink.u32_field(2, count)?;
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
        sink.bytes_field(3, &attr_buf)?;
        Ok(())
    }

    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError> {
        self.write_props(enc)
    }
}

/// Decoded owned representation of [`WebElementProps`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedWebElementProps {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
}

impl DecodeNativeProps for DecodedWebElementProps {
    fn read_props<S: NativePropSource>(src: &S) -> Result<Self, NativeViewCodecError> {
        let tag = src.require_str_field(1)?.to_owned();
        let count = src.require_u32_field(2)? as usize;
        let raw = src.require_bytes_field(3)?;
        let mut pos = 0usize;
        if count > raw.len() / 8 {
            return Err(NativeViewCodecError::Truncated);
        }
        let mut attrs = Vec::with_capacity(count);
        for _ in 0..count {
            let key = read_len_str(raw, &mut pos)?.to_owned();
            let val = read_len_str(raw, &mut pos)?.to_owned();
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
