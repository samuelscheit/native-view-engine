//! Direct typed keyboard/pointer/input event routing, CJK IME composition guards, and
//! refcounted lazy single-root `wasm32` DOM event delegation backed by [`SurfaceEventTracker`].

use nve_abi::{EventId, NodeId};
use nve_tables::tables::{Interaction, Layout, Link, Scroll, TextInput};
use nve_store::{ChangeSet, HasEvents, NodeStore, SurfaceEventTracker, TableSlot};

pub use crate::input_attrs::{ImeCompositionGuard, InputSyncTracker, IME_PROCESS_KEY_CODE};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use nve_abi::TimestampUs;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
pub(crate) use crate::events_dom::RootEventDelegator;
#[cfg(target_arch = "wasm32")]
use crate::RenderCallback;

/// Dense node-kind flag: node carries a [`TextInput`] table.
pub const NODE_KIND_INPUT: u8 = 1 << 0;
/// Dense node-kind flag: node carries a [`Link`] table.
pub const NODE_KIND_LINK: u8 = 1 << 1;

/// Per-surface event subscription tracker + dense node-index kind flags (zero `HashMap`).
#[derive(Debug, Clone)]
pub struct WebEventState {
    pub tracker: SurfaceEventTracker,
    pub node_kinds: Vec<u8>,
    pub input_count: u32,
}

impl Default for WebEventState {
    fn default() -> Self {
        Self::new()
    }
}

impl WebEventState {
    pub fn new() -> Self {
        let mut tracker = SurfaceEventTracker::new();
        tracker.track::<Interaction>();
        tracker.track::<TextInput>();
        tracker.track::<Scroll>();
        tracker.track::<Layout>();
        Self {
            tracker,
            node_kinds: Vec::new(),
            input_count: 0,
        }
    }

    #[inline]
    pub fn is_input(&self, id: NodeId) -> bool {
        (self.node_kinds.get(id.index()).copied().unwrap_or(0) & NODE_KIND_INPUT) != 0
    }

    #[inline]
    pub fn is_link(&self, id: NodeId) -> bool {
        (self.node_kinds.get(id.index()).copied().unwrap_or(0) & NODE_KIND_LINK) != 0
    }

    #[inline]
    pub fn listens<T: HasEvents>(&self, id: NodeId, event: T::Event) -> bool {
        self.tracker.listeners_of::<T>(id).contains(event)
    }

    pub fn sync(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        on_transition: impl FnMut(TableSlot, EventId, bool),
    ) {
        for &removed in &changes.removed {
            let idx = removed.index();
            if idx < self.node_kinds.len() {
                let prev = self.node_kinds[idx];
                if (prev & NODE_KIND_INPUT) != 0 {
                    self.input_count = self.input_count.saturating_sub(1);
                }
                self.node_kinds[idx] = 0;
            }
        }
        for &id in changes.created.iter().chain(changes.touched.iter()) {
            if !store.is_live(id) {
                continue;
            }
            let mut next = 0u8;
            if store.has::<TextInput>(id) {
                next |= NODE_KIND_INPUT;
            }
            if store.has::<Link>(id) {
                next |= NODE_KIND_LINK;
            }
            let idx = id.index();
            let prev = self.node_kinds.get(idx).copied().unwrap_or(0);
            if prev != next {
                if (next != 0) && idx >= self.node_kinds.len() {
                    self.node_kinds.resize(idx + 1, 0);
                }
                if idx < self.node_kinds.len() {
                    self.node_kinds[idx] = next;
                }
                match ((prev & NODE_KIND_INPUT) != 0, (next & NODE_KIND_INPUT) != 0) {
                    (false, true) => self.input_count += 1,
                    (true, false) => self.input_count = self.input_count.saturating_sub(1),
                    _ => {}
                }
            }
        }
        self.tracker.sync(store, changes, on_transition);
    }

    pub fn reset(&mut self) {
        self.tracker.reset();
        self.node_kinds.fill(0);
        self.input_count = 0;
    }
}

/// Returns `true` when an `<a href="...">` explicitly opts out of client-side SPA interception
/// via `data-nve-hard-nav="true"`.
#[inline]
pub fn is_hard_nav_anchor(hard_nav_attr: Option<&str>) -> bool {
    hard_nav_attr.is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// Returns `true` when a hydrated click on an `<a href="...">` inside an interactive
/// `InteractionEvent::Press` node should call `preventDefault()` and emit `Press` for
/// zero-reload SPA navigation.
#[inline]
pub fn should_intercept_anchor_click(
    button: i16,
    meta: bool,
    ctrl: bool,
    shift: bool,
    alt: bool,
    target_attr: Option<&str>,
    has_download: bool,
) -> bool {
    if button != 0 || meta || ctrl || shift || alt || has_download {
        return false;
    }
    if let Some(target) = target_attr {
        let trimmed = target.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("_self") {
            return false;
        }
    }
    true
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_us() -> TimestampUs {
    let us = web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| (p.now() * 1000.0) as u64)
        .unwrap_or(0);
    TimestampUs(us)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_dom_input_value(el: &web_sys::Element) -> String {
    crate::input_attrs::read_dom_control_value(el).unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn target_to_element(target: Option<web_sys::EventTarget>) -> Option<web_sys::Element> {
    let t = target?;
    if let Ok(el) = t.clone().dyn_into::<web_sys::Element>() {
        Some(el)
    } else if let Ok(node) = t.dyn_into::<web_sys::Node>() {
        node.parent_element()
    } else {
        None
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn collect_ancestors(
    start: &web_sys::Element,
    root: &web_sys::Element,
) -> Vec<(NodeId, web_sys::Element)> {
    let mut out = Vec::with_capacity(8);
    let mut cur = Some(start.clone());
    while let Some(el) = cur {
        if let Some(id) = el
            .get_attribute("data-nve-id")
            .and_then(|raw| crate::key_events::parse_dom_node_id(&raw))
        {
            out.push((id, el.clone()));
        }
        if el.is_same_node(Some(root)) {
            break;
        }
        cur = el.parent_element();
    }
    out
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct DomListenerGuard {
    target: web_sys::EventTarget,
    name: &'static str,
    capture: bool,
    cb: Closure<dyn Fn(web_sys::Event)>,
}

#[cfg(target_arch = "wasm32")]
impl DomListenerGuard {
    pub(crate) fn attach(
        target: web_sys::EventTarget,
        name: &'static str,
        capture: bool,
        cb: Closure<dyn Fn(web_sys::Event)>,
    ) -> Self {
        let _ = target.add_event_listener_with_callback_and_bool(
            name,
            cb.as_ref().unchecked_ref(),
            capture,
        );
        Self {
            target,
            name,
            capture,
            cb,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for DomListenerGuard {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback_and_bool(
            self.name,
            self.cb.as_ref().unchecked_ref(),
            self.capture,
        );
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn notify_event(on_event: &Rc<RefCell<Option<RenderCallback>>>) {
    let cb = on_event.borrow().clone();
    if let Some(cb) = cb {
        cb();
    }
}

#[cfg(test)]
mod tests {
    use super::{is_hard_nav_anchor, should_intercept_anchor_click};

    #[test]
    fn hard_nav_anchor_attribute_bypasses_spa_interception() {
        assert!(is_hard_nav_anchor(Some("true")));
        assert!(is_hard_nav_anchor(Some(" TRUE ")));
        assert!(!is_hard_nav_anchor(None));
        assert!(!is_hard_nav_anchor(Some("")));
        assert!(!is_hard_nav_anchor(Some("false")));
        assert!(should_intercept_anchor_click(
            0, false, false, false, false, None, false
        ));
    }
}
