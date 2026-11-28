//! Plugin-declared native DOM event bridges (`wasm32`): capability plugins list DOM events (e.g.
//! `<dialog>` `close`, `[popover]` `toggle`) whose occurrences must reach the app as
//! named custom (`NVE_TABLE_CUSTOM`) events, without the backend knowing any element semantics.

use nve_abi::EventFlags;
use wasm_bindgen::closure::Closure;

use crate::events::{target_to_element, DomListenerGuard};
use crate::events_dom::EventSinks;
use crate::key_events::parse_dom_node_id;
use crate::table_system::WebTableRegistry;

/// Maps a native DOM event on `target` (the event target element inside the mount) to a custom
/// event name, or `None` to ignore it (e.g. a `close` superseded by a programmatic reopen).
pub type NativeEventMap = fn(&web_sys::Event, &web_sys::Element) -> Option<&'static str>;

/// One native DOM event a [`crate::WebTableSystem`] bridges into the event queue as a named
/// custom event.
#[derive(Debug, Clone, Copy)]
pub struct NativeEventBridge {
    pub event: &'static str,
    /// Listen in the capture phase (needed for non-bubbling events such as `close`).
    pub capture: bool,
    /// Mirrors the DOM event's `bubbles`: `false` sets [`EventFlags::NO_BUBBLE`] so dispatch
    /// targets the node only (a nested overlay's `close` never closes an enclosing one).
    pub bubbles: bool,
    pub map: NativeEventMap,
}

/// Mount-root listeners mirroring the registry's bridges, rebuilt per registry revision.
#[derive(Default)]
pub(crate) struct NativeEventBridges {
    revision: Option<u32>,
    listeners: Vec<DomListenerGuard>,
}

impl NativeEventBridges {
    #[inline]
    pub(crate) fn is_stale(&self, registry: &WebTableRegistry) -> bool {
        self.revision != Some(registry.revision())
    }

    /// Re-attaches one listener per distinct `(event, capture, bubbles)` declared by `registry`.
    pub(crate) fn refresh(
        &mut self,
        registry: &WebTableRegistry,
        root: &web_sys::Element,
        sinks: &EventSinks,
    ) {
        self.revision = Some(registry.revision());
        // Dropping the previous set detaches its listeners.
        self.listeners.clear();
        let mut groups: Vec<(NativeEventBridge, Vec<NativeEventMap>)> = Vec::new();
        for bridge in registry
            .iter()
            .iter()
            .flat_map(|sys| sys.native_event_bridges())
        {
            match groups.iter_mut().find(|(head, _)| {
                head.event == bridge.event
                    && head.capture == bridge.capture
                    && head.bubbles == bridge.bubbles
            }) {
                Some((_, maps)) => maps.push(bridge.map),
                None => groups.push((*bridge, vec![bridge.map])),
            }
        }
        self.listeners.extend(groups.into_iter().map(|(head, maps)| {
            let cb = bridge_listener(root.clone(), head.bubbles, maps, sinks.clone());
            DomListenerGuard::attach(root.clone().into(), head.event, head.capture, cb)
        }));
    }
}

/// Pushes the named custom event at the event target's own node (target-only via
/// [`EventFlags::NO_BUBBLE`] when `!bubbles`), only when the target IS an NVE node: a nested
/// non-NVE element's close must not reach an ancestor overlay. `Fn` (not `FnMut`): a listener
/// re-entered by a synchronous synthetic event dispatched from app code must not hit
/// wasm-bindgen's recursive-borrow abort.
fn bridge_listener(
    root: web_sys::Element,
    bubbles: bool,
    maps: Vec<NativeEventMap>,
    sinks: EventSinks,
) -> Closure<dyn Fn(web_sys::Event)> {
    Closure::<dyn Fn(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let Some(el) = target_to_element(ev.target()).filter(|el| root.contains(Some(el))) else {
            return;
        };
        let Some(name) = maps.iter().find_map(|map| map(&ev, &el)) else {
            return;
        };
        let Some(node_id) = el
            .get_attribute("data-nve-id")
            .and_then(|raw| parse_dom_node_id(&raw))
        else {
            return;
        };
        let flags = EventFlags::EMPTY.with_bubbles(bubbles);
        sinks.emit(|q, sid, ts| q.push_custom_with_flags(sid, node_id, ts, name, &[], flags));
    })
}
