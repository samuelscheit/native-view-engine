//! Local user notifications capability (`SERVICE_NOTIFICATIONS = 0x0015`) with binary codecs.

use nve_abi::SERVICE_NOTIFICATIONS;

use crate::codec::{BinReader, BinWriter};
use crate::error::SdkError;
use crate::provider::{HostPushEvent, HostServiceProvider};

pub const METHOD_REQUEST_PERMISSION: u32 = 1;
pub const METHOD_SHOW: u32 = 2;
pub const METHOD_CANCEL: u32 = 3;

/// Host push event ID emitted when a user taps a delivered notification.
pub const EVENT_NOTIFICATION_TAPPED: u32 = 1;

/// Authorization state returned by [`NotificationsClient::request_permission`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationPermission {
    NotDetermined = 0,
    Denied = 1,
    Authorized = 2,
    Provisional = 3,
}

impl NotificationPermission {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::NotDetermined),
            1 => Some(Self::Denied),
            2 => Some(Self::Authorized),
            3 => Some(Self::Provisional),
            _ => None,
        }
    }
}

/// Request payload for [`NotificationsClient::show`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRequest {
    pub id: String,
    pub title: String,
    pub body: String,
    pub subtitle: Option<String>,
    pub sound: bool,
    pub badge: Option<u32>,
    pub action_url: Option<String>,
}

impl NotificationRequest {
    pub fn new(id: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            subtitle: None,
            sound: true,
            badge: None,
            action_url: None,
        }
    }
}

/// Push event emitted when the user activates a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationTapped {
    pub id: String,
    pub action_url: Option<String>,
}

pub fn encode_permission_response(perm: NotificationPermission) -> Vec<u8> {
    vec![perm as u8]
}

pub fn decode_permission_response(bytes: &[u8]) -> Result<NotificationPermission, SdkError> {
    let mut r = BinReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    NotificationPermission::from_u8(raw)
        .ok_or_else(|| SdkError::Codec(format!("invalid NotificationPermission {raw}")))
}

pub fn encode_show_request(req: &NotificationRequest) -> Result<Vec<u8>, SdkError> {
    if req.id.is_empty() {
        return Err(SdkError::InvalidArg("notification id must not be empty".into()));
    }
    if req.title.is_empty() {
        return Err(SdkError::InvalidArg("notification title must not be empty".into()));
    }
    let mut w = BinWriter::new();
    w.write_str(&req.id)?;
    w.write_str(&req.title)?;
    w.write_str(&req.body)?;
    w.write_opt_str(req.subtitle.as_deref())?;
    w.write_bool(req.sound);
    w.write_opt_u32(req.badge);
    w.write_opt_str(req.action_url.as_deref())?;
    Ok(w.into_bytes())
}

pub fn decode_show_request(bytes: &[u8]) -> Result<NotificationRequest, SdkError> {
    let mut r = BinReader::new(bytes);
    let id = r.read_string()?;
    let title = r.read_string()?;
    let body = r.read_string()?;
    let subtitle = r.read_opt_string()?;
    let sound = r.read_bool()?;
    let badge = r.read_opt_u32()?;
    let action_url = r.read_opt_string()?;
    r.finish()?;
    if id.is_empty() || title.is_empty() {
        return Err(SdkError::InvalidArg(
            "notification id and title must not be empty".into(),
        ));
    }
    Ok(NotificationRequest {
        id,
        title,
        body,
        subtitle,
        sound,
        badge,
        action_url,
    })
}

pub fn encode_cancel_request(id: &str) -> Result<Vec<u8>, SdkError> {
    if id.is_empty() {
        return Err(SdkError::InvalidArg("notification id must not be empty".into()));
    }
    let mut w = BinWriter::with_capacity(4 + id.len());
    w.write_str(id)?;
    Ok(w.into_bytes())
}

pub fn decode_cancel_request(bytes: &[u8]) -> Result<String, SdkError> {
    let mut r = BinReader::new(bytes);
    let id = r.read_string()?;
    r.finish()?;
    if id.is_empty() {
        return Err(SdkError::InvalidArg("notification id must not be empty".into()));
    }
    Ok(id)
}

pub fn encode_tapped_event(ev: &NotificationTapped) -> Result<Vec<u8>, SdkError> {
    if ev.id.is_empty() {
        return Err(SdkError::InvalidArg("notification id must not be empty".into()));
    }
    let mut w = BinWriter::new();
    w.write_str(&ev.id)?;
    w.write_opt_str(ev.action_url.as_deref())?;
    Ok(w.into_bytes())
}

pub fn decode_tapped_event(bytes: &[u8]) -> Result<NotificationTapped, SdkError> {
    let mut r = BinReader::new(bytes);
    let id = r.read_string()?;
    let action_url = r.read_opt_string()?;
    r.finish()?;
    if id.is_empty() {
        return Err(SdkError::InvalidArg("notification id must not be empty".into()));
    }
    Ok(NotificationTapped { id, action_url })
}

/// Typed ergonomic client for local OS notifications.
#[derive(Debug, Clone, Copy)]
pub struct NotificationsClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> NotificationsClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn request_permission(&self) -> Result<NotificationPermission, SdkError> {
        let resp = self
            .provider
            .invoke_sync(SERVICE_NOTIFICATIONS, METHOD_REQUEST_PERMISSION, &[])?;
        decode_permission_response(&resp)
    }

    pub fn show(&self, req: &NotificationRequest) -> Result<(), SdkError> {
        let encoded = encode_show_request(req)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_NOTIFICATIONS, METHOD_SHOW, &encoded)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec("unexpected non-empty notifications::show response".into()));
        }
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> Result<(), SdkError> {
        let encoded = encode_cancel_request(id)?;
        let resp = self
            .provider
            .invoke_sync(SERVICE_NOTIFICATIONS, METHOD_CANCEL, &encoded)?;
        if !resp.is_empty() {
            return Err(SdkError::Codec(
                "unexpected non-empty notifications::cancel response".into(),
            ));
        }
        Ok(())
    }

    /// Decodes a [`HostPushEvent`] if it belongs to [`SERVICE_NOTIFICATIONS`].
    pub fn decode_push_event(
        event: &HostPushEvent,
    ) -> Result<Option<NotificationTapped>, SdkError> {
        if event.service_id != SERVICE_NOTIFICATIONS {
            return Ok(None);
        }
        match event.event_id {
            EVENT_NOTIFICATION_TAPPED => Ok(Some(decode_tapped_event(&event.payload)?)),
            other => Err(SdkError::Codec(format!(
                "unknown notifications event_id {other}"
            ))),
        }
    }
}
