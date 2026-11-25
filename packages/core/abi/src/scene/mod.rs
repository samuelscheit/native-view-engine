//! Single-source 64-byte cache-line `NveRenderNode` and zero-decode `NveSceneView` ABI.
//!
//! Instead of decoding a variable-length `CommandBuffer` or serializing JSON on every host
//! frame, runtimes publish a flat contiguous array of 64-byte `#[repr(C, align(64))]`
//! `NveRenderNode` records alongside three flat slabs (`text_slab`, `children_slab`,
//! `dirty_indices`) plus `removed_ids`. Native hosts (C/C++, Swift, Dart `dart:ffi`,
//! Android `DirectByteBuffer`, and Web WASM `DataView`) read dirty nodes directly in
//! `O(changed)` with zero decoding and zero per-frame allocations.

mod raw_view;
mod table;
#[cfg(test)]
mod tests;
mod types;

pub use raw_view::NveSceneRef;
pub use table::NveSceneTable;
pub use types::{
    component_kind_to_scene_u8, NveRenderNode, NveSceneView, SceneError, SceneNodeInput,
    NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE, NVE_NODE_KIND_CUSTOM, NVE_NODE_KIND_IMAGE,
    NVE_NODE_KIND_PRESSABLE, NVE_NODE_KIND_ROOT, NVE_NODE_KIND_SCROLL_VIEW,
    NVE_NODE_KIND_SKIA_CANVAS, NVE_NODE_KIND_TEXT, NVE_NODE_KIND_TEXT_INPUT, NVE_NODE_KIND_VIEW,
    NVE_NODE_LEAF_TEXT, NVE_RENDER_NODE_ALIGN, NVE_RENDER_NODE_SIZE, NVE_SCENE_SCHEMA_HASH,
};
