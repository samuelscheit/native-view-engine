//! Direct FFI (`NveEventBatchRef`) event dispatch into Dioxus listeners for built-in typed
//! pointer, scroll, text, focus, composition, keyboard, submit, wheel, and custom events.

use std::any::Any;
use std::rc::Rc;

use dioxus_core::Event;
use nve_abi::{
    native_events_to_direct, EventMask, NativeEvent, NativeEventKind, NveEventBatchRef,
};

use crate::elements::{
    self, CompositionEventData, FocusEventData, KeyEventData, ScrollEventData, SubmitEventData,
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
                (
                    Some(
                        kind @ (NativeEventKind::CompositionStart
                        | NativeEventKind::CompositionUpdate
                        | NativeEventKind::CompositionEnd),
                    ),
                    0 | 3,
                ) if self.writer.has_event_mask(ev.node_id(), EventMask::COMPOSITION) => {
                    let wire_name = composition_event_wire_name(kind);
                    let text = if ev.payload_tag() == 3 {
                        ev.text().to_owned()
                    } else {
                        String::new()
                    };
                    let data: Rc<dyn Any> = Rc::new(CompositionEventData { text });
                    self.vdom
                        .runtime()
                        .handle_event(wire_name, Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::KeyDown), 6)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::KEY_DOWN) =>
                {
                    let data: Rc<dyn Any> = Rc::new(KeyEventData {
                        key: ev.key().to_owned(),
                        meta: ev.meta(),
                        ctrl: ev.ctrl(),
                        shift: ev.shift(),
                        alt: ev.alt(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("keydown", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::KeyUp), 6)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::KEY_UP) =>
                {
                    let data: Rc<dyn Any> = Rc::new(KeyEventData {
                        key: ev.key().to_owned(),
                        meta: ev.meta(),
                        ctrl: ev.ctrl(),
                        shift: ev.shift(),
                        alt: ev.alt(),
                    });
                    self.vdom
                        .runtime()
                        .handle_event("keyup", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::Submit), 0 | 3)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::SUBMIT) =>
                {
                    let value = if ev.payload_tag() == 3 {
                        ev.text().to_owned()
                    } else {
                        String::new()
                    };
                    let data: Rc<dyn Any> = Rc::new(SubmitEventData { value });
                    self.vdom
                        .runtime()
                        .handle_event("submit", Event::new(data, true), el_id);
                    handled = true;
                }
                (
                    Some(
                        kind @ (NativeEventKind::PointerDown
                        | NativeEventKind::PointerMove
                        | NativeEventKind::PointerUp
                        | NativeEventKind::PointerEnter
                        | NativeEventKind::PointerLeave),
                    ),
                    7,
                ) if self
                    .writer
                    .has_event_mask_in_chain(ev.node_id(), pointer_event_mask(kind)) =>
                {
                    let data: Rc<dyn Any> = Rc::new(elements::PressEventData {
                        x: ev.x(),
                        y: ev.y(),
                    });
                    self.vdom.runtime().handle_event(
                        pointer_event_wire_name(kind),
                        Event::new(data, true),
                        el_id,
                    );
                    handled = true;
                }
                (Some(NativeEventKind::Wheel), 8)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::WHEEL) =>
                {
                    let data: Rc<dyn Any> = Rc::new(ScrollEventData {
                        x: ev.delta_x(),
                        y: ev.delta_y(),
                        content_width: 0.0,
                        content_height: 0.0,
                    });
                    self.vdom
                        .runtime()
                        .handle_event("wheel", Event::new(data, true), el_id);
                    handled = true;
                }
                (Some(NativeEventKind::Custom), 5)
                    if self
                        .writer
                        .has_event_mask_in_chain(ev.node_id(), EventMask::CUSTOM) =>
                {
                    let name = ev.name();
                    let wire_name = normalize_custom_event_name(name);
                    let raw_bytes: &[u8] = if let Some(b) = ev.value().as_bytes() {
                        b
                    } else if let Some(s) = ev.value().as_str() {
                        s.as_bytes()
                    } else if !ev.text().is_empty() {
                        ev.text().as_bytes()
                    } else {
                        &[]
                    };
                    let payload = build_custom_event_payload(&wire_name, name, raw_bytes);
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
}

#[inline]
fn composition_event_wire_name(kind: NativeEventKind) -> &'static str {
    match kind {
        NativeEventKind::CompositionStart => "compositionstart",
        NativeEventKind::CompositionUpdate => "compositionupdate",
        NativeEventKind::CompositionEnd => "compositionend",
        _ => "compositionupdate",
    }
}

#[inline]
fn pointer_event_mask(kind: NativeEventKind) -> u64 {
    match kind {
        NativeEventKind::PointerDown => EventMask::POINTER_DOWN,
        NativeEventKind::PointerMove => EventMask::POINTER_MOVE,
        NativeEventKind::PointerUp => EventMask::POINTER_UP,
        NativeEventKind::PointerEnter => EventMask::POINTER_ENTER,
        NativeEventKind::PointerLeave => EventMask::POINTER_LEAVE,
        _ => EventMask::NONE,
    }
}

#[inline]
fn pointer_event_wire_name(kind: NativeEventKind) -> &'static str {
    match kind {
        NativeEventKind::PointerDown => "pointerdown",
        NativeEventKind::PointerMove => "pointermove",
        NativeEventKind::PointerUp => "pointerup",
        NativeEventKind::PointerEnter => "pointerenter",
        NativeEventKind::PointerLeave => "pointerleave",
        _ => "pointermove",
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

fn build_custom_event_payload(wire_name: &str, raw_name: &str, data: &[u8]) -> Rc<dyn Any> {
    match wire_name {
        "submit" => Rc::new(SubmitEventData {
            value: String::from_utf8_lossy(data).into_owned(),
        }),
        _ => Rc::new(CustomEventData {
            name: raw_name.to_owned(),
            data: data.to_vec(),
        }),
    }
}
