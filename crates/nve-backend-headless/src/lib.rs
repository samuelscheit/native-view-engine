//! Pure-Rust headless RenderBackend generic over any [`LayoutEngine`] (defaulting to [`TaffyLayoutEngine`]).
//!
//! Provides <50ms edit->test verification of any Rust UI frontend, mutation diffing,
//! coordinate hit-testing, simulated native events, live layout-engine swapping, and an
//! ASCII tree + rect inspector.

pub mod slot_map;
pub mod taffy_engine;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nve_abi::{
    ColorRgba, CommandDecoder, CommandRef, ComponentKind, Dimension, LayoutProps, NativeEvent,
    NodeId, RawPropValue, StyleProps, SurfaceId, ROOT_NODE_ID,
};
use nve_engine::{
    BackendApplyStats, BackendError, HeuristicTextMeasurer, LayoutEngine, LayoutRect,
    RenderBackend, TextMeasurer, Viewport,
};

pub use slot_map::{NodeSlotMap, DENSE_SLOT_LIMIT};
use slot_map::VisitStamps;
pub use taffy_engine::{to_taffy_dim, to_taffy_lpa, to_taffy_style, TaffyLayoutEngine};

const MAX_TREE_DEPTH: usize = 512;

fn is_valid_scalar(v: f32) -> bool {
    v.is_finite() && v.abs() <= 1e7
}

fn is_valid_dim(d: Dimension) -> bool {
    match d {
        Dimension::Auto => true,
        Dimension::Points(v) | Dimension::Percent(v) => is_valid_scalar(v),
    }
}

fn validate_layout(l: &LayoutProps) -> Result<(), BackendError> {
    let ok = is_valid_dim(l.width)
        && is_valid_dim(l.height)
        && is_valid_dim(l.min_width)
        && is_valid_dim(l.min_height)
        && is_valid_dim(l.max_width)
        && is_valid_dim(l.max_height)
        && is_valid_scalar(l.flex_grow)
        && l.flex_grow >= 0.0
        && is_valid_scalar(l.flex_shrink)
        && l.flex_shrink >= 0.0
        && is_valid_scalar(l.gap)
        && l.gap >= 0.0
        && is_valid_scalar(l.padding.top)
        && is_valid_scalar(l.padding.right)
        && is_valid_scalar(l.padding.bottom)
        && is_valid_scalar(l.padding.left)
        && is_valid_scalar(l.margin.top)
        && is_valid_scalar(l.margin.right)
        && is_valid_scalar(l.margin.bottom)
        && is_valid_scalar(l.margin.left);
    if ok {
        Ok(())
    } else {
        Err(BackendError::Internal(
            "invalid non-finite or out-of-range layout scalar".into(),
        ))
    }
}

fn validate_style(s: &StyleProps) -> Result<(), BackendError> {
    let ok = is_valid_scalar(s.border_width)
        && is_valid_scalar(s.border_radius)
        && is_valid_scalar(s.opacity)
        && is_valid_scalar(s.font_size);
    if ok {
        Ok(())
    } else {
        Err(BackendError::Internal(
            "invalid non-finite or out-of-range style scalar".into(),
        ))
    }
}

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
    surface_id: SurfaceId,
    viewport: Viewport,
    nodes: NodeSlotMap<HeadlessNode>,
    layout: L,
    text_measurer: Arc<dyn TextMeasurer>,
    prev_rects: NodeSlotMap<(u64, LayoutRect)>,
    rect_gen: u64,
    track_mutations: bool,
    text_style_dirty: bool,
    has_nested_text: bool,
    visit_stamps: VisitStamps,
    scratch_children: Vec<NodeId>,
    scratch_old_children: Vec<NodeId>,
    scratch_prev_parents: Vec<NodeId>,
    scratch_walk: Vec<(NodeId, usize)>,
    scratch_text_stack: Vec<TextStyleFrame>,
    scratch_resync_styles: Vec<(NodeId, StyleProps)>,
    last_mutations: Vec<HeadlessMutation>,
    pending_events: Vec<NativeEvent>,
    dispatched_commands: Vec<(NodeId, String, Vec<u8>)>,
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
    ///
    /// Defaults to `true` on [`HeadlessBackend::new()`] / [`HeadlessBackend::with_layout()`].
    /// Rendering backends that consume the retained node tree directly (such as `SkiaBackend`)
    /// disable mutation tracking to avoid unused per-frame diff allocations.
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
    /// (nodes, layouts, styles, text, children, and active [`TextMeasurer`]) and recomputing rects.
    pub fn swap_layout_engine<NewL: LayoutEngine>(self, mut next: NewL) -> HeadlessBackend<NewL> {
        next.reset();
        next.set_text_measurer(Arc::clone(&self.text_measurer));

        // Pass 1: create all nodes and replay their layout, style, and text state.
        for node in self.nodes.values() {
            next.upsert_node(node.id, node.kind);
            next.set_layout(node.id, &node.layout);
            next.set_style(node.id, &node.style);
            if let Some(text) = node.text.as_deref() {
                next.set_text(node.id, text);
            }
        }
        // Pass 2: attach children once every node exists in `next`.
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
        self.fmt_node(ROOT_NODE_ID, 0, &mut out);
        out
    }

    fn fmt_node(&self, id: NodeId, depth: usize, out: &mut String) {
        if depth > MAX_TREE_DEPTH {
            return;
        }
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        let indent = "  ".repeat(depth);
        let tag = match (&node.kind, &node.custom_name) {
            (ComponentKind::Custom, Some(name)) => format!("Custom<{name}>"),
            _ => format!("{:?}", node.kind),
        };
        let r = node.computed_rect;
        out.push_str(&format!(
            "{indent}{tag}#{id} [{:.1}, {:.1}, {:.1}x{:.1}]",
            r.x, r.y, r.width, r.height
        ));
        if let Some(text) = node.text.as_deref() {
            out.push_str(&format!(" \"{text}\""));
        }
        if node.event_mask != 0 {
            out.push_str(&format!(" events=0x{:x}", node.event_mask));
        }
        out.push('\n');
        for &child in &node.children {
            self.fmt_node(child, depth + 1, out);
        }
    }

    fn ensure_node(&mut self, id: NodeId, kind: ComponentKind, custom_name: Option<&str>) {
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
        let default_layout = LayoutProps::default();
        let default_style = StyleProps::default();
        self.layout.upsert_node(id, kind);
        self.nodes.insert(
            id,
            HeadlessNode {
                id,
                kind,
                custom_name: custom_name.map(ToOwned::to_owned),
                text: None,
                layout: default_layout,
                style: default_style,
                has_explicit_style: false,
                raw_props: HashMap::new(),
                event_mask: 0,
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

    fn run_layout_and_diff(&mut self, record_to_last_mutations: bool) -> (u64, u64) {
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

impl<L: LayoutEngine> RenderBackend for HeadlessBackend<L> {
    fn name(&self) -> &'static str {
        match self.layout.name() {
            "taffy" => "headless-taffy",
            "yoga" => "headless-yoga",
            _ => "headless-custom",
        }
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        self.surface_id = surface_id;
        self.viewport = viewport;
    }

    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError> {
        let t_decode = Instant::now();
        self.last_mutations.clear();
        let track_mutations = self.track_mutations;
        let mut committed = false;

        let decoder = CommandDecoder::new(command_bytes)?;
        if track_mutations {
            let est_ops = (decoder.header().op_count as usize).min(command_bytes.len() / 2);
            self.last_mutations.reserve(est_ops);
        }

        let header = decoder.for_each(|cmd| {
            match cmd {
                CommandRef::CreateNode {
                    node_id,
                    kind,
                    custom_name,
                } => {
                    self.ensure_node(node_id, kind, custom_name);
                    if track_mutations {
                        self.last_mutations
                            .push(HeadlessMutation::Create(node_id, kind));
                    }
                }
                CommandRef::RemoveNode { node_id } => {
                    if node_id == ROOT_NODE_ID {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    let mut parent_to_resync: Option<NodeId> = None;
                    if let Some(removed) = self.nodes.remove(node_id) {
                        if let Some(pid) = removed.parent
                            && let Some(pnode) = self.nodes.get_mut(pid)
                        {
                            pnode.children.retain(|&c| c != node_id);
                            parent_to_resync = Some(pid);
                        }
                        for cid in removed.children {
                            if let Some(cnode) = self.nodes.get_mut(cid)
                                && cnode.parent == Some(node_id)
                            {
                                cnode.parent = None;
                            }
                        }
                        if self.has_nested_text {
                            self.text_style_dirty = true;
                        }
                    }
                    if let Some(pid) = parent_to_resync
                        && let Some(pnode) = self.nodes.get(pid)
                    {
                        self.layout.set_children(pid, &pnode.children);
                    }
                    self.layout.remove_node(node_id);
                    self.prev_rects.remove(node_id);
                    if track_mutations {
                        self.last_mutations.push(HeadlessMutation::Remove(node_id));
                    }
                }
                CommandRef::SetChildren {
                    parent_id,
                    children,
                } => {
                    let Some((parent_kind, parent_parent)) =
                        self.nodes.get(parent_id).map(|n| (n.kind, n.parent))
                    else {
                        return Err(BackendError::InvalidNode(parent_id));
                    };
                    // Reject self-parenting, root-as-child, cycles, and duplicate children.
                    let anc_gen = if parent_parent.is_some() {
                        let g = self.visit_stamps.next_gen();
                        let mut walk = parent_parent;
                        for _ in 0..=self.nodes.len() {
                            let Some(cur) = walk else { break };
                            if !self.visit_stamps.mark(cur, g) {
                                break;
                            }
                            walk = self.nodes.get(cur).and_then(|n| n.parent);
                        }
                        Some(g)
                    } else {
                        None
                    };

                    let check_dups = children.len() > 1;
                    let child_gen = if check_dups {
                        Some(self.visit_stamps.next_gen())
                    } else {
                        None
                    };
                    self.scratch_children.clear();
                    self.scratch_children.reserve(children.len());
                    for cid in children {
                        if cid == parent_id
                            || cid == ROOT_NODE_ID
                            || anc_gen.is_some_and(|g| self.visit_stamps.has(cid, g))
                        {
                            return Err(BackendError::Internal(format!(
                                "cyclic or root child {cid} under parent {parent_id}"
                            )));
                        }
                        if let Some(g) = child_gen
                            && !self.visit_stamps.mark(cid, g)
                        {
                            return Err(BackendError::Internal(format!(
                                "duplicate child {cid} under parent {parent_id}"
                            )));
                        }
                        self.scratch_children.push(cid);
                    }

                    self.scratch_old_children.clear();
                    if let Some(parent) = self.nodes.get_mut(parent_id)
                        && !parent.children.is_empty()
                    {
                        self.scratch_old_children.extend_from_slice(&parent.children);
                    }
                    for &old_cid in &self.scratch_old_children {
                        if let Some(cnode) = self.nodes.get_mut(old_cid)
                            && cnode.parent == Some(parent_id)
                        {
                            cnode.parent = None;
                        }
                    }

                    self.scratch_prev_parents.clear();
                    for &cid in &self.scratch_children {
                        if let Some(cnode) = self.nodes.get_mut(cid) {
                            let prev_parent = cnode.parent;
                            cnode.parent = Some(parent_id);
                            if let Some(prev_pid) = prev_parent
                                && prev_pid != parent_id
                            {
                                if let Some(prev_pnode) = self.nodes.get_mut(prev_pid) {
                                    prev_pnode.children.retain(|&c| c != cid);
                                }
                                self.scratch_prev_parents.push(prev_pid);
                            }
                        }
                    }

                    if let Some(parent) = self.nodes.get_mut(parent_id) {
                        parent.children.clear();
                        parent.children.extend_from_slice(&self.scratch_children);
                    }

                    for &prev_pid in &self.scratch_prev_parents {
                        if let Some(pnode) = self.nodes.get(prev_pid) {
                            self.layout.set_children(prev_pid, &pnode.children);
                        }
                    }
                    self.layout.set_children(parent_id, &self.scratch_children);

                    if (parent_kind == ComponentKind::Text && !self.scratch_children.is_empty())
                        || self.has_nested_text
                    {
                        if parent_kind == ComponentKind::Text && !self.scratch_children.is_empty() {
                            self.has_nested_text = true;
                        }
                        self.text_style_dirty = true;
                    }
                }
                CommandRef::SetText { node_id, text } => {
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    if node.text.as_deref() != Some(text) {
                        node.text = Some(Arc::from(text));
                    }
                    if track_mutations {
                        self.last_mutations
                            .push(HeadlessMutation::UpdateText(node_id, text.to_owned()));
                    }
                    self.layout.set_text(node_id, text);
                }
                CommandRef::SetLayout { node_id, layout } => {
                    validate_layout(&layout)?;
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    node.layout = layout;
                    self.layout.set_layout(node_id, &layout);
                }
                CommandRef::SetStyle { node_id, style } => {
                    validate_style(&style)?;
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    node.style = style;
                    node.has_explicit_style = true;
                    if node.kind == ComponentKind::Text && self.has_nested_text {
                        self.text_style_dirty = true;
                    }
                    if track_mutations {
                        self.last_mutations
                            .push(HeadlessMutation::UpdateStyle(node_id));
                    }
                    self.layout.set_style(node_id, &style);
                }
                CommandRef::SetRawProp {
                    node_id,
                    key,
                    value,
                } => {
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    if let Some(existing) = node.raw_props.get_mut(key) {
                        *existing = value.to_owned();
                    } else {
                        node.raw_props.insert(key.to_owned(), value.to_owned());
                    }
                }
                CommandRef::SetEventMask { node_id, mask } => {
                    let Some(node) = self.nodes.get_mut(node_id) else {
                        return Err(BackendError::InvalidNode(node_id));
                    };
                    node.event_mask = mask;
                }
                CommandRef::DispatchCommand {
                    node_id,
                    command_name,
                    payload,
                } => {
                    self.dispatched_commands
                        .push((node_id, command_name.to_owned(), payload.to_vec()));
                }
                CommandRef::Commit { .. } => {
                    committed = true;
                }
            }
            Ok::<(), BackendError>(())
        })?;

        let decode_us = t_decode.elapsed().as_micros() as u64;
        let (layout_us, diff_us) = if committed {
            self.run_layout_and_diff(true)
        } else {
            (0, 0)
        };

        let mutation_count = self.last_mutations.len() as u32;

        Ok(BackendApplyStats {
            op_count: header.op_count,
            mutation_count,
            decode_us,
            layout_us,
            diff_us,
            mount_us: 0,
        })
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        std::mem::take(&mut self.pending_events)
    }

    #[inline]
    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.nodes.get(node_id).map(|n| n.computed_rect)
    }
}

#[cfg(test)]
mod tests;
