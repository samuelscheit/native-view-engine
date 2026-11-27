//! Standalone opt-in TV module for Native View Engine (`nve-tv`).
//!
//! Provides:
//! - Deterministic 2D geometric [`SpatialFocusEngine`] keyed by `nve_store::NodeId` for D-Pad,
//!   Siri Remote, and gamepad navigation: explicit neighbor overrides, per-group focus memory,
//!   modal focus traps, optional wrap-around, auto-repeat throttling, and scroll-into-view math.
//! - Typed [`RemoteAction`] with a dense `u8` code space and W3C key names.
//! - Zero-allocation web `KeyboardEvent` normalizer ([`map_web_tv_key`]) covering HbbTV, Tizen,
//!   webOS, Android TV / Fire TV, Xbox, and PlayStation browsers.
//! - W3C standard-gamepad repeater ([`GamepadRepeater`]).
//! - Title-safe TV overscan safe-area calculator ([`TvOverscanInsets`]).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod action;
pub mod gamepad;
pub mod geometry;
pub mod overscan;
pub mod spatial_focus;
pub mod throttle;
pub mod web_keys;

pub use action::{DecimalDigit, FocusDirection, RemoteAction};
pub use gamepad::{GamepadFrame, GamepadRepeater, standard_gamepad_button};
pub use geometry::{ScrollAdjustment, ScrollRegion, SpatialRect};
pub use nve_store::NodeId;
pub use overscan::TvOverscanInsets;
pub use spatial_focus::{
    FocusOutcome, GROUP_MEMORY_CAPACITY, RemoteInput, SpatialFocusEngine, SpatialFocusGroupId,
    SpatialFocusNode,
};
pub use throttle::{DEFAULT_REPEAT_INTERVAL_US, RepeatThrottle};
pub use web_keys::{TextInputContext, WebKeyInput, WebTvPlatform, map_web_tv_key};
