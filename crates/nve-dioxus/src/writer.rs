//! `NveMutationWriter` — bridges `dioxus_core::WriteMutations` directly into a batched
//! `nve_abi::CommandBuffer` with single-pass `DRAW_LIST_ATTR_PREFIX` -> `RawPropValue::Bytes`
//! decoding and persistent zero-alloc scratch buffers.

use nve_abi::hash::HashMap;

use dioxus_core::{AttributeValue, ElementId, Template, TemplateAttribute, TemplateNode, WriteMutations};
use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, EventMask, FrameId, LayoutProps, NodeId,
    RawPropValue, RawPropValueRef, StyleProps, SurfaceId, ROOT_NODE_ID,
};
use nve_components::{
    apply_typed_attr_with_raw, canonical_raw_style_key, resolve_tag, DrawListDecoder,
    DRAW_LIST_ATTR_PREFIX,
};
use nve_interop::native_view::{NativeViewHandleBinder, NATIVE_VIEW_KIND_PROP_KEY};

use crate::hot_patch;
use crate::native_view::{
    NativeBinaryProp, NativeHandleAttr, NATIVE_VIEW_HANDLE_ATTR, NATIVE_VIEW_KIND_ATTR,
    NATIVE_VIEW_TAG,
};

#[derive(Debug, Clone)]
pub(crate) struct NodeRecord {
    pub(crate) kind: ComponentKind,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    pub(crate) layout: LayoutProps,
    pub(crate) style: StyleProps,
    pub(crate) has_explicit_style: bool,
    pub(crate) font_weight: Option<String>,
    pub(crate) text_align: Option<String>,
    pub(crate) event_mask: u64,
    pub(crate) dioxus_id: Option<ElementId>,
    pub(crate) test_id: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) deferred_native_create: bool,
    pub(crate) native_kind: Option<String>,
    pub(crate) native_handle_binder: Option<NativeViewHandleBinder>,
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
            font_weight: None,
            text_align: None,
            event_mask: 0,
            dioxus_id: None,
            test_id: None,
            text: None,
            deferred_native_create: false,
            native_kind: None,
            native_handle_binder: None,
        }
    }
}

#[derive(Debug, Clone)]
struct TextStyleWalkEntry {
    node_id: NodeId,
    color: ColorRgba,
    font_size: f32,
    font_weight: Option<String>,
    text_align: Option<String>,
}

/// Bridges `dioxus_core::WriteMutations` into a single batched `nve_abi::CommandBuffer`.
pub struct NveMutationWriter {
    surface_id: SurfaceId,
    next_node_id: NodeId,
    /// Maps Dioxus `ElementId` -> NVE `NodeId`. (`ElementId(0)` == `ROOT_NODE_ID`).
    el_to_node: HashMap<ElementId, NodeId>,
    node_to_el: HashMap<NodeId, ElementId>,
    pub(crate) nodes: HashMap<NodeId, NodeRecord>,
    stack: Vec<NodeId>,
    /// Persistent scratch buffers reused across frames (cleared + sorted/deduped in place).
    style_sources: Vec<NodeId>,
    style_walk_stack: Vec<TextStyleWalkEntry>,
    dirty_children: Vec<NodeId>,
    dirty_layout: Vec<NodeId>,
    dirty_style: Vec<NodeId>,
    dirty_text_props: Vec<NodeId>,
    dirty_events: Vec<NodeId>,
    deferred_native_nodes: Vec<NodeId>,
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

    pub fn finish_frame(&mut self, sync_mount: bool) -> &[u8] {
        // Ensure any `<NativeView>` node whose `__nve_native_kind` attribute was omitted still
        // emits `CreateNode` before layout/style/children ops.
        if !self.deferred_native_nodes.is_empty() {
            let pending = std::mem::take(&mut self.deferred_native_nodes);
            for nid in pending {
                self.ensure_native_node_created(nid, None);
            }
        }
        // Propagate inherited text_color, font_size, font_weight, and text_align from styled Text
        // parents to unstyled child Text leaves (Dioxus rsx! emits `text { style: "...", "leaf" }`
        // as parent Text -> child Text).
        if !self.dirty_children.is_empty()
            || !self.dirty_style.is_empty()
            || !self.dirty_text_props.is_empty()
        {
            self.style_sources.clear();
            self.style_sources.extend_from_slice(&self.dirty_children);
            self.style_sources.extend_from_slice(&self.dirty_style);
            self.style_sources.extend_from_slice(&self.dirty_text_props);
            self.style_sources.sort_unstable();
            self.style_sources.dedup();
            for i in 0..self.style_sources.len() {
                let pid = self.style_sources[i];
                self.propagate_text_style_down(pid);
            }
            self.style_sources.clear();
            self.dirty_text_props.clear();
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

    fn alloc_deferred_native_node(&mut self) -> NodeId {
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

    fn ensure_native_node_created(&mut self, node_id: NodeId, kind_override: Option<&str>) {
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
        let root_font_weight = root.font_weight.clone();
        let root_text_align = root.text_align.clone();
        for &cid in &root.children {
            self.style_walk_stack.push(TextStyleWalkEntry {
                node_id: cid,
                color: root_color,
                font_size: root_font_size,
                font_weight: root_font_weight.clone(),
                text_align: root_text_align.clone(),
            });
        }

        while let Some(entry) = self.style_walk_stack.pop() {
            let cid = entry.node_id;
            if let Some(child) = self.nodes.get_mut(&cid)
                && child.kind == ComponentKind::Text
            {
                if !child.has_explicit_style
                    && (child.style.text_color != entry.color
                        || (child.style.font_size - entry.font_size).abs() > f32::EPSILON)
                {
                    child.style.text_color = entry.color;
                    child.style.font_size = entry.font_size;
                    self.dirty_style.push(cid);
                }
                if child.font_weight.is_none()
                    && let Some(ref w) = entry.font_weight
                {
                    child.font_weight = Some(w.clone());
                    self.builder
                        .set_raw_prop(cid, "font-weight", &RawPropValue::String(w.clone()));
                }
                if child.text_align.is_none()
                    && let Some(ref a) = entry.text_align
                {
                    child.text_align = Some(a.clone());
                    self.builder
                        .set_raw_prop(cid, "text-align", &RawPropValue::String(a.clone()));
                }
                let next_color = child.style.text_color;
                let next_font_size = child.style.font_size;
                let next_weight = child.font_weight.clone();
                let next_align = child.text_align.clone();
                for &grandchild in &child.children {
                    self.style_walk_stack.push(TextStyleWalkEntry {
                        node_id: grandchild,
                        color: next_color,
                        font_size: next_font_size,
                        font_weight: next_weight.clone(),
                        text_align: next_align.clone(),
                    });
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
                let id = if *tag == NATIVE_VIEW_TAG {
                    self.alloc_deferred_native_node()
                } else {
                    let (kind, custom) = resolve_tag(tag);
                    self.alloc_node(kind, custom)
                };
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
            let mut text_prop_changed = false;
            let builder = &mut self.builder;
            let handled = apply_typed_attr_with_raw(
                &mut rec.layout,
                &mut rec.style,
                name,
                value,
                |raw_k, raw_v| {
                    match raw_k {
                        "font-weight" if rec.font_weight.as_deref() != Some(raw_v) => {
                            rec.font_weight = Some(raw_v.to_owned());
                            text_prop_changed = true;
                        }
                        "text-align" if rec.text_align.as_deref() != Some(raw_v) => {
                            rec.text_align = Some(raw_v.to_owned());
                            text_prop_changed = true;
                        }
                        _ => {}
                    }
                    builder.set_raw_prop(node_id, raw_k, &RawPropValue::String(raw_v.to_owned()));
                },
            );
            if handled {
                if rec.layout != prev_layout {
                    self.dirty_layout.push(node_id);
                }
                if rec.style != prev_style {
                    rec.has_explicit_style = true;
                    self.dirty_style.push(node_id);
                }
                if text_prop_changed {
                    self.dirty_text_props.push(node_id);
                }
                return;
            }
            if let Some(canonical) = canonical_raw_style_key(name) {
                match canonical {
                    "font-weight" => {
                        rec.font_weight = Some(value.to_owned());
                        self.dirty_text_props.push(node_id);
                    }
                    "text-align" => {
                        rec.text_align = Some(value.to_owned());
                        self.dirty_text_props.push(node_id);
                    }
                    _ => {}
                }
            }
        }
        // Decode `nve-draw1;b64,...` attributes directly into binary `RawPropValue::Bytes`
        // in one pass — eliminating any full-frame CommandBuffer decode+re-encode pass.
        if value.starts_with(DRAW_LIST_ATTR_PREFIX)
            && let Ok(raw_draw_bytes) = DrawListDecoder::decode_attr_bytes(value)
        {
            self.builder
                .set_raw_prop(node_id, name, &RawPropValue::Bytes(raw_draw_bytes));
            return;
        }
        let prop_name = canonical_raw_style_key(name)
            .or_else(|| canonical_a11y_attr_key(name))
            .unwrap_or(name);
        self.builder
            .set_raw_prop(node_id, prop_name, &RawPropValue::String(value.to_owned()));
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
        if name == NATIVE_VIEW_KIND_ATTR {
            if let AttributeValue::Text(kind_str) = value {
                let was_deferred = self
                    .nodes
                    .get(&node_id)
                    .is_some_and(|r| r.deferred_native_create);
                if was_deferred {
                    self.ensure_native_node_created(node_id, Some(kind_str));
                    self.builder
                        .set_raw_prop_str(node_id, NATIVE_VIEW_KIND_PROP_KEY, kind_str);
                } else if let Some(rec) = self.nodes.get_mut(&node_id)
                    && rec.native_kind.as_deref() != Some(kind_str.as_str())
                {
                    rec.native_kind = Some(kind_str.clone());
                    self.builder
                        .set_raw_prop_str(node_id, NATIVE_VIEW_KIND_PROP_KEY, kind_str);
                }
            }
            return;
        }
        if self
            .nodes
            .get(&node_id)
            .is_some_and(|r| r.deferred_native_create)
        {
            self.ensure_native_node_created(node_id, None);
        }
        match value {
            AttributeValue::Text(s) => self.apply_str_attr(node_id, name, s),
            AttributeValue::Float(f) => {
                let mut s = String::new();
                nve_abi::num::write_f64(&mut s, *f);
                self.apply_str_attr(node_id, name, &s);
            }
            AttributeValue::Int(i) => {
                let mut s = String::new();
                nve_abi::num::write_i64(&mut s, *i);
                self.apply_str_attr(node_id, name, &s);
            }
            AttributeValue::Bool(b) => {
                let prop_name = canonical_a11y_attr_key(name).unwrap_or(name);
                self.builder
                    .set_raw_prop(node_id, prop_name, &RawPropValue::Bool(*b));
            }
            AttributeValue::None => {
                if hot_patch::is_test_id_attr_name(name) {
                    self.clear_test_id(node_id);
                }
                let prop_name = canonical_raw_style_key(name)
                    .or_else(|| canonical_a11y_attr_key(name))
                    .unwrap_or(name);
                self.builder
                    .set_raw_prop(node_id, prop_name, &RawPropValue::Null);
            }
            AttributeValue::Any(any) => {
                if let Some(bin) = any.as_any().downcast_ref::<NativeBinaryProp>() {
                    self.builder.set_raw_prop_ref(
                        node_id,
                        name,
                        RawPropValueRef::Bytes(&bin.0),
                    );
                } else if name == NATIVE_VIEW_HANDLE_ATTR
                    && let Some(handle_attr) = any.as_any().downcast_ref::<NativeHandleAttr>()
                {
                    let surface_id = self.surface_id;
                    if let Some(rec) = self.nodes.get_mut(&node_id) {
                        if let Some(prev) = rec.native_handle_binder.take()
                            && !handle_attr
                                .0
                                .as_ref()
                                .is_some_and(|next| next.ptr_eq(&prev))
                        {
                            prev.unbind();
                        }
                        if let Some(ref binder) = handle_attr.0 {
                            binder.bind(surface_id, node_id);
                            rec.native_handle_binder = Some(binder.clone());
                        }
                    }
                }
            }
            AttributeValue::Listener(_) => {}
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

/// Canonicalizes ARIA state and focus attribute keys (kebab-case, snake_case, or camelCase)
/// into their standard camelCase raw-prop wire names with zero binary ABI changes.
#[inline]
pub(crate) fn canonical_a11y_attr_key(name: &str) -> Option<&'static str> {
    match name {
        "tabIndex" | "tab-index" | "tab_index" => Some("tabIndex"),
        "ariaChecked" | "aria-checked" | "aria_checked" => Some("ariaChecked"),
        "ariaSelected" | "aria-selected" | "aria_selected" => Some("ariaSelected"),
        "ariaExpanded" | "aria-expanded" | "aria_expanded" => Some("ariaExpanded"),
        "ariaModal" | "aria-modal" | "aria_modal" => Some("ariaModal"),
        "ariaLive" | "aria-live" | "aria_live" => Some("ariaLive"),
        "ariaControls" | "aria-controls" | "aria_controls" => Some("ariaControls"),
        "ariaLabelledBy" | "aria-labelledby" | "aria_labelledby" => Some("ariaLabelledBy"),
        "ariaDescribedBy" | "aria-describedby" | "aria_describedby" => Some("ariaDescribedBy"),
        "ariaValueNow" | "aria-valuenow" | "aria_valuenow" => Some("ariaValueNow"),
        "ariaValueMin" | "aria-valuemin" | "aria_valuemin" => Some("ariaValueMin"),
        "ariaValueMax" | "aria-valuemax" | "aria_valuemax" => Some("ariaValueMax"),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{Command, CommandDecoder};

    #[test]
    fn inline_style_forwards_font_weight_text_align_box_shadow_and_inherits_text_props() {
        let mut writer = NveMutationWriter::new(1);
        writer.begin_frame(1, 1);

        let card_id = writer.alloc_node(ComponentKind::View, None);
        writer.apply_str_attr(
            card_id,
            "style",
            "background-color: #1e293b; border-radius: 10px; box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);",
        );

        let parent_text = writer.alloc_node(ComponentKind::Text, None);
        writer.apply_str_attr(
            parent_text,
            "style",
            "font-size: 18px; font-weight: 700; text-align: center; color: #ffffff;",
        );
        let leaf_text = writer.alloc_node(ComponentKind::Text, None);
        writer.record_text(leaf_text, "Title");
        writer.builder.set_text(leaf_text, "Title");

        writer.nodes.get_mut(&parent_text).unwrap().children.push(leaf_text);
        writer.nodes.get_mut(&leaf_text).unwrap().parent = Some(parent_text);
        writer.dirty_children.push(parent_text);

        let bytes = writer.finish_frame(true);
        let (_, cmds) = CommandDecoder::new(bytes)
            .expect("valid header")
            .decode_all()
            .expect("valid command buffer");

        let has_raw = |nid: NodeId, k: &str, expected: &str| {
            cmds.iter().any(|c| {
                matches!(
                    c,
                    Command::SetRawProp {
                        node_id,
                        key,
                        value: RawPropValue::String(v)
                    } if *node_id == nid && key == k && v == expected
                )
            })
        };

        assert!(has_raw(card_id, "box-shadow", "0 4px 12px rgba(0, 0, 0, 0.35)"));
        assert!(has_raw(parent_text, "font-weight", "700"));
        assert!(has_raw(parent_text, "text-align", "center"));
        assert!(has_raw(leaf_text, "font-weight", "700"));
        assert!(has_raw(leaf_text, "text-align", "center"));
    }
}
