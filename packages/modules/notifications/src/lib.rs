//! Local user notifications capability (`SERVICE_NOTIFICATIONS = 0x0015`) using direct typed
//! Rust structs ([`NotificationRequest`], [`NotificationEvent`], [`NotificationsBackend`]),
//! zero-serialization [`TypedEventBus`] delivery, deterministic [`MemoryNotifications`], and
//! OS [`NativeNotifications`] handlers.

pub mod native;

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub use nve_abi::SERVICE_NOTIFICATIONS;
use nve_host_services::{
    pack_typed, resolve_typed, HostServiceHandler, HostServiceProvider, ServiceError,
    TypedEventBus, TypedEventSubscription,
};

pub use native::NativeNotifications;

pub const METHOD_REQUEST_PERMISSION: u32 = 1;
pub const METHOD_SHOW: u32 = 2;
pub const METHOD_CANCEL: u32 = 3;

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

/// Event emitted when the user activates a delivered notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationTapped {
    pub id: String,
    pub action_url: Option<String>,
}

/// Direct typed notification event delivered over [`TypedEventBus`] with zero serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    Tapped(NotificationTapped),
}

fn validate_notification_request(req: &NotificationRequest) -> Result<(), ServiceError> {
    if req.id.is_empty() {
        return Err(ServiceError::InvalidArg(
            "notification id must not be empty".into(),
        ));
    }
    if req.title.is_empty() {
        return Err(ServiceError::InvalidArg(
            "notification title must not be empty".into(),
        ));
    }
    Ok(())
}

fn validate_notification_id(id: &str) -> Result<(), ServiceError> {
    if id.is_empty() {
        return Err(ServiceError::InvalidArg(
            "notification id must not be empty".into(),
        ));
    }
    Ok(())
}

/// Validates and publishes a typed [`NotificationEvent`] directly onto `bus` with zero serialization.
pub fn emit_notification_event(
    bus: &TypedEventBus,
    event: NotificationEvent,
) -> Result<usize, ServiceError> {
    match &event {
        NotificationEvent::Tapped(tapped) => validate_notification_id(&tapped.id)?,
    }
    Ok(bus.publish(event))
}

/// Direct typed in-process backend for [`SERVICE_NOTIFICATIONS`].
pub trait NotificationsBackend: Send + Sync {
    fn request_permission(&self) -> Result<NotificationPermission, ServiceError>;
    fn show(&self, req: &NotificationRequest) -> Result<(), ServiceError>;
    fn cancel(&self, id: &str) -> Result<(), ServiceError>;
}

/// Typed ergonomic client for local OS notifications (zero in-process byte serialization).
#[derive(Debug, Clone, Copy)]
pub struct NotificationsClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> NotificationsClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn request_permission(&self) -> Result<NotificationPermission, ServiceError> {
        if let Some(backend) = resolve_typed::<dyn NotificationsBackend>(
            self.provider,
            SERVICE_NOTIFICATIONS,
            METHOD_REQUEST_PERMISSION,
        ) {
            return backend.request_permission();
        }
        let _ =
            self.provider
                .invoke_sync(SERVICE_NOTIFICATIONS, METHOD_REQUEST_PERMISSION, &[])?;
        Err(ServiceError::Unsupported)
    }

    pub fn show(&self, req: &NotificationRequest) -> Result<(), ServiceError> {
        validate_notification_request(req)?;
        if let Some(backend) = resolve_typed::<dyn NotificationsBackend>(
            self.provider,
            SERVICE_NOTIFICATIONS,
            METHOD_SHOW,
        ) {
            return backend.show(req);
        }
        let _ = self
            .provider
            .invoke_sync(SERVICE_NOTIFICATIONS, METHOD_SHOW, &[])?;
        Err(ServiceError::Unsupported)
    }

    pub fn cancel(&self, id: &str) -> Result<(), ServiceError> {
        validate_notification_id(id)?;
        if let Some(backend) = resolve_typed::<dyn NotificationsBackend>(
            self.provider,
            SERVICE_NOTIFICATIONS,
            METHOD_CANCEL,
        ) {
            return backend.cancel(id);
        }
        let _ = self
            .provider
            .invoke_sync(SERVICE_NOTIFICATIONS, METHOD_CANCEL, &[])?;
        Err(ServiceError::Unsupported)
    }

    /// Subscribes directly to typed [`NotificationEvent`]s on the provider's [`TypedEventBus`].
    pub fn subscribe_events<F>(&self, handler: F) -> Result<TypedEventSubscription, ServiceError>
    where
        F: Fn(&NotificationEvent) + Send + Sync + 'static,
    {
        let bus = self
            .provider
            .typed_event_bus()
            .ok_or(ServiceError::Unsupported)?;
        Ok(bus.subscribe::<NotificationEvent, F>(false, handler))
    }

    /// Drains queued typed [`NotificationEvent`]s directly from the provider's [`TypedEventBus`].
    pub fn drain_events(&self) -> Vec<Arc<NotificationEvent>> {
        self.provider
            .typed_event_bus()
            .map(|bus| bus.drain_typed::<NotificationEvent>())
            .unwrap_or_default()
    }
}

struct MemoryNotificationsState {
    permission: NotificationPermission,
    active: BTreeMap<String, NotificationRequest>,
    cancelled: Vec<String>,
}

/// Deterministic in-memory [`HostServiceHandler`] for [`SERVICE_NOTIFICATIONS`].
pub struct MemoryNotifications {
    state: Mutex<MemoryNotificationsState>,
}

impl Default for MemoryNotifications {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryNotifications {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryNotificationsState {
                permission: NotificationPermission::Authorized,
                active: BTreeMap::new(),
                cancelled: Vec::new(),
            }),
        }
    }

    pub fn set_permission(&self, perm: NotificationPermission) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .permission = perm;
    }

    pub fn active_notifications(&self) -> Vec<NotificationRequest> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .values()
            .cloned()
            .collect()
    }

    pub fn cancelled_notifications(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancelled
            .clone()
    }

    /// Emits a typed [`NotificationEvent::Tapped`] directly onto `bus` with zero serialization.
    pub fn emit_tapped(
        &self,
        bus: &TypedEventBus,
        tapped: NotificationTapped,
    ) -> Result<usize, ServiceError> {
        emit_notification_event(bus, NotificationEvent::Tapped(tapped))
    }
}

impl NotificationsBackend for MemoryNotifications {
    fn request_permission(&self) -> Result<NotificationPermission, ServiceError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .permission)
    }

    fn show(&self, req: &NotificationRequest) -> Result<(), ServiceError> {
        validate_notification_request(req)?;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.permission == NotificationPermission::Denied {
            return Err(ServiceError::PermissionDenied);
        }
        st.active.insert(req.id.clone(), req.clone());
        Ok(())
    }

    fn cancel(&self, id: &str) -> Result<(), ServiceError> {
        validate_notification_id(id)?;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.active.remove(id);
        st.cancelled.push(id.to_owned());
        Ok(())
    }
}

impl HostServiceHandler for MemoryNotifications {
    fn service_id(&self) -> u32 {
        SERVICE_NOTIFICATIONS
    }

    fn handle(&self, _method_id: u32, _request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn NotificationsBackend>(self))
    }
}
