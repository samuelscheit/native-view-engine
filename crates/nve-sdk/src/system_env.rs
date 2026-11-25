//! Live System Theme (`NVT2` 16-byte fixed binary codec) and Locale (`NVL1` binary codec)
//! (`SERVICE_SYSTEM_ENV = 0x0014`), wired directly onto `NveChannelBus` (`CHANNEL_ID_THEME = 1`,
//! `CHANNEL_ID_LOCALE = 2`) with zero JSON.

use nve_abi::{SurfaceId, SERVICE_SYSTEM_ENV};
use nve_interop::{
    ChannelPayload, NveChannelBus, PayloadEncoding, SubscriptionId, CHANNEL_ID_LOCALE,
    CHANNEL_ID_THEME,
};

use crate::codec::{BinReader, BinWriter};
use crate::error::SdkError;
use crate::provider::{HostEvent, HostServiceProvider};

pub const METHOD_GET_THEME: u32 = 1;
pub const METHOD_GET_LOCALE: u32 = 2;

pub const EVENT_THEME_CHANGED: u32 = 1;
pub const EVENT_LOCALE_CHANGED: u32 = 2;

/// Magic header for fixed 16-byte [`ThemeSnapshot`] binary wire records.
pub const THEME_WIRE_MAGIC: [u8; 4] = *b"NVT2";

/// Exact byte length of an encoded `NVT2` [`ThemeSnapshot`].
pub const THEME_WIRE_LEN: usize = 16;

/// Magic header for variable-length [`LocaleSnapshot`] binary wire records.
pub const LOCALE_WIRE_MAGIC: [u8; 4] = *b"NVL1";
pub const LOCALE_WIRE_VERSION: u8 = 1;

const THEME_FLAG_REDUCED_MOTION: u8 = 1 << 0;
const THEME_FLAG_REDUCED_TRANSPARENCY: u8 = 1 << 1;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorScheme {
    #[default]
    Unspecified = 0,
    Light = 1,
    Dark = 2,
}

impl ColorScheme {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Unspecified),
            1 => Some(Self::Light),
            2 => Some(Self::Dark),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContrastLevel {
    #[default]
    Standard = 0,
    High = 1,
}

impl ContrastLevel {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Standard),
            1 => Some(Self::High),
            _ => None,
        }
    }
}

/// Fixed 16-byte `NVT2` system theme & display accessibility snapshot.
///
/// Layout (little-endian):
/// - `[0..4]`: `"NVT2"`
/// - `[4]`: `color_scheme` (`0=Unspecified, 1=Light, 2=Dark`)
/// - `[5]`: `contrast` (`0=Standard, 1=High`)
/// - `[6]`: `flags` (bit 0: `reduced_motion`, bit 1: `reduced_transparency`)
/// - `[7]`: reserved (`0`)
/// - `[8..12]`: `accent_rgba` (`u32` LE, `0xRRGGBBAA`, `0` if unavailable)
/// - `[12..14]`: `font_scale_x1000` (`u16` LE, e.g. `1000` = `1.0x`)
/// - `[14..16]`: `scale_factor_x100` (`u16` LE, e.g. `200` = `2.0x`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThemeSnapshot {
    pub color_scheme: ColorScheme,
    pub contrast: ContrastLevel,
    pub reduced_motion: bool,
    pub reduced_transparency: bool,
    pub accent_rgba: u32,
    pub font_scale_x1000: u16,
    pub scale_factor_x100: u16,
}

impl Default for ThemeSnapshot {
    fn default() -> Self {
        Self {
            color_scheme: ColorScheme::Light,
            contrast: ContrastLevel::Standard,
            reduced_motion: false,
            reduced_transparency: false,
            accent_rgba: 0x007A_FFFF,
            font_scale_x1000: 1000,
            scale_factor_x100: 100,
        }
    }
}

impl ThemeSnapshot {
    pub fn font_scale(&self) -> f32 {
        f32::from(self.font_scale_x1000) / 1000.0
    }

    pub fn scale_factor(&self) -> f32 {
        f32::from(self.scale_factor_x100) / 100.0
    }

    /// Encodes this snapshot into an exact 16-byte `NVT2` array (zero heap allocation).
    pub const fn encode_nvt2(&self) -> [u8; THEME_WIRE_LEN] {
        let mut flags = 0u8;
        if self.reduced_motion {
            flags |= THEME_FLAG_REDUCED_MOTION;
        }
        if self.reduced_transparency {
            flags |= THEME_FLAG_REDUCED_TRANSPARENCY;
        }
        let accent = self.accent_rgba.to_le_bytes();
        let font = self.font_scale_x1000.to_le_bytes();
        let scale = self.scale_factor_x100.to_le_bytes();
        [
            THEME_WIRE_MAGIC[0],
            THEME_WIRE_MAGIC[1],
            THEME_WIRE_MAGIC[2],
            THEME_WIRE_MAGIC[3],
            self.color_scheme as u8,
            self.contrast as u8,
            flags,
            0,
            accent[0],
            accent[1],
            accent[2],
            accent[3],
            font[0],
            font[1],
            scale[0],
            scale[1],
        ]
    }

    /// Strictly decodes an exact 16-byte `NVT2` slice.
    pub fn decode_nvt2(bytes: &[u8]) -> Result<Self, SdkError> {
        if bytes.len() != THEME_WIRE_LEN {
            return Err(SdkError::Codec(format!(
                "NVT2 payload must be exactly {THEME_WIRE_LEN} bytes, got {}",
                bytes.len()
            )));
        }
        if bytes[0..4] != THEME_WIRE_MAGIC {
            return Err(SdkError::Codec("invalid NVT2 magic".into()));
        }
        let color_scheme = ColorScheme::from_u8(bytes[4])
            .ok_or_else(|| SdkError::Codec(format!("invalid NVT2 color_scheme {}", bytes[4])))?;
        let contrast = ContrastLevel::from_u8(bytes[5])
            .ok_or_else(|| SdkError::Codec(format!("invalid NVT2 contrast {}", bytes[5])))?;
        let flags = bytes[6];
        let accent_rgba = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let font_scale_x1000 = u16::from_le_bytes([bytes[12], bytes[13]]);
        let scale_factor_x100 = u16::from_le_bytes([bytes[14], bytes[15]]);
        Ok(Self {
            color_scheme,
            contrast,
            reduced_motion: (flags & THEME_FLAG_REDUCED_MOTION) != 0,
            reduced_transparency: (flags & THEME_FLAG_REDUCED_TRANSPARENCY) != 0,
            accent_rgba,
            font_scale_x1000,
            scale_factor_x100,
        })
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextDirection {
    #[default]
    Ltr = 0,
    Rtl = 1,
}

impl TextDirection {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Ltr),
            1 => Some(Self::Rtl),
            _ => None,
        }
    }
}

/// System locale & regional formatting snapshot encoded with the binary `NVL1` codec.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocaleSnapshot {
    /// Canonical BCP-47 tag (e.g. `"en-US"`, `"ar-EG"`, `"de-DE"`).
    pub tag: String,
    /// Primary ISO-639 language subtag (e.g. `"en"`).
    pub language: String,
    /// Optional ISO-3166 region subtag (e.g. `"US"`, or empty if absent).
    pub region: String,
    /// Reading direction (`Ltr` or `Rtl`).
    pub text_direction: TextDirection,
    /// Whether the user prefers 24-hour time formatting.
    pub uses_24h_clock: bool,
    /// IANA timezone name (e.g. `"UTC"`, `"Europe/Berlin"`).
    pub timezone: String,
}

impl Default for LocaleSnapshot {
    fn default() -> Self {
        Self {
            tag: "en-US".to_owned(),
            language: "en".to_owned(),
            region: "US".to_owned(),
            text_direction: TextDirection::Ltr,
            uses_24h_clock: false,
            timezone: "UTC".to_owned(),
        }
    }
}

impl LocaleSnapshot {
    /// Encodes this snapshot in binary `NVL1` wire format.
    pub fn encode_nvl1(&self) -> Result<Vec<u8>, SdkError> {
        let mut w = BinWriter::with_capacity(
            24 + self.tag.len() + self.language.len() + self.region.len() + self.timezone.len(),
        );
        w.write_raw(&LOCALE_WIRE_MAGIC);
        w.write_u8(LOCALE_WIRE_VERSION);
        w.write_u8(self.text_direction as u8);
        w.write_bool(self.uses_24h_clock);
        w.write_u8(0);
        w.write_str(&self.tag)?;
        w.write_str(&self.language)?;
        w.write_str(&self.region)?;
        w.write_str(&self.timezone)?;
        Ok(w.into_bytes())
    }

    /// Strictly decodes a binary `NVL1` byte slice.
    pub fn decode_nvl1(bytes: &[u8]) -> Result<Self, SdkError> {
        let mut r = BinReader::new(bytes);
        let magic = r.read_exact(4)?;
        if magic != LOCALE_WIRE_MAGIC {
            return Err(SdkError::Codec("invalid NVL1 magic".into()));
        }
        let version = r.read_u8()?;
        if version != LOCALE_WIRE_VERSION {
            return Err(SdkError::Codec(format!("unsupported NVL1 version {version}")));
        }
        let dir_raw = r.read_u8()?;
        let text_direction = TextDirection::from_u8(dir_raw)
            .ok_or_else(|| SdkError::Codec(format!("invalid NVL1 text_direction {dir_raw}")))?;
        let uses_24h_clock = r.read_bool()?;
        let _reserved = r.read_u8()?;
        let tag = r.read_string()?;
        let language = r.read_string()?;
        let region = r.read_string()?;
        let timezone = r.read_string()?;
        r.finish()?;
        Ok(Self {
            tag,
            language,
            region,
            text_direction,
            uses_24h_clock,
            timezone,
        })
    }
}

/// Typed ergonomic client for querying system theme and locale snapshots.
#[derive(Debug, Clone, Copy)]
pub struct SystemEnvClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> SystemEnvClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn get_theme(&self) -> Result<ThemeSnapshot, SdkError> {
        let resp = self
            .provider
            .invoke_sync(SERVICE_SYSTEM_ENV, METHOD_GET_THEME, &[])?;
        ThemeSnapshot::decode_nvt2(&resp)
    }

    pub fn get_locale(&self) -> Result<LocaleSnapshot, SdkError> {
        let resp = self
            .provider
            .invoke_sync(SERVICE_SYSTEM_ENV, METHOD_GET_LOCALE, &[])?;
        LocaleSnapshot::decode_nvl1(&resp)
    }
}

/// Bridges live system theme (`NVT2`) and locale (`NVL1`) snapshots onto [`NveChannelBus`]
/// using retained zero-JSON binary payloads on `CHANNEL_ID_THEME = 1` and `CHANNEL_ID_LOCALE = 2`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnvBridge;

impl SystemEnvBridge {
    pub const fn new() -> Self {
        Self
    }

    /// Publishes a retained 16-byte binary `NVT2` theme snapshot onto `CHANNEL_ID_THEME`.
    pub fn publish_theme(
        bus: &mut NveChannelBus,
        theme: &ThemeSnapshot,
    ) -> Result<usize, SdkError> {
        let bytes = theme.encode_nvt2();
        bus.publish_by_id(
            CHANNEL_ID_THEME,
            None,
            PayloadEncoding::Binary,
            ChannelPayload::from_slice(&bytes),
            true,
        )
        .map_err(|e| SdkError::InternalError(e.to_string()))
    }

    /// Publishes a retained binary `NVL1` locale snapshot onto `CHANNEL_ID_LOCALE`.
    pub fn publish_locale(
        bus: &mut NveChannelBus,
        locale: &LocaleSnapshot,
    ) -> Result<usize, SdkError> {
        let bytes = locale.encode_nvl1()?;
        bus.publish_by_id(
            CHANNEL_ID_LOCALE,
            None,
            PayloadEncoding::Binary,
            ChannelPayload::from_vec(bytes),
            true,
        )
        .map_err(|e| SdkError::InternalError(e.to_string()))
    }

    /// Queries the active [`HostServiceProvider`] for theme + locale and publishes both as
    /// retained binary snapshots on `bus` (and via `provider.channel_publish`).
    pub fn sync_from_provider<P: HostServiceProvider + ?Sized>(
        &self,
        provider: &P,
        bus: &mut NveChannelBus,
    ) -> Result<(ThemeSnapshot, LocaleSnapshot), SdkError> {
        let client = SystemEnvClient::new(provider);
        let theme = client.get_theme()?;
        let locale = client.get_locale()?;
        let theme_bytes = theme.encode_nvt2();
        let locale_bytes = locale.encode_nvl1()?;
        Self::publish_theme(bus, &theme)?;
        Self::publish_locale(bus, &locale)?;
        let _ = provider.channel_publish(CHANNEL_ID_THEME, true, &theme_bytes);
        let _ = provider.channel_publish(CHANNEL_ID_LOCALE, true, &locale_bytes);
        Ok((theme, locale))
    }

    /// Subscribes to retained/live `NVT2` theme updates on `CHANNEL_ID_THEME`.
    pub fn subscribe_theme<F>(
        bus: &mut NveChannelBus,
        owner_surface: Option<SurfaceId>,
        replay_retained: bool,
        handler: F,
    ) -> Result<SubscriptionId, SdkError>
    where
        F: Fn(ThemeSnapshot) + Send + Sync + 'static,
    {
        bus.subscribe_by_id(
            CHANNEL_ID_THEME,
            owner_surface,
            replay_retained,
            move |msg| {
                if let Ok(theme) = ThemeSnapshot::decode_nvt2(msg.as_bytes()) {
                    handler(theme);
                }
            },
        )
        .map_err(|e| SdkError::InternalError(e.to_string()))
    }

    /// Subscribes to retained/live `NVL1` locale updates on `CHANNEL_ID_LOCALE`.
    pub fn subscribe_locale<F>(
        bus: &mut NveChannelBus,
        owner_surface: Option<SurfaceId>,
        replay_retained: bool,
        handler: F,
    ) -> Result<SubscriptionId, SdkError>
    where
        F: Fn(LocaleSnapshot) + Send + Sync + 'static,
    {
        bus.subscribe_by_id(
            CHANNEL_ID_LOCALE,
            owner_surface,
            replay_retained,
            move |msg| {
                if let Ok(locale) = LocaleSnapshot::decode_nvl1(msg.as_bytes()) {
                    handler(locale);
                }
            },
        )
        .map_err(|e| SdkError::InternalError(e.to_string()))
    }

    /// Processes an inbound [`HostEvent`]; if it is a theme or locale update (either via
    /// `SERVICE_SYSTEM_ENV` push event or `CHANNEL_ID_THEME`/`CHANNEL_ID_LOCALE` channel message),
    /// validates and publishes the retained binary payload onto `bus`.
    pub fn handle_host_event(
        bus: &mut NveChannelBus,
        event: &HostEvent,
    ) -> Result<bool, SdkError> {
        match event {
            HostEvent::Push(push) if push.service_id == SERVICE_SYSTEM_ENV => match push.event_id {
                EVENT_THEME_CHANGED => {
                    let theme = ThemeSnapshot::decode_nvt2(&push.payload)?;
                    Self::publish_theme(bus, &theme)?;
                    Ok(true)
                }
                EVENT_LOCALE_CHANGED => {
                    let locale = LocaleSnapshot::decode_nvl1(&push.payload)?;
                    Self::publish_locale(bus, &locale)?;
                    Ok(true)
                }
                other => Err(SdkError::Codec(format!(
                    "unknown system_env event_id {other}"
                ))),
            },
            HostEvent::Channel(ch) if ch.channel_id == CHANNEL_ID_THEME => {
                let theme = ThemeSnapshot::decode_nvt2(&ch.payload)?;
                Self::publish_theme(bus, &theme)?;
                Ok(true)
            }
            HostEvent::Channel(ch) if ch.channel_id == CHANNEL_ID_LOCALE => {
                let locale = LocaleSnapshot::decode_nvl1(&ch.payload)?;
                Self::publish_locale(bus, &locale)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
