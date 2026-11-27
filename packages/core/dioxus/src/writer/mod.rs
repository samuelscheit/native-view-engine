//! `NveMutationWriter` & ephemeral `MutationTx<'_, S: RenderSink>` — bridges
//! `dioxus_core::WriteMutations` directly into any monomorphized [`nve_engine::RenderSink`]
//! with zero intermediate serialization, zero vtable dispatch, and zero out-of-band sync loops.

mod attr_keys;
mod attributes;
mod flex_basis;
mod flush;
mod mutations;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use nve_abi::hash::HashMap;

use dioxus_core::ElementId;
use nve_abi::{
    ColorRgba, ComponentKind, FrameId, LayoutProps, NodeId, StyleProps, SurfaceId, ROOT_NODE_ID,
};
use nve_components::canvas_ir::DrawList;
use nve_components::interaction::{CursorKind, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::native_view::TypedNativeViewProps;
use nve_components::paint::BackgroundFill;
use nve_components::svg::SvgPicture;
use nve_components::transform::Transform2D;
use nve_engine::RenderSink;
use nve_interop::native_view::NativeViewHandleBinder;

use crate::native_view::NATIVE_VIEW_TAG;
use crate::typed_style::TypedStyle;

pub(crate) const EXT_LAYOUT_EXT: u16 = 1 << 0;
pub(crate) const EXT_TRANSFORM: u16 = 1 << 1;
pub(crate) const EXT_CURSOR: u16 = 1 << 2;
pub(crate) const EXT_STATE_RULES: u16 = 1 << 3;
pub(crate) const EXT_TRANSITIONS: u16 = 1 << 4;
pub(crate) const EXT_SHOW_ON: u16 = 1 << 5;
pub(crate) const EXT_RESPONSIVE_RULES: u16 = 1 << 6;
pub(crate) const EXT_BACKGROUND_FILL: u16 = 1 << 7;
pub(crate) const EXT_SVG: u16 = 1 << 8;
pub(crate) const EXT_DRAW_LIST: u16 = 1 << 9;
pub(crate) const EXT_NATIVE_VIEW_PROPS: u16 = 1 << 10;

/// Cold optional per-node metadata boxed behind `Option<Box<NodeExtras>>` so hot `NodeRecord`
/// entries stay compact (`<= 200` bytes).
#[derive(Debug, Clone, Default)]
pub(crate) struct NodeExtras {
    pub(crate) typed_style: Option<TypedStyle>,
    pub(crate) inline_style_override: Option<String>,
    pub(crate) font_weight: Option<Cow<'static, str>>,
    pub(crate) text_align: Option<Cow<'static, str>>,
    pub(crate) box_shadow: Option<Cow<'static, str>>,
    pub(crate) overflow: Option<Cow<'static, str>>,
    pub(crate) object_fit: Option<Cow<'static, str>>,
    pub(crate) test_id: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) native_kind: Option<String>,
    pub(crate) native_props: Option<TypedNativeViewProps>,
    pub(crate) native_handle_binder: Option<NativeViewHandleBinder>,
    pub(crate) draw_list: Option<DrawList>,
    pub(crate) explicit_show_on: Option<MediaQuery>,
    pub(crate) show_on: Option<MediaQuery>,
    pub(crate) responsive_rules: Vec<ResponsiveRule>,
    pub(crate) cursor: Option<CursorKind>,
    pub(crate) state_rules: Vec<StateStyleRule>,
    pub(crate) transitions: Vec<TransitionSpec>,
    pub(crate) layout_ext: Option<LayoutExt>,
    pub(crate) transform: Option<Transform2D>,
    pub(crate) background_fill: Option<BackgroundFill>,
    pub(crate) svg_picture: Option<SvgPicture>,
    /// Bitmask of `EXT_*` fields modified during the current frame (cleared on flush).
    pub(crate) dirty_ext_mask: u16,
}

const FLAG_EXPLICIT_STYLE: u8 = 1 << 0;
const FLAG_EXPLICIT_FONT_WEIGHT: u8 = 1 << 1;
const FLAG_EXPLICIT_TEXT_ALIGN: u8 = 1 << 2;
const FLAG_DEFERRED_NATIVE_CREATE: u8 = 1 << 3;

#[derive(Debug, Clone)]
pub(crate) struct NodeRecord {
    pub(crate) layout: LayoutProps,
    pub(crate) style: StyleProps,
    pub(crate) children: Vec<NodeId>,
    pub(crate) dioxus_id: Option<ElementId>,
    pub(crate) event_mask: u64,
    pub(crate) extras: Option<Box<NodeExtras>>,
    /// `0` means no parent (valid node IDs start at `ROOT_NODE_ID == 1`).
    parent_raw: NodeId,
    pub(crate) kind: ComponentKind,
    flags: u8,
}

impl Default for NodeRecord {
    fn default() -> Self {
        Self {
            layout: LayoutProps::default(),
            style: StyleProps::default(),
            children: Vec::new(),
            dioxus_id: None,
            event_mask: 0,
            extras: None,
            parent_raw: 0,
            kind: ComponentKind::View,
            flags: 0,
        }
    }
}

impl NodeRecord {
    #[inline]
    pub(crate) fn parent(&self) -> Option<NodeId> {
        (self.parent_raw != 0).then_some(self.parent_raw)
    }

    #[inline]
    pub(crate) fn set_parent(&mut self, parent: Option<NodeId>) {
        self.parent_raw = parent.unwrap_or(0);
    }

    #[inline]
    pub(crate) fn extras(&self) -> Option<&NodeExtras> {
        self.extras.as_deref()
    }

    #[inline]
    pub(crate) fn extras_mut(&mut self) -> &mut NodeExtras {
        self.extras
            .get_or_insert_with(|| Box::new(NodeExtras::default()))
    }

    #[inline]
    pub(crate) fn has_explicit_style(&self) -> bool {
        (self.flags & FLAG_EXPLICIT_STYLE) != 0
    }

    #[inline]
    pub(crate) fn set_has_explicit_style(&mut self, v: bool) {
        if v {
            self.flags |= FLAG_EXPLICIT_STYLE;
        } else {
            self.flags &= !FLAG_EXPLICIT_STYLE;
        }
    }

    #[inline]
    pub(crate) fn has_explicit_font_weight(&self) -> bool {
        (self.flags & FLAG_EXPLICIT_FONT_WEIGHT) != 0
    }

    #[inline]
    pub(crate) fn set_has_explicit_font_weight(&mut self, v: bool) {
        if v {
            self.flags |= FLAG_EXPLICIT_FONT_WEIGHT;
        } else {
            self.flags &= !FLAG_EXPLICIT_FONT_WEIGHT;
        }
    }

    #[inline]
    pub(crate) fn has_explicit_text_align(&self) -> bool {
        (self.flags & FLAG_EXPLICIT_TEXT_ALIGN) != 0
    }

    #[inline]
    pub(crate) fn set_has_explicit_text_align(&mut self, v: bool) {
        if v {
            self.flags |= FLAG_EXPLICIT_TEXT_ALIGN;
        } else {
            self.flags &= !FLAG_EXPLICIT_TEXT_ALIGN;
        }
    }

    #[inline]
    pub(crate) fn deferred_native_create(&self) -> bool {
        (self.flags & FLAG_DEFERRED_NATIVE_CREATE) != 0
    }

    #[inline]
    pub(crate) fn set_deferred_native_create(&mut self, v: bool) {
        if v {
            self.flags |= FLAG_DEFERRED_NATIVE_CREATE;
        } else {
            self.flags &= !FLAG_DEFERRED_NATIVE_CREATE;
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

/// Non-generic persistent Dioxus-to-NVE shadow tree & dirty-coalescing state.
///
/// Pair with any concrete [`RenderSink`] via [`Self::begin_tx`] to stream mutations with zero
/// intermediate serialization and zero vtable indirection.
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
    pub(super) dirty_extended: Vec<NodeId>,
    pub(super) deferred_native_nodes: Vec<NodeId>,
    pub(crate) patch_ids: Vec<NodeId>,
    pub(crate) patch_leaf_text_ids: Vec<NodeId>,
}

/// Ephemeral monomorphized frame transaction pairing [`NveMutationWriter`] with any [`RenderSink`].
///
/// Implements `dioxus_core::WriteMutations` so Dioxus writes directly into `S` without dynamic
/// dispatch or intermediate binary encoding.
pub struct MutationTx<'a, S: RenderSink> {
    pub(crate) writer: &'a mut NveMutationWriter,
    pub(crate) sink: &'a mut S,
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
            dirty_extended: Vec::new(),
            deferred_native_nodes: Vec::new(),
            patch_ids: Vec::new(),
            patch_leaf_text_ids: Vec::new(),
        }
    }

    /// Starts a monomorphized frame transaction targeting `sink`.
    pub fn begin_tx<'a, S: RenderSink>(
        &'a mut self,
        sink: &'a mut S,
        surface_id: SurfaceId,
        frame_id: FrameId,
    ) -> MutationTx<'a, S> {
        self.surface_id = surface_id;
        self.stack.clear();
        self.style_sources.clear();
        self.style_walk_stack.clear();
        self.dirty_children.clear();
        self.dirty_layout.clear();
        self.dirty_style.clear();
        self.dirty_text_props.clear();
        self.dirty_events.clear();
        self.dirty_extended.clear();
        self.deferred_native_nodes.clear();
        sink.begin_frame(surface_id, frame_id);
        MutationTx { writer: self, sink }
    }

    pub fn element_id_of(&self, node_id: NodeId) -> Option<ElementId> {
        self.node_to_el.get(&node_id).copied()
    }

    #[inline]
    pub fn node_show_on(&self, node_id: NodeId) -> Option<MediaQuery> {
        self.nodes.get(&node_id).and_then(|r| r.extras()?.show_on)
    }

    #[inline]
    pub fn node_responsive_rules(&self, node_id: NodeId) -> &[ResponsiveRule] {
        self.nodes
            .get(&node_id)
            .and_then(|r| r.extras())
            .map(|e| e.responsive_rules.as_slice())
            .unwrap_or(&[])
    }

    #[inline]
    pub fn node_cursor(&self, node_id: NodeId) -> Option<CursorKind> {
        self.nodes.get(&node_id).and_then(|r| r.extras()?.cursor)
    }

    #[inline]
    pub fn node_state_rules(&self, node_id: NodeId) -> &[StateStyleRule] {
        self.nodes
            .get(&node_id)
            .and_then(|r| r.extras())
            .map(|e| e.state_rules.as_slice())
            .unwrap_or(&[])
    }

    #[inline]
    pub fn node_transitions(&self, node_id: NodeId) -> &[TransitionSpec] {
        self.nodes
            .get(&node_id)
            .and_then(|r| r.extras())
            .map(|e| e.transitions.as_slice())
            .unwrap_or(&[])
    }

    #[inline]
    pub fn node_layout_ext(&self, node_id: NodeId) -> Option<&LayoutExt> {
        self.nodes
            .get(&node_id)
            .and_then(|r| r.extras()?.layout_ext.as_ref())
    }

    #[inline]
    pub fn node_transform(&self, node_id: NodeId) -> Option<Transform2D> {
        self.nodes.get(&node_id).and_then(|r| r.extras()?.transform)
    }

    #[inline]
    pub fn node_draw_list(&self, node_id: NodeId) -> Option<&DrawList> {
        self.nodes
            .get(&node_id)
            .and_then(|r| r.extras()?.draw_list.as_ref())
    }

    #[inline]
    pub fn node_native_view_props(&self, node_id: NodeId) -> Option<&TypedNativeViewProps> {
        self.nodes
            .get(&node_id)
            .and_then(|r| r.extras()?.native_props.as_ref())
    }

    #[inline]
    pub fn node_background_fill(&self, node_id: NodeId) -> Option<&BackgroundFill> {
        self.nodes
            .get(&node_id)
            .and_then(|r| r.extras()?.background_fill.as_ref())
    }

    #[inline]
    pub fn node_svg_picture(&self, node_id: NodeId) -> Option<&SvgPicture> {
        self.nodes
            .get(&node_id)
            .and_then(|r| r.extras()?.svg_picture.as_ref())
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
            match rec.parent() {
                Some(parent) => node_id = parent,
                None => break,
            }
        }
        false
    }

    pub(super) fn alloc_node<S: RenderSink>(
        &mut self,
        sink: &mut S,
        kind: ComponentKind,
        custom_name: Option<&str>,
    ) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.insert(
            id,
            NodeRecord {
                kind,
                ..Default::default()
            },
        );
        sink.create_node(id, kind, custom_name);
        id
    }

    pub(super) fn alloc_deferred_native_node(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        let mut rec = NodeRecord {
            kind: ComponentKind::Custom,
            ..Default::default()
        };
        rec.set_deferred_native_create(true);
        self.nodes.insert(id, rec);
        self.deferred_native_nodes.push(id);
        id
    }

    pub(super) fn ensure_native_node_created<S: RenderSink>(
        &mut self,
        sink: &mut S,
        node_id: NodeId,
        kind_override: Option<&str>,
    ) {
        let Some(rec) = self.nodes.get_mut(&node_id) else {
            return;
        };
        if !rec.deferred_native_create() {
            return;
        }
        rec.set_deferred_native_create(false);
        let resolved = kind_override.unwrap_or(NATIVE_VIEW_TAG);
        rec.extras_mut().native_kind = Some(resolved.to_owned());
        sink.create_node(node_id, ComponentKind::Custom, Some(resolved));
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
        if let Some(parent_id) = self.nodes.get(&node_id).and_then(NodeRecord::parent) {
            if let Some(parent) = self.nodes.get_mut(&parent_id) {
                parent.children.retain(|&c| c != node_id);
            }
            self.dirty_children.push(parent_id);
        }
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            rec.set_parent(None);
        }
    }

    pub(super) fn replace_node_in_parent_from_stack<S: RenderSink>(
        &mut self,
        sink: &mut S,
        target_id: NodeId,
        split: usize,
    ) {
        let Some(parent_id) = self.nodes.get(&target_id).and_then(NodeRecord::parent) else {
            self.stack.truncate(split);
            return;
        };
        for i in split..self.stack.len() {
            let r = self.stack[i];
            self.detach_from_parent(r);
            if let Some(rec) = self.nodes.get_mut(&r) {
                rec.set_parent(Some(parent_id));
            }
        }
        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            if let Some(pos) = parent.children.iter().position(|&c| c == target_id) {
                parent
                    .children
                    .splice(pos..=pos, self.stack.drain(split..));
            } else {
                self.stack.truncate(split);
            }
        } else {
            self.stack.truncate(split);
        }
        self.dirty_children.push(parent_id);
        self.remove_subtree(sink, target_id);
    }

    pub(super) fn remove_subtree<S: RenderSink>(&mut self, sink: &mut S, node_id: NodeId) {
        if let Some(rec) = self.nodes.remove(&node_id) {
            if let Some(binder) = rec.extras.and_then(|e| e.native_handle_binder) {
                binder.unbind();
            }
            if let Some(el_id) = rec.dioxus_id {
                self.el_to_node.remove(&el_id);
            }
            self.node_to_el.remove(&node_id);
            for child in rec.children {
                self.remove_subtree(sink, child);
            }
            sink.remove_node(node_id);
        }
    }
}

#[inline]
pub(super) fn mark_ext_dirty(
    dirty_extended: &mut Vec<NodeId>,
    ext: &mut NodeExtras,
    node_id: NodeId,
    bit: u16,
) {
    if ext.dirty_ext_mask == 0 {
        dirty_extended.push(node_id);
    }
    ext.dirty_ext_mask |= bit;
}
