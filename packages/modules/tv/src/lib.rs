//! Standalone opt-in TV module for Native View Engine (`nve-tv`).
//!
//! Provides:
//! - Deterministic 2D geometric [`SpatialFocusEngine`] for D-Pad, Siri Remote, and gamepad
//!   navigation with per-group focus memory, modal focus trapping, optional axis wrap-around,
//!   and scroll-into-view calculation.
//! - Typed [`RemoteAction`] and W3C / NVE `NK1` binary keydown encoder/decoder
//!   ([`Nk1KeyDown`], [`Nk1Flags`]) matching `packages/core/abi/src/events.rs` byte-for-byte.
//! - Title-safe TV overscan safe-area calculator ([`TvOverscanInsets`]).

pub mod overscan;
pub mod spatial_focus;

pub use overscan::{
    NK1_FLAG_ALT, NK1_FLAG_CTRL, NK1_FLAG_META, NK1_FLAG_SHIFT, NK1_HEADER_SIZE, NK1_MAGIC,
    NK1_MIN_LEN, Nk1Error, Nk1Flags, Nk1KeyDown, TvOverscanInsets,
};
pub use spatial_focus::{
    FocusDirection, RemoteAction, ScrollAdjustment, ScrollRegion, SpatialFocusEngine,
    SpatialFocusGroupId, SpatialFocusNode, SpatialRect,
};
