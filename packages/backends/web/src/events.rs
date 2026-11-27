//! Direct typed keyboard/pointer/input event routing, CJK IME composition guards, and
//! single-root `wasm32` DOM event delegation (`closest('[data-nve-id]')` -> `NodeEventMeta`).

use nve_abi::NodeId;

pub use crate::input_attrs::{ImeCompositionGuard, InputSyncTracker, IME_PROCESS_KEY_CODE};

#[cfg(target_arch = "wasm32")]
use std::cell::{Cell, RefCell};
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use nve_abi::hash::HashMap;
#[cfg(target_arch = "wasm32")]
use nve_abi::{
    pack_key_modifiers, EventMask, EventPayload, NativeEvent, NativeEventKind, SurfaceId,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::input_attrs::{
    web_composition_cancel_on_blur, web_composition_end, web_composition_start,
    web_composition_update,
};
#[cfg(target_arch = "wasm32")]
use crate::RenderCallback;

/// Per-node event routing metadata consulted by the root event delegator (zero `ComponentKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeEventMeta {
    pub is_input: bool,
    pub is_link: bool,
    pub mask: u64,
}

/// Pure ancestor-chain resolver used by root event delegation (testable on native targets).
pub fn resolve_delegated_node<F>(
    ancestor_chain: &[(NodeId, NodeEventMeta)],
    predicate: F,
) -> Option<NodeId>
where
    F: Fn(NodeEventMeta) -> bool,
{
    ancestor_chain
        .iter()
        .find_map(|&(id, meta)| predicate(meta).then_some(id))
}

/// Returns `true` when an `<a href="...">` explicitly opts out of client-side SPA interception
/// via `data-nve-hard-nav="true"`.
#[inline]
pub fn is_hard_nav_anchor(hard_nav_attr: Option<&str>) -> bool {
    hard_nav_attr.is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// Returns `true` when a hydrated click on an `<a href="...">` inside an interactive
/// `EventMask::PRESS` node should call `preventDefault()` and emit `NativeEventKind::Press` for
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
pub(crate) fn now_us() -> u64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| (p.now() * 1000.0) as u64)
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
fn read_dom_input_value(el: &web_sys::Element) -> String {
    if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        inp.value()
    } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        ta.value()
    } else if let Some(sel) = el.dyn_ref::<web_sys::HtmlSelectElement>() {
        sel.value()
    } else {
        String::new()
    }
}

#[cfg(target_arch = "wasm32")]
fn target_to_element(target: Option<web_sys::EventTarget>) -> Option<web_sys::Element> {
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
fn collect_ancestors(
    start: &web_sys::Element,
    root: &web_sys::Element,
    meta_map: &HashMap<NodeId, NodeEventMeta>,
) -> Vec<(NodeId, NodeEventMeta, web_sys::Element)> {
    let mut out = Vec::with_capacity(8);
    let mut cur = Some(start.clone());
    while let Some(el) = cur {
        if let Some(raw) = el.get_attribute("data-nve-id")
            && let Ok(id) = raw.parse::<NodeId>()
        {
            let meta = meta_map.get(&id).copied().unwrap_or_default();
            out.push((id, meta, el.clone()));
        }
        if el.is_same_node(Some(root)) {
            break;
        }
        cur = el.parent_element();
    }
    out
}

/// Owns the delegated DOM event closures attached to the single mount root on `wasm32`.
#[cfg(target_arch = "wasm32")]
type DomEventListener = (web_sys::EventTarget, &'static str, bool, Closure<dyn FnMut(web_sys::Event)>);

#[cfg(target_arch = "wasm32")]
pub(crate) struct RootEventDelegator {
    listeners: Vec<DomEventListener>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for RootEventDelegator {
    fn drop(&mut self) {
        for (target, name, capture, cb) in &self.listeners {
            let _ = target.remove_event_listener_with_callback_and_bool(
                name,
                cb.as_ref().unchecked_ref(),
                *capture,
            );
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn notify_event(on_event: &Rc<RefCell<Option<RenderCallback>>>) {
    let cb = on_event.borrow().clone();
    if let Some(cb) = cb {
        cb();
    }
}

#[cfg(target_arch = "wasm32")]
fn register_listener(
    listeners: &mut Vec<DomEventListener>,
    target: &web_sys::EventTarget,
    name: &'static str,
    capture: bool,
    cb: Closure<dyn FnMut(web_sys::Event)>,
) {
    let _ = target.add_event_listener_with_callback_and_bool(
        name,
        cb.as_ref().unchecked_ref(),
        capture,
    );
    listeners.push((target.clone(), name, capture, cb));
}

#[cfg(target_arch = "wasm32")]
impl RootEventDelegator {
    pub(crate) fn attach(
        root: web_sys::Element,
        surface_id: Rc<Cell<SurfaceId>>,
        events: Rc<RefCell<Vec<NativeEvent>>>,
        meta: Rc<RefCell<HashMap<NodeId, NodeEventMeta>>>,
        input_sync: Rc<RefCell<InputSyncTracker>>,
        ime: Rc<ImeCompositionGuard>,
        on_event: Rc<RefCell<Option<RenderCallback>>>,
    ) -> Self {
        let mut listeners: Vec<DomEventListener> = Vec::new();
        let root_target: web_sys::EventTarget = root.clone().into();

        // Click -> NativeEventKind::Press
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                let press_ev = if let Some((id, _, _target_el)) = chain
                    .iter()
                    .find(|(_, meta, _)| (meta.mask & EventMask::PRESS) != 0)
                    && let Some(me) = ev.dyn_ref::<web_sys::MouseEvent>()
                {
                    if let Ok(Some(anchor)) = el.closest("a[href]")
                        && r.contains(Some(&anchor))
                    {
                        let hard = is_hard_nav_anchor(
                            anchor.get_attribute("data-nve-hard-nav").as_deref(),
                        );
                        if !hard
                            && should_intercept_anchor_click(
                                me.button(),
                                me.meta_key(),
                                me.ctrl_key(),
                                me.shift_key(),
                                me.alt_key(),
                                anchor.get_attribute("target").as_deref(),
                                anchor.has_attribute("download"),
                            )
                        {
                            ev.prevent_default();
                        }
                    }
                    Some(NativeEvent {
                        kind: NativeEventKind::Press,
                        surface_id: sid.get(),
                        node_id: *id,
                        timestamp_us: now_us(),
                        payload: EventPayload::Point {
                            x: me.client_x() as f32,
                            y: me.client_y() as f32,
                        },
                    })
                } else {
                    None
                };
                if let Some(ev) = press_ev {
                    evs.borrow_mut().push(ev);
                    notify_event(&notify);
                }
            });
            register_listener(&mut listeners, &root_target, "click", false, cb);
        }

        // Input -> NativeEventKind::TextChange
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let sync = Rc::clone(&input_sync);
            let ime_g = Rc::clone(&ime);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                if ime_g.should_suppress_input() {
                    return;
                }
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                let found = chain
                    .iter()
                    .find(|(_, meta, _)| meta.is_input || (meta.mask & EventMask::TEXT_CHANGE) != 0)
                    .map(|(id, _, target_el)| (*id, read_dom_input_value(target_el)));
                if let Some((id, val)) = found {
                    let changed = sync.borrow_mut().on_input_event(id, val, false);
                    if let Some(changed) = changed {
                        evs.borrow_mut().push(NativeEvent {
                            kind: NativeEventKind::TextChange,
                            surface_id: sid.get(),
                            node_id: id,
                            timestamp_us: now_us(),
                            payload: EventPayload::Text(changed),
                        });
                        notify_event(&notify);
                    }
                }
            });
            register_listener(&mut listeners, &root_target, "input", false, cb);
        }

        // Composition start / update / end
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let ime_g = Rc::clone(&ime);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                if let Some((id, _, _)) = chain.first() {
                    evs.borrow_mut()
                        .push(web_composition_start(&ime_g, sid.get(), *id, now_us()));
                    notify_event(&notify);
                }
            });
            register_listener(&mut listeners, &root_target, "compositionstart", false, cb);
        }
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let ime_g = Rc::clone(&ime);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                if let Some((id, _, _)) = chain.first() {
                    let data = ev
                        .dyn_ref::<web_sys::CompositionEvent>()
                        .and_then(|c| c.data())
                        .unwrap_or_default();
                    evs.borrow_mut().push(web_composition_update(
                        &ime_g,
                        sid.get(),
                        *id,
                        now_us(),
                        data,
                    ));
                    notify_event(&notify);
                }
            });
            register_listener(&mut listeners, &root_target, "compositionupdate", false, cb);
        }
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let sync = Rc::clone(&input_sync);
            let ime_g = Rc::clone(&ime);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                if let Some((id, _, target_el)) = chain.first() {
                    let committed = ev
                        .dyn_ref::<web_sys::CompositionEvent>()
                        .and_then(|c| c.data())
                        .unwrap_or_default();
                    let full = read_dom_input_value(target_el);
                    let pair = {
                        let mut guard = sync.borrow_mut();
                        web_composition_end(
                            &ime_g,
                            &mut guard,
                            sid.get(),
                            *id,
                            now_us(),
                            committed,
                            full,
                        )
                    };
                    evs.borrow_mut().extend(pair);
                    notify_event(&notify);
                }
            });
            register_listener(&mut listeners, &root_target, "compositionend", false, cb);
        }

        // Focus / Blur
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                let focus_id = chain
                    .iter()
                    .find(|(_, meta, _)| (meta.mask & EventMask::FOCUS) != 0)
                    .map(|(id, _, _)| *id);
                if let Some(id) = focus_id {
                    evs.borrow_mut().push(NativeEvent {
                        kind: NativeEventKind::Focus,
                        surface_id: sid.get(),
                        node_id: id,
                        timestamp_us: now_us(),
                        payload: EventPayload::Empty,
                    });
                    notify_event(&notify);
                }
            });
            register_listener(&mut listeners, &root_target, "focusin", false, cb);
        }
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let ime_g = Rc::clone(&ime);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                let cancel = chain
                    .first()
                    .and_then(|(id, _, _)| web_composition_cancel_on_blur(&ime_g, sid.get(), *id, now_us()));
                let blur_id = chain
                    .iter()
                    .find(|(_, meta, _)| (meta.mask & EventMask::BLUR) != 0)
                    .map(|(id, _, _)| *id);
                let mut pushed = false;
                if let Some(c) = cancel {
                    evs.borrow_mut().push(c);
                    pushed = true;
                }
                if let Some(id) = blur_id {
                    evs.borrow_mut().push(NativeEvent {
                        kind: NativeEventKind::Blur,
                        surface_id: sid.get(),
                        node_id: id,
                        timestamp_us: now_us(),
                        payload: EventPayload::Empty,
                    });
                    pushed = true;
                }
                if pushed {
                    notify_event(&notify);
                }
            });
            register_listener(&mut listeners, &root_target, "focusout", false, cb);
        }

        // Scroll (capture phase since `scroll` does not bubble in DOM)
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                let scroll_target = chain
                    .iter()
                    .find(|(_, meta, _)| (meta.mask & EventMask::SCROLL) != 0)
                    .map(|(id, _, target_el)| {
                        (
                            *id,
                            target_el.scroll_left() as f32,
                            target_el.scroll_top() as f32,
                            target_el.scroll_width() as f32,
                            target_el.scroll_height() as f32,
                        )
                    });
                if let Some((id, x, y, content_width, content_height)) = scroll_target {
                    evs.borrow_mut().push(NativeEvent {
                        kind: NativeEventKind::Scroll,
                        surface_id: sid.get(),
                        node_id: id,
                        timestamp_us: now_us(),
                        payload: EventPayload::Scroll {
                            x,
                            y,
                            content_width,
                            content_height,
                        },
                    });
                    notify_event(&notify);
                }
            });
            register_listener(&mut listeners, &root_target, "scroll", true, cb);
        }

        // KeyDown (attached to window so global shortcuts like Cmd+K / Escape always reach ROOT)
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let ime_g = Rc::clone(&ime);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let key_target: web_sys::EventTarget = web_sys::window()
                .map(Into::into)
                .unwrap_or_else(|| root_target.clone());
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(ke) = ev.dyn_ref::<web_sys::KeyboardEvent>() else { return };
                if ime_g.should_suppress_keydown(ke.is_composing(), ke.key_code()) {
                    return;
                }
                let el = target_to_element(ev.target());
                // Scope global keydown to this surface: ignore events targeting another mounted
                // NVE surface's subtree so multi-surface pages never cross-fire.
                if let Some(ref e) = el
                    && !r.contains(Some(e))
                    && e.closest("[data-nve-live],[data-nve-id]").ok().flatten().is_some()
                {
                    return;
                }
                let chain = el
                    .as_ref()
                    .filter(|e| r.contains(Some(e)))
                    .map(|e| collect_ancestors(e, &r, &m.borrow()))
                    .unwrap_or_default();
                let key = ke.key();
                let in_this_surface = el.as_ref().is_none_or(|e| {
                    r.contains(Some(e))
                        || e.tag_name().eq_ignore_ascii_case("BODY")
                        || e.tag_name().eq_ignore_ascii_case("HTML")
                });
                if in_this_surface
                    && (ke.meta_key() || ke.ctrl_key())
                    && key.eq_ignore_ascii_case("k")
                {
                    ev.prevent_default();
                }

                let submit_ev = if key == "Enter"
                    && let Some(ref target_el) = el
                    && target_el.dyn_ref::<web_sys::HtmlInputElement>().is_some()
                    && let Some((submit_id, _, _)) = chain
                        .iter()
                        .find(|(_, meta, _)| (meta.mask & EventMask::SUBMIT) != 0)
                {
                    ev.prevent_default();
                    Some(NativeEvent {
                        kind: NativeEventKind::Submit,
                        surface_id: sid.get(),
                        node_id: *submit_id,
                        timestamp_us: now_us(),
                        payload: EventPayload::Text(read_dom_input_value(target_el)),
                    })
                } else {
                    None
                };

                if let Some(ev) = submit_ev {
                    evs.borrow_mut().push(ev);
                    notify_event(&notify);
                    return;
                }
                if !in_this_surface {
                    return;
                }

                let target_id = chain
                    .iter()
                    .find(|(_, meta, _)| (meta.mask & EventMask::KEY_DOWN) != 0)
                    .map(|(id, _, _)| *id)
                    .or_else(|| chain.first().map(|(id, _, _)| *id))
                    .unwrap_or(nve_abi::ROOT_NODE_ID);
                let modifiers = pack_key_modifiers(
                    ke.meta_key(),
                    ke.ctrl_key(),
                    ke.shift_key(),
                    ke.alt_key(),
                    ke.repeat(),
                );
                evs.borrow_mut().push(NativeEvent {
                    kind: NativeEventKind::KeyDown,
                    surface_id: sid.get(),
                    node_id: target_id,
                    timestamp_us: now_us(),
                    payload: EventPayload::Key {
                        key,
                        code: ke.code(),
                        modifiers,
                        key_code: ke.key_code(),
                        is_composing: ke.is_composing(),
                    },
                });
                notify_event(&notify);
            });
            register_listener(&mut listeners, &key_target, "keydown", false, cb);
        }

        // Form Submit -> NativeEventKind::Submit
        {
            let r = root.clone();
            let evs = Rc::clone(&events);
            let m = Rc::clone(&meta);
            let sid = Rc::clone(&surface_id);
            let notify = Rc::clone(&on_event);
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else { return };
                let chain = collect_ancestors(&el, &r, &m.borrow());
                let submit_id = chain
                    .iter()
                    .find(|(_, meta, _)| (meta.mask & EventMask::SUBMIT) != 0)
                    .map(|(id, _, _)| *id);
                if let Some(id) = submit_id {
                    ev.prevent_default();
                    let val = el
                        .query_selector("input,textarea,select")
                        .ok()
                        .flatten()
                        .map(|ctrl| read_dom_input_value(&ctrl))
                        .unwrap_or_else(|| read_dom_input_value(&el));
                    evs.borrow_mut().push(NativeEvent {
                        kind: NativeEventKind::Submit,
                        surface_id: sid.get(),
                        node_id: id,
                        timestamp_us: now_us(),
                        payload: EventPayload::Text(val),
                    });
                    notify_event(&notify);
                }
            });
            register_listener(&mut listeners, &root_target, "submit", false, cb);
        }

        Self { listeners }
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
