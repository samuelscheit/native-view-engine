//! `dioxus_core::WriteMutations` implementation over [`StoreMutationTx`].

use dioxus_core::{AttributeValue, ElementId, Template, WriteMutations};
use nve_components::tables::TextContent;

use super::{event_name_to_mask, StoreMutationTx};

impl StoreMutationTx<'_, '_> {
    fn drain_incoming_from_stack(&mut self, m: usize) {
        let split = self.stack.len().saturating_sub(m);
        self.incoming_scratch.clear();
        self.incoming_scratch.extend(self.stack.drain(split..));
    }

    fn discard_incoming_scratch(&mut self) {
        let mut idx = 0;
        while idx < self.incoming_scratch.len() {
            let child = self.incoming_scratch[idx];
            idx += 1;
            self.remove_subtree(child);
        }
        self.incoming_scratch.clear();
    }

    fn detach_incoming_from_parents(&mut self) {
        for i in 0..self.incoming_scratch.len() {
            let child = self.incoming_scratch[i];
            self.detach_from_parent(child);
        }
    }
}

impl WriteMutations for StoreMutationTx<'_, '_> {
    fn append_children(&mut self, id: ElementId, m: usize) {
        self.drain_incoming_from_stack(m);
        let Some(parent) = self.resolve_el(id) else {
            self.discard_incoming_scratch();
            return;
        };
        self.detach_incoming_from_parents();
        self.child_scratch.clear();
        self.child_scratch
            .extend_from_slice(self.tx.store().children(parent));
        self.child_scratch.extend_from_slice(self.incoming_scratch);
        self.tx.set_children(parent, self.child_scratch);
        self.incoming_scratch.clear();
    }

    fn assign_node_id(&mut self, path: &'static [u8], id: ElementId) {
        let Some(&top) = self.stack.last() else {
            return;
        };
        if let Some(target) = self.walk_path(top, path) {
            self.bind_element(id, target);
        }
    }

    fn create_placeholder(&mut self, id: ElementId) {
        let node = self.tx.create();
        self.bind_element(id, node);
        self.stack.push(node);
    }

    fn create_text_node(&mut self, value: &str, id: ElementId) {
        let node = self.tx.create();
        self.tx.edit::<TextContent>(node).text(value);
        self.bind_element(id, node);
        self.stack.push(node);
    }

    fn load_template(&mut self, template: Template, index: usize, id: ElementId) {
        let node = if let Some(root_node) = template.roots.get(index) {
            self.instantiate_template_node(root_node)
        } else {
            self.tx.create()
        };
        self.bind_element(id, node);
        self.stack.push(node);
    }

    fn replace_node_with(&mut self, id: ElementId, m: usize) {
        self.drain_incoming_from_stack(m);
        let Some(old) = self.resolve_el(id) else {
            self.discard_incoming_scratch();
            return;
        };
        let Some(parent) = self.tx.store().parent(old) else {
            self.discard_incoming_scratch();
            self.remove_subtree(old);
            return;
        };
        self.detach_incoming_from_parents();
        self.child_scratch.clear();
        self.child_scratch
            .extend_from_slice(self.tx.store().children(parent));
        if let Some(pos) = self.child_scratch.iter().position(|&c| c == old) {
            self.child_scratch
                .splice(pos..=pos, self.incoming_scratch.iter().copied());
            self.tx.set_children(parent, self.child_scratch);
            self.incoming_scratch.clear();
        } else {
            self.discard_incoming_scratch();
        }
        self.remove_subtree(old);
    }

    fn replace_placeholder_with_nodes(&mut self, path: &'static [u8], m: usize) {
        let split = self.stack.len().saturating_sub(m);
        if split == 0 {
            self.drain_incoming_from_stack(m);
            self.discard_incoming_scratch();
            return;
        }
        let template_root = self.stack[split - 1];
        self.drain_incoming_from_stack(m);
        let Some(placeholder) = self.walk_path(template_root, path) else {
            self.discard_incoming_scratch();
            return;
        };
        let Some(parent) = self.tx.store().parent(placeholder) else {
            self.discard_incoming_scratch();
            return;
        };
        self.detach_incoming_from_parents();
        self.child_scratch.clear();
        self.child_scratch
            .extend_from_slice(self.tx.store().children(parent));
        if let Some(pos) = self.child_scratch.iter().position(|&c| c == placeholder) {
            self.child_scratch
                .splice(pos..=pos, self.incoming_scratch.iter().copied());
            self.tx.set_children(parent, self.child_scratch);
            self.incoming_scratch.clear();
        } else {
            self.discard_incoming_scratch();
        }
        self.remove_subtree(placeholder);
    }

    fn insert_nodes_after(&mut self, id: ElementId, m: usize) {
        self.drain_incoming_from_stack(m);
        let Some(anchor) = self.resolve_el(id) else {
            self.discard_incoming_scratch();
            return;
        };
        let Some(parent) = self.tx.store().parent(anchor) else {
            self.discard_incoming_scratch();
            return;
        };
        self.detach_incoming_from_parents();
        self.child_scratch.clear();
        self.child_scratch
            .extend_from_slice(self.tx.store().children(parent));
        if let Some(pos) = self.child_scratch.iter().position(|&c| c == anchor) {
            self.child_scratch
                .splice(pos + 1..pos + 1, self.incoming_scratch.iter().copied());
            self.tx.set_children(parent, self.child_scratch);
            self.incoming_scratch.clear();
        } else {
            self.discard_incoming_scratch();
        }
    }

    fn insert_nodes_before(&mut self, id: ElementId, m: usize) {
        self.drain_incoming_from_stack(m);
        let Some(anchor) = self.resolve_el(id) else {
            self.discard_incoming_scratch();
            return;
        };
        let Some(parent) = self.tx.store().parent(anchor) else {
            self.discard_incoming_scratch();
            return;
        };
        self.detach_incoming_from_parents();
        self.child_scratch.clear();
        self.child_scratch
            .extend_from_slice(self.tx.store().children(parent));
        if let Some(pos) = self.child_scratch.iter().position(|&c| c == anchor) {
            self.child_scratch
                .splice(pos..pos, self.incoming_scratch.iter().copied());
            self.tx.set_children(parent, self.child_scratch);
            self.incoming_scratch.clear();
        } else {
            self.discard_incoming_scratch();
        }
    }

    fn set_attribute(
        &mut self,
        name: &'static str,
        _ns: Option<&'static str>,
        value: &AttributeValue,
        id: ElementId,
    ) {
        self.set_attribute_impl(name, value, id);
    }

    fn set_node_text(&mut self, value: &str, id: ElementId) {
        if let Some(node) = self.resolve_el(id) {
            self.tx.edit::<TextContent>(node).text(value);
        }
    }

    fn create_event_listener(&mut self, name: &'static str, id: ElementId) {
        if let Some(node) = self.resolve_el(id) {
            self.update_event_mask(node, event_name_to_mask(name), true);
        }
    }

    fn remove_event_listener(&mut self, name: &'static str, id: ElementId) {
        if let Some(node) = self.resolve_el(id) {
            self.update_event_mask(node, event_name_to_mask(name), false);
        }
    }

    fn remove_node(&mut self, id: ElementId) {
        if let Some(node) = self.resolve_el(id) {
            self.detach_from_parent(node);
            self.remove_subtree(node);
        }
    }

    fn push_root(&mut self, id: ElementId) {
        if let Some(node) = self.resolve_el(id) {
            self.stack.push(node);
        }
    }
}
