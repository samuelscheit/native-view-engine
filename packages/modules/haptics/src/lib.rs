//! Tactile haptic feedback capability (`SERVICE_HAPTICS = 0x0016`) with direct zero-serialization
//! in-process dispatch ([`HapticsBackend`]), cross-language `NVE_IFACE_HOST` C ABI codecs,
//! deterministic [`MemoryHaptics`], and OS [`NativeHaptics`] handlers.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod native;

use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex};

pub use nve_abi::SERVICE_HAPTICS;
use nve_host_services::{
    pack_typed, resolve_typed, ByteReader, HostServiceHandler, HostServiceProvider, NveCodec,
    ServiceError,
};

pub use native::NativeHaptics;

pub const METHOD_IMPACT: u32 = 1;
pub const METHOD_NOTIFICATION: u32 = 2;
pub const METHOD_SELECTION: u32 = 3;

/// Impact intensity style for [`HapticsClient::impact`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveCodec)]
#[nve_codec(crate_path = "::nve_host_services")]
pub enum ImpactStyle {
    Light = 1,
    Medium = 2,
    Heavy = 3,
    Soft = 4,
    Rigid = 5,
}

impl ImpactStyle {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        Self::from_repr(raw)
    }
}

/// Notification feedback style for [`HapticsClient::notification`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveCodec)]
#[nve_codec(crate_path = "::nve_host_services")]
pub enum NotificationHaptic {
    Success = 1,
    Warning = 2,
    Error = 3,
}

impl NotificationHaptic {
    pub const fn from_u8(raw: u8) -> Option<Self> {
        Self::from_repr(raw)
    }
}

/// Recorded haptic trigger captured by [`MemoryHaptics`] for test assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedHaptic {
    Impact(ImpactStyle),
    Notification(NotificationHaptic),
    Selection,
}

pub fn encode_impact_request(style: ImpactStyle) -> [u8; 1] {
    [style as u8]
}

pub fn decode_impact_request(bytes: &[u8]) -> Result<ImpactStyle, ServiceError> {
    let mut r = ByteReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    ImpactStyle::from_u8(raw)
        .ok_or_else(|| ServiceError::InvalidArg(format!("invalid ImpactStyle {raw}")))
}

pub fn encode_notification_haptic_request(kind: NotificationHaptic) -> [u8; 1] {
    [kind as u8]
}

pub fn decode_notification_haptic_request(bytes: &[u8]) -> Result<NotificationHaptic, ServiceError> {
    let mut r = ByteReader::new(bytes);
    let raw = r.read_u8()?;
    r.finish()?;
    NotificationHaptic::from_u8(raw)
        .ok_or_else(|| ServiceError::InvalidArg(format!("invalid NotificationHaptic {raw}")))
}

/// Direct typed in-process backend for [`SERVICE_HAPTICS`].
///
/// In-process Rust callers bypass binary wire encoding completely via
/// [`nve_host_services::resolve_typed`].
pub trait HapticsBackend: Send + Sync {
    fn impact(&self, style: ImpactStyle) -> Result<(), ServiceError>;
    fn notification(&self, kind: NotificationHaptic) -> Result<(), ServiceError>;
    fn selection(&self) -> Result<(), ServiceError>;
}

/// Bridges a [`HapticsBackend`] onto the cross-language `NVE_IFACE_HOST = 5` binary wire ABI.
pub fn dispatch_haptics_wire<B: HapticsBackend + ?Sized>(
    backend: &B,
    method_id: u32,
    request: &[u8],
) -> Result<Vec<u8>, ServiceError> {
    match method_id {
        METHOD_IMPACT => {
            let style = decode_impact_request(request)?;
            backend.impact(style)?;
            Ok(Vec::new())
        }
        METHOD_NOTIFICATION => {
            let kind = decode_notification_haptic_request(request)?;
            backend.notification(kind)?;
            Ok(Vec::new())
        }
        METHOD_SELECTION => {
            backend.selection()?;
            Ok(Vec::new())
        }
        _ => Err(ServiceError::Unsupported),
    }
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

    pub fn impact(&self, style: ImpactStyle) -> Result<(), ServiceError> {
        if let Some(backend) =
            resolve_typed::<dyn HapticsBackend>(self.provider, SERVICE_HAPTICS, METHOD_IMPACT)
        {
            return backend.impact(style);
        }
        let req = encode_impact_request(style);
        let resp = self
            .provider
            .invoke_sync(SERVICE_HAPTICS, METHOD_IMPACT, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty haptics::impact response".into(),
            ));
        }
        Ok(())
    }

    pub fn notification(&self, kind: NotificationHaptic) -> Result<(), ServiceError> {
        if let Some(backend) =
            resolve_typed::<dyn HapticsBackend>(self.provider, SERVICE_HAPTICS, METHOD_NOTIFICATION)
        {
            return backend.notification(kind);
        }
        let req = encode_notification_haptic_request(kind);
        let resp = self
            .provider
            .invoke_sync(SERVICE_HAPTICS, METHOD_NOTIFICATION, &req)?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty haptics::notification response".into(),
            ));
        }
        Ok(())
    }

    pub fn selection(&self) -> Result<(), ServiceError> {
        if let Some(backend) =
            resolve_typed::<dyn HapticsBackend>(self.provider, SERVICE_HAPTICS, METHOD_SELECTION)
        {
            return backend.selection();
        }
        let resp = self
            .provider
            .invoke_sync(SERVICE_HAPTICS, METHOD_SELECTION, &[])?;
        if !resp.is_empty() {
            return Err(ServiceError::Codec(
                "unexpected non-empty haptics::selection response".into(),
            ));
        }
        Ok(())
    }
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_HAPTICS`].
#[derive(Debug, Default)]
pub struct MemoryHaptics {
    log: Mutex<Vec<RecordedHaptic>>,
}

impl MemoryHaptics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn log(&self) -> Vec<RecordedHaptic> {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl HapticsBackend for MemoryHaptics {
    fn impact(&self, style: ImpactStyle) -> Result<(), ServiceError> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(RecordedHaptic::Impact(style));
        Ok(())
    }

    fn notification(&self, kind: NotificationHaptic) -> Result<(), ServiceError> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(RecordedHaptic::Notification(kind));
        Ok(())
    }

    fn selection(&self) -> Result<(), ServiceError> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(RecordedHaptic::Selection);
        Ok(())
    }
}

impl HostServiceHandler for MemoryHaptics {
    fn service_id(&self) -> u32 {
        SERVICE_HAPTICS
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_haptics_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn HapticsBackend>(self))
    }
}
