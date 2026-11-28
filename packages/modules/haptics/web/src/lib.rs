//! Browser `wasm32` leaf provider for `SERVICE_HAPTICS` (`0x0016`) via `navigator.vibrate`.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_HAPTICS;
use nve_haptics::{
    dispatch_haptics_wire, HapticsBackend, ImpactStyle, NotificationHaptic,
};
use nve_host_services::{
    pack_typed, CompletionSink, HandlerReply, HostServiceHandler, ServiceError,
};

/// Static vibration pattern (in milliseconds) for selection tick feedback.
pub const SELECTION_PATTERN_MS: &[u32] = &[6];

/// Returns the static vibration duration pattern (in milliseconds) for an [`ImpactStyle`].
#[inline]
pub const fn impact_pattern_ms(style: ImpactStyle) -> &'static [u32] {
    match style {
        ImpactStyle::Soft => &[8],
        ImpactStyle::Light => &[10],
        ImpactStyle::Medium => &[20],
        ImpactStyle::Rigid => &[25],
        ImpactStyle::Heavy => &[35],
    }
}

/// Returns the static vibration duration pattern (in milliseconds) for a [`NotificationHaptic`].
#[inline]
pub const fn notification_pattern_ms(kind: NotificationHaptic) -> &'static [u32] {
    match kind {
        NotificationHaptic::Success => &[12, 40, 18],
        NotificationHaptic::Warning => &[20, 40, 20],
        NotificationHaptic::Error => &[30, 30, 30, 30, 40],
    }
}

/// Browser [`HapticsBackend`] & [`HostServiceHandler`] for [`SERVICE_HAPTICS`].
#[derive(Debug, Default, Clone, Copy)]
pub struct WebHaptics;

impl WebHaptics {
    pub const fn new() -> Self {
        Self
    }

    fn trigger_pattern(pattern: &[u32]) -> Result<(), ServiceError> {
        #[cfg(target_arch = "wasm32")]
        {
            if wasm_impl::vibrate(pattern) {
                Ok(())
            } else {
                Err(ServiceError::Unsupported)
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = pattern;
            Err(ServiceError::Unsupported)
        }
    }
}

impl HapticsBackend for WebHaptics {
    fn impact(&self, style: ImpactStyle) -> Result<(), ServiceError> {
        Self::trigger_pattern(impact_pattern_ms(style))
    }

    fn notification(&self, kind: NotificationHaptic) -> Result<(), ServiceError> {
        Self::trigger_pattern(notification_pattern_ms(kind))
    }

    fn selection(&self) -> Result<(), ServiceError> {
        Self::trigger_pattern(SELECTION_PATTERN_MS)
    }
}

impl HostServiceHandler for WebHaptics {
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

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = "
export function nve_web_vibrate(pattern) {
  try {
    if (typeof navigator === 'undefined' || typeof navigator.vibrate !== 'function') return false;
    return Boolean(navigator.vibrate(Array.from(pattern)));
  } catch (_) {
    return false;
  }
}
")]
    extern "C" {
        fn nve_web_vibrate(pattern: &[u32]) -> bool;
    }

    pub(super) fn vibrate(pattern: &[u32]) -> bool {
        nve_web_vibrate(pattern)
    }
}
