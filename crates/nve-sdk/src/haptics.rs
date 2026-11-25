//! Tactile haptic feedback capability (`SERVICE_HAPTICS = 0x0016`) with single-byte codecs.

use nve_abi::SERVICE_HAPTICS;

use crate::codec::BinReader;
use crate::error::SdkError;
use crate::provider::HostServiceProvider;

pub const METHOD_IMPACT: u32 = 1;
pub const METHOD_NOTIFICATION: u32 = 2;
pub const METHOD_SELECTION: u32 = 3;

/// Impact intensity style for [`HapticsClient::impact`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImpactStyle {
    Light = 1,
    Medium = 2,
    Heavy = 3,
    Soft = 4,
    Rigid = 5,
}

impl ImpactStyle {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Light),
            2 => Some(Self::Medium),
            3 => Some(Self::Heavy),
            4 => Some(Self::Soft),
            5 => Some(Self::Rigid),
            _ => None,
        }
    }
}

/// Notification feedback style for [`HapticsClient::notification`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationHaptic {
    Success = 1,
    Warning = 2,
    Error = 3,
}

impl NotificationHaptic {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Success),
            2 => Some(Self::Warning),
            3 => Some(Self::Error),
            _ => None,
        }
    }
}

pub fn encode_impact_request(style: ImpactStyle) -> [u8; 1] {
    [style as u8]
}

pub fn decode_impact_request(bytes: &[u8]) -> Result<ImpactStyle, SdkError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    ImpactStyle::from_u8(raw)
        .ok_or_else(|| SdkError::InvalidArg(format!("invalid ImpactStyle {raw}")))
}

pub fn encode_notification_haptic_request(kind: NotificationHaptic) -> [u8; 1] {
    [kind as u8]
}

pub fn decode_notification_haptic_request(bytes: &[u8]) -> Result<NotificationHaptic, SdkError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    NotificationHaptic::from_u8(raw)
        .ok_or_else(|| SdkError::InvalidArg(format!("invalid NotificationHaptic {raw}")))
}

/// Typed ergonomic client for platform haptic feedback.
#[derive(Debug, Clone, Copy)]
pub struct HapticsClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> HapticsClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn impact(&self, style: ImpactStyle) -> Result<(), SdkError> {
        let req = encode_impact_request(style);
        let resp = self.provider.invoke_sync(SERVICE_HAPTICS, METHOD_IMPACT, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec("unexpected non-empty haptics::impact response".into()));
        }
        Ok(())
    }

    pub fn notification(&self, kind: NotificationHaptic) -> Result<(), SdkError> {
        let req = encode_notification_haptic_request(kind);
        let resp = self
            .provider
            .invoke_sync(SERVICE_HAPTICS, METHOD_NOTIFICATION, &req)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec(
                "unexpected non-empty haptics::notification response".into(),
            ));
        }
        Ok(())
    }

    pub fn selection(&self) -> Result<(), SdkError> {
        let resp = self
            .provider
            .invoke_sync(SERVICE_HAPTICS, METHOD_SELECTION, &[])?;
        if !resp.is_empty() {
            return Err(SdkError::Codec(
                "unexpected non-empty haptics::selection response".into(),
            ));
        }
        Ok(())
    }
}
