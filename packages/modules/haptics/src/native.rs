//! Zero-dependency native OS haptics backends (macOS `NSHapticFeedbackManager`, safe fallback).

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_HAPTICS;
use nve_host_services::{
    pack_typed, CompletionSink, HandlerReply, HostServiceHandler, ServiceError,
};

use crate::{dispatch_haptics_wire, HapticsBackend, ImpactStyle, NotificationHaptic};

/// Native OS [`HostServiceHandler`] for [`SERVICE_HAPTICS`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeHaptics;

impl NativeHaptics {
    pub const fn new() -> Self {
        Self
    }
}

impl HapticsBackend for NativeHaptics {
    fn impact(&self, style: ImpactStyle) -> Result<(), ServiceError> {
        platform::impact(style)
    }

    fn notification(&self, kind: NotificationHaptic) -> Result<(), ServiceError> {
        platform::notification(kind)
    }

    fn selection(&self) -> Result<(), ServiceError> {
        platform::selection()
    }
}

impl HostServiceHandler for NativeHaptics {
    fn service_id(&self) -> u32 {
        SERVICE_HAPTICS
    }

    fn handle(
        &self,
        method_id: u32,
        _request_id: u64,
        request: &[u8],
        _done: &CompletionSink,
    ) -> Result<HandlerReply, ServiceError> {
        dispatch_haptics_wire(self, method_id, request).map(HandlerReply::ready)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn HapticsBackend>(self))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use nve_host_services::apple_objc::{
        cls, msg_id0, msg_void_isize2, sel, with_autorelease_pool,
    };
    use nve_host_services::ServiceError;

    use crate::{ImpactStyle, NotificationHaptic};

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    fn perform_ns_haptic(pattern: isize) -> Result<(), ServiceError> {
        // SAFETY: Runs inside an active autorelease pool using static NUL-terminated AppKit class/selector names.
        with_autorelease_pool(|| unsafe {
            let mgr_cls = cls(b"NSHapticFeedbackManager\0");
            if mgr_cls.is_null() {
                return Err(ServiceError::Unsupported);
            }
            let performer = msg_id0(mgr_cls, sel(b"defaultPerformer\0"));
            if performer.is_null() {
                return Err(ServiceError::Unsupported);
            }
            msg_void_isize2(
                performer,
                sel(b"performFeedbackPattern:performanceTime:\0"),
                pattern,
                0,
            );
            Ok(())
        })
    }

    pub fn impact(style: ImpactStyle) -> Result<(), ServiceError> {
        let pattern = match style {
            ImpactStyle::Light | ImpactStyle::Soft => 1,
            ImpactStyle::Medium | ImpactStyle::Heavy | ImpactStyle::Rigid => 2,
        };
        perform_ns_haptic(pattern)
    }

    pub fn notification(_kind: NotificationHaptic) -> Result<(), ServiceError> {
        perform_ns_haptic(0)
    }

    pub fn selection() -> Result<(), ServiceError> {
        perform_ns_haptic(1)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use nve_host_services::ServiceError;

    use crate::{ImpactStyle, NotificationHaptic};

    pub fn impact(_style: ImpactStyle) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn notification(_kind: NotificationHaptic) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn selection() -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }
}
