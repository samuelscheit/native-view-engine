//! [`LaidOutScene`]: one surface's laid-out view of a `NodeStore` shared by every native backend
//! and `nve-runtime` — the layout engine, viewport, text measurer, interaction pseudo-state
//! ([`InteractionTracker`]), scroll offsets, and reusable zero-alloc hit-walk scratch.
//!
//! Backends keep only what is theirs: `HeadlessBackend` adds mutation tracking and Frame-0
//! export, `SkiaBackend` adds painting, text focus, and motion, and `nve-runtime` adds the
//! zero-copy `NveSceneView` publisher (the only one) and C ABI.

mod hit_test;
mod interaction;
mod scroll;
mod snapshot;

use std::cell::Cell;
use std::sync::Arc;

use nve_components::interaction::InteractionTracker;
use nve_components::tables::Interaction;
use nve_engine::{LayoutRect, Viewport};
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::{HeuristicTextMeasurer, StoreLayoutEngine, TaffyStoreLayout, TextMeasurer};

pub use scroll::{ScrollOffsets, compute_scroll_content_size};
pub use snapshot::{layout_snapshot_of, length_to_dim, visual_snapshot_of};

/// One surface's laid-out view of a `NodeStore`.
///
/// Holds zero copies of the tree: every query reads `&NodeStore` plus the engine's rects.
pub struct LaidOutScene<L: StoreLayoutEngine = TaffyStoreLayout> {
    pub(crate) layout: L,
    pub(crate) viewport: Viewport,
    text_measurer: Arc<dyn TextMeasurer>,
    pub(crate) interaction: InteractionTracker,
    pub(crate) scroll: ScrollOffsets,
    /// `(z, index)` sort keys reused across hit-test queries (mixed-`z_index` sibling lists
    /// longer than the walk's stack buffer); keeps its high-water capacity. A `Cell` suffices:
    /// a query never re-enters, and `&mut self` is required for mutations.
    pub(crate) hit_scratch: Cell<Vec<(i32, u32)>>,
}

impl Default for LaidOutScene<TaffyStoreLayout> {
    fn default() -> Self {
        Self::new(TaffyStoreLayout::new())
    }
}

impl<L: StoreLayoutEngine> LaidOutScene<L> {
    /// Wraps `layout`, installing the deterministic [`HeuristicTextMeasurer`].
    pub fn new(layout: L) -> Self {
        Self::with_text_measurer(layout, Arc::new(HeuristicTextMeasurer))
    }

    /// Wraps `layout` and installs `text_measurer` into it.
    pub fn with_text_measurer(mut layout: L, text_measurer: Arc<dyn TextMeasurer>) -> Self {
        layout.set_text_measurer(Arc::clone(&text_measurer));
        Self {
            layout,
            viewport: Viewport::default(),
            text_measurer,
            interaction: InteractionTracker::new(),
            scroll: ScrollOffsets::default(),
            hit_scratch: Cell::new(Vec::new()),
        }
    }

    #[inline]
    pub fn layout_engine_name(&self) -> &'static str {
        self.layout.name()
    }

    #[inline]
    pub fn layout_engine(&self) -> &L {
        &self.layout
    }

    #[inline]
    pub fn layout_engine_mut(&mut self) -> &mut L {
        &mut self.layout
    }

    #[inline]
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Sets the viewport; returns `true` when it changed (the next pass re-runs layout).
    pub fn set_viewport(&mut self, viewport: Viewport) -> bool {
        let changed = self.viewport != viewport;
        self.viewport = viewport;
        changed
    }

    #[inline]
    pub fn text_measurer(&self) -> &Arc<dyn TextMeasurer> {
        &self.text_measurer
    }

    pub fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.text_measurer = Arc::clone(&measurer);
        self.layout.set_text_measurer(measurer);
    }

    /// Hot-swaps the layout engine, keeping viewport, text measurer, scroll, and interaction
    /// state. Follow with [`Self::recompute_layout`] for an immediate pass.
    pub fn swap_layout_engine<L2: StoreLayoutEngine>(self, mut next: L2) -> LaidOutScene<L2> {
        next.set_text_measurer(Arc::clone(&self.text_measurer));
        LaidOutScene {
            layout: next,
            viewport: self.viewport,
            text_measurer: self.text_measurer,
            interaction: self.interaction,
            scroll: self.scroll,
            hit_scratch: self.hit_scratch,
        }
    }

    /// Replaces the layout engine in-place when keeping the same engine type `L` (e.g.,
    /// `DynLayoutEngine`), avoiding any intermediate allocation.
    pub fn replace_layout_engine(&mut self, mut next: L) {
        next.set_text_measurer(Arc::clone(&self.text_measurer));
        self.layout = next;
    }

    /// Absolute (surface-space) rect of `id` from the latest layout pass.
    #[inline]
    pub fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.layout.layout_of(id)
    }

    /// Syncs interaction removals/disabled state, scroll removals, and layout changes without
    /// running the layout pass yet (useful when callers time layout separately).
    pub fn sync_changes(&mut self, store: &NodeStore, changes: &ChangeSet) {
        for &id in &changes.removed {
            self.interaction.remove_node(id.raw());
            self.scroll.remove(id.raw());
        }
        for &id in changes.created.iter().chain(&changes.touched) {
            if !store.is_live(id) {
                continue;
            }
            let disabled = store.try_row::<Interaction>(id).is_some_and(|i| i.disabled());
            if self.interaction.is_disabled(id.raw()) != disabled {
                let _ = self.interaction.set_disabled(id.raw(), disabled);
            }
        }
        self.layout.sync_changes(store, changes);
    }

    /// Applies one committed `ChangeSet` and runs the layout engine's self-gated pass.
    /// Returns whether a layout pass ran.
    pub fn commit(&mut self, store: &NodeStore, changes: &ChangeSet) -> bool {
        self.sync_changes(store, changes);
        self.recompute_layout(store)
    }

    /// Runs the engine's self-gated layout pass for the store root at the current viewport.
    pub fn recompute_layout(&mut self, store: &NodeStore) -> bool {
        self.layout.compute_layout(store, NodeId::ROOT, self.viewport)
    }

    /// Clears layout, interaction, and scroll state (keeps viewport and text measurer).
    pub fn reset(&mut self) {
        self.layout.reset();
        self.interaction.clear();
        self.scroll.clear();
    }

    /// Current length and high-water capacity of the reusable hit-test sort scratch buffer.
    #[inline]
    pub fn hit_scratch_stats(&self) -> (usize, usize) {
        let scratch = self.hit_scratch.take();
        let stats = (scratch.len(), scratch.capacity());
        self.hit_scratch.set(scratch);
        stats
    }
}
