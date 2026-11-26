//! Standalone opt-in TV module for Native View Engine (`nve-tv`).
//!
//! Provides:
//! - Deterministic 2D geometric [`SpatialFocusEngine`] for D-Pad, Siri Remote, and gamepad
//!   navigation with per-group focus memory, modal focus trapping, optional axis wrap-around,
//!   and scroll-into-view calculation.
//! - Typed [`RemoteAction`] mapping standard W3C / TV remote key names to focus & playback actions.
//! - Title-safe TV overscan safe-area calculator ([`TvOverscanInsets`]).

pub mod overscan;
pub mod spatial_focus;

pub use overscan::{FocusDirection, RemoteAction, TvOverscanInsets};
pub use spatial_focus::{
    ScrollAdjustment, ScrollRegion, SpatialFocusEngine, SpatialFocusGroupId, SpatialFocusNode,
    SpatialRect,
};
