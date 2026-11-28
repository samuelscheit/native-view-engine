//! Pure-Rust headless [`nve_engine::StoreBackend`] generic over any [`StoreLayoutEngine`]
//! (defaulting to `nve_layout::TaffyStoreLayout` directly over `NodeStore` word columns).
//!
//! Wraps [`LaidOutScene`] and adds mutation diffing, hit testing, and ASCII tree + rect
//! inspection.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod commit;
mod hit_test;
mod interaction;

use std::fmt::Write as _;
use std::sync::Arc;

use nve_abi::{NativeEvent, NodeId, SurfaceId};
use nve_engine::{LayoutRect, Viewport};
use nve_layout::{LaidOutScene, StoreLayoutEngine, TaffyStoreLayout, TextMeasurer};
use nve_components::tables::TextContent;
use nve_store::{NodeId as StoreNodeId, NodeStore, TableSlot};

pub use nve_components::interaction::{
    CursorKind, FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
pub use nve_components::layout_ext::{
    GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack, GridTrackPlacement,
    GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges, PositionInsets, PositionMode,
};
pub use nve_components::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
pub use nve_components::style_patch::{Display, LayoutSnapshot, StylePatch, VisualSnapshot};
pub use nve_layout::{layout_snapshot_of, visual_snapshot_of};

pub(crate) use nve_layout::MAX_TREE_DEPTH;

#[derive(Debug, Clone, PartialEq)]
pub enum HeadlessMutation {
    Create(NodeId, TableSlot),
    Remove(NodeId),
    UpdateLayout(NodeId, LayoutRect),
    UpdateText(NodeId, String),
    UpdateStyle(NodeId),
}

/// Pure-Rust headless [`StoreBackend`] backed by a zero-copy [`LaidOutScene`].
///
/// Holds ZERO duplicate copies of the UI tree, strings, or style props — reads directly from
/// `&NodeStore` during `commit` and inspection methods.
pub struct HeadlessBackend<L: StoreLayoutEngine = TaffyStoreLayout> {
    pub(crate) surface_id: SurfaceId,
    pub(crate) scene: LaidOutScene<L>,
    pub(crate) prev_rects: Vec<(u64, LayoutRect)>,
    pub(crate) rect_gen: u64,
    pub(crate) track_mutations: bool,
    pub(crate) last_mutations: Vec<HeadlessMutation>,
    pub(crate) pending_events: Vec<NativeEvent>,
    pub(crate) walk_stack: Vec<(StoreNodeId, usize)>,
}

impl Default for HeadlessBackend<TaffyStoreLayout> {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessBackend<TaffyStoreLayout> {
    pub fn new() -> Self {
        Self::with_layout(TaffyStoreLayout::new())
    }
}

impl<L: StoreLayoutEngine> HeadlessBackend<L> {
    pub fn with_layout(layout: L) -> Self {
        Self {
            surface_id: 1,
            scene: LaidOutScene::new(layout),
            prev_rects: Vec::new(),
            rect_gen: 0,
            track_mutations: true,
            last_mutations: Vec::new(),
            pending_events: Vec::new(),
            walk_stack: Vec::new(),
        }
    }

    #[inline]
    pub fn scene(&self) -> &LaidOutScene<L> {
        &self.scene
    }

    #[inline]
    pub fn scene_mut(&mut self) -> &mut LaidOutScene<L> {
        &mut self.scene
    }

    pub fn with_mutation_tracking(mut self, enabled: bool) -> Self {
        self.track_mutations = enabled;
        self
    }

    #[inline]
    pub fn set_mutation_tracking(&mut self, enabled: bool) {
        self.track_mutations = enabled;
    }

    #[inline]
    pub fn layout_engine_name(&self) -> &'static str {
        self.scene.layout_engine_name()
    }

    #[inline]
    pub fn layout_engine(&self) -> &L {
        self.scene.layout_engine()
    }

    #[inline]
    pub fn layout_engine_mut(&mut self) -> &mut L {
        self.scene.layout_engine_mut()
    }

    #[inline]
    pub fn viewport(&self) -> Viewport {
        self.scene.viewport()
    }

    #[inline]
    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    pub fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.scene.set_text_measurer(measurer);
    }

    #[inline]
    pub fn text_measurer(&self) -> &Arc<dyn TextMeasurer> {
        self.scene.text_measurer()
    }

    /// Hot-swaps the active layout engine while preserving viewport, scroll, and interaction state.
    pub fn swap_layout_engine<L2: StoreLayoutEngine>(
        self,
        new_layout: L2,
    ) -> HeadlessBackend<L2> {
        HeadlessBackend {
            surface_id: self.surface_id,
            scene: self.scene.swap_layout_engine(new_layout),
            prev_rects: self.prev_rects,
            rect_gen: self.rect_gen,
            track_mutations: self.track_mutations,
            last_mutations: self.last_mutations,
            pending_events: self.pending_events,
            walk_stack: self.walk_stack,
        }
    }

    /// Recomputes layout for `store` immediately (e.g. after swapping layout engines or viewport);
    /// returns whether the engine ran a pass (it self-gates clean frames).
    pub fn recompute_layout(&mut self, store: &NodeStore) -> bool {
        self.scene.recompute_layout(store)
    }

    #[inline]
    pub fn last_mutations(&self) -> &[HeadlessMutation] {
        &self.last_mutations
    }

    /// Renders a human-readable indented tree of live nodes, layout rects, test IDs, and text.
    pub fn render_ascii_tree(&self, store: &NodeStore) -> String {
        use core::fmt::Write as _;
        fn walk<L: StoreLayoutEngine>(
            backend: &HeadlessBackend<L>,
            store: &NodeStore,
            id: StoreNodeId,
            depth: usize,
            out: &mut String,
        ) {
            for _ in 0..depth {
                out.push_str("  ");
            }
            let r = backend.layout_of(id.raw()).unwrap_or_default();
            let _ = write!(
                out,
                "Node#{} ({:.0},{:.0} {:.0}×{:.0})",
                id.raw(),
                r.x,
                r.y,
                r.width,
                r.height
            );
            if let Some(dbg) = store.try_row::<nve_components::tables::DebugId>(id) {
                let tid = dbg.test_id();
                if !tid.is_empty() {
                    let _ = write!(out, " [{tid}]");
                }
            }
            if let Some(tc) = store.try_row::<nve_components::tables::TextContent>(id) {
                let t = tc.text();
                if !t.is_empty() {
                    let _ = write!(out, " \"{t}\"");
                }
            }
            if let Some(ti) = store.try_row::<nve_components::tables::TextInput>(id) {
                let v = ti.value();
                if !v.is_empty() {
                    let _ = write!(out, " value=\"{v}\"");
                }
            }
            out.push('\n');
            for &child in store.children(id) {
                walk(backend, store, child, depth + 1, out);
            }
        }
        let mut out = String::new();
        walk(self, store, StoreNodeId::ROOT, 0, &mut out);
        out
    }

    #[inline]
    pub fn push_event(&mut self, event: NativeEvent) {
        self.pending_events.push(event);
    }

    #[inline]
    pub fn set_scroll_offset(&mut self, id: NodeId, dx: f32, dy: f32) {
        self.scene.set_scroll_offset_raw(id, dx, dy);
    }

    #[inline]
    pub fn scroll_offset(&self, id: NodeId) -> (f32, f32) {
        self.scene.scroll_offset(id)
    }

    #[inline]
    pub fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
        self.scene.layout_of(StoreNodeId(id))
    }

    /// Formats a human-readable ASCII tree + layout rect dump of `store` for debugging and snapshot tests.
    pub fn dump_tree(&self, store: &NodeStore) -> String {
        let mut out = String::new();
        self.dump_rec(store, StoreNodeId::ROOT, 0, &mut out);
        out
    }

    fn dump_rec(&self, store: &NodeStore, id: StoreNodeId, depth: usize, out: &mut String) {
        if depth > MAX_TREE_DEPTH || !store.is_live(id) {
            return;
        }
        for _ in 0..depth {
            out.push_str("  ");
        }
        let rect = self.scene.layout_of(id).unwrap_or_default();
        let slot = store.primary_slot(id);
        let _ = write!(
            out,
            "#{} [slot={}] ({:.1}, {:.1}, {:.1}x{:.1})",
            id.raw(),
            slot.0,
            rect.x,
            rect.y,
            rect.width,
            rect.height
        );
        if let Some(t) = store.try_row::<TextContent>(id) {
            let s = t.text();
            if !s.is_empty() {
                let _ = write!(out, " {:?}", s);
            }
        }
        out.push('\n');
        for &cid in store.children(id) {
            self.dump_rec(store, cid, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests;
