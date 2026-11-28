#![cfg_attr(doctest, doc = include_str!("../README.md"))]
//! Platform-agnostic AccessKit accessibility tree projection, incremental `ChangeSet` diffing,
//! and epoch-guarded action translation over [`nve_store::NodeStore`].

pub mod action;
mod dirty;
pub mod geometry;
pub mod id;
pub mod node_build;
pub mod role;
pub mod tree;

pub use action::{
    byte_offset_to_char_index, char_index_to_byte_offset, translate_action_request, A11yAction,
    ResolvedA11yAction, DEFAULT_SCROLL_STEP_ITEM, DEFAULT_SCROLL_STEP_PAGE,
};
pub use geometry::{transform_2d_to_affine, A11yGeometryProvider, A11yNodeHash};
pub use id::{
    decode_a11y_id, decode_store_node_id, encode_store_node_id, encode_text_run_id, DecodedA11yId,
    StoreEpoch,
};
pub use role::{aria_role_to_accesskit, resolve_node_role, text_input_role};
pub use tree::AccessKitTreeBridge;

#[cfg(test)]
mod tests;
