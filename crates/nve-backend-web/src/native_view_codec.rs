//! Zero-copy binary `NVNP` prop and `NVNC` command decoders plus URL/tag/attribute
//! allowlist validators shared across `WebBackend`'s `NativeView` adapters.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeViewCodecError {
    Truncated,
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u8),
    InvalidTag(u8),
    InvalidBool(u8),
    InvalidEnum(&'static str, u32),
    InvalidUtf8,
    MissingField(u16),
    MissingArg(usize),
    UnexpectedArgCount { expected: usize, actual: usize },
    TypeMismatch(&'static str),
    TrailingBytes(usize),
    UnknownCommandOpcode(u16),
    TooManyFields(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeValueRef<'a> {
    Null,
    Bool(bool),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Str(&'a str),
    Bytes(&'a [u8]),
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
        as_bool => Bool(bool), "bool";
        as_u32 => U32(u32), "u32";
        as_u64 => U64(u64), "u64";
        as_f32 => F32(f32), "f32";
        as_str => Str(&'a str), "str";
        as_bytes => Bytes(&'a [u8]), "bytes";
    }
    pub fn as_opt_u32(self) -> Result<Option<u32>, NativeViewCodecError> {
        match self {
            Self::Null => Ok(None),
            Self::U32(v) => Ok(Some(v)),
            _ => Err(NativeViewCodecError::TypeMismatch("Option<u32>")),
        }
    }
    pub fn as_opt_str(self) -> Result<Option<&'a str>, NativeViewCodecError> {
        match self {
            Self::Null => Ok(None),
            Self::Str(s) => Ok(Some(s)),
            _ => Err(NativeViewCodecError::TypeMismatch("Option<str>")),
        }
    }
}

#[inline]
fn take_slice<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
    len: usize,
) -> Result<&'a [u8], NativeViewCodecError> {
    let end = pos
        .checked_add(len)
        .ok_or(NativeViewCodecError::Truncated)?;
    let slice = bytes.get(*pos..end).ok_or(NativeViewCodecError::Truncated)?;
    *pos = end;
    Ok(slice)
}

#[inline]
fn take_arr<const N: usize>(
    bytes: &[u8],
    pos: &mut usize,
) -> Result<[u8; N], NativeViewCodecError> {
    Ok(take_slice(bytes, pos, N)?.try_into().unwrap())
}

#[inline]
pub(crate) fn read_len_str<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
) -> Result<&'a str, NativeViewCodecError> {
    let len = u32::from_le_bytes(take_arr(bytes, pos)?) as usize;
    core::str::from_utf8(take_slice(bytes, pos, len)?)
        .map_err(|_| NativeViewCodecError::InvalidUtf8)
}

fn read_value<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
) -> Result<NativeValueRef<'a>, NativeViewCodecError> {
    let tag = *bytes.get(*pos).ok_or(NativeViewCodecError::Truncated)?;
    *pos += 1;
    match tag {
        0 => Ok(NativeValueRef::Null),
        1 => match *bytes.get(*pos).ok_or(NativeViewCodecError::Truncated)? {
            0 => {
                *pos += 1;
                Ok(NativeValueRef::Bool(false))
            }
            1 => {
                *pos += 1;
                Ok(NativeValueRef::Bool(true))
            }
            other => Err(NativeViewCodecError::InvalidBool(other)),
        },
        2 => Ok(NativeValueRef::U32(u32::from_le_bytes(take_arr(bytes, pos)?))),
        3 => Ok(NativeValueRef::I64(i64::from_le_bytes(take_arr(bytes, pos)?))),
        4 => Ok(NativeValueRef::U64(u64::from_le_bytes(take_arr(bytes, pos)?))),
        5 => Ok(NativeValueRef::F32(f32::from_le_bytes(take_arr(bytes, pos)?))),
        6 => Ok(NativeValueRef::F64(f64::from_le_bytes(take_arr(bytes, pos)?))),
        7 => Ok(NativeValueRef::Str(read_len_str(bytes, pos)?)),
        8 => {
            let len = u32::from_le_bytes(take_arr(bytes, pos)?) as usize;
            Ok(NativeValueRef::Bytes(take_slice(bytes, pos, len)?))
        }
        other => Err(NativeViewCodecError::InvalidTag(other)),
    }
}

/// Validated zero-copy decoder for binary `NVNP` native view prop buffers.
#[derive(Debug, Clone)]
pub struct NativePropDecoder<'a> {
    fields: Vec<(u16, NativeValueRef<'a>)>,
}

impl<'a> NativePropDecoder<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, NativeViewCodecError> {
        if bytes.len() < NATIVE_PROPS_HEADER_SIZE {
            return Err(NativeViewCodecError::Truncated);
        }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != NATIVE_PROPS_MAGIC {
            return Err(NativeViewCodecError::InvalidMagic(magic));
        }
        if bytes[4] != NATIVE_PROPS_VERSION {
            return Err(NativeViewCodecError::UnsupportedVersion(bytes[4]));
        }
        let count = usize::from(u16::from_le_bytes([bytes[5], bytes[6]]));
        if count > MAX_NATIVE_PROP_FIELDS {
            return Err(NativeViewCodecError::TooManyFields(count));
        }
        let mut pos = NATIVE_PROPS_HEADER_SIZE;
        if count > (bytes.len() - pos) / 3 {
            return Err(NativeViewCodecError::Truncated);
        }
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            let id = u16::from_le_bytes(take_arr(bytes, &mut pos)?);
            fields.push((id, read_value(bytes, &mut pos)?));
        }
        if pos != bytes.len() {
            return Err(NativeViewCodecError::TrailingBytes(bytes.len() - pos));
        }
        Ok(Self { fields })
    }

    pub fn get(&self, id: u16) -> Option<NativeValueRef<'a>> {
        self.fields
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, v)| *v)
    }

    pub fn require(&self, id: u16) -> Result<NativeValueRef<'a>, NativeViewCodecError> {
        self.get(id).ok_or(NativeViewCodecError::MissingField(id))
    }
}

/// Validated zero-copy decoder for binary `NVNC` imperative command payloads.
#[derive(Debug, Clone)]
pub struct NativeCommandDecoder<'a> {
    opcode: u16,
    args: Vec<NativeValueRef<'a>>,
}

impl<'a> NativeCommandDecoder<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, NativeViewCodecError> {
        if bytes.len() < NATIVE_CMD_HEADER_SIZE {
            return Err(NativeViewCodecError::Truncated);
        }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != NATIVE_CMD_MAGIC {
            return Err(NativeViewCodecError::InvalidMagic(magic));
        }
        if bytes[4] != NATIVE_CMD_VERSION {
            return Err(NativeViewCodecError::UnsupportedVersion(bytes[4]));
        }
        let opcode = u16::from_le_bytes([bytes[5], bytes[6]]);
        let arg_count = usize::from(bytes[7]);
        if arg_count > MAX_NATIVE_CMD_ARGS {
            return Err(NativeViewCodecError::TooManyFields(arg_count));
        }
        let mut pos = NATIVE_CMD_HEADER_SIZE;
        if arg_count > bytes.len() - pos {
            return Err(NativeViewCodecError::Truncated);
        }
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            args.push(read_value(bytes, &mut pos)?);
        }
        if pos != bytes.len() {
            return Err(NativeViewCodecError::TrailingBytes(bytes.len() - pos));
        }
        Ok(Self { opcode, args })
    }

    #[inline]
    pub fn opcode(&self) -> u16 {
        self.opcode
    }

    pub fn expect_arg_count(&self, expected: usize) -> Result<(), NativeViewCodecError> {
        if self.args.len() != expected {
            return Err(NativeViewCodecError::UnexpectedArgCount {
                expected,
                actual: self.args.len(),
            });
        }
        Ok(())
    }

    pub fn require(&self, idx: usize) -> Result<NativeValueRef<'a>, NativeViewCodecError> {
        self.args
            .get(idx)
            .copied()
            .ok_or(NativeViewCodecError::MissingArg(idx))
    }
}

#[cfg(feature = "nv-blur")]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlurMaterial {
    UltraThin = 0,
    Thin = 1,
    Regular = 2,
    Thick = 3,
    Chrome = 4,
    SystemMaterial = 5,
}

#[cfg(feature = "nv-blur")]
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

    pub(crate) fn css_params(self) -> (f32, &'static str) {
        match self {
            Self::UltraThin => (8.0, "0.08"),
            Self::Thin => (12.0, "0.14"),
            Self::Regular => (20.0, "0.22"),
            Self::Thick => (32.0, "0.34"),
            Self::Chrome => (24.0, "0.28"),
            Self::SystemMaterial => (16.0, "0.18"),
        }
    }
}

#[cfg(feature = "nv-blur")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurSurfaceProps {
    pub material: BlurMaterial,
    pub intensity: f32,
    pub tint: Option<u32>,
}

#[cfg(feature = "nv-webview")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebViewProps {
    pub url: String,
    pub allows_back_forward: bool,
}

#[cfg(feature = "nv-webview")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebViewCommand {
    Navigate(String),
    Reload,
    GoBack,
    GoForward,
    EvaluateScript(String),
}

#[cfg(feature = "nv-video")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPlayerProps {
    pub source_uri: String,
    pub autoplay: bool,
    pub muted: bool,
    pub looping: bool,
    pub controls: bool,
}

#[cfg(feature = "nv-video")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoPlayerCommand {
    Play,
    Pause,
    SeekMs(u64),
    SetVolume(f32),
}

#[cfg(feature = "nv-web-element")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebElementProps {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
}

#[inline]
pub fn is_native_view_prop(key: &str) -> bool {
    matches!(
        key,
        NATIVE_VIEW_KIND_PROP_KEY
            | NATIVE_VIEW_KIND_PROP_ALIAS
            | NATIVE_VIEW_PROPS_PROP_KEY
            | NATIVE_VIEW_PROPS_PROP_ALIAS
    )
}

#[inline]
pub fn is_native_kind_prop(key: &str) -> bool {
    matches!(key, NATIVE_VIEW_KIND_PROP_KEY | NATIVE_VIEW_KIND_PROP_ALIAS)
}

#[cfg(feature = "nv-web-element")]
pub fn allowlisted_builtin_web_tag(tag: &str) -> Option<&'static str> {
    match tag {
        "video" => Some("video"),
        "audio" => Some("audio"),
        "svg" => Some("svg"),
        "iframe" => Some("iframe"),
        "canvas" => Some("canvas"),
        "input" => Some("input"),
        "select" => Some("select"),
        "textarea" => Some("textarea"),
        "progress" => Some("progress"),
        "meter" => Some("meter"),
        "details" => Some("details"),
        "summary" => Some("summary"),
        _ => None,
    }
}

/// Validates `"web.<tag>"` against the safe HTML5 allowlist or W3C custom-element rules.
#[cfg(feature = "nv-web-element")]
pub fn validate_web_element_tag(tag: &str) -> Option<(String, Option<&'static str>)> {
    if let Some(st) = allowlisted_builtin_web_tag(tag) {
        return Some((st.to_owned(), Some(st)));
    }
    let bytes = tag.as_bytes();
    if tag.len() < 3
        || tag.len() > 64
        || !tag.contains('-')
        || tag.ends_with('-')
        || !bytes[0].is_ascii_lowercase()
        || !bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_'))
        || matches!(
            tag,
            "annotation-xml"
                | "color-profile"
                | "font-face"
                | "font-face-src"
                | "font-face-uri"
                | "font-face-format"
                | "font-face-name"
                | "missing-glyph"
        )
    {
        return None;
    }
    Some((tag.to_owned(), None))
}

/// Rejects `javascript:`, `vbscript:`, and `data:` URI schemes (even with embedded control/whitespace).
#[cfg(any(feature = "nv-webview", feature = "nv-video", feature = "nv-web-element"))]
pub fn is_safe_web_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    let norm: String = trimmed
        .chars()
        .filter(|c| !c.is_ascii_control() && !c.is_ascii_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if norm.starts_with("javascript:")
        || norm.starts_with("vbscript:")
        || norm.starts_with("data:")
    {
        return false;
    }
    if norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank" {
        return true;
    }
    let sep = norm.find(['/', '?', '#']).unwrap_or(norm.len());
    !norm[..sep].contains(':')
}

/// Validates `WebElement` attributes, rejecting `on*` event handlers, `srcdoc`, `data-nve-id`, and unsafe URIs.
#[cfg(feature = "nv-web-element")]
pub fn is_safe_web_attr(name: &str, value: &str) -> bool {
    let bytes = name.as_bytes();
    if name.is_empty()
        || name.len() > 64
        || !bytes[0].is_ascii_alphabetic()
        || !bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
    {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("on") || lower == "srcdoc" || lower == "data-nve-id" {
        return false;
    }
    if matches!(
        lower.as_str(),
        "src" | "href" | "xlink:href" | "action" | "formaction" | "poster" | "data"
    ) {
        return is_safe_web_url(value);
    }
    true
}

#[cfg(feature = "nv-blur")]
pub fn decode_blur_surface_props(
    dec: &NativePropDecoder<'_>,
) -> Result<BlurSurfaceProps, NativeViewCodecError> {
    Ok(BlurSurfaceProps {
        material: BlurMaterial::from_u32(dec.require(1)?.as_u32()?)?,
        intensity: dec.require(2)?.as_f32()?,
        tint: dec.require(3)?.as_opt_u32()?,
    })
}

#[cfg(feature = "nv-webview")]
pub fn decode_web_view_props(
    dec: &NativePropDecoder<'_>,
) -> Result<WebViewProps, NativeViewCodecError> {
    Ok(WebViewProps {
        url: dec.require(1)?.as_str()?.to_owned(),
        allows_back_forward: dec.require(2)?.as_bool()?,
    })
}

#[cfg(feature = "nv-webview")]
pub fn decode_web_view_command(bytes: &[u8]) -> Result<WebViewCommand, NativeViewCodecError> {
    let dec = NativeCommandDecoder::decode(bytes)?;
    match dec.opcode() {
        1 => {
            dec.expect_arg_count(1)?;
            Ok(WebViewCommand::Navigate(
                dec.require(0)?.as_str()?.to_owned(),
            ))
        }
        2 => {
            dec.expect_arg_count(0)?;
            Ok(WebViewCommand::Reload)
        }
        3 => {
            dec.expect_arg_count(0)?;
            Ok(WebViewCommand::GoBack)
        }
        4 => {
            dec.expect_arg_count(0)?;
            Ok(WebViewCommand::GoForward)
        }
        5 => {
            dec.expect_arg_count(1)?;
            Ok(WebViewCommand::EvaluateScript(
                dec.require(0)?.as_str()?.to_owned(),
            ))
        }
        other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
    }
}

#[cfg(feature = "nv-video")]
pub fn decode_video_player_props(
    dec: &NativePropDecoder<'_>,
) -> Result<VideoPlayerProps, NativeViewCodecError> {
    Ok(VideoPlayerProps {
        source_uri: dec.require(1)?.as_str()?.to_owned(),
        autoplay: dec.require(2)?.as_bool()?,
        muted: dec.require(3)?.as_bool()?,
        looping: dec.require(4)?.as_bool()?,
        controls: dec.require(5)?.as_bool()?,
    })
}

#[cfg(feature = "nv-video")]
pub fn decode_video_player_command(
    bytes: &[u8],
) -> Result<VideoPlayerCommand, NativeViewCodecError> {
    let dec = NativeCommandDecoder::decode(bytes)?;
    match dec.opcode() {
        1 => {
            dec.expect_arg_count(0)?;
            Ok(VideoPlayerCommand::Play)
        }
        2 => {
            dec.expect_arg_count(0)?;
            Ok(VideoPlayerCommand::Pause)
        }
        3 => {
            dec.expect_arg_count(1)?;
            Ok(VideoPlayerCommand::SeekMs(dec.require(0)?.as_u64()?))
        }
        4 => {
            dec.expect_arg_count(1)?;
            Ok(VideoPlayerCommand::SetVolume(dec.require(0)?.as_f32()?))
        }
        other => Err(NativeViewCodecError::UnknownCommandOpcode(other)),
    }
}

#[cfg(feature = "nv-web-element")]
pub fn decode_web_element_props(
    dec: &NativePropDecoder<'_>,
) -> Result<WebElementProps, NativeViewCodecError> {
    let tag = dec.require(1)?.as_str()?.to_owned();
    let count = dec.require(2)?.as_u32()? as usize;
    let raw = dec.require(3)?.as_bytes()?;
    let mut pos = 0usize;
    if count > raw.len() / 8 {
        return Err(NativeViewCodecError::Truncated);
    }
    let mut attrs = Vec::with_capacity(count);
    for _ in 0..count {
        let (key, val) = (read_len_str(raw, &mut pos)?, read_len_str(raw, &mut pos)?);
        if is_safe_web_attr(key, val) {
            attrs.push((key.to_owned(), val.to_owned()));
        }
    }
    if pos != raw.len() {
        return Err(NativeViewCodecError::TrailingBytes(raw.len() - pos));
    }
    Ok(WebElementProps { tag, attrs })
}
