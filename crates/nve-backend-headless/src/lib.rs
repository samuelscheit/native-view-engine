//! Pure-Rust headless RenderBackend generic over any [`LayoutEngine`] (defaulting to [`TaffyLayoutEngine`]).
//!
//! Provides <50ms edit->test verification of any Rust UI frontend, mutation diffing,
//! coordinate hit-testing, simulated native events, live layout-engine swapping, and an
//! ASCII tree + rect inspector.

pub mod taffy_engine;

<<<<<<< HEAD
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nve_abi::{
    Command, CommandDecoder, ComponentKind, LayoutProps, NativeEvent, NodeId, RawPropValue,
    StyleProps, SurfaceId, ROOT_NODE_ID,
=======
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use nve_abi::{
    ColorRgba, Command, CommandDecoder, ComponentKind, Dimension, FlexAlign, FlexDirection,
    FlexJustify, LayoutProps, NativeEvent, NodeId, RawPropValue, StyleProps, SurfaceId,
    ROOT_NODE_ID,
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
};
use nve_engine::{
    BackendApplyStats, BackendError, HeuristicTextMeasurer, LayoutEngine, LayoutRect,
    RenderBackend, TextMeasurer, Viewport,
};

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
        Err(BackendError::Internal("invalid non-finite or out-of-range layout scalar".into()))
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
        Err(BackendError::Internal("invalid non-finite or out-of-range style scalar".into()))
    }
}

#[derive(Debug, Clone)]
pub struct HeadlessNode {
    pub id: NodeId,
    pub kind: ComponentKind,
    pub custom_name: Option<String>,
    pub text: Option<String>,
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

pub struct HeadlessBackend<L: LayoutEngine = TaffyLayoutEngine> {
    surface_id: SurfaceId,
    viewport: Viewport,
    nodes: HashMap<NodeId, HeadlessNode>,
    layout: L,
    text_measurer: Arc<dyn TextMeasurer>,
    prev_rects: HashMap<NodeId, LayoutRect>,
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
            nodes: HashMap::new(),
            layout,
            text_measurer,
            prev_rects: HashMap::new(),
            last_mutations: Vec::new(),
            pending_events: Vec::new(),
            dispatched_commands: Vec::new(),
        };
        this.ensure_node(ROOT_NODE_ID, ComponentKind::Root, None);
        this
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
            if let Some(text) = &node.text {
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
            last_mutations: self.last_mutations,
            pending_events: self.pending_events,
            dispatched_commands: self.dispatched_commands,
        };
        swapped.recompute_layout();
        swapped
    }

    /// Re-runs the active layout engine against the retained tree and refreshes all `computed_rect`s.
    pub fn recompute_layout(&mut self) {
        let mut scratch = Vec::new();
        self.run_layout_and_diff(&mut scratch);
    }

    pub fn nodes(&self) -> &HashMap<NodeId, HeadlessNode> {
        &self.nodes
    }

    pub fn node(&self, id: NodeId) -> Option<&HeadlessNode> {
        self.nodes.get(&id)
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
        let node = self.nodes.get(&id)?;
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
        let Some(node) = self.nodes.get(&id) else {
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
        if let Some(text) = &node.text {
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

    fn ensure_node(&mut self, id: NodeId, kind: ComponentKind, custom_name: Option<String>) {
        if let Some(existing) = self.nodes.get_mut(&id) {
            existing.kind = kind;
            existing.custom_name = custom_name;
            self.layout.upsert_node(id, kind);
            return;
        }
        let default_layout = LayoutProps::default();
        let default_style = StyleProps::default();
        self.layout.upsert_node(id, kind);
        self.layout.set_layout(id, &default_layout);
        self.layout.set_style(id, &default_style);
        self.nodes.insert(
            id,
            HeadlessNode {
                id,
                kind,
                custom_name,
                text: None,
<<<<<<< HEAD
                layout: default_layout,
                style: default_style,
=======
                layout: LayoutProps::default(),
                style: StyleProps::default(),
                has_explicit_style: false,
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
                raw_props: HashMap::new(),
                event_mask: 0,
                parent: None,
                children: Vec::new(),
                computed_rect: LayoutRect::default(),
            },
        );
    }

<<<<<<< HEAD
    fn run_layout_and_diff(&mut self, mutations: &mut Vec<HeadlessMutation>) -> (u64, u64) {
        let t_layout = Instant::now();
        self.layout.compute_layout(ROOT_NODE_ID, self.viewport);
=======
    fn available_wrap_width(&self, id: NodeId) -> f32 {
        let mut cur = self.nodes.get(&id).and_then(|n| n.parent);
        let mut hops = 0usize;
        while let Some(pid) = cur {
            if hops > MAX_TREE_DEPTH {
                break;
            }
            hops += 1;
            let Some(pnode) = self.nodes.get(&pid) else {
                break;
            };
            let pad_h = (pnode.layout.padding.left + pnode.layout.padding.right).max(0.0);
            match pnode.layout.width {
                Dimension::Points(w) if w > 0.0 => return (w - pad_h).max(1.0),
                Dimension::Percent(pct) if pct > 0.0 => {
                    return (self.viewport.width * (pct / 100.0) - pad_h).max(1.0);
                }
                _ => cur = pnode.parent,
            }
        }
        self.viewport.width.max(1.0)
    }

    fn sync_taffy_style(&mut self, id: NodeId) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        let Some(&tid) = self.taffy_ids.get(&id) else {
            return;
        };
        let mut style = to_taffy_style(&node.layout);
        if id == ROOT_NODE_ID {
            style.size = tf::Size {
                width: tf::Dimension::length(self.viewport.width),
                height: tf::Dimension::length(self.viewport.height),
            };
        } else if node.kind == ComponentKind::Text && node.children.is_empty() {
            // Deterministic intrinsic text sizing with multi-line wrapping estimate.
            if let Some(text) = &node.text {
                let font_size = if node.style.font_size > 0.0 {
                    node.style.font_size
                } else {
                    16.0
                };
                let char_w = font_size * 0.58;
                let line_h = font_size * 1.35;
                let unwrapped_w = if text.is_empty() {
                    0.0
                } else {
                    (text.chars().count() as f32 * char_w).max(1.0)
                };
                let max_w = match node.layout.width {
                    Dimension::Points(w) if w > 0.0 => w,
                    Dimension::Percent(p) if p > 0.0 => (self.viewport.width * (p / 100.0)).max(1.0),
                    _ => self.available_wrap_width(id),
                };
                let eff_w = unwrapped_w.min(max_w);
                if matches!(node.layout.width, Dimension::Auto) {
                    style.size.width = tf::Dimension::length(eff_w);
                }
                if matches!(node.layout.height, Dimension::Auto) {
                    let lines = if eff_w > 0.0 && unwrapped_w > eff_w {
                        (unwrapped_w / eff_w).ceil()
                    } else if text.is_empty() {
                        0.0
                    } else {
                        1.0
                    };
                    style.size.height = tf::Dimension::length(lines * line_h);
                }
            }
        }
        let _ = self.taffy.set_style(tid, style);
    }

    fn propagate_text_styles(&mut self) {
        struct Frame {
            id: NodeId,
            color: ColorRgba,
            font_size: f32,
            has_inherited: bool,
        }
        let mut visited = HashSet::with_capacity(self.nodes.len());
        let mut stack = vec![Frame {
            id: ROOT_NODE_ID,
            color: ColorRgba::BLACK,
            font_size: 16.0,
            has_inherited: false,
        }];
        let mut resync_ids = Vec::new();

        while let Some(frame) = stack.pop() {
            if !visited.insert(frame.id) {
                continue;
            }
            let Some(node) = self.nodes.get_mut(&frame.id) else {
                continue;
            };
            let mut next_color = frame.color;
            let mut next_font_size = frame.font_size;
            let mut next_has = frame.has_inherited;

            if node.kind == ComponentKind::Text {
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
                        node.style.text_color = target_color;
                        node.style.font_size = target_font;
                        resync_ids.push(frame.id);
                    }
                }
                next_color = node.style.text_color;
                next_font_size = node.style.font_size;
                next_has = true;
            }

            let children = node.children.clone();
            for cid in children.into_iter().rev() {
                stack.push(Frame {
                    id: cid,
                    color: next_color,
                    font_size: next_font_size,
                    has_inherited: next_has,
                });
            }
        }

        for id in resync_ids {
            self.sync_taffy_style(id);
        }
    }

    fn run_layout_and_diff(&mut self, mutations: &mut Vec<HeadlessMutation>) -> (u64, u64) {
        let t_layout = Instant::now();
        self.propagate_text_styles();
        // Re-sync leaf text nodes in case ancestor widths or viewport changed.
        let leaf_text_ids: Vec<NodeId> = self
            .nodes
            .values()
            .filter(|n| n.kind == ComponentKind::Text && n.children.is_empty() && n.text.is_some())
            .map(|n| n.id)
            .collect();
        for tid in leaf_text_ids {
            self.sync_taffy_style(tid);
        }
        self.sync_taffy_style(ROOT_NODE_ID);
        if let Some(&root_tid) = self.taffy_ids.get(&ROOT_NODE_ID) {
            let avail = tf::Size {
                width: tf::AvailableSpace::Definite(self.viewport.width),
                height: tf::AvailableSpace::Definite(self.viewport.height),
            };
            let _ = self.taffy.compute_layout(root_tid, avail);
        }
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
        let layout_us = t_layout.elapsed().as_micros() as u64;

        let t_diff = Instant::now();
        let mut new_rects = HashMap::with_capacity(self.nodes.len());
<<<<<<< HEAD
        self.collect_rects(ROOT_NODE_ID, &mut new_rects);
=======
        self.collect_rects(ROOT_NODE_ID, 0.0, 0.0, 0, &mut new_rects);
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
        for (&id, &rect) in &new_rects {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.computed_rect = rect;
            }
            if self.prev_rects.get(&id) != Some(&rect) {
                mutations.push(HeadlessMutation::UpdateLayout(id, rect));
            }
        }
        self.prev_rects = new_rects;
        let diff_us = t_diff.elapsed().as_micros() as u64;
        (layout_us, diff_us)
    }

<<<<<<< HEAD
    fn collect_rects(&self, id: NodeId, out: &mut HashMap<NodeId, LayoutRect>) {
        let Some(rect) = self.layout.layout_of(id) else {
=======
    fn collect_rects(
        &self,
        id: NodeId,
        origin_x: f32,
        origin_y: f32,
        depth: usize,
        out: &mut HashMap<NodeId, LayoutRect>,
    ) {
        if depth > MAX_TREE_DEPTH {
            return;
        }
        let Some(&tid) = self.taffy_ids.get(&id) else {
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
            return;
        };
        out.insert(id, rect);
        if let Some(node) = self.nodes.get(&id) {
            for &child in &node.children {
<<<<<<< HEAD
                self.collect_rects(child, out);
=======
                self.collect_rects(child, rect.x, rect.y, depth + 1, out);
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
            }
        }
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
        let (header, commands) = CommandDecoder::new(command_bytes)?.decode_all()?;
        let mut mutations = Vec::new();
        let mut committed = false;

        for cmd in commands {
            match cmd {
                Command::CreateNode {
                    node_id,
                    kind,
                    custom_name,
                } => {
                    self.ensure_node(node_id, kind, custom_name);
                    mutations.push(HeadlessMutation::Create(node_id, kind));
                }
                Command::RemoveNode { node_id } => {
<<<<<<< HEAD
                    self.nodes.remove(&node_id);
                    self.layout.remove_node(node_id);
=======
                    if node_id == ROOT_NODE_ID {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    if let Some(removed) = self.nodes.remove(&node_id) {
                        if let Some(pid) = removed.parent {
                            if let Some(pnode) = self.nodes.get_mut(&pid) {
                                pnode.children.retain(|&c| c != node_id);
                            }
                        }
                        for cid in removed.children {
                            if let Some(cnode) = self.nodes.get_mut(&cid) {
                                if cnode.parent == Some(node_id) {
                                    cnode.parent = None;
                                }
                            }
                        }
                    }
                    if let Some(tid) = self.taffy_ids.remove(&node_id) {
                        let _ = self.taffy.remove(tid);
                    }
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
                    self.prev_rects.remove(&node_id);
                    mutations.push(HeadlessMutation::Remove(node_id));
                }
                Command::SetChildren {
                    parent_id,
                    children,
                } => {
                    if !self.nodes.contains_key(&parent_id) {
                        return Err(BackendError::InvalidNode(parent_id));
                    }
                    // Reject self-parenting, root-as-child, cycles, and duplicate children.
                    let mut ancestors = HashSet::new();
                    let mut walk = Some(parent_id);
                    for _ in 0..=self.nodes.len() {
                        let Some(cur) = walk else { break };
                        if !ancestors.insert(cur) {
                            break;
                        }
                        walk = self.nodes.get(&cur).and_then(|n| n.parent);
                    }
                    let mut seen_children = HashSet::with_capacity(children.len());
                    for &cid in &children {
                        if cid == parent_id || cid == ROOT_NODE_ID || ancestors.contains(&cid) {
                            return Err(BackendError::Internal(format!(
                                "cyclic or root child {cid} under parent {parent_id}"
                            )));
                        }
                        if !seen_children.insert(cid) {
                            return Err(BackendError::Internal(format!(
                                "duplicate child {cid} under parent {parent_id}"
                            )));
                        }
                    }

                    let old_children = self
                        .nodes
                        .get(&parent_id)
                        .map(|p| p.children.clone())
                        .unwrap_or_default();
                    for old_cid in old_children {
                        if let Some(cnode) = self.nodes.get_mut(&old_cid) {
                            if cnode.parent == Some(parent_id) {
                                cnode.parent = None;
                            }
                        }
                    }
                    let mut prev_parents_to_resync = Vec::new();
                    for &cid in &children {
                        let prev_parent = self.nodes.get(&cid).and_then(|n| n.parent);
                        if let Some(prev_pid) = prev_parent {
                            if prev_pid != parent_id {
                                if let Some(prev_pnode) = self.nodes.get_mut(&prev_pid) {
                                    prev_pnode.children.retain(|&c| c != cid);
                                }
                                prev_parents_to_resync.push(prev_pid);
                            }
                        }
                        if let Some(cnode) = self.nodes.get_mut(&cid) {
                            cnode.parent = Some(parent_id);
                        }
                    }
                    if let Some(parent) = self.nodes.get_mut(&parent_id) {
                        parent.children = children.clone();
                    }
                    for prev_pid in prev_parents_to_resync {
                        if let (Some(&ptid), Some(pnode)) =
                            (self.taffy_ids.get(&prev_pid), self.nodes.get(&prev_pid))
                        {
                            let ctids: Vec<tf::NodeId> = pnode
                                .children
                                .iter()
                                .filter_map(|c| self.taffy_ids.get(c).copied())
                                .collect();
                            let _ = self.taffy.set_children(ptid, &ctids);
                        }
                    }
<<<<<<< HEAD
                    self.layout.set_children(parent_id, &children);
=======
                    if let Some(&parent_tid) = self.taffy_ids.get(&parent_id) {
                        let child_tids: Vec<tf::NodeId> = children
                            .iter()
                            .filter_map(|cid| self.taffy_ids.get(cid).copied())
                            .collect();
                        let _ = self.taffy.set_children(parent_tid, &child_tids);
                    }
                    self.sync_taffy_style(parent_id);
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
                }
                Command::SetText { node_id, text } => {
                    if let Some(node) = self.nodes.get_mut(&node_id) {
                        node.text = Some(text.clone());
                        mutations.push(HeadlessMutation::UpdateText(node_id, text.clone()));
                    } else {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    self.layout.set_text(node_id, &text);
                }
                Command::SetLayout { node_id, layout } => {
                    validate_layout(&layout)?;
                    if let Some(node) = self.nodes.get_mut(&node_id) {
                        node.layout = layout;
                    } else {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    self.layout.set_layout(node_id, &layout);
                }
                Command::SetStyle { node_id, style } => {
                    validate_style(&style)?;
                    if let Some(node) = self.nodes.get_mut(&node_id) {
                        node.style = style;
                        node.has_explicit_style = true;
                        mutations.push(HeadlessMutation::UpdateStyle(node_id));
                    } else {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    self.layout.set_style(node_id, &style);
                }
                Command::SetRawProp {
                    node_id,
                    key,
                    value,
                } => {
                    if let Some(node) = self.nodes.get_mut(&node_id) {
                        node.raw_props.insert(key, value);
                    } else {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                }
                Command::SetEventMask { node_id, mask } => {
                    if let Some(node) = self.nodes.get_mut(&node_id) {
                        node.event_mask = mask;
                    } else {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                }
                Command::DispatchCommand {
                    node_id,
                    command_name,
                    payload,
                } => {
                    self.dispatched_commands
                        .push((node_id, command_name, payload));
                }
                Command::Commit { .. } => {
                    committed = true;
                }
            }
        }

        let decode_us = t_decode.elapsed().as_micros() as u64;
        let (layout_us, diff_us) = if committed {
            self.run_layout_and_diff(&mut mutations)
        } else {
            (0, 0)
        };

        let mutation_count = mutations.len() as u32;
        self.last_mutations = mutations;

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

    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.nodes.get(&node_id).map(|n| n.computed_rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{CommandBufferBuilder, Dimension, EdgeInsets, EventMask, FlexAlign, FlexDirection};
    use nve_engine::DynLayoutEngine;

    /// Minimal custom [`LayoutEngine`] stacking every child vertically in fixed-height bands.
    struct BandStackLayout {
        band_height: f32,
        children: HashMap<NodeId, Vec<NodeId>>,
        texts: HashMap<NodeId, String>,
        font_sizes: HashMap<NodeId, f32>,
        kinds: HashMap<NodeId, ComponentKind>,
        rects: HashMap<NodeId, LayoutRect>,
        measurer: Arc<dyn TextMeasurer>,
    }

    impl BandStackLayout {
        fn new(band_height: f32) -> Self {
            Self {
                band_height,
                children: HashMap::new(),
                texts: HashMap::new(),
                font_sizes: HashMap::new(),
                kinds: HashMap::new(),
                rects: HashMap::new(),
                measurer: Arc::new(HeuristicTextMeasurer),
            }
        }

        fn layout_rec(&mut self, id: NodeId, x: f32, y: f32, width: f32) -> f32 {
            let is_text = self.kinds.get(&id) == Some(&ComponentKind::Text);
            let (node_w, node_h) = if is_text {
                let text = self.texts.get(&id).map(String::as_str).unwrap_or("");
                let fs = self.font_sizes.get(&id).copied().unwrap_or(16.0);
                self.measurer.measure(text, fs, Some(width))
            } else {
                (width, self.band_height)
            };

            let child_ids = self.children.get(&id).cloned().unwrap_or_default();
            let mut cursor_y = y;
            for child in child_ids {
                cursor_y += self.layout_rec(child, x + 10.0, cursor_y, (width - 20.0).max(0.0));
            }
            let total_h = if cursor_y > y { cursor_y - y } else { node_h };
            self.rects.insert(
                id,
                LayoutRect {
                    x,
                    y,
                    width: node_w,
                    height: total_h,
                },
            );
            total_h
        }
    }

    impl LayoutEngine for BandStackLayout {
        fn name(&self) -> &'static str {
            "band-stack"
        }

        fn upsert_node(&mut self, id: NodeId, kind: ComponentKind) {
            self.kinds.insert(id, kind);
        }

        fn remove_node(&mut self, id: NodeId) {
            self.kinds.remove(&id);
            self.children.remove(&id);
            self.texts.remove(&id);
            self.font_sizes.remove(&id);
            self.rects.remove(&id);
        }

        fn set_layout(&mut self, _id: NodeId, _layout: &LayoutProps) {}

        fn set_style(&mut self, id: NodeId, style: &StyleProps) {
            self.font_sizes.insert(id, style.font_size);
        }

        fn set_text(&mut self, id: NodeId, text: &str) {
            self.texts.insert(id, text.to_owned());
        }

        fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
            self.children.insert(parent, children.to_vec());
        }

        fn set_text_measurer(&mut self, measurer: Arc<dyn TextMeasurer>) {
            self.measurer = measurer;
        }

        fn compute_layout(&mut self, root: NodeId, viewport: Viewport) {
            self.rects.clear();
            let _ = self.layout_rec(root, 0.0, 0.0, viewport.width);
            if let Some(r) = self.rects.get_mut(&root) {
                r.width = viewport.width;
                r.height = viewport.height;
            }
        }

        fn layout_of(&self, id: NodeId) -> Option<LayoutRect> {
            self.rects.get(&id).copied()
        }

        fn reset(&mut self) {
            self.children.clear();
            self.texts.clear();
            self.font_sizes.clear();
            self.kinds.clear();
            self.rects.clear();
        }
    }

    fn build_sample_frame() -> Vec<u8> {
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Points(120.0),
                flex_direction: FlexDirection::Row,
                align_items: FlexAlign::FlexStart,
                padding: EdgeInsets::all(16.0),
                gap: 12.0,
                ..Default::default()
            },
        );
        b.create_node(3, ComponentKind::Pressable, None);
        b.set_layout(
            3,
            &LayoutProps {
                width: Dimension::Points(100.0),
                height: Dimension::Points(44.0),
                ..Default::default()
            },
        );
        b.set_event_mask(3, EventMask::PRESS);
        b.create_node(4, ComponentKind::Text, None);
        b.set_text(4, "Tap me");
        b.set_style(
            4,
            &StyleProps {
                font_size: 20.0,
                ..Default::default()
            },
        );
        b.set_children(3, &[4]);
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        b.into_bytes()
    }

    #[test]
    fn computes_flexbox_layout_and_hit_testing() {
        let mut backend = HeadlessBackend::new();
        backend.set_viewport(
            1,
            Viewport {
                width: 400.0,
                height: 800.0,
                scale_factor: 2.0,
            },
        );

        let stats = backend.submit_frame(&build_sample_frame()).unwrap();
        assert!(stats.mutation_count >= 4);
        assert_eq!(backend.name(), "headless-taffy");

        let btn = backend.layout_of(3).unwrap();
        assert_eq!(btn.x, 16.0);
        assert_eq!(btn.y, 16.0);
        assert_eq!(btn.width, 100.0);
        assert_eq!(btn.height, 44.0);

        assert_eq!(backend.hit_test(50.0, 30.0, true), Some(3));
        assert_eq!(backend.hit_test(350.0, 30.0, true), None);
    }

    #[test]
<<<<<<< HEAD
    fn supports_monomorphized_custom_layout_engine_and_live_swap() {
        let frame = build_sample_frame();
        let viewport = Viewport {
            width: 400.0,
            height: 800.0,
            scale_factor: 1.0,
        };

        // Start with Taffy
        let mut taffy_backend = HeadlessBackend::new();
        taffy_backend.set_viewport(1, viewport);
        taffy_backend.submit_frame(&frame).unwrap();
        assert_eq!(taffy_backend.layout_of(3).unwrap().x, 16.0);
        assert_eq!(taffy_backend.hit_test(50.0, 30.0, true), Some(3));

        // Live-swap to compile-time generic BandStackLayout: tree, styles, text, and hit-testing survive.
        let custom_backend: HeadlessBackend<BandStackLayout> =
            taffy_backend.swap_layout_engine(BandStackLayout::new(60.0));
        assert_eq!(custom_backend.name(), "headless-custom");
        assert_eq!(custom_backend.layout_engine().name(), "band-stack");
        assert_eq!(
            custom_backend.node(4).and_then(|n| n.text.as_deref()),
            Some("Tap me")
        );
        // BandStackLayout indents each nesting level by 10px: root(0) -> view2(10) -> btn3(20)
        let btn_custom = custom_backend.layout_of(3).unwrap();
        assert_eq!(btn_custom.x, 20.0);
        assert_eq!(btn_custom.y, 0.0);
        assert_eq!(custom_backend.hit_test(35.0, 10.0, true), Some(3));

        // Swap back to Taffy and verify original Flexbox geometry is restored identically.
        let restored: HeadlessBackend<TaffyLayoutEngine> =
            custom_backend.swap_layout_engine(TaffyLayoutEngine::new());
        let btn_restored = restored.layout_of(3).unwrap();
        assert_eq!(btn_restored.x, 16.0);
        assert_eq!(btn_restored.y, 16.0);
        assert_eq!(btn_restored.width, 100.0);
        assert_eq!(btn_restored.height, 44.0);
        assert_eq!(restored.hit_test(50.0, 30.0, true), Some(3));
    }

    #[test]
    fn supports_runtime_boxed_dyn_layout_engine() {
        let frame = build_sample_frame();
        let viewport = Viewport {
            width: 400.0,
            height: 800.0,
            scale_factor: 1.0,
        };

        let boxed: DynLayoutEngine = Box::new(TaffyLayoutEngine::new());
        let mut backend: HeadlessBackend<DynLayoutEngine> = HeadlessBackend::with_layout(boxed);
        backend.set_viewport(1, viewport);
        backend.submit_frame(&frame).unwrap();
        assert_eq!(backend.layout_engine().name(), "taffy");
        assert_eq!(backend.layout_of(3).unwrap().x, 16.0);

        // Swap inside the same concrete type HeadlessBackend<DynLayoutEngine>!
        let swapped: HeadlessBackend<DynLayoutEngine> =
            backend.swap_layout_engine(Box::new(BandStackLayout::new(50.0)) as DynLayoutEngine);
        assert_eq!(swapped.layout_engine().name(), "band-stack");
        assert_eq!(swapped.layout_of(3).unwrap().x, 20.0);
    }

    #[test]
    fn custom_text_measurer_propagates_into_taffy_and_survives_swap() {
=======
    fn rejects_cycles_duplicates_and_non_finite_scalars() {
        let mut backend = HeadlessBackend::new();

        // Self-cycle
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_children(2, &[2]);
        assert!(backend.submit_frame(b.finish()).is_err());

        // 2-node cycle
        let mut b2 = CommandBufferBuilder::new(1, 2);
        b2.create_node(3, ComponentKind::View, None);
        b2.create_node(4, ComponentKind::View, None);
        b2.set_children(3, &[4]);
        b2.set_children(4, &[3]);
        assert!(backend.submit_frame(b2.finish()).is_err());

        // Root as child
        let mut b3 = CommandBufferBuilder::new(1, 3);
        b3.set_children(2, &[ROOT_NODE_ID]);
        assert!(backend.submit_frame(b3.finish()).is_err());

        // Duplicate child
        let mut b4 = CommandBufferBuilder::new(1, 4);
        b4.create_node(5, ComponentKind::View, None);
        b4.set_children(2, &[5, 5]);
        assert!(backend.submit_frame(b4.finish()).is_err());

        // NaN layout scalar
        let mut b5 = CommandBufferBuilder::new(1, 5);
        b5.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Points(f32::NAN),
                ..Default::default()
            },
        );
        assert!(backend.submit_frame(b5.finish()).is_err());
    }

    #[test]
    fn inherits_text_styles_and_wraps_multiline_text() {
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
        let mut backend = HeadlessBackend::new();
        backend.set_viewport(
            1,
            Viewport {
<<<<<<< HEAD
                width: 300.0,
                height: 400.0,
                scale_factor: 1.0,
            },
        );
        // Custom measurer returning fixed 123x45 for any non-empty text.
        backend.set_text_measurer(Arc::new(|text: &str, _fs: f32, _mw: Option<f32>| {
            if text.is_empty() {
                (0.0, 0.0)
            } else {
                (123.0, 45.0)
            }
        }));
=======
                width: 400.0,
                height: 600.0,
                scale_factor: 2.0,
            },
        );
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)

        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
<<<<<<< HEAD
                width: Dimension::Points(260.0),
                flex_direction: FlexDirection::Column,
                align_items: FlexAlign::FlexStart,
=======
                width: Dimension::Points(100.0),
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
                ..Default::default()
            },
        );
        b.create_node(3, ComponentKind::Text, None);
<<<<<<< HEAD
        b.set_text(3, "Measured");
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);

        backend.submit_frame(b.finish()).unwrap();
        let rect = backend.layout_of(3).unwrap();
        assert_eq!(rect.width, 123.0);
        assert_eq!(rect.height, 45.0);

        // Swapping engines preserves the custom TextMeasurer!
        let swapped = backend.swap_layout_engine(BandStackLayout::new(40.0));
        let swapped_text_rect = swapped.layout_of(3).unwrap();
        assert_eq!(swapped_text_rect.width, 123.0);
        assert_eq!(swapped_text_rect.height, 45.0);
=======
        b.set_style(
            3,
            &StyleProps {
                text_color: ColorRgba::rgb(56, 189, 248),
                font_size: 20.0,
                ..Default::default()
            },
        );
        b.create_node(4, ComponentKind::Text, None);
        b.set_text(4, "This is a long line that must wrap across multiple lines");
        b.set_children(ROOT_NODE_ID, &[2]);
        b.set_children(2, &[3]);
        b.set_children(3, &[4]);
        b.commit(true);

        backend.submit_frame(b.finish()).unwrap();
        let leaf = backend.node(4).unwrap();
        assert_eq!(leaf.style.text_color, ColorRgba::rgb(56, 189, 248));
        assert!((leaf.style.font_size - 20.0).abs() < 1e-3);
        // Wrapped in a 100pt box at 20pt font -> multiple lines (height > 2 * 27pt)
        assert!(leaf.computed_rect.width <= 100.5);
        assert!(leaf.computed_rect.height > 54.0);
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
    }
}
