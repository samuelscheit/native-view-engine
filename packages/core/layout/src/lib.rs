//! Shared `NodeStore` layout layer used by every native backend and host runtime.
//!
//! Owns the [`StoreLayoutEngine`] / [`TextMeasurer`] traits, the deterministic
//! [`HeuristicTextMeasurer`], and the default [`TaffyStoreLayout`] engine (direct Taffy 0.16
//! low-level layout over `NodeStore` word columns). `nve-backend-headless`, `nve-backend-skia`,
//! `nve-runtime`, and opt-in engines such as `packages/modules/layout-yoga` all depend on this
//! crate, so no core crate ever depends on a backend to lay out a tree.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod engine;
mod measure;
pub mod scene;
mod taffy_store;

pub use engine::{DynLayoutEngine, StoreLayoutEngine};
pub use measure::{HeuristicTextMeasurer, TextMeasurer};
pub use scene::{
    LaidOutScene, ScrollOffsets, compute_scroll_content_size, is_display_none, layout_snapshot_of,
    length_to_dim, visual_snapshot_of,
};
pub use taffy_store::{ResolvedTextStyle, TaffyStoreLayout};

/// Maximum tree depth any layout, hit-test, or publication walk descends before truncating.
pub const MAX_TREE_DEPTH: usize = 512;
