//! `NveMutationWriter` — bridges `dioxus_core::WriteMutations` directly into a batched
//! `nve_abi::CommandBuffer` with zero-string-parse [`TypedStyle`] props, single-pass
//! `DRAW_LIST_ATTR_PREFIX` -> `RawPropValue::Bytes` decoding, and persistent scratch buffers.

mod attributes;
mod flush;
mod mutations;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use nve_abi::hash::HashMap;

use dioxus_core::ElementId;
use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, FrameId, LayoutProps, NodeId, StyleProps,
    SurfaceId, ROOT_NODE_ID,
};
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_interop::native_view::NativeViewHandleBinder;

use crate::native_view::NATIVE_VIEW_TAG;
use crate::typed_style::TypedStyle;

#[derive(Debug, Clone)]
pub(crate) struct NodeRecord {
    pub(crate) kind: ComponentKind,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    pub(crate) layout: LayoutProps,
    pub(crate) style: StyleProps,
    pub(crate) has_explicit_style: bool,
    pub(crate) has_explicit_font_weight: bool,
    pub(crate) has_explicit_text_align: bool,
    pub(crate) typed_style: Option<TypedStyle>,
    pub(crate) inline_style_override: Option<String>,
    pub(crate) font_weight: Option<Cow<'static, str>>,
    pub(crate) text_align: Option<Cow<'static, str>>,
    pub(crate) box_shadow: Option<Cow<'static, str>>,
    pub(crate) overflow: Option<Cow<'static, str>>,
    pub(crate) object_fit: Option<Cow<'static, str>>,
    pub(crate) event_mask: u64,
    pub(crate) dioxus_id: Option<ElementId>,
    pub(crate) test_id: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) deferred_native_create: bool,
    pub(crate) native_kind: Option<String>,
    pub(crate) native_handle_binder: Option<NativeViewHandleBinder>,
    pub(crate) explicit_show_on: Option<MediaQuery>,
    pub(crate) show_on: Option<MediaQuery>,
    pub(crate) responsive_rules: Vec<ResponsiveRule>,
}

impl Default for NodeRecord {
    fn default() -> Self {
        Self {
            kind: ComponentKind::View,
            parent: None,
            children: Vec::new(),
            layout: LayoutProps::default(),
            style: StyleProps::default(),
            has_explicit_style: false,
            has_explicit_font_weight: false,
            has_explicit_text_align: false,
            typed_style: None,
            inline_style_override: None,
            font_weight: None,
            text_align: None,
            box_shadow: None,
            overflow: None,
            object_fit: None,
            event_mask: 0,
            dioxus_id: None,
            test_id: None,
            text: None,
            deferred_native_create: false,
            native_kind: None,
            native_handle_binder: None,
            explicit_show_on: None,
            show_on: None,
            responsive_rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct TextStyleWalkEntry {
    pub(super) node_id: NodeId,
    pub(super) color: ColorRgba,
    pub(super) font_size: f32,
    pub(super) font_weight: Option<Cow<'static, str>>,
    pub(super) text_align: Option<Cow<'static, str>>,
}

/// Bridges `dioxus_core::WriteMutations` into a single batched `nve_abi::CommandBuffer`.
pub struct NveMutationWriter {
    pub(super) surface_id: SurfaceId,
    next_node_id: NodeId,
    /// Maps Dioxus `ElementId` -> NVE `NodeId`. (`ElementId(0)` == `ROOT_NODE_ID`).
    el_to_node: HashMap<ElementId, NodeId>,
    node_to_el: HashMap<NodeId, ElementId>,
    pub(crate) nodes: HashMap<NodeId, NodeRecord>,
    pub(super) stack: Vec<NodeId>,
    /// Persistent scratch buffers reused across frames (cleared + sorted/deduped in place).
    pub(super) style_sources: Vec<NodeId>,
    pub(super) style_walk_stack: Vec<TextStyleWalkEntry>,
    pub(super) dirty_children: Vec<NodeId>,
    pub(super) dirty_layout: Vec<NodeId>,
    pub(super) dirty_style: Vec<NodeId>,
    pub(super) dirty_text_props: Vec<NodeId>,
    pub(super) dirty_events: Vec<NodeId>,
    pub(super) deferred_native_nodes: Vec<NodeId>,
    pub(crate) patch_ids: Vec<NodeId>,
    pub(crate) patch_leaf_text_ids: Vec<NodeId>,
    pub(crate) builder: CommandBufferBuilder,
}

impl NveMutationWriter {
    pub fn new(surface_id: SurfaceId) -> Self {
        let mut el_to_node = HashMap::with_capacity_and_hasher(16, Default::default());
        let mut node_to_el = HashMap::with_capacity_and_hasher(16, Default::default());
        let mut nodes = HashMap::with_capacity_and_hasher(16, Default::default());
        el_to_node.insert(ElementId(0), ROOT_NODE_ID);
        node_to_el.insert(ROOT_NODE_ID, ElementId(0));
        nodes.insert(
            ROOT_NODE_ID,
            NodeRecord {
                dioxus_id: Some(ElementId(0)),
                ..Default::default()
            },
        );
        Self {
            surface_id,
            next_node_id: ROOT_NODE_ID + 1,
            el_to_node,
            node_to_el,
            nodes,
            stack: Vec::with_capacity(64),
            style_sources: Vec::with_capacity(16),
            style_walk_stack: Vec::new(),
            dirty_children: Vec::with_capacity(16),
            dirty_layout: Vec::with_capacity(16),
            dirty_style: Vec::with_capacity(16),
            dirty_text_props: Vec::new(),
            dirty_events: Vec::new(),
            deferred_native_nodes: Vec::new(),
            patch_ids: Vec::new(),
            patch_leaf_text_ids: Vec::new(),
            builder: CommandBufferBuilder::new(surface_id, 1),
        }
    }

    pub fn begin_frame(&mut self, surface_id: SurfaceId, frame_id: FrameId) {
        self.surface_id = surface_id;
        self.builder.reset(surface_id, frame_id);
        self.stack.clear();
        self.style_sources.clear();
        self.style_walk_stack.clear();
        self.dirty_children.clear();
        self.dirty_layout.clear();
        self.dirty_style.clear();
        self.dirty_text_props.clear();
        self.dirty_events.clear();
        self.deferred_native_nodes.clear();
    }

    pub fn element_id_of(&self, node_id: NodeId) -> Option<ElementId> {
        self.node_to_el.get(&node_id).copied()
    }

    /// Returns the direct typed [`MediaQuery`] visibility gate for `node_id` (zero serialization).
    #[inline]
    pub fn node_show_on(&self, node_id: NodeId) -> Option<MediaQuery> {
        self.nodes.get(&node_id).and_then(|r| r.show_on)
    }

    /// Returns the direct typed [`ResponsiveRule`] slice for `node_id` (zero serialization).
    #[inline]
    pub fn node_responsive_rules(&self, node_id: NodeId) -> &[ResponsiveRule] {
        self.nodes
            .get(&node_id)
            .map(|r| r.responsive_rules.as_slice())
            .unwrap_or(&[])
    }

    /// Iterates over all active nodes with a declarative [`MediaQuery`] visibility gate.
    #[inline]
    pub fn iter_show_on(&self) -> impl Iterator<Item = (NodeId, MediaQuery)> + '_ {
        self.nodes
            .iter()
            .filter_map(|(&id, rec)| rec.show_on.map(|q| (id, q)))
    }

    /// Iterates over all active nodes with one or more [`ResponsiveRule`] style overrides.
    #[inline]
    pub fn iter_responsive_rules(&self) -> impl Iterator<Item = (NodeId, &[ResponsiveRule])> + '_ {
        self.nodes.iter().filter_map(|(&id, rec)| {
            (!rec.responsive_rules.is_empty()).then_some((id, rec.responsive_rules.as_slice()))
        })
    }

    pub(crate) fn has_event_mask(&self, node_id: NodeId, bit: u64) -> bool {
        self.nodes
            .get(&node_id)
            .is_some_and(|n| (n.event_mask & bit) != 0)
    }

    pub(crate) fn has_event_mask_in_chain(&self, mut node_id: NodeId, bit: u64) -> bool {
        while let Some(rec) = self.nodes.get(&node_id) {
            if (rec.event_mask & bit) != 0 {
                return true;
            }
            match rec.parent {
                Some(parent) => node_id = parent,
                None => break,
            }
        }
        false
    }

    pub(super) fn alloc_node(&mut self, kind: ComponentKind, custom_name: Option<&str>) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.insert(
            id,
            NodeRecord {
                kind,
                ..Default::default()
            },
        );
        self.builder.create_node(id, kind, custom_name);
        id
    }

    pub(super) fn alloc_deferred_native_node(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.insert(
            id,
            NodeRecord {
                kind: ComponentKind::Custom,
                deferred_native_create: true,
                ..Default::default()
            },
        );
        self.deferred_native_nodes.push(id);
        id
    }

    pub(super) fn ensure_native_node_created(
        &mut self,
        node_id: NodeId,
        kind_override: Option<&str>,
    ) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if !rec.deferred_native_create {
            return;
        }
        rec.deferred_native_create = false;
        let resolved = kind_override.unwrap_or(NATIVE_VIEW_TAG);
        rec.native_kind = Some(resolved.to_owned());
        self.builder
            .create_node(node_id, ComponentKind::Custom, Some(resolved));
    }

    pub(super) fn bind_element_id(&mut self, node_id: NodeId, el_id: ElementId) {
        self.el_to_node.insert(el_id, node_id);
        self.node_to_el.insert(node_id, el_id);
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            rec.dioxus_id = Some(el_id);
        }
    }

    pub(super) fn resolve_el(&self, el_id: ElementId) -> Option<NodeId> {
        self.el_to_node.get(&el_id).copied()
    }

    pub(super) fn walk_path(&self, root: NodeId, path: &[u8]) -> Option<NodeId> {
        let mut cur = root;
        for &idx in path {
            let rec = self.nodes.get(&cur)?;
            cur = *rec.children.get(idx as usize)?;
        }
        Some(cur)
    }

    pub(super) fn detach_from_parent(&mut self, node_id: NodeId) {
        if let Some(parent_id) = self.nodes.get(&node_id).and_then(|r| r.parent) {
            if let Some(parent) = self.nodes.get_mut(&parent_id) {
                parent.children.retain(|&c| c != node_id);
            }
            self.dirty_children.push(parent_id);
        }
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            rec.parent = None;
        }
    }

    pub(super) fn replace_node_in_parent_from_stack(&mut self, target_id: NodeId, split: usize) {
        let Some(parent_id) = self.nodes.get(&target_id).and_then(|r| r.parent) else {
            self.stack.truncate(split);
            return;
        };
        for i in split..self.stack.len() {
            let r = self.stack[i];
            self.detach_from_parent(r);
            if let Some(rec) = self.nodes.get_mut(&r) {
                rec.parent = Some(parent_id);
            }
        }
        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            if let Some(pos) = parent.children.iter().position(|&c| c == target_id) {
                parent.children.splice(pos..=pos, self.stack.drain(split..));
            } else {
                self.stack.truncate(split);
            }
        } else {
            self.stack.truncate(split);
        }
        self.dirty_children.push(parent_id);
        self.remove_subtree(target_id);
    }

    pub(super) fn remove_subtree(&mut self, node_id: NodeId) {
        if let Some(rec) = self.nodes.remove(&node_id) {
            if let Some(binder) = rec.native_handle_binder {
                binder.unbind();
            }
            if let Some(el_id) = rec.dioxus_id {
                self.el_to_node.remove(&el_id);
            }
            self.node_to_el.remove(&node_id);
            for child in rec.children {
                self.remove_subtree(child);
            }
            self.builder.remove_node(node_id);
        }
    }
}
