//! Core [`HostServiceProvider`] abstraction shared by [`crate::FfiHostServices`],
//! [`crate::MemoryHostServices`], and [`crate::native::NativeHostServices`].

use std::sync::Arc;

use crate::codec::BinReader;
use crate::error::SdkError;

/// Shared hardware/capability permission state used across Camera, Location, and Audio services.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionStatus {
    NotDetermined = 0,
    Denied = 1,
    Authorized = 2,
    Restricted = 3,
}

impl PermissionStatus {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::NotDetermined),
            1 => Some(Self::Denied),
            2 => Some(Self::Authorized),
            3 => Some(Self::Restricted),
            _ => None,
        }
    }

    pub fn encode(self) -> Vec<u8> {
        vec![self as u8]
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SdkError> {
        let mut r = BinReader::new(bytes);
        let raw = r.read_u8()?;
        r.finish()?;
        Self::from_u8(raw)
            .ok_or_else(|| SdkError::Codec(format!("invalid PermissionStatus {raw}")))
    }
}

/// Immediate or asynchronous result from [`HostServiceProvider::invoke_raw`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvokeResult {
    /// Host completed synchronously and returned the binary response payload.
    Ready(Vec<u8>),
    /// Host accepted the call asynchronously; completion arrives via `NVH1` under `request_id`.
    Pending { request_id: u64 },
}

/// Unsolicited host-to-app push event (e.g., incoming deep link URL, tapped notification,
/// or live system theme/locale change).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPushEvent {
    pub service_id: u32,
    pub event_id: u32,
    pub payload: Vec<u8>,
}

/// Binary channel message delivered from the host onto an `NveChannelBus` topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostChannelEvent {
    pub channel_id: u32,
    pub retain: bool,
    pub payload: Vec<u8>,
}

/// Inbound event yielded by [`HostServiceProvider::poll_host_events`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    Push(HostPushEvent),
    Channel(HostChannelEvent),
}

/// Unified zero-JSON host capability transport implemented by FFI, in-memory mock, and OS backends.
pub trait HostServiceProvider: Send + Sync {
    /// Invokes `service_id::method_id` with a binary `request` slice.
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError>;

    /// Convenience helper requiring a synchronous response payload.
    fn invoke_sync(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<Vec<u8>, SdkError> {
        match self.invoke_raw(service_id, method_id, request)? {
            InvokeResult::Ready(bytes) => Ok(bytes),
            InvokeResult::Pending { request_id } => Err(SdkError::Pending { request_id }),
        }
    }

    /// Publishes a binary payload onto `channel_id` (optionally retained on `NveChannelBus`).
    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), SdkError>;

    /// Drains queued inbound host push events and channel messages.
    fn poll_host_events(&self) -> Vec<HostEvent>;

    /// Consumes a completed async response for `request_id` if one has arrived.
    fn take_async_response(&self, _request_id: u64) -> Option<Result<Vec<u8>, SdkError>> {
        None
    }
}

impl<T: HostServiceProvider + ?Sized> HostServiceProvider for &T {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError> {
        (**self).invoke_raw(service_id, method_id, request)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), SdkError> {
        (**self).channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        (**self).poll_host_events()
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, SdkError>> {
        (**self).take_async_response(request_id)
    }
}

impl<T: HostServiceProvider + ?Sized> HostServiceProvider for Arc<T> {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError> {
        (**self).invoke_raw(service_id, method_id, request)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), SdkError> {
        (**self).channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        (**self).poll_host_events()
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, SdkError>> {
        (**self).take_async_response(request_id)
    }
}
