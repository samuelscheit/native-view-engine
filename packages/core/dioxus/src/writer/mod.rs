//! `NveMutationWriter` & ephemeral `StoreMutationTx<'_>` — bridges `dioxus_core::WriteMutations`
//! directly into an owned [`NodeStore`] via [`nve_store::Transaction`] with zero `NodeRecord`
//! mirroring, zero `Box<NodeExtras>`, zero `ComponentKind`, and zero `set_raw_prop`.

mod attributes;
mod current_color;
mod element_map;
mod identity_attrs;
mod listeners;
mod mutations;
mod standalone_style;
#[cfg(test)]
mod tests;

use dioxus_core::{ElementId, TemplateAttribute, TemplateNode};
use nve_abi::hash::{HashMap, HashSet};
use nve_abi::SurfaceId;
use nve_tables::interaction::CursorKind;
use nve_tables::tables::{
    self, AriaRole, Image, Interaction, Link, Scroll, TextContent, TextInput,
};
use nve_store::{ChangeSet, NodeId, NodeStore, Transaction};

use crate::typed_style::TypedStyle;
use element_map::ElementMap;
pub(crate) use standalone_style::{StandaloneStyleRef, StandaloneStyleVal};
use standalone_style::{BufferPool, StandaloneStyleList, TableAttrList};

/// Bridges Dioxus 0.7 `WriteMutations` straight into a single-owner [`NodeStore`].
pub struct NveMutationWriter {
    pub(crate) surface_id: SurfaceId,
    pub(crate) store: NodeStore,
    pub(crate) changes: ChangeSet,
    pub(crate) elements: ElementMap,
    /// Dioxus mutation stack; also the scratch stack of `remove_subtree` and template
    /// instantiation, which push above the live entries and truncate back (split-at-len).
    pub(crate) stack: Vec<NodeId>,
    pub(crate) child_scratch: Vec<NodeId>,
    pub(crate) incoming_scratch: Vec<NodeId>,
    // The four per-node `HashMap`s below are sparse (only nodes with table attrs or styles) and
    // keyed by generation-tagged `NodeId`; dense per-slot tables would make them O(1) loads like
    // `ElementMap` (docs/BACKLOG.md "Dense writer side tables").
    /// Tracks active [`crate::TableAttr`] writers per `(node, attr_name)` so multiple table attrs can
    /// coexist on one node and `AttributeValue::None` / replacement can call `clear_from`.
    pub(crate) attached_tables: HashMap<NodeId, TableAttrList>,
    /// Tracks active [`TypedStyle`] so inline `style` overrides merge cleanly and restore on clear.
    pub(crate) typed_styles: HashMap<NodeId, TypedStyle>,
    /// Tracks active inline `style="..."` override strings and their accepted CSS property bitmask.
    pub(crate) inline_styles: HashMap<NodeId, (String, u64)>,
    /// Tracks standalone CSS/style attributes (`width`, `color`, `opacity`, etc.) per node.
    pub(crate) standalone_style_attrs: HashMap<NodeId, StandaloneStyleList>,
    /// Nodes whose winning border color is CSS `currentColor` (`current_color.rs`).
    pub(crate) current_color_borders: HashSet<NodeId>,
    /// Recycled style lists and strings of removed nodes.
    pub(crate) buffer_pool: BufferPool,
    /// Dense node-index flags for non-table Dioxus listeners (bit 0 = `ontableevent`, bit 1 = custom).
    pub(crate) aux_event_flags: Vec<u8>,
    #[cfg(test)]
    pub(crate) standalone_replay_count: usize,
    #[cfg(test)]
    pub(crate) inline_reapply_count: usize,
}

impl NveMutationWriter {
    pub fn new(surface_id: SurfaceId) -> Self {
        tables::register_core_tables();
        let mut store = NodeStore::new();
        store.set_realization_order(tables::core_realization_order());
        Self {
            surface_id,
            store,
            changes: ChangeSet::default(),
            elements: ElementMap::with_root(),
            stack: Vec::with_capacity(64),
            child_scratch: Vec::with_capacity(32),
            incoming_scratch: Vec::with_capacity(32),
            attached_tables: HashMap::default(),
            typed_styles: HashMap::default(),
            inline_styles: HashMap::default(),
            standalone_style_attrs: HashMap::default(),
            current_color_borders: HashSet::default(),
            buffer_pool: BufferPool::default(),
            aux_event_flags: Vec::new(),
            #[cfg(test)]
            standalone_replay_count: 0,
            #[cfg(test)]
            inline_reapply_count: 0,
        }
    }

    #[inline]
    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    #[inline]
    pub fn store(&self) -> &NodeStore {
        &self.store
    }

    #[inline]
    pub fn store_mut(&mut self) -> &mut NodeStore {
        &mut self.store
    }

    #[inline]
    pub fn last_changes(&self) -> &ChangeSet {
        &self.changes
    }

    #[inline]
    pub fn resolve_el(&self, id: ElementId) -> Option<NodeId> {
        self.elements.node_of(id)
    }

    #[inline]
    pub fn element_id_of(&self, node_id: NodeId) -> Option<ElementId> {
        self.elements.element_of(node_id)
    }

    /// Runs `f` inside a single [`Transaction`] on the owned [`NodeStore`] and commits the
    /// resulting delta into `self.changes`.
    pub fn with_transaction<R>(
        &mut self,
        f: impl FnOnce(&mut StoreMutationTx<'_, '_>) -> R,
    ) -> R {
        let Self {
            store,
            changes,
            elements,
            stack,
            child_scratch,
            incoming_scratch,
            attached_tables,
            typed_styles,
            inline_styles,
            standalone_style_attrs,
            current_color_borders,
            buffer_pool,
            aux_event_flags,
            #[cfg(test)]
            standalone_replay_count,
            #[cfg(test)]
            inline_reapply_count,
            ..
        } = self;

        let mut tx = store.tx();
        let out = {
            let mut mtx = StoreMutationTx {
                tx: &mut tx,
                elements,
                stack,
                child_scratch,
                incoming_scratch,
                attached_tables,
                typed_styles,
                inline_styles,
                standalone_style_attrs,
                current_color_borders,
                buffer_pool,
                aux_event_flags,
                #[cfg(test)]
                standalone_replay_count,
                #[cfg(test)]
                inline_reapply_count,
            };
            let out = f(&mut mtx);
            if !mtx.current_color_borders.is_empty() {
                mtx.resolve_current_color_borders();
            }
            out
        };
        tx.commit_into(changes);
        out
    }
}

/// Ephemeral per-frame Dioxus `WriteMutations` adapter borrowing [`Transaction`] directly.
pub struct StoreMutationTx<'s, 'a> {
    pub(crate) tx: &'a mut Transaction<'s>,
    pub(crate) elements: &'a mut ElementMap,
    pub(crate) stack: &'a mut Vec<NodeId>,
    pub(crate) child_scratch: &'a mut Vec<NodeId>,
    pub(crate) incoming_scratch: &'a mut Vec<NodeId>,
    pub(crate) attached_tables: &'a mut HashMap<NodeId, TableAttrList>,
    pub(crate) typed_styles: &'a mut HashMap<NodeId, TypedStyle>,
    pub(crate) inline_styles: &'a mut HashMap<NodeId, (String, u64)>,
    pub(crate) standalone_style_attrs: &'a mut HashMap<NodeId, StandaloneStyleList>,
    pub(crate) current_color_borders: &'a mut HashSet<NodeId>,
    pub(crate) buffer_pool: &'a mut BufferPool,
    pub(crate) aux_event_flags: &'a mut Vec<u8>,
    #[cfg(test)]
    pub(crate) standalone_replay_count: &'a mut usize,
    #[cfg(test)]
    pub(crate) inline_reapply_count: &'a mut usize,
}

impl StoreMutationTx<'_, '_> {
    #[inline]
    pub(crate) fn bind_element(&mut self, el: ElementId, node: NodeId) {
        self.elements.bind(el, node);
    }

    #[inline]
    pub(crate) fn resolve_el(&self, id: ElementId) -> Option<NodeId> {
        self.elements.node_of(id)
    }

    #[inline]
    pub(crate) fn record_standalone_style(
        &mut self,
        node: NodeId,
        name: &str,
        val: StandaloneStyleRef<'_>,
    ) -> usize {
        let pool = &mut *self.buffer_pool;
        let list = self
            .standalone_style_attrs
            .entry(node)
            .or_insert_with(|| pool.list());
        if let Some(pos) = list.iter().position(|(k, _)| k == name) {
            pool.assign(&mut list[pos].1, val);
            return pos;
        }
        let rank = nve_tables::css_key_rank(name);
        let pos = list
            .iter()
            .rposition(|(k, _)| nve_tables::css_key_rank(k) <= rank)
            .map_or(0, |i| i + 1);
        list.insert(pos, (pool.string(name), pool.value(val)));
        pos
    }

    /// Removes a recorded standalone style attribute. Returns `true` iff an entry was removed.
    pub(crate) fn remove_standalone_style(&mut self, node: NodeId, name: &str) -> bool {
        let Some(list) = self.standalone_style_attrs.get_mut(&node) else {
            return false;
        };
        let Some(pos) = list.iter().position(|(k, _)| k == name) else {
            return false;
        };
        self.buffer_pool.recycle_entry(list.remove(pos));
        if list.is_empty()
            && let Some(list) = self.standalone_style_attrs.remove(&node)
        {
            self.buffer_pool.recycle_list(list);
        }
        true
    }

    #[inline]
    fn replay_one_standalone_entry(
        &mut self,
        node: NodeId,
        k: &str,
        val: &StandaloneStyleVal,
        layers: u8,
    ) {
        #[cfg(test)]
        {
            *self.standalone_replay_count += 1;
        }
        match val {
            StandaloneStyleVal::Float(f) => {
                let _ = self.apply_numeric_attr_untracked(node, k, *f);
            }
            StandaloneStyleVal::Int(i) => {
                self.tx
                    .edit::<tables::Visual>(node)
                    .z_index((*i).clamp(i32::MIN as i64, i32::MAX as i64) as i32);
            }
            StandaloneStyleVal::Str(s) => {
                nve_tables::apply_css_attr_layers_to_tx(self.tx, node, k, s, layers);
            }
        }
    }

    /// Replays standalone style attributes and inline `style="..."` for the specified `layers`
    /// in canonical precedence order: `standalone` < `inline style`.
    pub(crate) fn replay_standalone_and_inline(&mut self, node: NodeId, layers: u8) {
        if layers == 0 {
            return;
        }
        if let Some(saved) = self.standalone_style_attrs.remove(&node) {
            for (k, val) in &saved {
                let entry_layer = nve_tables::css_attr_layer(k);
                debug_assert!(entry_layer != 0, "standalone style key `{k}` must have a non-zero layer");
                if (entry_layer & layers) == 0 {
                    continue;
                }
                self.replay_one_standalone_entry(node, k, val, layers);
            }
            self.standalone_style_attrs.insert(node, saved);
        }
        self.reapply_inline_style(node, layers);
    }

    #[inline]
    pub(crate) fn reapply_inline_style(&mut self, node: NodeId, layers: u8) {
        if layers != 0
            && let Some((inline, mask)) = self.inline_styles.get(&node)
            && (nve_tables::prop_bits_to_layers(*mask) & layers) != 0
        {
            #[cfg(test)]
            {
                *self.inline_reapply_count += 1;
            }
            nve_tables::apply_css_attr_layers_to_tx(self.tx, node, "style", inline, layers);
        }
    }

    #[inline]
    pub(crate) fn finish_standalone_style(
        &mut self,
        node: NodeId,
        name: &str,
        val: StandaloneStyleRef<'_>,
    ) {
        let pos = self.record_standalone_style(node, name, val);
        if current_color::key_sets_border_color(name) {
            self.refresh_current_color_border(node);
        }
        let bits = nve_tables::css_key_bits(name);
        debug_assert!(bits.is_some(), "standalone style key `{name}` must have css_key_bits");
        let bits = bits.unwrap_or(0);
        let mut affected_bits = bits;

        // Read-only scan of trailing entries for transitive property-bit overlap; avoids HashMap
        // churn and re-parsing unrelated trailing attrs on 120 Hz standalone updates.
        let has_overlap = self.standalone_style_attrs.get(&node).is_some_and(|list| {
            let mut scan_bits = bits;
            let mut overlapped = false;
            for (k, _) in list.get(pos + 1..).unwrap_or(&[]) {
                let entry_bits = nve_tables::css_key_bits(k).unwrap_or(0);
                if (entry_bits & scan_bits) != 0 {
                    overlapped = true;
                    scan_bits |= entry_bits;
                }
            }
            overlapped
        });

        if has_overlap
            && let Some(saved) = self.standalone_style_attrs.remove(&node)
        {
            for (k, val) in saved.get(pos + 1..).unwrap_or(&[]) {
                let entry_bits = nve_tables::css_key_bits(k).unwrap_or(0);
                if (entry_bits & affected_bits) == 0 {
                    continue;
                }
                affected_bits |= entry_bits;
                let entry_layer = nve_tables::prop_bits_to_layers(entry_bits);
                self.replay_one_standalone_entry(node, k, val, entry_layer);
            }
            self.standalone_style_attrs.insert(node, saved);
        }

        let inline_layers = self.inline_styles.get(&node).map_or(0, |&(_, mask)| {
            nve_tables::prop_bits_to_layers(mask & affected_bits)
        });
        if inline_layers != 0 {
            self.reapply_inline_style(node, inline_layers);
        }
    }

    /// Updates `node`'s inline `style="..."` string in a single parse pass, preserving the
    /// `effect::COMPOSITE` fast lane whenever no previously accepted CSS property is dropped.
    pub(crate) fn apply_inline_style_attr(&mut self, node: NodeId, value: &str) {
        let prev_mask = match self.inline_styles.get(&node) {
            Some((prev_str, _)) if prev_str == value => return,
            Some(&(_, mask)) => mask,
            None => 0,
        };
        let next_mask = nve_tables::apply_inline_style_layers_to_tx(
            self.tx,
            node,
            value,
            nve_tables::STYLE_LAYER_ALL,
        );
        match self.inline_styles.get_mut(&node) {
            Some((inline, mask)) => {
                inline.clear();
                inline.push_str(value);
                *mask = next_mask;
            }
            None => {
                let inline = self.buffer_pool.inline(value);
                self.inline_styles.insert(node, (inline, next_mask));
            }
        }
        if nve_tables::mask_border_color_source(prev_mask | next_mask).is_some() {
            self.refresh_current_color_border(node);
        }
        let dropped = nve_tables::prop_bits_to_layers(prev_mask & !next_mask);
        if dropped != 0 {
            self.rebuild_style_layers_mask(node, dropped);
        }
    }

    /// Rebuilds the specified `{Layout, Visual, Typography}` layers in canonical precedence order:
    /// `TypedStyle` < standalone style attributes < inline `style="..."`.
    pub(crate) fn rebuild_style_layers_mask(&mut self, node: NodeId, layers: u8) {
        if layers == 0 {
            return;
        }
        if layers & nve_tables::STYLE_LAYER_LAYOUT != 0 {
            self.tx.detach::<tables::Layout>(node);
        }
        if layers & nve_tables::STYLE_LAYER_VISUAL != 0 {
            self.tx.detach::<tables::Visual>(node);
        }
        if layers & nve_tables::STYLE_LAYER_TYPOGRAPHY != 0 {
            self.tx.detach::<tables::Typography>(node);
        }
        if let Some(ts) = self.typed_styles.get(&node) {
            ts.apply_style_layers_to_tx(self.tx, node, layers);
        }
        self.replay_standalone_and_inline(node, layers);
    }

    /// Attaches initial core word tables for a static RSX tag name (zero `ComponentKind`).
    pub(crate) fn attach_tag_defaults(&mut self, node: NodeId, tag: &str) {
        match tag {
            "text" | "span" | "p" => {
                let _ = self.tx.edit::<TextContent>(node);
            }
            "scrollview" => {
                let _ = self.tx.edit::<Scroll>(node);
            }
            "image" => {
                let _ = self.tx.edit::<Image>(node);
            }
            "textinput" | "input" => {
                let _ = self.tx.edit::<TextInput>(node);
            }
            "textarea" => {
                self.tx.edit::<TextInput>(node).multiline(true);
            }
            "pressable" | "button" => {
                self.tx
                    .edit::<Interaction>(node)
                    .cursor(CursorKind::Pointer)
                    .focusable(true);
                self.tx
                    .edit::<tables::Accessibility>(node)
                    .role(Some(AriaRole::Button));
            }
            "a" => {
                let _ = self.tx.edit::<Link>(node);
                self.tx
                    .edit::<Interaction>(node)
                    .cursor(CursorKind::Pointer);
            }
            _ => {}
        }
    }

    /// Recursively instantiates a static Dioxus [`TemplateNode`] directly into `NodeStore`.
    pub(crate) fn instantiate_template_node(&mut self, tnode: &TemplateNode) -> NodeId {
        match tnode {
            TemplateNode::Element {
                tag,
                attrs,
                children,
                ..
            } => {
                let node = self.tx.create();
                self.attach_tag_defaults(node, tag);
                for attr in *attrs {
                    if let TemplateAttribute::Static { name, value, .. } = attr {
                        self.apply_str_attr(node, name, value);
                    }
                }
                if !children.is_empty() {
                    // Split-at-len: nested instantiations push and truncate above `base`.
                    let base = self.stack.len();
                    for child in *children {
                        let child_id = self.instantiate_template_node(child);
                        self.stack.push(child_id);
                    }
                    self.tx.set_children(node, &self.stack[base..]);
                    self.stack.truncate(base);
                }
                node
            }
            TemplateNode::Text { text } => {
                let node = self.tx.create();
                self.tx.edit::<TextContent>(node).text(text);
                node
            }
            TemplateNode::Dynamic { .. } => self.tx.create(),
        }
    }

    /// Removes `node` from its current parent's child list (if any).
    pub(crate) fn detach_from_parent(&mut self, node: NodeId) {
        let Some(parent) = self.tx.store().parent(node) else {
            return;
        };
        self.child_scratch.clear();
        self.child_scratch
            .extend_from_slice(self.tx.store().children(parent));
        let before = self.child_scratch.len();
        self.child_scratch.retain(|&c| c != node);
        if self.child_scratch.len() != before {
            self.tx.set_children(parent, self.child_scratch);
        }
    }

    /// Removes `node` and its entire descendant subtree from `NodeStore` and ID maps.
    pub(crate) fn remove_subtree(&mut self, root: NodeId) {
        // Depth-first over the scratch region above the live Dioxus stack (split-at-len): popping
        // keeps peak scratch at the DFS frontier instead of the whole removed subtree. Every
        // per-node cleanup is order-independent; `tx.remove` walks the subtree itself.
        let base = self.stack.len();
        self.stack.push(root);
        while self.stack.len() > base {
            let Some(nid) = self.stack.pop() else {
                break;
            };
            self.stack.extend_from_slice(self.tx.store().children(nid));
            self.elements.unbind_node(nid);
            if let Some(list) = self.attached_tables.remove(&nid) {
                self.buffer_pool.recycle_table_list(list);
            }
            self.typed_styles.remove(&nid);
            self.current_color_borders.remove(&nid);
            if let Some((inline, _)) = self.inline_styles.remove(&nid) {
                self.buffer_pool.recycle_inline(inline);
            }
            if let Some(list) = self.standalone_style_attrs.remove(&nid) {
                self.buffer_pool.recycle_list(list);
            }
            if let Some(slot) = self.aux_event_flags.get_mut(nid.index()) {
                *slot = 0;
            }
        }
        self.tx.remove(root);
    }

    /// Walks a Dioxus template child path `[i0, i1, ...]` starting at `root`.
    pub(crate) fn walk_path(&self, root: NodeId, path: &[u8]) -> Option<NodeId> {
        let mut cur = root;
        for &idx in path {
            cur = *self.tx.store().children(cur).get(idx as usize)?;
        }
        Some(cur)
    }
}
