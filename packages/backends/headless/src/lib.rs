//! Pure-Rust headless RenderBackend generic over any [`LayoutEngine`] (defaulting to [`TaffyLayoutEngine`]).
//!
//! Provides <50ms edit->test verification of any Rust UI frontend, mutation diffing,
//! coordinate hit-testing, interactive pseudo-state & cursor tracking (`InteractionTracker`),
//! simulated native events, live layout-engine swapping, and an ASCII tree + rect inspector.

mod frame;
mod interaction;
pub mod slot_map;
pub mod taffy_engine;
mod validate;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nve_abi::{
    ColorRgba, ComponentKind, LayoutProps, NativeEvent, NodeId, RawPropValue, StyleProps,
    SurfaceId, ROOT_NODE_ID,
};
use nve_engine::{
    HeuristicTextMeasurer, LayoutEngine, LayoutRect, TextMeasurer, Viewport,
};

pub use nve_components::interaction::{
    CursorKind, FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
pub use nve_components::style_patch::{Display, StylePatch};
pub use slot_map::{NodeSlotMap, DENSE_SLOT_LIMIT};
use slot_map::VisitStamps;
pub use taffy_engine::{to_taffy_dim, to_taffy_lpa, to_taffy_style, TaffyLayoutEngine};
use validate::fmt_node;

pub(crate) const MAX_TREE_DEPTH: usize = 512;

#[derive(Debug, Clone)]
pub struct HeadlessNode {
    pub id: NodeId,
    pub kind: ComponentKind,
    pub custom_name: Option<String>,
    pub text: Option<Arc<str>>,
    pub layout: LayoutProps,
    pub style: StyleProps,
    pub has_explicit_style: bool,
    pub raw_props: HashMap<String, RawPropValue>,
    pub event_mask: u64,
    pub cursor: Option<CursorKind>,
    pub state_rules: Vec<StateStyleRule>,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub computed_rect: LayoutRect,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HeadlessMutation {
    Create(NodeId, ComponentKind),
    Remove(NodeId),
    UpdateLayout(NodeId, LayoutRect),
    UpdateText(NodeId, String),
    UpdateStyle(NodeId),
}

struct TextStyleFrame {
    id: NodeId,
    color: ColorRgba,
    font_size: f32,
    has_inherited: bool,
}

pub struct HeadlessBackend<L: LayoutEngine = TaffyLayoutEngine> {
    pub(crate) surface_id: SurfaceId,
    pub(crate) viewport: Viewport,
    pub(crate) nodes: NodeSlotMap<HeadlessNode>,
    pub(crate) layout: L,
    text_measurer: Arc<dyn TextMeasurer>,
    pub(crate) prev_rects: NodeSlotMap<(u64, LayoutRect)>,
    rect_gen: u64,
    pub(crate) track_mutations: bool,
    pub(crate) text_style_dirty: bool,
    pub(crate) has_nested_text: bool,
    pub(crate) visit_stamps: VisitStamps,
    pub(crate) interaction: InteractionTracker,
    pub(crate) scratch_children: Vec<NodeId>,
    pub(crate) scratch_old_children: Vec<NodeId>,
    pub(crate) scratch_prev_parents: Vec<NodeId>,
    scratch_walk: Vec<(NodeId, usize)>,
    scratch_text_stack: Vec<TextStyleFrame>,
    scratch_resync_styles: Vec<(NodeId, StyleProps)>,
    pub(crate) last_mutations: Vec<HeadlessMutation>,
    pub(crate) pending_events: Vec<NativeEvent>,
    pub(crate) dispatched_commands: Vec<(NodeId, String, Vec<u8>)>,
}

impl Default for HeadlessBackend<TaffyLayoutEngine> {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessBackend<TaffyLayoutEngine> {
    pub fn new() -> Self {
        Self::with_layout(TaffyLayoutEngine::new())
    }
}

impl<L: LayoutEngine> HeadlessBackend<L> {
    /// Creates a headless backend backed by any concrete or boxed [`LayoutEngine`].
    pub fn with_layout(mut layout: L) -> Self {
        let text_measurer: Arc<dyn TextMeasurer> = Arc::new(HeuristicTextMeasurer);
        layout.set_text_measurer(Arc::clone(&text_measurer));
        let mut this = Self {
            surface_id: 1,
            viewport: Viewport::default(),
            nodes: NodeSlotMap::new(),
            layout,
            text_measurer,
            prev_rects: NodeSlotMap::new(),
            rect_gen: 0,
            track_mutations: true,
            text_style_dirty: false,
            has_nested_text: false,
            visit_stamps: VisitStamps::new(),
            interaction: InteractionTracker::new(),
            scratch_children: Vec::new(),
            scratch_old_children: Vec::new(),
            scratch_prev_parents: Vec::new(),
            scratch_walk: Vec::new(),
            scratch_text_stack: Vec::new(),
            scratch_resync_styles: Vec::new(),
            last_mutations: Vec::new(),
            pending_events: Vec::new(),
            dispatched_commands: Vec::new(),
        };
        this.ensure_node(ROOT_NODE_ID, ComponentKind::Root, None);
        this
    }

    /// Configures whether per-frame [`HeadlessMutation`] recording and rect diffing are active.
    pub fn with_mutation_tracking(mut self, enabled: bool) -> Self {
        self.track_mutations = enabled;
        self
    }

    pub fn set_mutation_tracking(&mut self, enabled: bool) {
        self.track_mutations = enabled;
    }

    pub fn mutation_tracking(&self) -> bool {
        self.track_mutations
    }

    pub fn layout_engine(&self) -> &L {
        &self.layout
    }

    pub fn layout_engine_mut(&mut self) -> &mut L {
        &mut self.layout
    }

    pub fn text_measurer(&self) -> &Arc<dyn TextMeasurer> {
        &self.text_measurer
    }

    /// Replaces the active [`TextMeasurer`] and propagates it to the underlying [`LayoutEngine`].
    pub fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
        self.text_measurer = Arc::clone(&measurer);
        self.layout.set_text_measurer(measurer);
    }

    /// Hot-swaps the active [`LayoutEngine`] to `next`, replaying the entire retained node tree
    /// and recomputing rects.
    pub fn swap_layout_engine<NewL: LayoutEngine>(self, mut next: NewL) -> HeadlessBackend<NewL> {
        next.reset();
        next.set_text_measurer(Arc::clone(&self.text_measurer));

        for node in self.nodes.values() {
            next.upsert_node(node.id, node.kind);
            next.set_layout(node.id, &node.layout);
            next.set_style(node.id, &node.style);
            if let Some(text) = node.text.as_deref() {
                next.set_text(node.id, text);
            }
        }
        for node in self.nodes.values() {
            if !node.children.is_empty() {
                next.set_children(node.id, &node.children);
            }
        }

        let mut swapped = HeadlessBackend {
            surface_id: self.surface_id,
            viewport: self.viewport,
            nodes: self.nodes,
            layout: next,
            text_measurer: self.text_measurer,
            prev_rects: self.prev_rects,
            rect_gen: self.rect_gen,
            track_mutations: self.track_mutations,
            text_style_dirty: self.text_style_dirty,
            has_nested_text: self.has_nested_text,
            visit_stamps: self.visit_stamps,
            interaction: self.interaction,
            scratch_children: self.scratch_children,
            scratch_old_children: self.scratch_old_children,
            scratch_prev_parents: self.scratch_prev_parents,
            scratch_walk: self.scratch_walk,
            scratch_text_stack: self.scratch_text_stack,
            scratch_resync_styles: self.scratch_resync_styles,
            last_mutations: self.last_mutations,
            pending_events: self.pending_events,
            dispatched_commands: self.dispatched_commands,
        };
        swapped.recompute_layout();
        swapped
    }

    /// Re-runs the active layout engine against the retained tree and refreshes all `computed_rect`s.
    pub fn recompute_layout(&mut self) {
        self.run_layout_and_diff(false);
    }

    pub fn nodes(&self) -> &NodeSlotMap<HeadlessNode> {
        &self.nodes
    }

    #[inline]
    pub fn node(&self, id: NodeId) -> Option<&HeadlessNode> {
        self.nodes.get(id)
    }

    pub fn last_mutations(&self) -> &[HeadlessMutation] {
        &self.last_mutations
    }

    pub fn dispatched_commands(&self) -> &[(NodeId, String, Vec<u8>)] {
        &self.dispatched_commands
    }

    /// Queues a synthetic native event (e.g. simulated tap or text edit) for the next poll.
    pub fn push_event(&mut self, event: NativeEvent) {
        self.pending_events.push(event);
    }

    /// Hit-tests the committed layout tree in reverse paint order, returning the deepest node
    /// containing `(x, y)` (preferring nodes that subscribe to events if `interactive_only`).
    pub fn hit_test(&self, x: f32, y: f32, interactive_only: bool) -> Option<NodeId> {
        self.hit_test_rec(ROOT_NODE_ID, x, y, interactive_only, 0)
    }

    fn hit_test_rec(
        &self,
        id: NodeId,
        x: f32,
        y: f32,
        interactive_only: bool,
        depth: usize,
    ) -> Option<NodeId> {
        if depth > MAX_TREE_DEPTH {
            return None;
        }
        let node = self.nodes.get(id)?;
        let r = node.computed_rect;
        let inside = x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height;
        if !inside && id != ROOT_NODE_ID {
            return None;
        }
        for &child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_rec(child, x, y, interactive_only, depth + 1) {
                return Some(hit);
            }
        }
        if inside && (!interactive_only || node.event_mask != 0) {
            Some(id)
        } else {
            None
        }
    }

    /// Formats a clean ASCII tree with computed Flexbox rects, text, and event masks.
    pub fn render_ascii_tree(&self) -> String {
        let mut out = String::new();
        fmt_node(&self.nodes, ROOT_NODE_ID, 0, &mut out);
        out
    }

    pub(crate) fn ensure_node(&mut self, id: NodeId, kind: ComponentKind, custom_name: Option<&str>) {
        if let Some(existing) = self.nodes.get_mut(id) {
            let kind_changed = existing.kind != kind;
            existing.kind = kind;
            match (&mut existing.custom_name, custom_name) {
                (Some(slot), Some(name)) => {
                    if slot.as_str() != name {
                        slot.clear();
                        slot.push_str(name);
                    }
                }
                (slot, name) => *slot = name.map(ToOwned::to_owned),
            }
            if kind_changed {
                if (kind == ComponentKind::Text && !existing.children.is_empty())
                    || self.has_nested_text
                {
                    if kind == ComponentKind::Text && !existing.children.is_empty() {
                        self.has_nested_text = true;
                    }
                    self.text_style_dirty = true;
                }
                self.layout.upsert_node(id, kind);
            }
            return;
        }
        self.layout.upsert_node(id, kind);
        self.nodes.insert(
            id,
            HeadlessNode {
                id,
                kind,
                custom_name: custom_name.map(ToOwned::to_owned),
                text: None,
                layout: LayoutProps::default(),
                style: StyleProps::default(),
                has_explicit_style: false,
                raw_props: HashMap::new(),
                event_mask: 0,
                cursor: None,
                state_rules: Vec::new(),
                parent: None,
                children: Vec::new(),
                computed_rect: LayoutRect::default(),
            },
        );
    }

    fn propagate_text_styles(&mut self) {
        if !self.text_style_dirty {
            return;
        }
        self.text_style_dirty = false;

        let visit_gen = self.visit_stamps.next_gen();
        self.scratch_text_stack.clear();
        self.scratch_text_stack.push(TextStyleFrame {
            id: ROOT_NODE_ID,
            color: ColorRgba::BLACK,
            font_size: 16.0,
            has_inherited: false,
        });
        self.scratch_resync_styles.clear();
        let mut found_text_with_children = false;

        while let Some(frame) = self.scratch_text_stack.pop() {
            if !self.visit_stamps.mark(frame.id, visit_gen) {
                continue;
            }
            let Some(node) = self.nodes.get(frame.id) else {
                continue;
            };
            let mut next_color = frame.color;
            let mut next_font_size = frame.font_size;
            let mut next_has = frame.has_inherited;

            if node.kind == ComponentKind::Text {
                if !node.children.is_empty() {
                    found_text_with_children = true;
                }
                if !node.has_explicit_style {
                    let target_color = if frame.has_inherited {
                        frame.color
                    } else {
                        ColorRgba::BLACK
                    };
                    let target_font = if frame.has_inherited {
                        frame.font_size
                    } else {
                        16.0
                    };
                    if node.style.text_color != target_color
                        || (node.style.font_size - target_font).abs() > 1e-3
                    {
                        let mut updated = node.style;
                        updated.text_color = target_color;
                        updated.font_size = target_font;
                        self.scratch_resync_styles.push((frame.id, updated));
                    }
                    next_color = target_color;
                    next_font_size = target_font;
                } else {
                    next_color = node.style.text_color;
                    next_font_size = node.style.font_size;
                }
                next_has = true;
            }

            for &cid in node.children.iter().rev() {
                self.scratch_text_stack.push(TextStyleFrame {
                    id: cid,
                    color: next_color,
                    font_size: next_font_size,
                    has_inherited: next_has,
                });
            }
        }

        self.has_nested_text = found_text_with_children;

        for &(id, style) in &self.scratch_resync_styles {
            if let Some(node) = self.nodes.get_mut(id) {
                node.style = style;
            }
            self.layout.set_style(id, &style);
        }
    }

    pub(crate) fn run_layout_and_diff(&mut self, record_to_last_mutations: bool) -> (u64, u64) {
        let t_layout = Instant::now();
        self.propagate_text_styles();
        self.layout.compute_layout(ROOT_NODE_ID, self.viewport);
        let layout_us = t_layout.elapsed().as_micros() as u64;

        let t_diff = Instant::now();
        let track = self.track_mutations;
        let prev_gen = self.rect_gen;
        let next_gen = if track {
            let mut g = self.rect_gen.wrapping_add(1);
            if g == 0 {
                self.prev_rects.clear();
                g = 1;
            }
            self.rect_gen = g;
            g
        } else {
            0
        };

        self.scratch_walk.clear();
        self.scratch_walk.push((ROOT_NODE_ID, 0));
        while let Some((id, depth)) = self.scratch_walk.pop() {
            if depth > MAX_TREE_DEPTH {
                continue;
            }
            let Some(rect) = self.layout.layout_of(id) else {
                continue;
            };
            let Some(node) = self.nodes.get_mut(id) else {
                continue;
            };
            node.computed_rect = rect;
            if track {
                let unchanged = self
                    .prev_rects
                    .insert(id, (next_gen, rect))
                    .is_some_and(|(g, prev)| g == prev_gen && prev == rect);
                if !unchanged && record_to_last_mutations {
                    self.last_mutations
                        .push(HeadlessMutation::UpdateLayout(id, rect));
                }
            }
            if depth < MAX_TREE_DEPTH {
                for &child in node.children.iter().rev() {
                    self.scratch_walk.push((child, depth + 1));
                }
            }
        }
        let diff_us = t_diff.elapsed().as_micros() as u64;
        (layout_us, diff_us)
    }
}

#[cfg(test)]
mod tests;
