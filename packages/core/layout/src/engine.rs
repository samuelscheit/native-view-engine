//! Direct `NodeStore` layout engine trait (`StoreLayoutEngine`) and its runtime-switchable
//! boxed form (`DynLayoutEngine`).
//!
//! Backends such as `HeadlessBackend<L>` and `SkiaBackend<L>` are generic over
//! [`StoreLayoutEngine`] (defaulting to `TaffyStoreLayout` for zero dynamic-dispatch overhead)
//! while also supporting dynamic runtime switching via [`DynLayoutEngine`].

use std::sync::Arc;

use nve_engine::{LayoutRect, Viewport};
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::TextMeasurer;

/// Direct `NodeStore` layout engine trait implemented by [`crate::TaffyStoreLayout`], real
/// React Native Yoga (`packages/modules/layout-yoga`), custom layout engines,
/// and [`DynLayoutEngine`] (`Box<dyn StoreLayoutEngine>`).
///
/// Computed rectangles returned by [`StoreLayoutEngine::layout_of`] are expressed in
/// surface-space (root-relative absolute) coordinates in logical points.
pub trait StoreLayoutEngine: Send {
    /// Human-readable engine identifier (e.g. `"taffy"`, `"yoga"`).
    fn name(&self) -> &'static str;

    /// Invalidates cached layout entries for every layout/measure/structural change in `changes`.
    fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet);

    /// Computes layout for `root` at `viewport` directly from `store` word columns.
    ///
    /// **Self-gating contract**: the engine is the single owner of the clean-frame gate. Callers
    /// invoke this on every frame after [`StoreLayoutEngine::sync_changes`]; when nothing that
    /// affects layout changed since the last pass (no layout/measure/structural change, same
    /// `root` and viewport, no measurer swap or reset), the engine returns `false` without
    /// running a pass and every [`StoreLayoutEngine::layout_of`] result stays current. Returns
    /// `true` when a pass ran (layout results may have changed). A pass that unwinds (e.g. a
    /// panicking host `TextMeasurer`) must leave the engine dirty so the next call retries.
    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool;

    /// Returns the absolute rounded layout rect of `id` from the latest pass, if live.
    fn layout_of(&self, id: NodeId) -> Option<LayoutRect>;

    /// Installs a custom text measurer and invalidates cached text measurements.
    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>);

    /// Clears all cached layout state.
    fn reset(&mut self);
}

/// Type alias for a runtime-switchable boxed `NodeStore` layout engine.
pub type DynLayoutEngine = Box<dyn StoreLayoutEngine>;

impl<L: StoreLayoutEngine + ?Sized> StoreLayoutEngine for Box<L> {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        (**self).sync_changes(store, changes);
    }

    fn compute_layout(&mut self, store: &NodeStore, root: NodeId, viewport: Viewport) -> bool {
        (**self).compute_layout(store, root, viewport)
    }

    fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        (**self).layout_of(id)
    }

    fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        (**self).set_text_measurer(measurer);
    }

    fn reset(&mut self) {
        (**self).reset();
    }
}

