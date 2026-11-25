use std::fmt;

use nve_abi::SurfaceId;

use crate::surface::LifecycleState;

/// Monotonic subscription identifier for `NveChannelBus` listeners.
pub type SubscriptionId = u64;

/// Unified error type for cross-framework surface orchestration, layout, gesture,
/// channel bus, and island operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteropError {
    SurfaceNotFound(SurfaceId),
    DuplicateSurface(SurfaceId),
    ParentNotFound(SurfaceId),
    InvalidParentCycle {
        surface: SurfaceId,
        proposed_parent: SurfaceId,
    },
    SurfaceDestroyed(SurfaceId),
    InvalidLifecycleTransition {
        surface: SurfaceId,
        from: LifecycleState,
        to: LifecycleState,
    },
    InvalidConstraints(String),
    InvalidTopic(String),
    ChannelNotFound(u32),
    SubscriptionNotFound(SubscriptionId),
    Serialization(String),
    SyncDelta(String),
    SyncTransport(String),
    Plugin(String),
    Window(String),
}

#[cfg(feature = "window")]
impl From<crate::window::WindowError> for InteropError {
    fn from(value: crate::window::WindowError) -> Self {
        Self::Window(value.to_string())
    }
}

impl fmt::Display for InteropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SurfaceNotFound(id) => write!(f, "surface {id} not found"),
            Self::DuplicateSurface(id) => write!(f, "surface {id} is already registered"),
            Self::ParentNotFound(id) => write!(f, "parent surface {id} not found"),
            Self::InvalidParentCycle {
                surface,
                proposed_parent,
            } => write!(
                f,
                "cannot reparent surface {surface} under {proposed_parent}: would create a cycle"
            ),
            Self::SurfaceDestroyed(id) => write!(f, "surface {id} has already been destroyed"),
            Self::InvalidLifecycleTransition { surface, from, to } => write!(
                f,
                "invalid lifecycle transition for surface {surface}: {from:?} -> {to:?}"
            ),
            Self::InvalidConstraints(msg) => write!(f, "invalid size constraints: {msg}"),
            Self::InvalidTopic(msg) => write!(f, "invalid channel topic: {msg}"),
            Self::ChannelNotFound(id) => write!(f, "channel id {id} not found"),
            Self::SubscriptionNotFound(id) => write!(f, "subscription id {id} not found"),
            Self::Serialization(msg) => write!(f, "serialization error: {msg}"),
            Self::SyncDelta(msg) => write!(f, "sync delta protocol error: {msg}"),
            Self::SyncTransport(msg) => write!(f, "sync transport error: {msg}"),
            Self::Plugin(msg) => write!(f, "plugin host error: {msg}"),
            Self::Window(msg) => write!(f, "window lifecycle error: {msg}"),
        }
    }
}

impl std::error::Error for InteropError {}
