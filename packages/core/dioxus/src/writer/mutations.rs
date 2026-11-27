//! `dioxus_core::WriteMutations` implementation and template instantiation for [`MutationTx`].

use dioxus_core::{AttributeValue, ElementId, Template, TemplateAttribute, TemplateNode, WriteMutations};
use nve_abi::{ComponentKind, EventMask, NodeId, RawPropValueRef};
use nve_components::resolve_tag;
use nve_components::style_patch::Display;
use nve_engine::RenderSink;

use crate::native_view::NATIVE_VIEW_TAG;
use crate::typed_style::TypedStyle;

use super::{MutationTx, NodeRecord};

impl<S: RenderSink> MutationTx<'_, S> {
    pub(super) fn instantiate_template_node(&mut self, tnode: &TemplateNode) -> NodeId {
        match tnode {
            TemplateNode::Element {
                tag,
                attrs,
                children,
                ..
            } => {
                let id = if *tag == NATIVE_VIEW_TAG {
                    self.writer.alloc_deferred_native_node()
                } else {
                    let (kind, custom) = resolve_tag(tag);
                    let id = self.writer.alloc_node(self.sink, kind, custom);
                    if matches!(*tag, "dialog" | "details" | "summary" | "form") {
                        self.sink
                            .set_raw_prop(id, "semanticTag", RawPropValueRef::String(tag));
                    }
                    id
                };
                for attr in *attrs {
                    if let TemplateAttribute::Static { name, value, .. } = attr {
                        self.writer.apply_str_attr(self.sink, id, name, value);
                    }
                }
                if !children.is_empty() {
                    let mut child_ids = Vec::with_capacity(children.len());
                    for child in *children {
                        let cid = self.instantiate_template_node(child);
                        if let Some(crec) = self.writer.nodes.get_mut(&cid) {
                            crec.set_parent(Some(id));
                        }
                        child_ids.push(cid);
                    }
                    if let Some(rec) = self.writer.nodes.get_mut(&id) {
                        rec.children = child_ids;
                    }
                    self.writer.dirty_children.push(id);
                }
                id
            }
            TemplateNode::Text { text } => {
                let id = self.writer.alloc_node(self.sink, ComponentKind::Text, None);
                self.writer.record_text(id, text);
                self.sink.set_text(id, text);
                id
            }
            TemplateNode::Dynamic { .. } => {
                // Placeholder slot replaced later by `replace_placeholder_with_nodes`.
                self.writer.alloc_node(self.sink, ComponentKind::View, None)
            }
        }
    }
}

impl<S: RenderSink> WriteMutations for MutationTx<'_, S> {
    fn append_children(&mut self, id: ElementId, m: usize) {
        let split = self.writer.stack.len().saturating_sub(m);
        let Some(parent_id) = self.writer.resolve_el(id) else {
            self.writer.stack.truncate(split);
            return;
        };
        for i in split..self.writer.stack.len() {
            let cid = self.writer.stack[i];
            self.writer.detach_from_parent(cid);
            if let Some(crec) = self.writer.nodes.get_mut(&cid) {
                crec.set_parent(Some(parent_id));
            }
        }
        if let Some(parent) = self.writer.nodes.get_mut(&parent_id) {
            parent.children.extend(self.writer.stack.drain(split..));
        } else {
            self.writer.stack.truncate(split);
        }
        self.writer.dirty_children.push(parent_id);
    }

    fn assign_node_id(&mut self, path: &'static [u8], id: ElementId) {
        let Some(&top) = self.writer.stack.last() else {
            return;
        };
        if let Some(target) = self.writer.walk_path(top, path) {
            self.writer.bind_element_id(target, id);
        }
    }

    fn create_placeholder(&mut self, id: ElementId) {
        let nid = self.writer.alloc_node(self.sink, ComponentKind::View, None);
        // Empty `if`/`match` arms must not occupy layout: as a plain `View` the placeholder is a
        // flex item and adds a spurious `gap` on every backend (Web CSS, Yoga/Skia).
        self.writer.apply_typed_style(
            self.sink,
            nid,
            TypedStyle::new().display(Display::None),
        );
        self.writer.bind_element_id(nid, id);
        self.writer.stack.push(nid);
    }

    fn create_text_node(&mut self, value: &str, id: ElementId) {
        let nid = self.writer.alloc_node(self.sink, ComponentKind::Text, None);
        self.writer.bind_element_id(nid, id);
        self.writer.record_text(nid, value);
        self.sink.set_text(nid, value);
        self.writer.stack.push(nid);
    }

    fn load_template(&mut self, template: Template, index: usize, id: ElementId) {
        if let Some(root_tpl) = template.roots.get(index) {
            let nid = self.instantiate_template_node(root_tpl);
            self.writer.bind_element_id(nid, id);
            self.writer.stack.push(nid);
        }
    }

    fn replace_node_with(&mut self, id: ElementId, m: usize) {
        let split = self.writer.stack.len().saturating_sub(m);
        if let Some(target_id) = self.writer.resolve_el(id) {
            self.writer
                .replace_node_in_parent_from_stack(self.sink, target_id, split);
        } else {
            self.writer.stack.truncate(split);
        }
    }

    fn replace_placeholder_with_nodes(&mut self, path: &'static [u8], m: usize) {
        let split = self.writer.stack.len().saturating_sub(m);
        let Some(&top) = split.checked_sub(1).and_then(|i| self.writer.stack.get(i)) else {
            self.writer.stack.truncate(split);
            return;
        };
        if let Some(placeholder) = self.writer.walk_path(top, path) {
            self.writer
                .replace_node_in_parent_from_stack(self.sink, placeholder, split);
        } else {
            self.writer.stack.truncate(split);
        }
    }

    fn insert_nodes_after(&mut self, id: ElementId, m: usize) {
        let split = self.writer.stack.len().saturating_sub(m);
        let Some(anchor) = self.writer.resolve_el(id) else {
            self.writer.stack.truncate(split);
            return;
        };
        let Some(parent_id) = self.writer.nodes.get(&anchor).and_then(NodeRecord::parent) else {
            self.writer.stack.truncate(split);
            return;
        };
        for i in split..self.writer.stack.len() {
            let n = self.writer.stack[i];
            self.writer.detach_from_parent(n);
            if let Some(rec) = self.writer.nodes.get_mut(&n) {
                rec.set_parent(Some(parent_id));
            }
        }
        if let Some(parent) = self.writer.nodes.get_mut(&parent_id) {
            if let Some(pos) = parent.children.iter().position(|&c| c == anchor) {
                parent
                    .children
                    .splice(pos + 1..pos + 1, self.writer.stack.drain(split..));
            } else {
                self.writer.stack.truncate(split);
            }
        } else {
            self.writer.stack.truncate(split);
        }
        self.writer.dirty_children.push(parent_id);
    }

    fn insert_nodes_before(&mut self, id: ElementId, m: usize) {
        let split = self.writer.stack.len().saturating_sub(m);
        let Some(anchor) = self.writer.resolve_el(id) else {
            self.writer.stack.truncate(split);
            return;
        };
        let Some(parent_id) = self.writer.nodes.get(&anchor).and_then(NodeRecord::parent) else {
            self.writer.stack.truncate(split);
            return;
        };
        for i in split..self.writer.stack.len() {
            let n = self.writer.stack[i];
            self.writer.detach_from_parent(n);
            if let Some(rec) = self.writer.nodes.get_mut(&n) {
                rec.set_parent(Some(parent_id));
            }
        }
        if let Some(parent) = self.writer.nodes.get_mut(&parent_id) {
            if let Some(pos) = parent.children.iter().position(|&c| c == anchor) {
                parent
                    .children
                    .splice(pos..pos, self.writer.stack.drain(split..));
            } else {
                self.writer.stack.truncate(split);
            }
        } else {
            self.writer.stack.truncate(split);
        }
        self.writer.dirty_children.push(parent_id);
    }

    fn set_attribute(
        &mut self,
        name: &'static str,
        _ns: Option<&'static str>,
        value: &AttributeValue,
        id: ElementId,
    ) {
        self.writer.set_attribute_impl(self.sink, name, value, id);
    }

    fn set_node_text(&mut self, value: &str, id: ElementId) {
        if let Some(node_id) = self.writer.resolve_el(id) {
            self.writer.record_text(node_id, value);
            self.sink.set_text(node_id, value);
        }
    }

    fn create_event_listener(&mut self, name: &'static str, id: ElementId) {
        let Some(node_id) = self.writer.resolve_el(id) else {
            return;
        };
        let bit = event_name_to_mask(name);
        if let Some(rec) = self.writer.nodes.get_mut(&node_id) {
            rec.event_mask |= bit;
            self.writer.dirty_events.push(node_id);
        }
    }

    fn remove_event_listener(&mut self, name: &'static str, id: ElementId) {
        let Some(node_id) = self.writer.resolve_el(id) else {
            return;
        };
        let bit = event_name_to_mask(name);
        if let Some(rec) = self.writer.nodes.get_mut(&node_id) {
            rec.event_mask &= !bit;
            self.writer.dirty_events.push(node_id);
        }
    }

    fn remove_node(&mut self, id: ElementId) {
        if let Some(node_id) = self.writer.resolve_el(id) {
            self.writer.detach_from_parent(node_id);
            self.writer.remove_subtree(self.sink, node_id);
        }
    }

    fn push_root(&mut self, id: ElementId) {
        if let Some(node_id) = self.writer.resolve_el(id) {
            self.writer.stack.push(node_id);
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
        "compositionstart"
        | "oncompositionstart"
        | "compositionupdate"
        | "oncompositionupdate"
        | "compositionend"
        | "oncompositionend" => EventMask::COMPOSITION,
        "keydown" | "onkeydown" => EventMask::KEY_DOWN,
        "keyup" | "onkeyup" => EventMask::KEY_UP,
        "submit" | "onsubmit" => EventMask::SUBMIT,
        "pointerdown" | "onpointerdown" => EventMask::POINTER_DOWN,
        "pointermove" | "onpointermove" => EventMask::POINTER_MOVE,
        "pointerup" | "onpointerup" => EventMask::POINTER_UP,
        "pointerenter" | "onpointerenter" => EventMask::POINTER_ENTER,
        "pointerleave" | "onpointerleave" => EventMask::POINTER_LEAVE,
        "wheel" | "onwheel" => EventMask::WHEEL,
        _ => EventMask::CUSTOM,
    }
}
