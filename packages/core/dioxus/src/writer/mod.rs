//! `NveMutationWriter` & ephemeral `StoreMutationTx<'_>` — bridges `dioxus_core::WriteMutations`
//! directly into an owned [`NodeStore`] via [`nve_store::Transaction`] with zero `NodeRecord`
//! mirroring, zero `Box<NodeExtras>`, zero `ComponentKind`, and zero `set_raw_prop`.

mod attributes;
mod element_map;
mod identity_attrs;
mod listeners;
mod mutations;
pub(crate) mod node_slot_table;
mod standalone_style;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod tests;

use dioxus_core::{ElementId, TemplateAttribute, TemplateNode};
use nve_abi::SurfaceId;
use nve_tables::interaction::CursorKind;
use nve_tables::tables::{
    self, AriaRole, Image, Interaction, Link, Scroll, TextContent, TextInput,
};
use nve_store::{ChangeSet, NodeId, NodeStore, Transaction};

use crate::hot_patch::{HotPatchIndex, NVE_TEMPLATE_ATTR};
use crate::typed_style::TypedStyle;
use element_map::ElementMap;
use node_slot_table::NodeSlotTable;
pub(crate) use standalone_style::StandaloneStyleVal;
use standalone_style::{
    numeric_attr_bits, numeric_attr_layer, numeric_attr_rank, BufferPool, StandaloneStyleList,
    TableAttrList,
};

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
    /// Dense generation-checked per-node side tables (O(1) indexed access, zero hashing).
    /// Tracks active [`crate::TableAttr`] writers per `(node, attr_name)` so multiple table attrs
    /// can coexist on one node and `AttributeValue::None` / replacement can call `clear_from`.
    pub(crate) attached_tables: NodeSlotTable<TableAttrList>,
    /// Tracks active [`TypedStyle`] so updates diff cleanly and standalone numeric attrs restore.
    pub(crate) typed_styles: NodeSlotTable<TypedStyle>,
    /// Tracks numeric standalone style attributes (`width`, `padding`, `opacity`, `z-index`, ...).
    pub(crate) standalone_style_attrs: NodeSlotTable<StandaloneStyleList>,
    /// Recycled style/table lists of removed nodes.
    pub(crate) buffer_pool: BufferPool,
    /// Dense node-index flags for non-table Dioxus listeners (bit 0 = `ontableevent`, bit 1 = custom).
    pub(crate) aux_event_flags: Vec<u8>,
    /// Live template anchor index (`__nve_tpl`) and zero-alloc hot-patch scratch buffers.
    pub(crate) hot_patch: HotPatchIndex,
    #[cfg(test)]
    pub(crate) standalone_replay_count: usize,
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
            attached_tables: NodeSlotTable::new(),
            typed_styles: NodeSlotTable::new(),
            standalone_style_attrs: NodeSlotTable::new(),
            buffer_pool: BufferPool::default(),
            aux_event_flags: Vec::new(),
            hot_patch: HotPatchIndex::new(),
            #[cfg(test)]
            standalone_replay_count: 0,
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

    /// Resets the writer to an empty root-only store, recycling pooled style/table buffers and
    /// releasing every per-node side table and element mapping.
    pub fn reset(&mut self) {
        let mut store = NodeStore::new();
        store.set_realization_order(tables::core_realization_order());
        self.store = store;
        self.changes = ChangeSet::default();
        self.elements.reset_to_root();
        self.stack.clear();
        self.child_scratch.clear();
        self.incoming_scratch.clear();
        let pool = &mut self.buffer_pool;
        self.attached_tables
            .reset_with(|list| pool.recycle_table_list(list));
        self.typed_styles.reset();
        self.standalone_style_attrs
            .reset_with(|list| pool.recycle_list(list));
        self.aux_event_flags = Vec::new();
        self.hot_patch.reset();
        #[cfg(test)]
        {
            self.standalone_replay_count = 0;
        }
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
            standalone_style_attrs,
            buffer_pool,
            aux_event_flags,
            hot_patch,
            #[cfg(test)]
            standalone_replay_count,
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
                standalone_style_attrs,
                buffer_pool,
                aux_event_flags,
                hot_patch,
                #[cfg(test)]
                standalone_replay_count,
            };
            f(&mut mtx)
        };
        tx.commit_into(changes);
        out
    }

    /// Applies one Tier-1 hot patch directly in a single [`Transaction`] with zero steady-state
    /// heap allocations.
    pub fn apply_hot_patch(&mut self, patch: &nve_engine::HotPatchMsg) -> bool {
        self.with_transaction(|mtx| mtx.apply_hot_patch(patch))
    }
}

/// Ephemeral per-frame Dioxus `WriteMutations` adapter borrowing [`Transaction`] directly.
pub struct StoreMutationTx<'s, 'a> {
    pub(crate) tx: &'a mut Transaction<'s>,
    pub(crate) elements: &'a mut ElementMap,
    pub(crate) stack: &'a mut Vec<NodeId>,
    pub(crate) child_scratch: &'a mut Vec<NodeId>,
    pub(crate) incoming_scratch: &'a mut Vec<NodeId>,
    pub(crate) attached_tables: &'a mut NodeSlotTable<TableAttrList>,
    pub(crate) typed_styles: &'a mut NodeSlotTable<TypedStyle>,
    pub(crate) standalone_style_attrs: &'a mut NodeSlotTable<StandaloneStyleList>,
    pub(crate) buffer_pool: &'a mut BufferPool,
    pub(crate) aux_event_flags: &'a mut Vec<u8>,
    pub(crate) hot_patch: &'a mut HotPatchIndex,
    #[cfg(test)]
    pub(crate) standalone_replay_count: &'a mut usize,
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

    /// Removes a recorded numeric standalone style attribute. Returns `true` iff removed.
    pub(crate) fn remove_standalone_style(&mut self, node: NodeId, name: &str) -> bool {
        let Some(list) = self.standalone_style_attrs.get_mut(node) else {
            return false;
        };
        let Some(pos) = list.iter().position(|(k, _)| *k == name) else {
            return false;
        };
        list.remove(pos);
        if list.is_empty()
            && let Some(list) = self.standalone_style_attrs.remove(node)
        {
            self.buffer_pool.recycle_list(list);
        }
        true
    }

    /// Replays numeric standalone style attributes for the specified `layers`.
    pub(crate) fn replay_standalone_styles(&mut self, node: NodeId, layers: u8) {
        if layers == 0 {
            return;
        }
        if let Some(saved) = self.standalone_style_attrs.get(node) {
            for &(k, val) in saved {
                let entry_layer = numeric_attr_layer(k);
                debug_assert!(
                    entry_layer != 0,
                    "standalone numeric style key `{k}` must have a non-zero layer"
                );
                if (entry_layer & layers) == 0 {
                    continue;
                }
                replay_one_standalone_entry(
                    self.tx,
                    #[cfg(test)]
                    self.standalone_replay_count,
                    node,
                    k,
                    val,
                );
            }
        }
    }

    #[inline]
    pub(crate) fn finish_standalone_style(
        &mut self,
        node: NodeId,
        name: &'static str,
        val: StandaloneStyleVal,
    ) {
        let pool = &mut *self.buffer_pool;
        let Some(list) = self
            .standalone_style_attrs
            .get_or_insert_with(node, || pool.list())
        else {
            return;
        };
        let pos = if let Some(pos) = list.iter().position(|(k, _)| *k == name) {
            list[pos].1 = val;
            pos
        } else {
            let rank = numeric_attr_rank(name);
            let pos = list
                .iter()
                .rposition(|(k, _)| numeric_attr_rank(k) <= rank)
                .map_or(0, |i| i + 1);
            list.insert(pos, (name, val));
            pos
        };

        let mut affected_bits = numeric_attr_bits(name);
        debug_assert!(
            affected_bits != 0,
            "standalone numeric style key `{name}` must have numeric_attr_bits"
        );

        // Single-pass replay of trailing entries whose property bits transitively overlap.
        for &(k, v) in list.get(pos + 1..).unwrap_or(&[]) {
            let entry_bits = numeric_attr_bits(k);
            if (entry_bits & affected_bits) == 0 {
                continue;
            }
            affected_bits |= entry_bits;
            replay_one_standalone_entry(
                self.tx,
                #[cfg(test)]
                self.standalone_replay_count,
                node,
                k,
                v,
            );
        }
    }

    /// Rebuilds the specified `{Layout, Visual, Typography}` layers in canonical precedence order:
    /// `TypedStyle` < numeric standalone style attributes.
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
        if let Some(ts) = self.typed_styles.get(node) {
            ts.apply_style_layers_to_tx(self.tx, node, layers);
        }
        self.replay_standalone_styles(node, layers);
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
                        if *name == NVE_TEMPLATE_ATTR {
                            self.hot_patch.register_tag(node, value);
                        } else {
                            self.apply_str_attr(node, name, value);
                        }
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
            self.hot_patch.unbind_node(nid);
            if let Some(list) = self.attached_tables.remove(nid) {
                self.buffer_pool.recycle_table_list(list);
            }
            self.typed_styles.remove(nid);
            if let Some(list) = self.standalone_style_attrs.remove(nid) {
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

#[inline]
fn replay_one_standalone_entry(
    tx: &mut Transaction<'_>,
    #[cfg(test)] standalone_replay_count: &mut usize,
    node: NodeId,
    k: &str,
    val: StandaloneStyleVal,
) {
    #[cfg(test)]
    {
        *standalone_replay_count += 1;
    }
    match val {
        StandaloneStyleVal::Float(f) => {
            let _ = attributes::apply_numeric_attr_untracked(tx, node, k, f);
        }
        StandaloneStyleVal::Int(i) => {
            tx.edit::<tables::Visual>(node)
                .z_index(i.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
        }
    }
}
