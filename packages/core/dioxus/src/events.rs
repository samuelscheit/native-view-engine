//! Ingress-validated (`ValidatedEvents`) event dispatch into Dioxus listeners with reused
//! concrete `Rc<T>` event payload slots (`Rc::get_mut` = zero heap allocations in steady state).

use std::any::Any;
use std::rc::Rc;

use dioxus_core::Event;
use nve_abi::{
    InteractionEvent, LayoutEvent, ScrollEvent, TextInputEvent, ValidatedEvents, NVE_TABLE_CUSTOM,
    NVE_TABLE_INTERACTION, NVE_TABLE_LAYOUT, NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT,
};
use nve_components::tables::{Interaction, Layout, Scroll, TextInput};

use crate::elements::{
    CompositionEventData, FocusEventData, KeyEventData, NveEventData, PressEventData,
    ScrollEventData, SubmitEventData, TableEventData, TextEventData,
};
use crate::surface::DioxusSurface;
use crate::CustomEventData;

/// Concrete reusable `Rc<T>` slots mutated in place via `Rc::get_mut` before `handle_event`.
///
/// When handlers don't retain clones of the `Event<T>`, strong count returns to `1` immediately
/// after `handle_event`, so subsequent events reuse both the `Rc` box and any `String`/`Vec`
/// capacity with zero heap allocations. If a handler clones and stashes the `Event<T>`,
/// `Rc::get_mut` transparently allocates a fresh `Rc` without mutating the retained value.
pub(crate) struct EventSlots {
    press: Rc<PressEventData>,
    text: Rc<TextEventData>,
    scroll: Rc<ScrollEventData>,
    focus: Rc<FocusEventData>,
    key: Rc<KeyEventData>,
    submit: Rc<SubmitEventData>,
    composition: Rc<CompositionEventData>,
    table: Rc<TableEventData>,
    custom: Rc<CustomEventData>,
    raw: Rc<NveEventData>,
}

impl EventSlots {
    pub(crate) fn new() -> Self {
        Self {
            press: Rc::new(PressEventData::default()),
            text: Rc::new(TextEventData::default()),
            scroll: Rc::new(ScrollEventData::default()),
            focus: Rc::new(FocusEventData),
            key: Rc::new(KeyEventData::default()),
            submit: Rc::new(SubmitEventData::default()),
            composition: Rc::new(CompositionEventData::default()),
            table: Rc::new(TableEventData::default()),
            custom: Rc::new(CustomEventData::default()),
            raw: Rc::new(NveEventData::default()),
        }
    }

    #[inline]
    fn reuse<T: Default + 'static>(slot: &mut Rc<T>, fill: impl FnOnce(&mut T)) -> Rc<dyn Any> {
        if let Some(inner) = Rc::get_mut(slot) {
            fill(inner);
        } else {
            let mut fresh = T::default();
            fill(&mut fresh);
            *slot = Rc::new(fresh);
        }
        Rc::clone(slot) as Rc<dyn Any>
    }
}

impl DioxusSurface {
    /// Routes ingress-validated events (`ValidatedEvents`) straight into Dioxus handlers with zero
    /// deserialization and reused concrete `Rc<T>` slots (0 steady-state allocations).
    pub fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        let mut handled = false;
        for ev in events.iter() {
            let node = ev.node();
            let Some(el_id) = self.writer.element_id_of(node) else {
                continue;
            };
            match ev.table() {
                NVE_TABLE_INTERACTION => match ev.interaction() {
                    Some(InteractionEvent::Press) => {
                        let data = EventSlots::reuse(&mut self.event_slots.press, |d| {
                            d.x = ev.x();
                            d.y = ev.y();
                        });
                        self.vdom
                            .runtime()
                            .handle_event("press", Event::new(data, true), el_id);
                        handled = true;
                    }
                    Some(InteractionEvent::LongPress)
                        if self
                            .writer
                            .has_listener_in_chain::<Interaction>(node, InteractionEvent::LongPress) =>
                    {
                        let data = EventSlots::reuse(&mut self.event_slots.press, |d| {
                            d.x = ev.x();
                            d.y = ev.y();
                        });
                        self.vdom
                            .runtime()
                            .handle_event("longpress", Event::new(data, true), el_id);
                        handled = true;
                    }
                    Some(
                        kind @ (InteractionEvent::PointerDown
                        | InteractionEvent::PointerMove
                        | InteractionEvent::PointerUp
                        | InteractionEvent::PointerEnter
                        | InteractionEvent::PointerLeave),
                    ) if self.writer.has_listener_in_chain::<Interaction>(node, kind) => {
                        let (name, bubbles) = match kind {
                            InteractionEvent::PointerDown => ("pointerdown", true),
                            InteractionEvent::PointerMove => ("pointermove", true),
                            InteractionEvent::PointerUp => ("pointerup", true),
                            InteractionEvent::PointerEnter => ("pointerenter", false),
                            InteractionEvent::PointerLeave => ("pointerleave", false),
                            _ => unreachable!(),
                        };
                        let data = EventSlots::reuse(&mut self.event_slots.press, |d| {
                            d.x = ev.x();
                            d.y = ev.y();
                        });
                        self.vdom
                            .runtime()
                            .handle_event(name, Event::new(data, bubbles), el_id);
                        handled = true;
                    }
                    Some(InteractionEvent::Wheel)
                        if self
                            .writer
                            .has_listener_in_chain::<Interaction>(node, InteractionEvent::Wheel) =>
                    {
                        let data = EventSlots::reuse(&mut self.event_slots.scroll, |d| {
                            d.x = ev.delta_x();
                            d.y = ev.delta_y();
                            d.content_width = ev.width();
                            d.content_height = ev.height();
                        });
                        self.vdom
                            .runtime()
                            .handle_event("wheel", Event::new(data, true), el_id);
                        handled = true;
                    }
                    Some(InteractionEvent::Focus)
                        if self.writer.has_listener::<Interaction>(node, InteractionEvent::Focus) =>
                    {
                        let data = EventSlots::reuse(&mut self.event_slots.focus, |_| {});
                        self.vdom
                            .runtime()
                            .handle_event("focus", Event::new(data, false), el_id);
                        handled = true;
                    }
                    Some(InteractionEvent::Blur)
                        if self.writer.has_listener::<Interaction>(node, InteractionEvent::Blur) =>
                    {
                        let data = EventSlots::reuse(&mut self.event_slots.focus, |_| {});
                        self.vdom
                            .runtime()
                            .handle_event("blur", Event::new(data, false), el_id);
                        handled = true;
                    }
                    Some(kind @ (InteractionEvent::KeyDown | InteractionEvent::KeyUp))
                        if self.writer.has_listener_in_chain::<Interaction>(node, kind) =>
                    {
                        let name = match kind {
                            InteractionEvent::KeyDown => "keydown",
                            InteractionEvent::KeyUp => "keyup",
                            _ => unreachable!(),
                        };
                        let data = EventSlots::reuse(&mut self.event_slots.key, |d| {
                            d.key.clear();
                            d.key.push_str(ev.key());
                            d.meta = ev.meta();
                            d.ctrl = ev.ctrl();
                            d.shift = ev.shift();
                            d.alt = ev.alt();
                        });
                        self.vdom
                            .runtime()
                            .handle_event(name, Event::new(data, true), el_id);
                        handled = true;
                    }
                    _ => {}
                },
                NVE_TABLE_TEXT_INPUT => match ev.text_input() {
                    Some(TextInputEvent::Change) => {
                        let data = EventSlots::reuse(&mut self.event_slots.text, |d| {
                            d.value.clear();
                            d.value.push_str(ev.text());
                        });
                        self.vdom
                            .runtime()
                            .handle_event("change", Event::new(data, true), el_id);
                        handled = true;
                    }
                    Some(TextInputEvent::Submit)
                        if self
                            .writer
                            .has_listener_in_chain::<TextInput>(node, TextInputEvent::Submit) =>
                    {
                        let data = EventSlots::reuse(&mut self.event_slots.submit, |d| {
                            d.value.clear();
                            d.value.push_str(ev.text());
                        });
                        self.vdom
                            .runtime()
                            .handle_event("submit", Event::new(data, true), el_id);
                        handled = true;
                    }
                    Some(
                        kind @ (TextInputEvent::CompositionStart
                        | TextInputEvent::CompositionUpdate
                        | TextInputEvent::CompositionEnd),
                    ) if self.writer.has_listener_in_chain::<TextInput>(node, kind) => {
                        let name = match kind {
                            TextInputEvent::CompositionStart => "compositionstart",
                            TextInputEvent::CompositionUpdate => "compositionupdate",
                            TextInputEvent::CompositionEnd => "compositionend",
                            _ => unreachable!(),
                        };
                        let data = EventSlots::reuse(&mut self.event_slots.composition, |d| {
                            d.text.clear();
                            d.text.push_str(ev.text());
                        });
                        self.vdom
                            .runtime()
                            .handle_event(name, Event::new(data, true), el_id);
                        handled = true;
                    }
                    _ => {}
                },
                NVE_TABLE_SCROLL => {
                    if matches!(ev.scroll_event(), Some(ScrollEvent::Scroll))
                        && self.writer.has_listener::<Scroll>(node, ScrollEvent::Scroll)
                    {
                        let data = EventSlots::reuse(&mut self.event_slots.scroll, |d| {
                            d.x = ev.x();
                            d.y = ev.y();
                            d.content_width = ev.width();
                            d.content_height = ev.height();
                        });
                        self.vdom
                            .runtime()
                            .handle_event("scroll", Event::new(data, false), el_id);
                        handled = true;
                    }
                }
                NVE_TABLE_LAYOUT => {
                    if matches!(ev.layout_event(), Some(LayoutEvent::Layout))
                        && self.writer.has_listener::<Layout>(node, LayoutEvent::Layout)
                    {
                        let data = EventSlots::reuse(&mut self.event_slots.raw, |d| {
                            d.raw = ev.raw();
                            d.bytes.clear();
                            d.bytes.extend_from_slice(ev.bytes());
                        });
                        self.vdom
                            .runtime()
                            .handle_event("layout", Event::new(data, false), el_id);
                        handled = true;
                    }
                }
                NVE_TABLE_CUSTOM if ev.is_custom() => {
                    let name = ev.custom_name();
                    let data = EventSlots::reuse(&mut self.event_slots.custom, |d| {
                        d.name.clear();
                        d.name.push_str(name);
                        d.data.clear();
                        d.data.extend_from_slice(ev.custom_data());
                    });
                    self.vdom
                        .runtime()
                        .handle_event(name, Event::new(data, true), el_id);
                    handled = true;
                }
                table_key => {
                    if self
                        .writer
                        .has_table_event_listener_in_chain(node, table_key, ev.event())
                    {
                        let data = EventSlots::reuse(&mut self.event_slots.table, |d| {
                            d.table_key = table_key.0;
                            d.event_id = ev.event().raw();
                            d.payload.clear();
                            d.payload.extend_from_slice(ev.bytes());
                        });
                        self.vdom
                            .runtime()
                            .handle_event("tableevent", Event::new(data, true), el_id);
                        handled = true;
                    }
                }
            }
        }
        handled
    }
}
