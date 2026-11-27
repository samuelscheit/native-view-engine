//! Plugin-declared native DOM event bridges (`wasm32`): capability plugins list DOM events (e.g.
//! `<dialog>` `close`, `[popover]` `toggle`) whose occurrences must reach the app as
//! `NativeEventKind::Custom` events, without the backend knowing any element semantics.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nve_abi::hash::HashMap;
use nve_abi::{EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId, SurfaceId};
use wasm_bindgen::closure::Closure;

use crate::RenderCallback;
use crate::events::{
    DomEventListener, NodeEventMeta, RootEventDelegator, collect_ancestors, notify_event, now_us,
    register_listener, target_to_element,
};
use crate::table_system::WebTableRegistry;

/// Maps a native DOM event on `target` (the event target element inside the mount) to a custom
/// event name, or `None` to ignore it (e.g. a `close` superseded by a programmatic reopen).
pub type NativeEventMap = fn(&web_sys::Event, &web_sys::Element) -> Option<&'static str>;

/// One native DOM event a [`crate::WebTableSystem`] bridges into the event queue.
#[derive(Debug, Clone, Copy)]
pub struct NativeEventBridge {
    pub event: &'static str,
    /// Listen in the capture phase (needed for non-bubbling events such as `close`).
    pub capture: bool,
    pub map: NativeEventMap,
}

/// Shared event-queue handles a bridge listener pushes into.
pub(crate) struct EventSinks {
    pub(crate) surface_id: Rc<Cell<SurfaceId>>,
    pub(crate) pending_events: Rc<RefCell<Vec<NativeEvent>>>,
    pub(crate) node_event_meta: Rc<RefCell<HashMap<NodeId, NodeEventMeta>>>,
    pub(crate) on_event: Rc<RefCell<Option<RenderCallback>>>,
}

/// Mount-root listeners mirroring the registry's bridges, rebuilt per registry revision.
#[derive(Default)]
pub(crate) struct NativeEventBridges {
    revision: Option<u32>,
    listeners: Option<RootEventDelegator>,
}

impl NativeEventBridges {
    #[inline]
    pub(crate) fn is_stale(&self, registry: &WebTableRegistry) -> bool {
        self.revision != Some(registry.revision())
    }

    /// Re-attaches one listener per distinct `(event, capture)` declared by `registry`.
    pub(crate) fn refresh(
        &mut self,
        registry: &WebTableRegistry,
        root: &web_sys::Element,
        sinks: &EventSinks,
    ) {
        self.revision = Some(registry.revision());
        // Dropping the previous set detaches its listeners.
        self.listeners = None;
        let mut groups: Vec<(&'static str, bool, Vec<NativeEventMap>)> = Vec::new();
        for bridge in registry
            .iter()
            .iter()
            .flat_map(|sys| sys.native_event_bridges())
        {
            match groups
                .iter_mut()
                .find(|(event, capture, _)| *event == bridge.event && *capture == bridge.capture)
            {
                Some((_, _, maps)) => maps.push(bridge.map),
                None => groups.push((bridge.event, bridge.capture, vec![bridge.map])),
            }
        }
        if groups.is_empty() {
            return;
        }
        let target: web_sys::EventTarget = root.clone().into();
        let mut listeners: Vec<DomEventListener> = Vec::with_capacity(groups.len());
        for (event, capture, maps) in groups {
            let cb = bridge_listener(root.clone(), maps, sinks);
            register_listener(&mut listeners, &target, event, capture, cb);
        }
        self.listeners = Some(RootEventDelegator::from_listeners(listeners));
    }
}

/// Pushes `Custom { name }` at the event target's own node (only when the target IS an NVE node:
/// a nested non-NVE element's close must not reach an ancestor overlay) when some node on its
/// ancestor chain listens for custom events.
fn bridge_listener(
    root: web_sys::Element,
    maps: Vec<NativeEventMap>,
    sinks: &EventSinks,
) -> Closure<dyn FnMut(web_sys::Event)> {
    let sid = Rc::clone(&sinks.surface_id);
    let evs = Rc::clone(&sinks.pending_events);
    let meta = Rc::clone(&sinks.node_event_meta);
    let notify = Rc::clone(&sinks.on_event);
    Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let Some(el) = target_to_element(ev.target()).filter(|el| root.contains(Some(el))) else {
            return;
        };
        let Some(name) = maps.iter().find_map(|map| map(&ev, &el)) else {
            return;
        };
        let chain = collect_ancestors(&el, &root, &meta.borrow());
        let Some((node_id, _, target_el)) = chain.first() else {
            return;
        };
        let node_id = *node_id;
        if !target_el.is_same_node(Some(&el)) {
            return;
        }
        if !chain
            .iter()
            .any(|(_, m, _)| m.mask & EventMask::CUSTOM != 0)
        {
            return;
        }
        evs.borrow_mut().push(NativeEvent {
            kind: NativeEventKind::Custom,
            surface_id: sid.get(),
            node_id,
            timestamp_us: now_us(),
            payload: EventPayload::Custom {
                name: name.to_owned(),
                data: Vec::new(),
            },
        });
        notify_event(&notify);
    })
}
