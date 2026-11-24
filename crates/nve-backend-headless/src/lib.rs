//! Pure-Rust headless RenderBackend powered by `taffy` Flexbox layout.
//!
//! Provides <50ms edit->test verification of any Rust UI frontend, mutation diffing,
//! coordinate hit-testing, simulated native events, and an ASCII tree + rect inspector.

use std::collections::HashMap;
use std::time::Instant;

use nve_abi::{
    Command, CommandDecoder, ComponentKind, Dimension, FlexAlign, FlexDirection, FlexJustify,
    LayoutProps, NativeEvent, NodeId, RawPropValue, StyleProps, SurfaceId, ROOT_NODE_ID,
};
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, RenderBackend, Viewport};
use taffy::prelude as tf;

#[derive(Debug, Clone)]
pub struct HeadlessNode {
    pub id: NodeId,
    pub kind: ComponentKind,
    pub custom_name: Option<String>,
    pub text: Option<String>,
    pub layout: LayoutProps,
    pub style: StyleProps,
    pub raw_props: HashMap<String, RawPropValue>,
    pub event_mask: u64,
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

pub struct HeadlessBackend {
    surface_id: SurfaceId,
    viewport: Viewport,
    nodes: HashMap<NodeId, HeadlessNode>,
    taffy: tf::TaffyTree<()>,
    taffy_ids: HashMap<NodeId, tf::NodeId>,
    prev_rects: HashMap<NodeId, LayoutRect>,
    last_mutations: Vec<HeadlessMutation>,
    pending_events: Vec<NativeEvent>,
    dispatched_commands: Vec<(NodeId, String, Vec<u8>)>,
}

impl Default for HeadlessBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessBackend {
    pub fn new() -> Self {
        let mut this = Self {
            surface_id: 1,
            viewport: Viewport::default(),
            nodes: HashMap::new(),
            taffy: tf::TaffyTree::new(),
            taffy_ids: HashMap::new(),
            prev_rects: HashMap::new(),
            last_mutations: Vec::new(),
            pending_events: Vec::new(),
            dispatched_commands: Vec::new(),
        };
        this.ensure_node(ROOT_NODE_ID, ComponentKind::Root, None);
        this
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
        self.hit_test_rec(ROOT_NODE_ID, x, y, interactive_only)
    }

    fn hit_test_rec(&self, id: NodeId, x: f32, y: f32, interactive_only: bool) -> Option<NodeId> {
        let node = self.nodes.get(&id)?;
        let r = node.computed_rect;
        let inside = x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height;
        if !inside && id != ROOT_NODE_ID {
            return None;
        }
        for &child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_rec(child, x, y, interactive_only) {
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
            return;
        }
        let taffy_id = self
            .taffy
            .new_leaf(tf::Style::default())
            .expect("taffy leaf allocation");
        self.taffy_ids.insert(id, taffy_id);
        self.nodes.insert(
            id,
            HeadlessNode {
                id,
                kind,
                custom_name,
                text: None,
                layout: LayoutProps::default(),
                style: StyleProps::default(),
                raw_props: HashMap::new(),
                event_mask: 0,
                children: Vec::new(),
                computed_rect: LayoutRect::default(),
            },
        );
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
        } else if node.kind == ComponentKind::Text {
            // Deterministic intrinsic text sizing for headless layout verification.
            if let Some(text) = &node.text {
                if matches!(node.layout.width, Dimension::Auto) {
                    let approx_w = (text.chars().count() as f32 * node.style.font_size * 0.58)
                        .min(self.viewport.width);
                    style.size.width = tf::Dimension::length(approx_w.max(1.0));
                }
                if matches!(node.layout.height, Dimension::Auto) {
                    style.size.height = tf::Dimension::length(node.style.font_size * 1.35);
                }
            }
        }
        let _ = self.taffy.set_style(tid, style);
    }

    fn run_layout_and_diff(&mut self, mutations: &mut Vec<HeadlessMutation>) -> (u64, u64) {
        let t_layout = Instant::now();
        self.sync_taffy_style(ROOT_NODE_ID);
        if let Some(&root_tid) = self.taffy_ids.get(&ROOT_NODE_ID) {
            let avail = tf::Size {
                width: tf::AvailableSpace::Definite(self.viewport.width),
                height: tf::AvailableSpace::Definite(self.viewport.height),
            };
            let _ = self.taffy.compute_layout(root_tid, avail);
        }
        let layout_us = t_layout.elapsed().as_micros() as u64;

        let t_diff = Instant::now();
        let mut new_rects = HashMap::with_capacity(self.nodes.len());
        self.collect_rects(ROOT_NODE_ID, 0.0, 0.0, &mut new_rects);
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

    fn collect_rects(
        &self,
        id: NodeId,
        origin_x: f32,
        origin_y: f32,
        out: &mut HashMap<NodeId, LayoutRect>,
    ) {
        let Some(&tid) = self.taffy_ids.get(&id) else {
            return;
        };
        let Ok(layout) = self.taffy.layout(tid) else {
            return;
        };
        let rect = LayoutRect {
            x: origin_x + layout.location.x,
            y: origin_y + layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        };
        out.insert(id, rect);
        if let Some(node) = self.nodes.get(&id) {
            for &child in &node.children {
                self.collect_rects(child, rect.x, rect.y, out);
            }
        }
    }
}

impl RenderBackend for HeadlessBackend {
    fn name(&self) -> &'static str {
        "headless-taffy"
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        self.surface_id = surface_id;
        self.viewport = viewport;
        self.sync_taffy_style(ROOT_NODE_ID);
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
                    self.nodes.remove(&node_id);
                    if let Some(tid) = self.taffy_ids.remove(&node_id) {
                        let _ = self.taffy.remove(tid);
                    }
                    self.prev_rects.remove(&node_id);
                    mutations.push(HeadlessMutation::Remove(node_id));
                }
                Command::SetChildren {
                    parent_id,
                    children,
                } => {
                    if let Some(parent) = self.nodes.get_mut(&parent_id) {
                        parent.children = children.clone();
                    } else {
                        return Err(BackendError::InvalidNode(parent_id));
                    }
                    if let Some(&parent_tid) = self.taffy_ids.get(&parent_id) {
                        let child_tids: Vec<tf::NodeId> = children
                            .iter()
                            .filter_map(|cid| self.taffy_ids.get(cid).copied())
                            .collect();
                        let _ = self.taffy.set_children(parent_tid, &child_tids);
                    }
                }
                Command::SetText { node_id, text } => {
                    if let Some(node) = self.nodes.get_mut(&node_id) {
                        node.text = Some(text.clone());
                        mutations.push(HeadlessMutation::UpdateText(node_id, text));
                    } else {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    self.sync_taffy_style(node_id);
                }
                Command::SetLayout { node_id, layout } => {
                    if let Some(node) = self.nodes.get_mut(&node_id) {
                        node.layout = layout;
                    } else {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    self.sync_taffy_style(node_id);
                }
                Command::SetStyle { node_id, style } => {
                    if let Some(node) = self.nodes.get_mut(&node_id) {
                        node.style = style;
                        mutations.push(HeadlessMutation::UpdateStyle(node_id));
                    } else {
                        return Err(BackendError::InvalidNode(node_id));
                    }
                    self.sync_taffy_style(node_id);
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

pub fn to_taffy_dim(d: Dimension) -> tf::Dimension {
    match d {
        Dimension::Auto => tf::Dimension::auto(),
        Dimension::Points(v) => tf::Dimension::length(v),
        Dimension::Percent(v) => tf::Dimension::percent(v / 100.0),
    }
}

pub fn to_taffy_lpa(d: Dimension) -> tf::LengthPercentageAuto {
    match d {
        Dimension::Auto => tf::LengthPercentageAuto::auto(),
        Dimension::Points(v) => tf::LengthPercentageAuto::length(v),
        Dimension::Percent(v) => tf::LengthPercentageAuto::percent(v / 100.0),
    }
}

pub fn to_taffy_style(l: &LayoutProps) -> tf::Style {
    use taffy::style::{
        AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignmentSafety,
    };
    let justify_kw = match l.justify_content {
        FlexJustify::FlexStart => AlignContentKeyword::FlexStart,
        FlexJustify::Center => AlignContentKeyword::Center,
        FlexJustify::FlexEnd => AlignContentKeyword::FlexEnd,
        FlexJustify::SpaceBetween => AlignContentKeyword::SpaceBetween,
        FlexJustify::SpaceAround => AlignContentKeyword::SpaceAround,
        FlexJustify::SpaceEvenly => AlignContentKeyword::SpaceEvenly,
    };
    let align_kw = match l.align_items {
        FlexAlign::Stretch => AlignItemsKeyword::Stretch,
        FlexAlign::FlexStart => AlignItemsKeyword::FlexStart,
        FlexAlign::Center => AlignItemsKeyword::Center,
        FlexAlign::FlexEnd => AlignItemsKeyword::FlexEnd,
        FlexAlign::Baseline => AlignItemsKeyword::Baseline,
    };
    tf::Style {
        display: tf::Display::Flex,
        flex_direction: match l.flex_direction {
            FlexDirection::Column => tf::FlexDirection::Column,
            FlexDirection::Row => tf::FlexDirection::Row,
            FlexDirection::ColumnReverse => tf::FlexDirection::ColumnReverse,
            FlexDirection::RowReverse => tf::FlexDirection::RowReverse,
        },
        justify_content: AlignContent {
            keyword: justify_kw,
            safety: AlignmentSafety::Default,
        },
        align_items: AlignItems {
            keyword: align_kw,
            safety: AlignmentSafety::Default,
        },
        flex_grow: l.flex_grow,
        flex_shrink: l.flex_shrink,
        gap: tf::Size {
            width: tf::LengthPercentage::length(l.gap),
            height: tf::LengthPercentage::length(l.gap),
        },
        size: tf::Size {
            width: to_taffy_dim(l.width),
            height: to_taffy_dim(l.height),
        },
        min_size: tf::Size {
            width: to_taffy_lpa(l.min_width),
            height: to_taffy_lpa(l.min_height),
        },
        max_size: tf::Size {
            width: to_taffy_lpa(l.max_width),
            height: to_taffy_lpa(l.max_height),
        },
        padding: tf::Rect {
            top: tf::LengthPercentage::length(l.padding.top),
            right: tf::LengthPercentage::length(l.padding.right),
            bottom: tf::LengthPercentage::length(l.padding.bottom),
            left: tf::LengthPercentage::length(l.padding.left),
        },
        margin: tf::Rect {
            top: tf::LengthPercentageAuto::length(l.margin.top),
            right: tf::LengthPercentageAuto::length(l.margin.right),
            bottom: tf::LengthPercentageAuto::length(l.margin.bottom),
            left: tf::LengthPercentageAuto::length(l.margin.left),
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{CommandBufferBuilder, EdgeInsets, EventMask};

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

        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Points(120.0),
                flex_direction: FlexDirection::Row,
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
        b.set_children(3, &[4]);
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);

        let stats = backend.submit_frame(b.finish()).unwrap();
        assert!(stats.mutation_count >= 4);

        let btn = backend.layout_of(3).unwrap();
        assert_eq!(btn.x, 16.0);
        assert_eq!(btn.y, 16.0);
        assert_eq!(btn.width, 100.0);
        assert_eq!(btn.height, 44.0);

        assert_eq!(backend.hit_test(50.0, 30.0, true), Some(3));
        assert_eq!(backend.hit_test(350.0, 30.0, true), None);
    }
}
