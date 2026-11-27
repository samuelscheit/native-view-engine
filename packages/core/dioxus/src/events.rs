//! Direct FFI (`NveEventBatchRef`) event dispatch into Dioxus listeners for built-in typed
//! pointer, scroll, text, focus, composition, keyboard, submit, table, and custom events.

use std::any::Any;
use std::rc::Rc;

use dioxus_core::Event;
use nve_abi::{
    native_events_to_direct, EventMask, NativeEvent, NativeEventKind, NveEventBatchRef,
    NVE_KEY_MOD_ALT, NVE_KEY_MOD_CTRL, NVE_KEY_MOD_META, NVE_KEY_MOD_SHIFT,
};

use crate::elements::{
    self, CompositionEventData, FocusEventData, KeyEventData, ScrollEventData, SubmitEventData,
    TableEventData,
};
use crate::surface::DioxusSurface;
use crate::CustomEventData;

impl DioxusSurface {
    /// Routes a slice of owned [`NativeEvent`]s by borrowing them directly into an
    /// [`NveEventBatchRef`] and dispatching through [`Self::dispatch_direct_events`].
    pub fn dispatch_events(&mut self, events: &[NativeEvent]) -> bool {
        if events.is_empty() {
            return false;
        }
        let mut direct = Vec::with_capacity(events.len());
        native_events_to_direct(events, &mut direct);
        let batch = unsafe { NveEventBatchRef::from_slice(&direct) };
        self.dispatch_direct_events(batch)
    }

    /// Routes direct FFI events (`NveEventBatchRef`) straight into Dioxus handlers with zero
    /// byte deserialization.
    pub fn dispatch_direct_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
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
                (Some(NativeEventKind::Press), 0) => {
                    let data: Rc<dyn Any> = Rc::new(elements::PressEventData::default());
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
                (Some(NativeEventKind::CompositionStart), 0 | 3)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::COMPOSITION) =>
                {
                    let data: Rc<dyn Any> = Rc::new(CompositionEventData {
                        text: ev.text().to_owned(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("compositionstart", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::CompositionUpdate), 3)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::COMPOSITION) =>
                {
                    let data: Rc<dyn Any> = Rc::new(CompositionEventData {
                        text: ev.text().to_owned(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("compositionupdate", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::CompositionEnd), 0 | 3)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::COMPOSITION) =>
                {
                    let data: Rc<dyn Any> = Rc::new(CompositionEventData {
                        text: ev.text().to_owned(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("compositionend", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::KeyDown), 6)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::KEY_DOWN) =>
                {
                    let m = ev.modifiers();
                    let data: Rc<dyn Any> = Rc::new(KeyEventData {
                        key: ev.text().to_owned(),
                        meta: (m & NVE_KEY_MOD_META) != 0,
                        ctrl: (m & NVE_KEY_MOD_CTRL) != 0,
                        shift: (m & NVE_KEY_MOD_SHIFT) != 0,
                        alt: (m & NVE_KEY_MOD_ALT) != 0,
                    });
                    self.vdom
                        .runtime()
                        .handle_event("keydown", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::KeyDown), 3)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::KEY_DOWN) =>
                {
                    let data: Rc<dyn Any> = Rc::new(KeyEventData {
                        key: ev.text().to_owned(),
                        meta: false,
                        ctrl: false,
                        shift: false,
                        alt: false,
                    });
                    self.vdom
                        .runtime()
                        .handle_event("keydown", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::Submit), 3)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::SUBMIT) =>
                {
                    let data: Rc<dyn Any> = Rc::new(SubmitEventData {
                        value: ev.text().to_owned(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("submit", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::TableEvent), 9)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::TABLE_EVENT) =>
                {
                    let data: Rc<dyn Any> = Rc::new(TableEventData {
                        table_key: ev.table_key().0,
                        event_id: ev.table_event_id(),
                        payload: ev.table_payload().to_vec(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("tableevent", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::Custom), 5) => {
                    let name = ev.name().to_owned();
                    let data: Rc<dyn Any> = Rc::new(CustomEventData {
                        name: name.clone(),
                        data: ev.value().as_bytes().unwrap_or(&[]).to_vec(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event(&name, Event::new(data, true), el_id);
                    handled = true;
                }
                _ => {}
            }
        }
        handled
    }
}
