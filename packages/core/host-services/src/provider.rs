//! Core [`HostServiceProvider`] and [`HostServiceHandler`] traits shared by
//! [`crate::FfiHostServices`], [`crate::HostServiceRegistry`], [`crate::MemoryHostServices`],
//! and [`crate::NativeHostServices`].

use std::any::{Any, TypeId};
use std::sync::Arc;

use crate::codec::BinReader;
use crate::error::ServiceError;
use crate::events::TypedEventBus;

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

    pub fn decode(bytes: &[u8]) -> Result<Self, ServiceError> {
        let mut r = BinReader::new(bytes);
        let raw = r.read_u8()?;
        r.finish()?;
        Self::from_u8(raw)
            .ok_or_else(|| ServiceError::Codec(format!("invalid PermissionStatus {raw}")))
    }
}

/// Immediate or asynchronous result from [`HostServiceProvider::invoke_raw`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvokeResult {
    /// Host completed synchronously and returned the binary response payload.
    Ready(Vec<u8>),
    /// Host accepted the call asynchronously; completion arrives under `request_id`.
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

/// Pluggable per-service handler registered on [`crate::HostServiceRegistry`],
/// [`crate::MemoryHostServices`], or [`crate::NativeHostServices`].
///
/// Standalone capability crates in `packages/modules/*` implement this trait for their own
/// `service_id` so `nve-host-services` has zero dependency on any capability module.
pub trait HostServiceHandler: Send + Sync {
    /// Built-in or custom `NVE_IFACE_HOST = 5` service identifier owned by this handler.
    fn service_id(&self) -> u32;

    /// Dispatches one binary `method_id` invocation across the cross-language C ABI boundary.
    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError>;

    /// Optional zero-serialization in-process typed backend hook called once on registration.
    ///
    /// Return `Some(pack_typed::<dyn MyBackend>(self))` so in-process Rust callers invoke the
    /// typed trait directly with zero `BinWriter`/`BinReader` serialization and zero allocation.
    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        None
    }
}

/// Packs an `Arc<B>` (typically `Arc<dyn SomeBackend>`) for [`HostServiceHandler::typed_backend`].
#[inline]
pub fn pack_typed<B: ?Sized + Send + Sync + 'static>(
    backend: Arc<B>,
) -> (TypeId, Arc<dyn Any + Send + Sync>) {
    (TypeId::of::<Arc<B>>(), Arc::new(backend))
}

/// Resolves a directly registered in-process typed backend `Arc<B>` from `provider` with zero
/// serialization and zero heap allocation.
///
/// Returns `None` for cross-language FFI providers (`FfiHostServices`) or when a test fault is
/// queued on `(service_id, method_id)`, allowing clients to fall back cleanly to `invoke_sync`.
#[inline]
pub fn resolve_typed<B: ?Sized + Send + Sync + 'static>(
    provider: &(impl HostServiceProvider + ?Sized),
    service_id: u32,
    method_id: u32,
) -> Option<Arc<B>> {
    let any = provider.typed_service(service_id, method_id, TypeId::of::<Arc<B>>())?;
    any.downcast_ref::<Arc<B>>().cloned()
}

/// Unified zero-JSON host capability transport implemented by FFI, registry, in-memory mock,
/// and OS backends.
pub trait HostServiceProvider: Send + Sync {
    /// Invokes `service_id::method_id` with a binary `request` slice (used across the
    /// cross-language `NVE_IFACE_HOST = 5` C ABI boundary and for fault injection).
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError>;

    /// Convenience helper requiring a synchronous response payload.
    fn invoke_sync(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<Vec<u8>, ServiceError> {
        match self.invoke_raw(service_id, method_id, request)? {
            InvokeResult::Ready(bytes) => Ok(bytes),
            InvokeResult::Pending { request_id } => Err(ServiceError::Pending { request_id }),
        }
    }

    /// Zero-serialization in-process lookup returning the typed backend `Arc<dyn Any + Send + Sync>`
    /// (containing `Arc<dyn SomeBackend>`) registered under `service_id`.
    ///
    /// Cross-language providers (`FfiHostServices`) return `None` to route over `NVE_IFACE_HOST = 5`.
    fn typed_service(
        &self,
        _service_id: u32,
        _method_id: u32,
        _type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        None
    }

    /// Publishes a binary payload onto `channel_id` (optionally retained on `NveChannelBus`).
    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError>;

    /// Drains queued inbound host push events and channel messages.
    fn poll_host_events(&self) -> Vec<HostEvent>;

    /// Zero-serialization in-process typed event bus for direct Rust <-> Rust event delivery.
    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        None
    }

    /// Consumes a completed async response for `request_id` if one has arrived.
    fn take_async_response(&self, _request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        None
    }
}

impl<T: HostServiceProvider + ?Sized> HostServiceProvider for &T {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        (**self).invoke_raw(service_id, method_id, request)
    }

    fn typed_service(
        &self,
        service_id: u32,
        method_id: u32,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        (**self).typed_service(service_id, method_id, type_id)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        (**self).channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        (**self).poll_host_events()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        (**self).typed_event_bus()
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        (**self).take_async_response(request_id)
    }
}

impl<T: HostServiceProvider + ?Sized> HostServiceProvider for Arc<T> {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        (**self).invoke_raw(service_id, method_id, request)
    }

    fn typed_service(
        &self,
        service_id: u32,
        method_id: u32,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        (**self).typed_service(service_id, method_id, type_id)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        (**self).channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        (**self).poll_host_events()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        (**self).typed_event_bus()
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        (**self).take_async_response(request_id)
    }
}
