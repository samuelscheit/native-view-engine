//! Zero-dependency native OS notifications backends (macOS bundle-ID safe guard + fallback).

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_NOTIFICATIONS;
use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::{NotificationPermission, NotificationRequest, NotificationsBackend};

/// Native OS [`HostServiceHandler`] for [`SERVICE_NOTIFICATIONS`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeNotifications;

impl NativeNotifications {
    pub const fn new() -> Self {
        Self
    }
}

impl NotificationsBackend for NativeNotifications {
    fn request_permission(&self) -> Result<NotificationPermission, ServiceError> {
        platform::request_permission()
    }

    fn show(&self, req: &NotificationRequest) -> Result<(), ServiceError> {
        platform::show(req)
    }

    fn cancel(&self, id: &str) -> Result<(), ServiceError> {
        platform::cancel(id)
    }
}

impl HostServiceHandler for NativeNotifications {
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

#[cfg(target_os = "macos")]
mod platform {
    use nve_host_services::apple_objc::{
        cls, msg_id0, nsstring_to_string, sel, with_autorelease_pool,
    };
    use nve_host_services::ServiceError;

    use crate::{NotificationPermission, NotificationRequest};

    fn has_valid_app_bundle_identifier() -> bool {
        with_autorelease_pool(|| unsafe {
            let bundle_cls = cls(b"NSBundle\0");
            if bundle_cls.is_null() {
                return false;
            }
            let main_bundle = msg_id0(bundle_cls, sel(b"mainBundle\0"));
            if main_bundle.is_null() {
                return false;
            }
            let bundle_id = msg_id0(main_bundle, sel(b"bundleIdentifier\0"));
            nsstring_to_string(bundle_id).is_some_and(|id| !id.trim().is_empty())
        })
    }

    pub fn request_permission() -> Result<NotificationPermission, ServiceError> {
        // Guard BEFORE touching `UNUserNotificationCenter.currentNotificationCenter`, which throws
        // an uncaught `NSInternalInconsistencyException` in unbundled CLI/test processes.
        if !has_valid_app_bundle_identifier() {
            return Err(ServiceError::Unsupported);
        }
        Ok(NotificationPermission::NotDetermined)
    }

    pub fn show(_req: &NotificationRequest) -> Result<(), ServiceError> {
        if !has_valid_app_bundle_identifier() {
            return Err(ServiceError::Unsupported);
        }
        Err(ServiceError::Unsupported)
    }

    pub fn cancel(_id: &str) -> Result<(), ServiceError> {
        if !has_valid_app_bundle_identifier() {
            return Err(ServiceError::Unsupported);
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use nve_host_services::ServiceError;

    use crate::{NotificationPermission, NotificationRequest};

    pub fn request_permission() -> Result<NotificationPermission, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn show(_req: &NotificationRequest) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn cancel(_id: &str) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
}
