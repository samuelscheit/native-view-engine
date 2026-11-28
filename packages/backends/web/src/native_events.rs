//! Plugin-declared native DOM event bridges (`wasm32`): capability plugins list DOM events (e.g.
//! `<dialog>` `close`, `[popover]` `toggle`) whose occurrences must reach the app as
//! named custom (`NVE_TABLE_CUSTOM`) events, without the backend knowing any element semantics.

#[cfg(target_arch = "wasm32")]
use nve_abi::EventFlags;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;

#[cfg(target_arch = "wasm32")]
use crate::events::{target_to_element, DomListenerGuard};
#[cfg(target_arch = "wasm32")]
use crate::events_dom::EventSinks;
#[cfg(target_arch = "wasm32")]
use crate::key_events::parse_dom_node_id;
#[cfg(target_arch = "wasm32")]
use crate::table_system::WebTableRegistry;

/// Grouped bridges sharing one `(event, capture)` DOM listener, in plugin priority order.
pub(crate) struct BridgeGroup<M> {
    pub(crate) event: &'static str,
    pub(crate) capture: bool,
    pub(crate) entries: Vec<(M, bool)>,
}

/// Groups bridge entries by `(event, capture)` while preserving plugin priority order and each
/// entry's `bubbles` flag, so two plugins sharing `(event, capture)` with different `bubbles`
/// attach one root listener with first-match-wins semantics.
pub(crate) fn group_native_bridges<M: Copy>(
    bridges: impl IntoIterator<Item = (&'static str, bool, bool, M)>,
) -> Vec<BridgeGroup<M>> {
    let mut groups: Vec<BridgeGroup<M>> = Vec::new();
    for (event, capture, bubbles, map) in bridges {
        match groups
            .iter_mut()
            .find(|g| g.event == event && g.capture == capture)
        {
            Some(group) => group.entries.push((map, bubbles)),
            None => groups.push(BridgeGroup {
                event,
                capture,
                entries: vec![(map, bubbles)],
            }),
        }
    }
    groups
}

/// Evaluates grouped `(map, bubbles)` entries in priority order, returning the first matched
/// `(custom_name, bubbles)` pair.
#[inline]
pub(crate) fn match_bridge_entry<M: Copy>(
    entries: &[(M, bool)],
    mut eval: impl FnMut(M) -> Option<&'static str>,
) -> Option<(&'static str, bool)> {
    entries
        .iter()
        .find_map(|&(map, bubbles)| eval(map).map(|name| (name, bubbles)))
}

/// Maps a native DOM event on `target` (the event target element inside the mount) to a custom
/// event name, or `None` to ignore it (e.g. a `close` superseded by a programmatic reopen).
#[cfg(target_arch = "wasm32")]
pub type NativeEventMap = fn(&web_sys::Event, &web_sys::Element) -> Option<&'static str>;

/// One native DOM event a [`crate::WebTableSystem`] bridges into the event queue as a named
/// custom event.
#[cfg(target_arch = "wasm32")]
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
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct NativeEventBridges {
    revision: Option<u32>,
    listeners: Vec<DomListenerGuard>,
}

#[cfg(target_arch = "wasm32")]
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
        self.listeners.clear();
        let groups = group_native_bridges(
            registry
                .iter()
                .iter()
                .flat_map(|sys| sys.native_event_bridges())
                .map(|b| (b.event, b.capture, b.bubbles, b.map)),
        );
        self.listeners.extend(groups.into_iter().map(|group| {
            let cb = bridge_listener(root.clone(), group.entries, sinks.clone());
            DomListenerGuard::attach(root.clone().into(), group.event, group.capture, cb)
        }));
    }
}

/// Pushes the named custom event at the event target's own node (target-only via
/// [`EventFlags::NO_BUBBLE`] when `!bubbles`), only when the target IS an NVE node: a nested
/// non-NVE element's close must not reach an ancestor overlay. `Fn` (not `FnMut`): a listener
/// re-entered by a synchronous synthetic event dispatched from app code must not hit
/// wasm-bindgen's recursive-borrow abort.
#[cfg(target_arch = "wasm32")]
fn bridge_listener(
    root: web_sys::Element,
    entries: Vec<(NativeEventMap, bool)>,
    sinks: EventSinks,
) -> Closure<dyn Fn(web_sys::Event)> {
    Closure::<dyn Fn(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let Some(el) = target_to_element(ev.target()).filter(|el| root.contains(Some(el))) else {
            return;
        };
        let Some((name, bubbles)) = match_bridge_entry(&entries, |map| map(&ev, &el)) else {
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

#[cfg(test)]
mod tests {
    use super::{group_native_bridges, match_bridge_entry};

    #[test]
    fn groups_by_event_and_capture_preserving_first_match_wins_with_per_entry_bubbles() {
        type MockMap = fn(&str) -> Option<&'static str>;
        fn high(kind: &str) -> Option<&'static str> {
            (kind == "popover").then_some("close")
        }
        fn low(kind: &str) -> Option<&'static str> {
            (kind == "popover" || kind == "details").then_some("toggle")
        }
        fn bubble_phase(kind: &str) -> Option<&'static str> {
            (kind == "other").then_some("other")
        }

        let groups = group_native_bridges([
            ("toggle", true, false, high as MockMap),
            ("toggle", true, true, low as MockMap),
            ("toggle", false, true, bubble_phase as MockMap),
            ("close", true, false, high as MockMap),
        ]);
        assert_eq!(groups.len(), 3, "same (event, capture) with different bubbles merges into 1 listener");
        assert_eq!(groups[0].event, "toggle");
        assert!(groups[0].capture);
        assert_eq!(groups[0].entries.len(), 2);

        // First match wins with its own `bubbles: false` (second plugin never runs).
        assert_eq!(
            match_bridge_entry(&groups[0].entries, |m| m("popover")),
            Some(("close", false))
        );
        // When first declines, second wins with its own `bubbles: true`.
        assert_eq!(
            match_bridge_entry(&groups[0].entries, |m| m("details")),
            Some(("toggle", true))
        );
        assert_eq!(match_bridge_entry(&groups[0].entries, |m| m("none")), None);

        // Distinct capture / event stay in separate groups.
        assert_eq!((groups[1].event, groups[1].capture), ("toggle", false));
        assert_eq!((groups[2].event, groups[2].capture), ("close", true));
    }
}
