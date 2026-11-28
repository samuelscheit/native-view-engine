//! Typed per-table listener subscriptions (`EventSet` bits in each table's `events_word`) plus
//! dense auxiliary flags for listeners without a table (`ontableevent`, custom named events).

use nve_abi::table::TableKey;
use nve_abi::{EventEnum, EventId, InteractionEvent, LayoutEvent, ScrollEvent, TextInputEvent};
use nve_tables::tables::{Interaction, Layout, Scroll, TextInput};
use nve_store::{HasEvents, NodeId};

use super::{NveMutationWriter, StoreMutationTx};

/// [`NveMutationWriter::aux_event_flags`] bit of a generic `ontableevent` handler.
const AUX_TABLE_EVENT: u8 = 1 << 0;
/// [`NveMutationWriter::aux_event_flags`] bit of any custom named (`CustomEvent`) handler.
const AUX_CUSTOM_EVENT: u8 = 1 << 1;

impl NveMutationWriter {
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

    #[inline]
    fn node_has_table_event_listener(
        &self,
        nid: NodeId,
        slot: Option<nve_store::TableSlot>,
        bit: u32,
    ) -> bool {
        if (self.aux_event_flags.get(nid.index()).copied().unwrap_or(0) & AUX_TABLE_EVENT) != 0 {
            return true;
        }
        slot.is_some_and(|s| (self.store.node_events_mask(s, nid) & bit) != 0)
    }

    /// Checks whether `node_id` itself carries either a generic `ontableevent` handler or a table's
    /// own `events_word` subscription bit (target-only check for `EventFlags::NO_BUBBLE`).
    pub fn has_table_event_listener(
        &self,
        node_id: NodeId,
        table: TableKey,
        event: EventId,
    ) -> bool {
        if !self.store.is_live(node_id) {
            return false;
        }
        let slot = nve_store::slot_of(table);
        let bit = if event.raw() < 32 { 1u32 << event.raw() } else { 0 };
        self.node_has_table_event_listener(node_id, slot, bit)
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
            if self.node_has_table_event_listener(nid, slot, bit) {
                return true;
            }
            cur = self.store.parent(nid);
        }
        false
    }
}

impl StoreMutationTx<'_, '_> {
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
            "tableevent" | "ontableevent" => self.set_aux_event_flag(node, AUX_TABLE_EVENT, listen),
            _ => self.set_aux_event_flag(node, AUX_CUSTOM_EVENT, listen),
        }
    }
}
