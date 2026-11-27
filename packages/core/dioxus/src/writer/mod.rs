//! `NveMutationWriter` & ephemeral `StoreMutationTx<'_>` — bridges `dioxus_core::WriteMutations`
//! directly into an owned [`NodeStore`] via [`nve_store::Transaction`] with zero `NodeRecord`
//! mirroring, zero `Box<NodeExtras>`, zero `ComponentKind`, and zero `set_raw_prop`.

mod attributes;
mod mutations;
#[cfg(test)]
mod tests;

use dioxus_core::{ElementId, TemplateAttribute, TemplateNode};
use nve_abi::hash::HashMap;
use nve_abi::table::TableKey;
use nve_abi::{
    EventEnum, EventId, InteractionEvent, LayoutEvent, ScrollEvent, SurfaceId, TextInputEvent,
};
use nve_components::interaction::CursorKind;
use nve_components::tables::{
    self, AriaRole, Image, Interaction, Layout, Link, Scroll, TextContent, TextInput,
};
use nve_store::{ChangeSet, HasEvents, NodeId, NodeStore, Transaction};

use crate::table_attr::TableAttr;
use crate::typed_style::TypedStyle;

/// Saved value of a standalone layout/visual/typography attribute so clearing `style="..."`
/// restores standalone attributes instead of wiping them.
#[derive(Debug, Clone)]
pub(crate) enum StandaloneStyleVal {
    Float(f64),
    Int(i64),
    Str(String),
}

/// Bridges Dioxus 0.7 `WriteMutations` straight into a single-owner [`NodeStore`].
pub struct NveMutationWriter {
    pub(crate) surface_id: SurfaceId,
    pub(crate) store: NodeStore,
    pub(crate) changes: ChangeSet,
    pub(crate) el_to_node: HashMap<ElementId, NodeId>,
    pub(crate) node_to_el: HashMap<NodeId, ElementId>,
    pub(crate) stack: Vec<NodeId>,
    pub(crate) child_scratch: Vec<NodeId>,
    pub(crate) incoming_scratch: Vec<NodeId>,
    /// Tracks active [`TableAttr`] writers per `(node, attr_name)` so multiple table attrs can
    /// coexist on one node and `AttributeValue::None` / replacement can call `clear_from`.
    pub(crate) attached_tables: HashMap<NodeId, Vec<(&'static str, TableAttr)>>,
    /// Tracks active [`TypedStyle`] so inline `style` overrides merge cleanly and restore on clear.
    pub(crate) typed_styles: HashMap<NodeId, TypedStyle>,
    /// Tracks active inline `style="..."` override strings and their accepted CSS property bitmask.
    pub(crate) inline_styles: HashMap<NodeId, (String, u64)>,
    /// Tracks standalone CSS/style attributes (`width`, `color`, `opacity`, etc.) per node.
    pub(crate) standalone_style_attrs: HashMap<NodeId, Vec<(String, StandaloneStyleVal)>>,
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
        let mut el_to_node = HashMap::default();
        let mut node_to_el = HashMap::default();
        el_to_node.insert(ElementId(0), NodeId::ROOT);
        node_to_el.insert(NodeId::ROOT, ElementId(0));
        Self {
            surface_id,
            store,
            changes: ChangeSet::default(),
            el_to_node,
            node_to_el,
            stack: Vec::with_capacity(64),
            child_scratch: Vec::with_capacity(32),
            incoming_scratch: Vec::with_capacity(32),
            attached_tables: HashMap::default(),
            typed_styles: HashMap::default(),
            inline_styles: HashMap::default(),
            standalone_style_attrs: HashMap::default(),
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
        self.el_to_node.get(&id).copied()
    }

    #[inline]
    pub fn element_id_of(&self, node_id: NodeId) -> Option<ElementId> {
        self.node_to_el.get(&node_id).copied()
    }

    #[inline]
    pub fn has_listener<T: HasEvents>(&self, node_id: NodeId, event: T::Event) -> bool {
        let mask = self.store.node_events_mask(T::slot(), node_id);
        (mask & event.bit()) != 0
    }

    /// Walks up from `node_id` to `NodeId::ROOT` checking if any ancestor listens to `(T, event)`.
    pub fn has_listener_in_chain<T: HasEvents>(&self, node_id: NodeId, event: T::Event) -> bool {
        let slot = T::slot();
        let bit = event.bit();
        let mut cur = Some(node_id);
        while let Some(nid) = cur {
            if !self.store.is_live(nid) {
                break;
            }
            if (self.store.node_events_mask(slot, nid) & bit) != 0 {
                return true;
            }
            cur = self.store.parent(nid);
        }
        false
    }

    /// Walks up from `node_id` checking for either a generic `ontableevent` handler or a table's
    /// own `events_word` subscription bit.
    pub fn has_table_event_listener_in_chain(
        &self,
        node_id: NodeId,
        table: TableKey,
        event: EventId,
    ) -> bool {
        let slot = nve_store::slot_of(table);
        let bit = if event.raw() < 32 { 1u32 << event.raw() } else { 0 };
        let mut cur = Some(node_id);
        while let Some(nid) = cur {
            if !self.store.is_live(nid) {
                break;
            }
            if (self.aux_event_flags.get(nid.index()).copied().unwrap_or(0) & 1) != 0 {
                return true;
            }
            if let Some(s) = slot
                && (self.store.node_events_mask(s, nid) & bit) != 0
            {
                return true;
            }
            cur = self.store.parent(nid);
        }
        false
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
            el_to_node,
            node_to_el,
            stack,
            child_scratch,
            incoming_scratch,
            attached_tables,
            typed_styles,
            inline_styles,
            standalone_style_attrs,
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
                el_to_node,
                node_to_el,
                stack,
                child_scratch,
                incoming_scratch,
                attached_tables,
                typed_styles,
                inline_styles,
                standalone_style_attrs,
                aux_event_flags,
                #[cfg(test)]
                standalone_replay_count,
                #[cfg(test)]
                inline_reapply_count,
            };
            f(&mut mtx)
        };
        tx.commit_into(changes);
        out
    }
}

/// Ephemeral per-frame Dioxus `WriteMutations` adapter borrowing [`Transaction`] directly.
pub struct StoreMutationTx<'s, 'a> {
    pub(crate) tx: &'a mut Transaction<'s>,
    pub(crate) el_to_node: &'a mut HashMap<ElementId, NodeId>,
    pub(crate) node_to_el: &'a mut HashMap<NodeId, ElementId>,
    pub(crate) stack: &'a mut Vec<NodeId>,
    pub(crate) child_scratch: &'a mut Vec<NodeId>,
    pub(crate) incoming_scratch: &'a mut Vec<NodeId>,
    pub(crate) attached_tables: &'a mut HashMap<NodeId, Vec<(&'static str, TableAttr)>>,
    pub(crate) typed_styles: &'a mut HashMap<NodeId, TypedStyle>,
    pub(crate) inline_styles: &'a mut HashMap<NodeId, (String, u64)>,
    pub(crate) standalone_style_attrs: &'a mut HashMap<NodeId, Vec<(String, StandaloneStyleVal)>>,
    pub(crate) aux_event_flags: &'a mut Vec<u8>,
    #[cfg(test)]
    pub(crate) standalone_replay_count: &'a mut usize,
    #[cfg(test)]
    pub(crate) inline_reapply_count: &'a mut usize,
}

impl StoreMutationTx<'_, '_> {
    #[inline]
    pub(crate) fn bind_element(&mut self, el: ElementId, node: NodeId) {
        self.el_to_node.insert(el, node);
        self.node_to_el.insert(node, el);
    }

    #[inline]
    pub(crate) fn resolve_el(&self, id: ElementId) -> Option<NodeId> {
        self.el_to_node.get(&id).copied()
    }

    #[inline]
    pub(crate) fn record_standalone_style(
        &mut self,
        node: NodeId,
        name: &str,
        val: StandaloneStyleVal,
    ) -> usize {
        let list = self.standalone_style_attrs.entry(node).or_default();
        if let Some(pos) = list.iter().position(|(k, _)| k == name) {
            list[pos].1 = val;
            return pos;
        }
        let rank = nve_components::css_key_rank(name);
        let pos = list
            .iter()
            .rposition(|(k, _)| nve_components::css_key_rank(k) <= rank)
            .map_or(0, |i| i + 1);
        list.insert(pos, (name.to_owned(), val));
        pos
    }

    /// Removes a recorded standalone style attribute. Returns `true` iff an entry was removed.
    pub(crate) fn remove_standalone_style(&mut self, node: NodeId, name: &str) -> bool {
        let Some(list) = self.standalone_style_attrs.get_mut(&node) else {
            return false;
        };
        let before = list.len();
        list.retain(|(k, _)| k != name);
        let removed = list.len() != before;
        if list.is_empty() {
            self.standalone_style_attrs.remove(&node);
        }
        removed
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
                nve_components::apply_css_attr_layers_to_tx(self.tx, node, k, s, layers);
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
                let entry_layer = nve_components::css_attr_layer(k);
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
            && (nve_components::prop_bits_to_layers(*mask) & layers) != 0
        {
            #[cfg(test)]
            {
                *self.inline_reapply_count += 1;
            }
            nve_components::apply_css_attr_layers_to_tx(self.tx, node, "style", inline, layers);
        }
    }

    #[inline]
    pub(crate) fn finish_standalone_style(
        &mut self,
        node: NodeId,
        name: &str,
        val: StandaloneStyleVal,
    ) {
        let pos = self.record_standalone_style(node, name, val);
        let bits = nve_components::css_key_bits(name);
        debug_assert!(bits.is_some(), "standalone style key `{name}` must have css_key_bits");
        let bits = bits.unwrap_or(0);
        let mut affected_bits = bits;

        // Read-only scan of trailing entries for transitive property-bit overlap; avoids HashMap
        // churn and re-parsing unrelated trailing attrs on 120 Hz standalone updates.
        let has_overlap = self.standalone_style_attrs.get(&node).is_some_and(|list| {
            let mut scan_bits = bits;
            let mut overlapped = false;
            for (k, _) in list.get(pos + 1..).unwrap_or(&[]) {
                let entry_bits = nve_components::css_key_bits(k).unwrap_or(0);
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
                let entry_bits = nve_components::css_key_bits(k).unwrap_or(0);
                if (entry_bits & affected_bits) == 0 {
                    continue;
                }
                affected_bits |= entry_bits;
                let entry_layer = nve_components::prop_bits_to_layers(entry_bits);
                self.replay_one_standalone_entry(node, k, val, entry_layer);
            }
            self.standalone_style_attrs.insert(node, saved);
        }

        let inline_layers = self.inline_styles.get(&node).map_or(0, |&(_, mask)| {
            nve_components::prop_bits_to_layers(mask & affected_bits)
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
        let next_mask = nve_components::apply_inline_style_layers_to_tx(
            self.tx,
            node,
            value,
            nve_components::STYLE_LAYER_ALL,
        );
        self.inline_styles.insert(node, (value.to_owned(), next_mask));
        let dropped = nve_components::prop_bits_to_layers(prev_mask & !next_mask);
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
        if layers & nve_components::STYLE_LAYER_LAYOUT != 0 {
            self.tx.detach::<tables::Layout>(node);
        }
        if layers & nve_components::STYLE_LAYER_VISUAL != 0 {
            self.tx.detach::<tables::Visual>(node);
        }
        if layers & nve_components::STYLE_LAYER_TYPOGRAPHY != 0 {
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
                    let mut child_ids = Vec::with_capacity(children.len());
                    for child in *children {
                        child_ids.push(self.instantiate_template_node(child));
                    }
                    self.tx.set_children(node, &child_ids);
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
        let mut to_visit = vec![root];
        let mut order = Vec::new();
        while let Some(nid) = to_visit.pop() {
            order.push(nid);
            to_visit.extend_from_slice(self.tx.store().children(nid));
        }
        for nid in order {
            if let Some(el) = self.node_to_el.remove(&nid) {
                self.el_to_node.remove(&el);
            }
            self.attached_tables.remove(&nid);
            self.typed_styles.remove(&nid);
            self.inline_styles.remove(&nid);
            self.standalone_style_attrs.remove(&nid);
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

    fn set_aux_event_flag(&mut self, node: NodeId, bit: u8, set: bool) {
        let idx = node.index();
        if set {
            if idx >= self.aux_event_flags.len() {
                self.aux_event_flags.resize(idx + 1, 0);
            }
            self.aux_event_flags[idx] |= bit;
        } else if let Some(slot) = self.aux_event_flags.get_mut(idx) {
            *slot &= !bit;
        }
    }

    pub(crate) fn set_event_listener(&mut self, node: NodeId, name: &str, listen: bool) {
        macro_rules! toggle {
            ($table:ty, $ev:expr) => {{
                if listen {
                    self.tx.edit::<$table>(node).listen($ev);
                } else if self.tx.store().has::<$table>(node) {
                    self.tx.edit::<$table>(node).unlisten($ev);
                }
            }};
        }
        match name {
            "press" | "onpress" | "click" | "onclick" => toggle!(Interaction, InteractionEvent::Press),
            "longpress" | "onlongpress" => toggle!(Interaction, InteractionEvent::LongPress),
            "pointerdown" | "onpointerdown" => toggle!(Interaction, InteractionEvent::PointerDown),
            "pointermove" | "onpointermove" => toggle!(Interaction, InteractionEvent::PointerMove),
            "pointerup" | "onpointerup" => toggle!(Interaction, InteractionEvent::PointerUp),
            "pointerenter" | "onpointerenter" => toggle!(Interaction, InteractionEvent::PointerEnter),
            "pointerleave" | "onpointerleave" => toggle!(Interaction, InteractionEvent::PointerLeave),
            "wheel" | "onwheel" => toggle!(Interaction, InteractionEvent::Wheel),
            "focus" | "onfocus" => toggle!(Interaction, InteractionEvent::Focus),
            "blur" | "onblur" => toggle!(Interaction, InteractionEvent::Blur),
            "keydown" | "onkeydown" => toggle!(Interaction, InteractionEvent::KeyDown),
            "keyup" | "onkeyup" => toggle!(Interaction, InteractionEvent::KeyUp),
            "change" | "onchange" | "input" | "oninput" => toggle!(TextInput, TextInputEvent::Change),
            "submit" | "onsubmit" => toggle!(TextInput, TextInputEvent::Submit),
            "compositionstart" | "oncompositionstart" => {
                toggle!(TextInput, TextInputEvent::CompositionStart)
            }
            "compositionupdate" | "oncompositionupdate" => {
                toggle!(TextInput, TextInputEvent::CompositionUpdate)
            }
            "compositionend" | "oncompositionend" => {
                toggle!(TextInput, TextInputEvent::CompositionEnd)
            }
            "scroll" | "onscroll" => toggle!(Scroll, ScrollEvent::Scroll),
            "layout" | "onlayout" => toggle!(Layout, LayoutEvent::Layout),
            "tableevent" | "ontableevent" => self.set_aux_event_flag(node, 1 << 0, listen),
            _ => self.set_aux_event_flag(node, 1 << 1, listen),
        }
    }
}
