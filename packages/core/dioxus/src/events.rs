//! Native `EventRing` and zero-copy direct FFI (`NveEventBatchRef`) event dispatch into Dioxus
//! listeners, using zero-JSON binary `NK1` / shorthand `decode_keydown_bytes`.

use std::any::Any;
use std::rc::Rc;

use dioxus_core::Event;
use nve_abi::events::decode_keydown_bytes;
use nve_abi::{EventMask, EventPayload, NativeEvent, NativeEventKind, NveEventBatchRef};

use crate::elements::{
    self, CompositionEventData, FocusEventData, KeyEventData, ScrollEventData, SubmitEventData,
};
use crate::surface::DioxusSurface;
use crate::CustomEventData;

impl DioxusSurface {
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
                (
                    kind @ (NativeEventKind::CompositionStart
                    | NativeEventKind::CompositionUpdate
                    | NativeEventKind::CompositionEnd),
                    EventPayload::Text(text),
                ) if self.writer.has_event_mask(ev.node_id, EventMask::COMPOSITION) => {
                    let wire_name = composition_event_wire_name(kind);
                    let data: Rc<dyn Any> = Rc::new(CompositionEventData { text: text.clone() });
                    self.vdom
                        .runtime()
                        .handle_event(wire_name, Event::new(data, true), el_id);
                    handled = true;
                }
                (
                    kind @ (NativeEventKind::CompositionStart
                    | NativeEventKind::CompositionUpdate
                    | NativeEventKind::CompositionEnd),
                    EventPayload::Empty,
                ) if self.writer.has_event_mask(ev.node_id, EventMask::COMPOSITION) => {
                    let wire_name = composition_event_wire_name(kind);
                    let data: Rc<dyn Any> = Rc::new(CompositionEventData::default());
                    self.vdom
                        .runtime()
                        .handle_event(wire_name, Event::new(data, true), el_id);
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
    decode_keydown_bytes(raw)
        .map(|kd| KeyEventData {
            key: kd.key,
            meta: kd.meta,
            ctrl: kd.ctrl,
            shift: kd.shift,
            alt: kd.alt,
        })
        .unwrap_or_default()
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
