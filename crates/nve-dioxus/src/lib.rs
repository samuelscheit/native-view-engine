//! Dioxus (`dioxus-core 0.7`) frontend adapter for `native-view-engine`.
//!
//! Implements `dioxus_core::WriteMutations` to emit a single flat, typed binary `CommandBuffer`
//! per frame — feeding Fabric (iOS/Android), Skia (Desktop/Mobile), Web (WASM), and Headless
//! backends identically.

pub mod elements;
pub mod hot_patch;
pub mod signal_archive;

pub use signal_archive::{
    signal_slot_id, BinValueSlot, HydrationReport, SignalArchive, SignalArchiveError, SignalCodec,
    SignalDelta, SignalDeltaOp, SignalRegistry, SignalTableSnapshot, SignalValue, SignalValueTag,
    NVE_SIGNAL_ARCHIVE_MAGIC, NVE_SIGNAL_ARCHIVE_VERSION, NVE_SIGNAL_DELTA_MAGIC,
};

use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use dioxus_core::{
    AttributeValue, Element, ElementId, Event, Template, TemplateAttribute, TemplateNode,
    VirtualDom, WriteMutations,
};
use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, EventMask, EventPayload, FrameId, LayoutProps,
    NativeEvent, NativeEventKind, NodeId, NveEventBatchRef, RawPropValue, StyleProps, SurfaceId,
    ROOT_NODE_ID,
};
use nve_components::{apply_typed_attr, resolve_tag};
use nve_engine::HotPatchMsg;

pub use elements::{
    FocusEvent, FocusEventData, KeyEvent, KeyEventData, ScrollEvent, ScrollEventData, SubmitEvent,
    SubmitEventData,
};

/// Alias matching `elements::events::onblur` (`FocusEvent`) so blur listeners share one canonical payload type.
pub type BlurEventData = FocusEventData;
pub type BlurEvent = FocusEvent;

/// Payload for generic native `Custom` events dispatched into Dioxus custom event listeners.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomEventData {
    pub name: String,
    pub data: Vec<u8>,
}

pub type CustomEvent = Event<CustomEventData>;

pub mod prelude {
    pub use crate::elements::{
        self as dioxus_elements, events::*, KeyEvent, KeyEventData, PressEvent, SubmitEvent,
        SubmitEventData, TextEvent,
    };
    pub use crate::use_signal;
    pub use crate::{
        BlurEvent, BlurEventData, CustomEvent, CustomEventData, FocusEvent, FocusEventData,
        ScrollEvent, ScrollEventData,
    };
    pub use dioxus_core;
    pub use dioxus_core::{Element, VirtualDom};
    pub use dioxus_core_macro::{component, rsx};
    pub use dioxus_signals;
    pub use dioxus_signals::{ReadableExt, Signal, WritableExt};
}

/// Lightweight `use_signal` hook backed directly by `dioxus_core::use_hook` + `dioxus_signals::Signal`.
pub fn use_signal<T: 'static>(init: impl FnOnce() -> T) -> dioxus_signals::Signal<T> {
    dioxus_core::use_hook(|| dioxus_signals::Signal::new(init()))
}

#[derive(Debug, Clone)]
pub(crate) struct NodeRecord {
    pub(crate) kind: ComponentKind,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    pub(crate) layout: LayoutProps,
    pub(crate) style: StyleProps,
    pub(crate) has_explicit_style: bool,
    pub(crate) event_mask: u64,
    pub(crate) dioxus_id: Option<ElementId>,
    pub(crate) test_id: Option<String>,
    pub(crate) text: Option<String>,
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
            event_mask: 0,
            dioxus_id: None,
            test_id: None,
            text: None,
        }
    }
}

/// Bridges `dioxus_core::WriteMutations` into a single batched `nve_abi::CommandBuffer`.
pub struct NveMutationWriter {
    next_node_id: NodeId,
    /// Maps Dioxus `ElementId` -> NVE `NodeId`. (`ElementId(0)` == `ROOT_NODE_ID`).
    el_to_node: HashMap<ElementId, NodeId>,
    node_to_el: HashMap<NodeId, ElementId>,
    pub(crate) nodes: HashMap<NodeId, NodeRecord>,
    stack: Vec<NodeId>,
    /// Persistent scratch buffers reused across frames (cleared + sorted/deduped in place).
    style_sources: Vec<NodeId>,
    style_walk_stack: Vec<(NodeId, ColorRgba, f32)>,
    dirty_children: Vec<NodeId>,
    dirty_layout: Vec<NodeId>,
    dirty_style: Vec<NodeId>,
    dirty_events: Vec<NodeId>,
    pub(crate) patch_ids: Vec<NodeId>,
    pub(crate) patch_leaf_text_ids: Vec<NodeId>,
    pub(crate) builder: CommandBufferBuilder,
}

impl NveMutationWriter {
    pub fn new(surface_id: SurfaceId) -> Self {
        let mut el_to_node = HashMap::with_capacity(16);
        let mut node_to_el = HashMap::with_capacity(16);
        let mut nodes = HashMap::with_capacity(16);
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
            dirty_events: Vec::new(),
            patch_ids: Vec::new(),
            patch_leaf_text_ids: Vec::new(),
            builder: CommandBufferBuilder::new(surface_id, 1),
        }
    }

    pub fn begin_frame(&mut self, surface_id: SurfaceId, frame_id: FrameId) {
        self.builder.reset(surface_id, frame_id);
        self.stack.clear();
        self.style_sources.clear();
        self.style_walk_stack.clear();
        self.dirty_children.clear();
        self.dirty_layout.clear();
        self.dirty_style.clear();
        self.dirty_events.clear();
    }

    pub fn finish_frame(&mut self, sync_mount: bool) -> &[u8] {
        // Propagate inherited text_color and font_size from styled Text parents to unstyled child
        // Text leaves (Dioxus rsx! emits `text { style: "...", "leaf" }` as parent Text -> child Text).
        if !self.dirty_children.is_empty() || !self.dirty_style.is_empty() {
            self.style_sources.clear();
            self.style_sources.extend_from_slice(&self.dirty_children);
            self.style_sources.extend_from_slice(&self.dirty_style);
            self.style_sources.sort_unstable();
            self.style_sources.dedup();
            for i in 0..self.style_sources.len() {
                let pid = self.style_sources[i];
                self.propagate_text_style_down(pid);
            }
            self.style_sources.clear();
        }

        // Coalesce all structural/style/layout mutations into one deterministic batch per frame
        // using persistent scratch Vecs sorted & deduped in place (zero per-frame heap allocs).
        if !self.dirty_children.is_empty() {
            self.dirty_children.sort_unstable();
            self.dirty_children.dedup();
            for &pid in &self.dirty_children {
                if let Some(rec) = self.nodes.get(&pid) {
                    self.builder.set_children(pid, &rec.children);
                }
            }
            self.dirty_children.clear();
        }

        if !self.dirty_layout.is_empty() {
            self.dirty_layout.sort_unstable();
            self.dirty_layout.dedup();
            for &nid in &self.dirty_layout {
                if let Some(rec) = self.nodes.get(&nid) {
                    self.builder.set_layout(nid, &rec.layout);
                }
            }
            self.dirty_layout.clear();
        }

        if !self.dirty_style.is_empty() {
            self.dirty_style.sort_unstable();
            self.dirty_style.dedup();
            for &nid in &self.dirty_style {
                if let Some(rec) = self.nodes.get(&nid) {
                    self.builder.set_style(nid, &rec.style);
                }
            }
            self.dirty_style.clear();
        }

        if !self.dirty_events.is_empty() {
            self.dirty_events.sort_unstable();
            self.dirty_events.dedup();
            for &nid in &self.dirty_events {
                if let Some(rec) = self.nodes.get(&nid) {
                    self.builder.set_event_mask(nid, rec.event_mask);
                }
            }
            self.dirty_events.clear();
        }

        self.builder.commit(sync_mount);
        self.builder.finish()
    }

    pub fn element_id_of(&self, node_id: NodeId) -> Option<ElementId> {
        self.node_to_el.get(&node_id).copied()
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

    fn alloc_node(&mut self, kind: ComponentKind, custom_name: Option<&str>) -> NodeId {
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

    fn propagate_text_style_down(&mut self, root_parent_id: NodeId) {
        self.style_walk_stack.clear();
        let Some(root) = self.nodes.get(&root_parent_id) else {
            return;
        };
        if root.kind != ComponentKind::Text || root.children.is_empty() {
            return;
        }
        let root_color = root.style.text_color;
        let root_font_size = root.style.font_size;
        for &cid in &root.children {
            self.style_walk_stack.push((cid, root_color, root_font_size));
        }

        while let Some((cid, parent_color, parent_font_size)) = self.style_walk_stack.pop() {
            if let Some(child) = self.nodes.get_mut(&cid) {
                if child.kind == ComponentKind::Text && !child.has_explicit_style {
                    if child.style.text_color != parent_color
                        || (child.style.font_size - parent_font_size).abs() > f32::EPSILON
                    {
                        child.style.text_color = parent_color;
                        child.style.font_size = parent_font_size;
                        self.dirty_style.push(cid);
                    }
                    let next_color = child.style.text_color;
                    let next_font_size = child.style.font_size;
                    for &grandchild in &child.children {
                        self.style_walk_stack
                            .push((grandchild, next_color, next_font_size));
                    }
                }
            }
        }
    }

    fn bind_element_id(&mut self, node_id: NodeId, el_id: ElementId) {
        self.el_to_node.insert(el_id, node_id);
        self.node_to_el.insert(node_id, el_id);
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            rec.dioxus_id = Some(el_id);
        }
    }

    fn resolve_el(&self, el_id: ElementId) -> Option<NodeId> {
        self.el_to_node.get(&el_id).copied()
    }

    fn instantiate_template_node(&mut self, tnode: &TemplateNode) -> NodeId {
        match tnode {
            TemplateNode::Element {
                tag,
                attrs,
                children,
                ..
            } => {
                let (kind, custom) = resolve_tag(tag);
                let id = self.alloc_node(kind, custom);
                for attr in *attrs {
                    if let TemplateAttribute::Static { name, value, .. } = attr {
                        self.apply_str_attr(id, name, value);
                    }
                }
                if !children.is_empty() {
                    let mut child_ids = Vec::with_capacity(children.len());
                    for child in *children {
                        let cid = self.instantiate_template_node(child);
                        if let Some(crec) = self.nodes.get_mut(&cid) {
                            crec.parent = Some(id);
                        }
                        child_ids.push(cid);
                    }
                    if let Some(rec) = self.nodes.get_mut(&id) {
                        rec.children = child_ids;
                    }
                    self.dirty_children.push(id);
                }
                id
            }
            TemplateNode::Text { text } => {
                let id = self.alloc_node(ComponentKind::Text, None);
                self.record_text(id, text);
                self.builder.set_text(id, text);
                id
            }
            TemplateNode::Dynamic { .. } => {
                // Placeholder slot replaced later by `replace_placeholder_with_nodes`.
                self.alloc_node(ComponentKind::View, None)
            }
        }
    }

    fn walk_path(&self, root: NodeId, path: &[u8]) -> Option<NodeId> {
        let mut cur = root;
        for &idx in path {
            let rec = self.nodes.get(&cur)?;
            cur = *rec.children.get(idx as usize)?;
        }
        Some(cur)
    }

    pub(crate) fn apply_str_attr(&mut self, node_id: NodeId, name: &str, value: &str) {
        if hot_patch::is_test_id_attr_name(name) {
            self.record_test_id(node_id, value);
        }
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            let prev_layout = rec.layout;
            let prev_style = rec.style;
            if apply_typed_attr(&mut rec.layout, &mut rec.style, name, value) {
                if rec.layout != prev_layout {
                    self.dirty_layout.push(node_id);
                }
                if rec.style != prev_style {
                    rec.has_explicit_style = true;
                    self.dirty_style.push(node_id);
                }
                return;
            }
        }
        self.builder
            .set_raw_prop(node_id, name, &RawPropValue::String(value.to_owned()));
    }

    fn detach_from_parent(&mut self, node_id: NodeId) {
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

    fn replace_node_in_parent_from_stack(&mut self, target_id: NodeId, split: usize) {
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

    fn remove_subtree(&mut self, node_id: NodeId) {
        if let Some(rec) = self.nodes.remove(&node_id) {
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

impl WriteMutations for NveMutationWriter {
    fn append_children(&mut self, id: ElementId, m: usize) {
        let split = self.stack.len().saturating_sub(m);
        let Some(parent_id) = self.resolve_el(id) else {
            self.stack.truncate(split);
            return;
        };
        for i in split..self.stack.len() {
            let cid = self.stack[i];
            self.detach_from_parent(cid);
            if let Some(crec) = self.nodes.get_mut(&cid) {
                crec.parent = Some(parent_id);
            }
        }
        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            parent.children.extend(self.stack.drain(split..));
        } else {
            self.stack.truncate(split);
        }
        self.dirty_children.push(parent_id);
    }

    fn assign_node_id(&mut self, path: &'static [u8], id: ElementId) {
        let Some(&top) = self.stack.last() else {
            return;
        };
        if let Some(target) = self.walk_path(top, path) {
            self.bind_element_id(target, id);
        }
    }

    fn create_placeholder(&mut self, id: ElementId) {
        let nid = self.alloc_node(ComponentKind::View, None);
        self.bind_element_id(nid, id);
        self.stack.push(nid);
    }

    fn create_text_node(&mut self, value: &str, id: ElementId) {
        let nid = self.alloc_node(ComponentKind::Text, None);
        self.bind_element_id(nid, id);
        self.record_text(nid, value);
        self.builder.set_text(nid, value);
        self.stack.push(nid);
    }

    fn load_template(&mut self, template: Template, index: usize, id: ElementId) {
        if let Some(root_tpl) = template.roots.get(index) {
            let nid = self.instantiate_template_node(root_tpl);
            self.bind_element_id(nid, id);
            self.stack.push(nid);
        }
    }

    fn replace_node_with(&mut self, id: ElementId, m: usize) {
        let split = self.stack.len().saturating_sub(m);
        if let Some(target_id) = self.resolve_el(id) {
            self.replace_node_in_parent_from_stack(target_id, split);
        } else {
            self.stack.truncate(split);
        }
    }

    fn replace_placeholder_with_nodes(&mut self, path: &'static [u8], m: usize) {
        let split = self.stack.len().saturating_sub(m);
        let Some(&top) = split.checked_sub(1).and_then(|i| self.stack.get(i)) else {
            self.stack.truncate(split);
            return;
        };
        if let Some(placeholder) = self.walk_path(top, path) {
            self.replace_node_in_parent_from_stack(placeholder, split);
        } else {
            self.stack.truncate(split);
        }
    }

    fn insert_nodes_after(&mut self, id: ElementId, m: usize) {
        let split = self.stack.len().saturating_sub(m);
        let Some(anchor) = self.resolve_el(id) else {
            self.stack.truncate(split);
            return;
        };
        let Some(parent_id) = self.nodes.get(&anchor).and_then(|r| r.parent) else {
            self.stack.truncate(split);
            return;
        };
        for i in split..self.stack.len() {
            let n = self.stack[i];
            self.detach_from_parent(n);
            if let Some(rec) = self.nodes.get_mut(&n) {
                rec.parent = Some(parent_id);
            }
        }
        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            if let Some(pos) = parent.children.iter().position(|&c| c == anchor) {
                parent
                    .children
                    .splice(pos + 1..pos + 1, self.stack.drain(split..));
            } else {
                self.stack.truncate(split);
            }
        } else {
            self.stack.truncate(split);
        }
        self.dirty_children.push(parent_id);
    }

    fn insert_nodes_before(&mut self, id: ElementId, m: usize) {
        let split = self.stack.len().saturating_sub(m);
        let Some(anchor) = self.resolve_el(id) else {
            self.stack.truncate(split);
            return;
        };
        let Some(parent_id) = self.nodes.get(&anchor).and_then(|r| r.parent) else {
            self.stack.truncate(split);
            return;
        };
        for i in split..self.stack.len() {
            let n = self.stack[i];
            self.detach_from_parent(n);
            if let Some(rec) = self.nodes.get_mut(&n) {
                rec.parent = Some(parent_id);
            }
        }
        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            if let Some(pos) = parent.children.iter().position(|&c| c == anchor) {
                parent.children.splice(pos..pos, self.stack.drain(split..));
            } else {
                self.stack.truncate(split);
            }
        } else {
            self.stack.truncate(split);
        }
        self.dirty_children.push(parent_id);
    }

    fn set_attribute(
        &mut self,
        name: &'static str,
        _ns: Option<&'static str>,
        value: &AttributeValue,
        id: ElementId,
    ) {
        let Some(node_id) = self.resolve_el(id) else {
            return;
        };
        match value {
            AttributeValue::Text(s) => self.apply_str_attr(node_id, name, s),
            AttributeValue::Float(f) => {
                self.apply_str_attr(node_id, name, &f.to_string());
            }
            AttributeValue::Int(i) => {
                self.apply_str_attr(node_id, name, &i.to_string());
            }
            AttributeValue::Bool(b) => {
                self.builder
                    .set_raw_prop(node_id, name, &RawPropValue::Bool(*b));
            }
            AttributeValue::None => {
                if hot_patch::is_test_id_attr_name(name) {
                    self.clear_test_id(node_id);
                }
                self.builder
                    .set_raw_prop(node_id, name, &RawPropValue::Null);
            }
            AttributeValue::Listener(_) | AttributeValue::Any(_) => {}
        }
    }

    fn set_node_text(&mut self, value: &str, id: ElementId) {
        if let Some(node_id) = self.resolve_el(id) {
            self.record_text(node_id, value);
            self.builder.set_text(node_id, value);
        }
    }

    fn create_event_listener(&mut self, name: &'static str, id: ElementId) {
        let Some(node_id) = self.resolve_el(id) else {
            return;
        };
        let bit = event_name_to_mask(name);
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            rec.event_mask |= bit;
            self.dirty_events.push(node_id);
        }
    }

    fn remove_event_listener(&mut self, name: &'static str, id: ElementId) {
        let Some(node_id) = self.resolve_el(id) else {
            return;
        };
        let bit = event_name_to_mask(name);
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            rec.event_mask &= !bit;
            self.dirty_events.push(node_id);
        }
    }

    fn remove_node(&mut self, id: ElementId) {
        if let Some(node_id) = self.resolve_el(id) {
            self.detach_from_parent(node_id);
            self.remove_subtree(node_id);
        }
    }

    fn push_root(&mut self, id: ElementId) {
        if let Some(node_id) = self.resolve_el(id) {
            self.stack.push(node_id);
        }
    }
}

fn event_name_to_mask(name: &str) -> u64 {
    match name {
        "press" | "onpress" | "click" | "onclick" => EventMask::PRESS,
        "longpress" | "onlongpress" => EventMask::LONG_PRESS,
        "scroll" | "onscroll" => EventMask::SCROLL,
        "change" | "onchange" | "input" | "oninput" => EventMask::TEXT_CHANGE,
        "focus" | "onfocus" => EventMask::FOCUS,
        "blur" | "onblur" => EventMask::BLUR,
        "layout" | "onlayout" => EventMask::LAYOUT,
        _ => EventMask::CUSTOM,
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct FrameTimer(Instant);

#[cfg(not(target_arch = "wasm32"))]
impl FrameTimer {
    fn start() -> Self {
        Self(Instant::now())
    }

    fn elapsed_us(&self) -> u64 {
        self.0.elapsed().as_micros() as u64
    }
}

#[cfg(target_arch = "wasm32")]
struct FrameTimer;

#[cfg(target_arch = "wasm32")]
impl FrameTimer {
    fn start() -> Self {
        Self
    }

    fn elapsed_us(&self) -> u64 {
        0
    }
}

/// Complete Dioxus-backed surface producing batched `nve-abi` `CommandBuffer` frames.
pub struct DioxusSurface {
    surface_id: SurfaceId,
    vdom: VirtualDom,
    writer: NveMutationWriter,
    initialized: bool,
    pending_hot_patches: Vec<HotPatchMsg>,
    signal_registry: SignalRegistry,
}

impl DioxusSurface {
    pub fn new(surface_id: SurfaceId, root: fn() -> Element) -> Self {
        Self {
            surface_id,
            vdom: VirtualDom::new(root),
            writer: NveMutationWriter::new(surface_id),
            initialized: false,
            pending_hot_patches: Vec::new(),
            signal_registry: SignalRegistry::new(1),
        }
    }

    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    pub fn signal_registry_mut(&mut self) -> &mut SignalRegistry {
        &mut self.signal_registry
    }

    /// Encodes all signals registered on this surface into a zero-JSON `NVE_SIG1` binary archive.
    pub fn snapshot_signals(&self) -> Result<Vec<u8>, SignalArchiveError> {
        self.signal_registry.encode_full()
    }

    /// Hydrates registered signals on this surface from a binary `NVE_SIG1` archive with
    /// schema-evolution safety.
    pub fn hydrate_signals(&mut self, bytes: &[u8]) -> Result<HydrationReport, SignalArchiveError> {
        self.signal_registry.hydrate_from_bytes(bytes)
    }

    /// Reconciles the `VirtualDom` and returns `(reconcile_us, encode_us, command_buffer_bytes)`.
    pub fn render_frame(&mut self, frame_id: FrameId) -> (u64, u64, Vec<u8>) {
        self.writer.begin_frame(self.surface_id, frame_id);
        let t_rec = FrameTimer::start();
        if !self.initialized {
            self.vdom.rebuild(&mut self.writer);
            self.initialized = true;
        } else {
            self.vdom.render_immediate(&mut self.writer);
        }
        for patch in self.pending_hot_patches.drain(..) {
            self.writer.apply_hot_patch(&patch);
        }
        let reconcile_us = t_rec.elapsed_us();

        let t_enc = FrameTimer::start();
        let bytes = self.writer.finish_frame(true).to_vec();
        let encode_us = t_enc.elapsed_us();
        (reconcile_us, encode_us, bytes)
    }

    /// Routes incoming native events from any backend (`EventRing`) back into Dioxus handlers.
    pub fn dispatch_events(&mut self, events: &[NativeEvent]) -> bool {
        let mut handled = false;
        for ev in events {
            let Some(el_id) = self.writer.element_id_of(ev.node_id) else {
                continue;
            };
            match (ev.kind, &ev.payload) {
                (NativeEventKind::Press, EventPayload::Point { x, y }) => {
                    let data: Rc<dyn Any> = Rc::new(elements::PressEventData { x: *x, y: *y });
                    self.vdom
                        .runtime()
                        .handle_event("press", Event::new(data, true), el_id);
                    handled = true;
                }
                (NativeEventKind::LongPress, EventPayload::Point { x, y })
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id, EventMask::LONG_PRESS) =>
                {
                    let data: Rc<dyn Any> = Rc::new(elements::PressEventData { x: *x, y: *y });
                    self.vdom
                        .runtime()
                        .handle_event("longpress", Event::new(data, true), el_id);
                    handled = true;
                }
                (NativeEventKind::LongPress, EventPayload::Empty)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id, EventMask::LONG_PRESS) =>
                {
                    let data: Rc<dyn Any> = Rc::new(elements::PressEventData::default());
                    self.vdom
                        .runtime()
                        .handle_event("longpress", Event::new(data, true), el_id);
                    handled = true;
                }
                (
                    NativeEventKind::Scroll,
                    EventPayload::Scroll {
                        x,
                        y,
                        content_width,
                        content_height,
                    },
                ) if self.writer.has_event_mask(ev.node_id, EventMask::SCROLL) => {
                    let data: Rc<dyn Any> = Rc::new(ScrollEventData {
                        x: *x,
                        y: *y,
                        content_width: *content_width,
                        content_height: *content_height,
                    });
                    self.vdom
                        .runtime()
                        .handle_event("scroll", Event::new(data, false), el_id);
                    handled = true;
                }
                (NativeEventKind::TextChange, EventPayload::Text(value)) => {
                    let data: Rc<dyn Any> = Rc::new(elements::TextEventData {
                        value: value.clone(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("change", Event::new(data, true), el_id);
                    handled = true;
                }
                (NativeEventKind::Focus, EventPayload::Empty)
                    if self.writer.has_event_mask(ev.node_id, EventMask::FOCUS) =>
                {
                    let data: Rc<dyn Any> = Rc::new(FocusEventData);
                    self.vdom
                        .runtime()
                        .handle_event("focus", Event::new(data, false), el_id);
                    handled = true;
                }
                (NativeEventKind::Blur, EventPayload::Empty)
                    if self.writer.has_event_mask(ev.node_id, EventMask::BLUR) =>
                {
                    let data: Rc<dyn Any> = Rc::new(FocusEventData);
                    self.vdom
                        .runtime()
                        .handle_event("blur", Event::new(data, false), el_id);
                    handled = true;
                }
                (NativeEventKind::Custom, EventPayload::Custom { name, data })
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id, EventMask::CUSTOM) =>
                {
                    let wire_name = normalize_custom_event_name(name);
                    let payload = build_custom_event_payload(&wire_name, name, data);
                    self.vdom
                        .runtime()
                        .handle_event(&wire_name, Event::new(payload, true), el_id);
                    handled = true;
                }
                _ => {}
            }
        }
        handled
    }

    /// Routes zero-copy direct FFI events (`NveEventBatchRef`) straight into Dioxus handlers
    /// without decoding an `EventRing`.
    pub fn dispatch_events_direct(&mut self, events: NveEventBatchRef<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            let Some(el_id) = self.writer.element_id_of(ev.node_id()) else {
                continue;
            };
            match (ev.kind(), ev.payload_tag()) {
                (Some(NativeEventKind::Press), 1) => {
                    let data: Rc<dyn Any> = Rc::new(elements::PressEventData {
                        x: ev.x(),
                        y: ev.y(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("press", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::LongPress), 1)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::LONG_PRESS) =>
                {
                    let data: Rc<dyn Any> = Rc::new(elements::PressEventData {
                        x: ev.x(),
                        y: ev.y(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("longpress", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::LongPress), 0)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::LONG_PRESS) =>
                {
                    let data: Rc<dyn Any> = Rc::new(elements::PressEventData::default());
                    self.vdom
                        .runtime()
                        .handle_event("longpress", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::Scroll), 2)
                    if self.writer.has_event_mask(ev.node_id(), EventMask::SCROLL) =>
                {
                    let data: Rc<dyn Any> = Rc::new(ScrollEventData {
                        x: ev.x(),
                        y: ev.y(),
                        content_width: ev.width(),
                        content_height: ev.height(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("scroll", Event::new(data, false), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::TextChange), 3) => {
                    let data: Rc<dyn Any> = Rc::new(elements::TextEventData {
                        value: ev.text().to_owned(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("change", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::Focus), 0)
                    if self.writer.has_event_mask(ev.node_id(), EventMask::FOCUS) =>
                {
                    let data: Rc<dyn Any> = Rc::new(FocusEventData);
                    self.vdom
                        .runtime()
                        .handle_event("focus", Event::new(data, false), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::Blur), 0)
                    if self.writer.has_event_mask(ev.node_id(), EventMask::BLUR) =>
                {
                    let data: Rc<dyn Any> = Rc::new(FocusEventData);
                    self.vdom
                        .runtime()
                        .handle_event("blur", Event::new(data, false), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::Custom), 5)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::CUSTOM) =>
                {
                    let name = ev.name();
                    let wire_name = normalize_custom_event_name(name);
                    let raw_bytes = if let Some(b) = ev.value().as_bytes() {
                        b.to_vec()
                    } else if let Some(s) = ev.value().as_str() {
                        s.as_bytes().to_vec()
                    } else if !ev.text().is_empty() {
                        ev.text().as_bytes().to_vec()
                    } else {
                        Vec::new()
                    };
                    let payload = build_custom_event_payload(&wire_name, name, &raw_bytes);
                    self.vdom
                        .runtime()
                        .handle_event(&wire_name, Event::new(payload, true), el_id);
                    handled = true;
                }
                _ => {}
            }
        }
        handled
    }

    pub fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        if matches!(patch, HotPatchMsg::TemplatePatch { .. }) {
            self.pending_hot_patches.push(patch.clone());
            true
        } else {
            false
        }
    }
}

#[inline]
fn normalize_custom_event_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let stripped = lower.strip_prefix("on").unwrap_or(&lower);
    if stripped.is_empty() {
        "custom".to_owned()
    } else {
        stripped.to_owned()
    }
}

fn parse_key_event_data(raw: &[u8]) -> KeyEventData {
    let text = String::from_utf8_lossy(raw);
    let trimmed = text.trim();
    let mut out = KeyEventData::default();
    if trimmed.is_empty() {
        return out;
    }
    if trimmed.starts_with('{') {
        if let Some(key) = extract_json_string_field(trimmed, "key") {
            out.key = key;
        }
        out.meta = extract_json_bool_field(trimmed, "meta")
            .or_else(|| extract_json_bool_field(trimmed, "metaKey"))
            .unwrap_or(false);
        out.ctrl = extract_json_bool_field(trimmed, "ctrl")
            .or_else(|| extract_json_bool_field(trimmed, "ctrlKey"))
            .unwrap_or(false);
        out.shift = extract_json_bool_field(trimmed, "shift")
            .or_else(|| extract_json_bool_field(trimmed, "shiftKey"))
            .unwrap_or(false);
        out.alt = extract_json_bool_field(trimmed, "alt")
            .or_else(|| extract_json_bool_field(trimmed, "altKey"))
            .unwrap_or(false);
        return out;
    }
    // Support modifier-prefixed notation (e.g. "cmd+k", "ctrl+shift+Enter") or bare key names.
    if trimmed.contains('+') && trimmed != "+" {
        let parts: Vec<&str> = trimmed
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if let Some((&last, mods)) = parts.split_last() {
            for &mod_tok in mods {
                match mod_tok.to_ascii_lowercase().as_str() {
                    "cmd" | "meta" | "super" => out.meta = true,
                    "ctrl" | "control" => out.ctrl = true,
                    "shift" => out.shift = true,
                    "alt" | "opt" | "option" => out.alt = true,
                    _ => {}
                }
            }
            out.key = last.to_owned();
        }
    } else {
        out.key = trimmed.to_owned();
    }
    out
}

fn find_json_field_value<'a>(json: &'a str, field: &str) -> Option<&'a str> {
    let needle = format!("\"{field}\"");
    let mut search = json;
    while let Some(pos) = search.find(&needle) {
        let after = search[pos + needle.len()..].trim_start();
        if let Some(rest) = after.strip_prefix(':') {
            return Some(rest.trim_start());
        }
        search = &search[pos + needle.len()..];
    }
    None
}

fn extract_json_bool_field(json: &str, field: &str) -> Option<bool> {
    let rest = find_json_field_value(json, field)?;
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    let rest = find_json_field_value(json, field)?;
    let mut chars = rest.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn build_custom_event_payload(wire_name: &str, raw_name: &str, data: &[u8]) -> Rc<dyn Any> {
    match wire_name {
        "submit" => Rc::new(SubmitEventData {
            value: String::from_utf8_lossy(data).into_owned(),
        }),
        "keydown" => Rc::new(parse_key_event_data(data)),
        _ => Rc::new(CustomEventData {
            name: raw_name.to_owned(),
            data: data.to_vec(),
        }),
    }
}

#[cfg(test)]
mod event_dispatch_tests {
    use super::*;
    use crate::prelude::*;
    use dioxus_core::{Attribute, AttributeValue, Element, VNode};
    use nve_abi::{
        Command, CommandDecoder, EventPayload, NativeEvent, NativeEventKind, NveBufferSlice,
        NveDirectEvent, NveEventBatch, NveStrSlice, NveValue, RawPropValue,
    };

    #[component]
    fn AllEventsApp() -> Element {
        let mut log = use_signal(String::new);

        let on_scroll = Attribute::new(
            "onscroll",
            AttributeValue::listener(move |ev: ScrollEvent| {
                let d = ev.data();
                log.set(format!(
                    "scroll:{},{},{},{}",
                    d.x, d.y, d.content_width, d.content_height
                ));
            }),
            None,
            false,
        );
        let on_focus = Attribute::new(
            "onfocus",
            AttributeValue::listener(move |_ev: FocusEvent| {
                log.set("focus".into());
            }),
            None,
            false,
        );
        let on_blur = Attribute::new(
            "onblur",
            AttributeValue::listener(move |_ev: BlurEvent| {
                log.set("blur".into());
            }),
            None,
            false,
        );
        let on_longpress = Attribute::new(
            "onlongpress",
            AttributeValue::listener(move |ev: elements::PressEvent| {
                let d = ev.data();
                log.set(format!("longpress:{},{}", d.x, d.y));
            }),
            None,
            false,
        );
        let on_custom = Attribute::new(
            "onmetrics",
            AttributeValue::listener(move |ev: CustomEvent| {
                let d = ev.data();
                log.set(format!("custom:{}:{}", d.name, d.data.len()));
            }),
            None,
            false,
        );
        let on_submit = elements::events::onsubmit(move |ev: SubmitEvent| {
            log.set(format!("submit:{}", ev.data().value));
        });
        let on_keydown = elements::events::onkeydown(move |ev: KeyEvent| {
            let d = ev.data();
            log.set(format!(
                "keydown:{}:{}:{}:{}:{}",
                d.key, d.meta, d.ctrl, d.shift, d.alt
            ));
        });

        let _ = VNode::empty();
        rsx! {
            view {
                test_id: "event-target",
                ..vec![on_scroll, on_focus, on_blur, on_longpress, on_custom, on_submit, on_keydown],
                text { "{log}" }
            }
        }
    }

    fn extract_target_and_text(bytes: &[u8]) -> (Option<NodeId>, Vec<String>) {
        let (_, cmds) = CommandDecoder::new(bytes).unwrap().decode_all().unwrap();
        let mut target = None;
        let mut texts = Vec::new();
        for cmd in cmds {
            match cmd {
                Command::SetRawProp {
                    node_id,
                    key,
                    value: RawPropValue::String(v),
                } if key == "testID" && v == "event-target" => {
                    target = Some(node_id);
                }
                Command::SetText { text, .. } => texts.push(text),
                _ => {}
            }
        }
        (target, texts)
    }

    #[test]
    fn dispatches_scroll_focus_blur_longpress_and_custom_via_ring_and_direct_ffi() {
        let mut surface = DioxusSurface::new(1, AllEventsApp);
        let (_, _, f1) = surface.render_frame(1);
        let (target_opt, _) = extract_target_and_text(&f1);
        let target = target_opt.expect("event-target node mounted");

        // 1. Scroll (EventRing)
        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 10,
            kind: NativeEventKind::Scroll,
            payload: EventPayload::Scroll {
                x: 12.0,
                y: 240.0,
                content_width: 393.0,
                content_height: 2400.0,
            },
        }]));
        let (_, _, f2) = surface.render_frame(2);
        assert!(extract_target_and_text(&f2)
            .1
            .contains(&"scroll:12,240,393,2400".to_owned()));

        // 2. Focus (EventRing)
        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 20,
            kind: NativeEventKind::Focus,
            payload: EventPayload::Empty,
        }]));
        let (_, _, f3) = surface.render_frame(3);
        assert!(extract_target_and_text(&f3).1.contains(&"focus".to_owned()));

        // 3. Blur (EventRing)
        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 30,
            kind: NativeEventKind::Blur,
            payload: EventPayload::Empty,
        }]));
        let (_, _, f4) = surface.render_frame(4);
        assert!(extract_target_and_text(&f4).1.contains(&"blur".to_owned()));

        // 4. LongPress (EventRing)
        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 40,
            kind: NativeEventKind::LongPress,
            payload: EventPayload::Point { x: 75.5, y: 32.25 },
        }]));
        let (_, _, f5) = surface.render_frame(5);
        assert!(extract_target_and_text(&f5)
            .1
            .contains(&"longpress:75.5,32.25".to_owned()));

        // 5. Custom (EventRing — with leading "on" prefix normalized cleanly)
        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 50,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "onmetrics".into(),
                data: vec![1, 2, 3, 4],
            },
        }]));
        let (_, _, f6) = surface.render_frame(6);
        assert!(extract_target_and_text(&f6)
            .1
            .contains(&"custom:onmetrics:4".to_owned()));

        // 6. Direct FFI path: Scroll, Focus, Blur, LongPress, Custom
        let direct_scroll = [NveDirectEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 60,
            kind: NativeEventKind::Scroll as u8,
            payload_tag: 2,
            name: NveStrSlice::EMPTY,
            x: 5.0,
            y: 99.0,
            width: 320.0,
            height: 1200.0,
            text: NveStrSlice::EMPTY,
            value: NveValue::null(),
        }];
        let batch_scroll = unsafe {
            NveEventBatchRef::from_raw(NveEventBatch {
                ptr: direct_scroll.as_ptr(),
                len: 1,
            })
        };
        assert!(surface.dispatch_events_direct(batch_scroll));
        let (_, _, f7) = surface.render_frame(7);
        assert!(extract_target_and_text(&f7)
            .1
            .contains(&"scroll:5,99,320,1200".to_owned()));

        let direct_focus = [NveDirectEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 70,
            kind: NativeEventKind::Focus as u8,
            payload_tag: 0,
            ..Default::default()
        }];
        let batch_focus = unsafe {
            NveEventBatchRef::from_raw(NveEventBatch {
                ptr: direct_focus.as_ptr(),
                len: 1,
            })
        };
        assert!(surface.dispatch_events_direct(batch_focus));
        let (_, _, f8) = surface.render_frame(8);
        assert!(extract_target_and_text(&f8).1.contains(&"focus".to_owned()));

        let direct_blur = [NveDirectEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 80,
            kind: NativeEventKind::Blur as u8,
            payload_tag: 0,
            ..Default::default()
        }];
        let batch_blur = unsafe {
            NveEventBatchRef::from_raw(NveEventBatch {
                ptr: direct_blur.as_ptr(),
                len: 1,
            })
        };
        assert!(surface.dispatch_events_direct(batch_blur));
        let (_, _, f9) = surface.render_frame(9);
        assert!(extract_target_and_text(&f9).1.contains(&"blur".to_owned()));

        let direct_lp = [NveDirectEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 90,
            kind: NativeEventKind::LongPress as u8,
            payload_tag: 1,
            x: 11.0,
            y: 22.0,
            ..Default::default()
        }];
        let batch_lp = unsafe {
            NveEventBatchRef::from_raw(NveEventBatch {
                ptr: direct_lp.as_ptr(),
                len: 1,
            })
        };
        assert!(surface.dispatch_events_direct(batch_lp));
        let (_, _, f10) = surface.render_frame(10);
        assert!(extract_target_and_text(&f10)
            .1
            .contains(&"longpress:11,22".to_owned()));

        let custom_bytes = [7u8, 8, 9];
        let direct_custom = [NveDirectEvent::custom(
            1,
            target,
            100,
            "metrics",
            NveValue::bytes(NveBufferSlice::from_slice(&custom_bytes)),
        )];
        let batch_custom = unsafe {
            NveEventBatchRef::from_raw(NveEventBatch {
                ptr: direct_custom.as_ptr(),
                len: 1,
            })
        };
        assert!(surface.dispatch_events_direct(batch_custom));
        let (_, _, f11) = surface.render_frame(11);
        assert!(extract_target_and_text(&f11)
            .1
            .contains(&"custom:metrics:3".to_owned()));

        // 7. Typed Submit & KeyDown (both EventRing & direct FFI; shorthand & JSON keydown)
        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 110,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "submit".into(),
                data: b"ship v2".to_vec(),
            },
        }]));
        let (_, _, f12) = surface.render_frame(12);
        assert!(extract_target_and_text(&f12)
            .1
            .contains(&"submit:ship v2".to_owned()));

        assert!(surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: target,
            timestamp_us: 120,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "keydown".into(),
                data: br#"{"key":"k","meta":true,"ctrl":false,"shift":true,"alt":false}"#.to_vec(),
            },
        }]));
        let (_, _, f13) = surface.render_frame(13);
        assert!(extract_target_and_text(&f13)
            .1
            .contains(&"keydown:k:true:false:true:false".to_owned()));

        let direct_submit = [NveDirectEvent::custom(
            1,
            target,
            130,
            "onsubmit",
            NveValue::str(NveStrSlice::from_str("direct-msg")),
        )];
        let batch_submit = unsafe {
            NveEventBatchRef::from_raw(NveEventBatch {
                ptr: direct_submit.as_ptr(),
                len: 1,
            })
        };
        assert!(surface.dispatch_events_direct(batch_submit));
        let (_, _, f14) = surface.render_frame(14);
        assert!(extract_target_and_text(&f14)
            .1
            .contains(&"submit:direct-msg".to_owned()));

        let direct_keydown = [NveDirectEvent::custom(
            1,
            target,
            140,
            "keydown",
            NveValue::str(NveStrSlice::from_str("Ctrl+Alt+Enter")),
        )];
        let batch_keydown = unsafe {
            NveEventBatchRef::from_raw(NveEventBatch {
                ptr: direct_keydown.as_ptr(),
                len: 1,
            })
        };
        assert!(surface.dispatch_events_direct(batch_keydown));
        let (_, _, f15) = surface.render_frame(15);
        assert!(extract_target_and_text(&f15)
            .1
            .contains(&"keydown:Enter:false:true:false:true".to_owned()));
    }
}
