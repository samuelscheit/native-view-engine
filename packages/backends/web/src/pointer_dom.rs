//! `wasm32` root-delegated `Pointer*` event handlers, synchronous declarative pointer capture
//! (`data-nve-capture="primary"` -> `setPointerCapture`), and W3C `pointerover`/`pointerout` ->
//! non-bubbling `PointerLeave` (bottom-up) / `PointerEnter` (top-down) synthesis.

use std::cell::RefCell;
use std::rc::Rc;

use nve_abi::{
    EventFlags, InteractionEvent, NodeId, NVE_POINTER_TYPE_MOUSE, NVE_POINTER_TYPE_PEN,
    NVE_POINTER_TYPE_TOUCH,
};
use nve_tables::tables::Interaction;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::events::{collect_ancestors, target_to_element, DomListenerGuard};
use crate::events_dom::EventSinks;

#[inline]
fn pointer_type_code(pe: &web_sys::PointerEvent) -> u8 {
    let kind = pe.pointer_type();
    if kind == "pen" {
        NVE_POINTER_TYPE_PEN
    } else if kind == "touch" {
        NVE_POINTER_TYPE_TOUCH
    } else {
        NVE_POINTER_TYPE_MOUSE
    }
}

#[inline]
fn pointer_flags(pe: &web_sys::PointerEvent, button: u8, extra: EventFlags) -> EventFlags {
    (EventFlags::from_modifiers(
        pe.meta_key(),
        pe.ctrl_key(),
        pe.shift_key(),
        pe.alt_key(),
        false,
    ) | extra)
        .with_pointer_button(button)
}

fn on_root(
    root: &web_sys::Element,
    name: &'static str,
    cb: impl Fn(web_sys::Event) + 'static,
) -> DomListenerGuard {
    let cb = Closure::<dyn Fn(web_sys::Event)>::new(cb);
    DomListenerGuard::attach(root.clone().into(), name, false, cb)
}

/// Attaches core delegated pointer listeners (`pointerdown`, `pointermove`, `pointerup`,
/// `pointercancel`, `lostpointercapture`) on `root`.
pub(crate) fn attach_pointer_core(
    root: &web_sys::Element,
    sinks: &EventSinks,
    active_capture: &Rc<RefCell<Option<(i32, NodeId)>>>,
) -> [DomListenerGuard; 5] {
    [
        attach_pointerdown(root, sinks, active_capture),
        attach_pointermove(root, sinks, active_capture),
        attach_pointerup(root, sinks, active_capture),
        attach_pointercancel(root, sinks, active_capture),
        attach_lostpointercapture(root, sinks, active_capture),
    ]
}

/// Attaches `pointerover` + `pointerout` on `root` to synthesize non-bubbling W3C `PointerLeave`
/// (bottom-up) and `PointerEnter` (top-down) transitions via a reusable `entered_chain`.
pub(crate) fn attach_pointer_hover(
    root: &web_sys::Element,
    sinks: &EventSinks,
    entered_chain: &Rc<RefCell<Vec<NodeId>>>,
) -> [DomListenerGuard; 2] {
    [
        attach_pointerover(root, sinks, entered_chain),
        attach_pointerout(root, sinks, entered_chain),
    ]
}

fn attach_pointerdown(
    root: &web_sys::Element,
    sinks: &EventSinks,
    active_capture: &Rc<RefCell<Option<(i32, NodeId)>>>,
) -> DomListenerGuard {
    let r = root.clone();
    let sinks = sinks.clone();
    let capture = Rc::clone(active_capture);
    on_root(root, "pointerdown", move |ev| {
        let Some(pe) = ev.dyn_ref::<web_sys::PointerEvent>() else { return };
        let Some(el) = target_to_element(ev.target()) else { return };
        let chain = collect_ancestors(&el, &r);
        let Some(&(target_id, _)) = chain.first() else { return };

        let mut captured = false;
        if pe.button() == 0 {
            let cap_match = {
                let st = sinks.state.borrow();
                chain.iter().find(|(id, node_el)| {
                    st.has_capture_primary(*id)
                        || node_el.get_attribute("data-nve-capture").as_deref() == Some("primary")
                })
            };
            if let Some((cap_id, cap_el)) = cap_match {
                let _ = cap_el.set_pointer_capture(pe.pointer_id());
                *capture.borrow_mut() = Some((pe.pointer_id(), *cap_id));
                captured = true;
            }
        }

        let listens = sinks
            .find(&chain, |st, id| {
                st.listens::<Interaction>(id, InteractionEvent::PointerDown)
            })
            .is_some();
        if !listens {
            return;
        }
        let extra = if captured {
            EventFlags::CAPTURE
        } else {
            EventFlags::EMPTY
        };
        let (x, y) = (pe.client_x() as f32, pe.client_y() as f32);
        let pid = pe.pointer_id().max(0) as u32;
        let btn = pe.button().clamp(0, 7) as u8;
        let btns = pe.buttons();
        let ptype = pointer_type_code(pe);
        let flags = pointer_flags(pe, btn, extra);
        sinks.emit(|q, sid, ts| {
            q.push_pointer_ex(
                sid,
                target_id,
                ts,
                InteractionEvent::PointerDown,
                x,
                y,
                pid,
                btns,
                ptype,
                flags,
            );
        });
    })
}

fn attach_pointermove(
    root: &web_sys::Element,
    sinks: &EventSinks,
    active_capture: &Rc<RefCell<Option<(i32, NodeId)>>>,
) -> DomListenerGuard {
    let r = root.clone();
    let sinks = sinks.clone();
    let capture = Rc::clone(active_capture);
    on_root(root, "pointermove", move |ev| {
        let Some(pe) = ev.dyn_ref::<web_sys::PointerEvent>() else { return };
        let cap_target = capture
            .borrow()
            .filter(|&(pid, _)| pid == pe.pointer_id())
            .map(|(_, id)| id);
        let (target_id, extra) = if let Some(cap_id) = cap_target {
            (cap_id, EventFlags::CAPTURE | EventFlags::NO_BUBBLE)
        } else {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let Some(&(leaf_id, _)) = chain.first() else { return };
            let listens = sinks
                .find(&chain, |st, id| {
                    st.listens::<Interaction>(id, InteractionEvent::PointerMove)
                })
                .is_some();
            if !listens {
                return;
            }
            (leaf_id, EventFlags::EMPTY)
        };
        let (x, y) = (pe.client_x() as f32, pe.client_y() as f32);
        let pid = pe.pointer_id().max(0) as u32;
        let btn = pe.button().clamp(0, 7) as u8;
        let btns = pe.buttons();
        let ptype = pointer_type_code(pe);
        let flags = pointer_flags(pe, btn, extra);
        sinks.emit(|q, sid, ts| {
            q.push_pointer_ex(
                sid,
                target_id,
                ts,
                InteractionEvent::PointerMove,
                x,
                y,
                pid,
                btns,
                ptype,
                flags,
            );
        });
    })
}

fn attach_pointerup(
    root: &web_sys::Element,
    sinks: &EventSinks,
    active_capture: &Rc<RefCell<Option<(i32, NodeId)>>>,
) -> DomListenerGuard {
    let r = root.clone();
    let sinks = sinks.clone();
    let capture = Rc::clone(active_capture);
    on_root(root, "pointerup", move |ev| {
        let Some(pe) = ev.dyn_ref::<web_sys::PointerEvent>() else { return };
        let cap_target = {
            let mut slot = capture.borrow_mut();
            if slot.is_some_and(|(pid, _)| pid == pe.pointer_id()) {
                slot.take().map(|(_, id)| id)
            } else {
                None
            }
        };
        let (target_id, extra) = if let Some(cap_id) = cap_target {
            if let Some(el) = target_to_element(ev.target()) {
                let _ = el.release_pointer_capture(pe.pointer_id());
            }
            (cap_id, EventFlags::CAPTURE | EventFlags::NO_BUBBLE)
        } else {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let Some(&(leaf_id, _)) = chain.first() else { return };
            let listens = sinks
                .find(&chain, |st, id| {
                    st.listens::<Interaction>(id, InteractionEvent::PointerUp)
                })
                .is_some();
            if !listens {
                return;
            }
            (leaf_id, EventFlags::EMPTY)
        };
        let (x, y) = (pe.client_x() as f32, pe.client_y() as f32);
        let pid = pe.pointer_id().max(0) as u32;
        let btn = pe.button().clamp(0, 7) as u8;
        let btns = pe.buttons();
        let ptype = pointer_type_code(pe);
        let flags = pointer_flags(pe, btn, extra);
        sinks.emit(|q, sid, ts| {
            q.push_pointer_ex(
                sid,
                target_id,
                ts,
                InteractionEvent::PointerUp,
                x,
                y,
                pid,
                btns,
                ptype,
                flags,
            );
        });
    })
}

fn attach_pointercancel(
    root: &web_sys::Element,
    sinks: &EventSinks,
    active_capture: &Rc<RefCell<Option<(i32, NodeId)>>>,
) -> DomListenerGuard {
    let r = root.clone();
    let sinks = sinks.clone();
    let capture = Rc::clone(active_capture);
    on_root(root, "pointercancel", move |ev| {
        let Some(pe) = ev.dyn_ref::<web_sys::PointerEvent>() else { return };
        let cap_target = {
            let mut slot = capture.borrow_mut();
            if slot.is_some_and(|(pid, _)| pid == pe.pointer_id()) {
                slot.take().map(|(_, id)| id)
            } else {
                None
            }
        };
        let (target_id, extra) = if let Some(cap_id) = cap_target {
            (cap_id, EventFlags::CAPTURE | EventFlags::NO_BUBBLE)
        } else {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let Some(&(leaf_id, _)) = chain.first() else { return };
            let listens = sinks
                .find(&chain, |st, id| {
                    st.listens::<Interaction>(id, InteractionEvent::PointerCancel)
                })
                .is_some();
            if !listens {
                return;
            }
            (leaf_id, EventFlags::EMPTY)
        };
        let (x, y) = (pe.client_x() as f32, pe.client_y() as f32);
        let pid = pe.pointer_id().max(0) as u32;
        let ptype = pointer_type_code(pe);
        let flags = pointer_flags(pe, 0, extra);
        sinks.emit(|q, sid, ts| {
            q.push_pointer_ex(
                sid,
                target_id,
                ts,
                InteractionEvent::PointerCancel,
                x,
                y,
                pid,
                0,
                ptype,
                flags,
            );
        });
    })
}

fn attach_lostpointercapture(
    root: &web_sys::Element,
    sinks: &EventSinks,
    active_capture: &Rc<RefCell<Option<(i32, NodeId)>>>,
) -> DomListenerGuard {
    let sinks = sinks.clone();
    let capture = Rc::clone(active_capture);
    on_root(root, "lostpointercapture", move |ev| {
        let Some(pe) = ev.dyn_ref::<web_sys::PointerEvent>() else { return };
        let cap_id = {
            let mut slot = capture.borrow_mut();
            if slot.is_some_and(|(pid, _)| pid == pe.pointer_id()) {
                slot.take().map(|(_, id)| id)
            } else {
                None
            }
        };
        let Some(cap_id) = cap_id else { return };
        let (x, y) = (pe.client_x() as f32, pe.client_y() as f32);
        let pid = pe.pointer_id().max(0) as u32;
        let ptype = pointer_type_code(pe);
        let flags = pointer_flags(pe, 0, EventFlags::CAPTURE | EventFlags::NO_BUBBLE);
        sinks.emit(|q, sid, ts| {
            q.push_pointer_ex(
                sid,
                cap_id,
                ts,
                InteractionEvent::PointerCancel,
                x,
                y,
                pid,
                0,
                ptype,
                flags,
            );
        });
    })
}

fn attach_pointerover(
    root: &web_sys::Element,
    sinks: &EventSinks,
    entered_chain: &Rc<RefCell<Vec<NodeId>>>,
) -> DomListenerGuard {
    let r = root.clone();
    let sinks = sinks.clone();
    let entered = Rc::clone(entered_chain);
    on_root(root, "pointerover", move |ev| {
        let Some(pe) = ev.dyn_ref::<web_sys::PointerEvent>() else { return };
        let Some(el) = target_to_element(ev.target()) else { return };
        let next_ancestors = collect_ancestors(&el, &r);
        let (to_leave, to_enter) = {
            let mut prev = entered.borrow_mut();
            let st = sinks.state.borrow();
            let mut leaves = Vec::new();
            for &id in prev.iter() {
                if !next_ancestors.iter().any(|(nid, _)| *nid == id)
                    && st.listens::<Interaction>(id, InteractionEvent::PointerLeave)
                {
                    leaves.push(id);
                }
            }
            let mut enters = Vec::new();
            for &(id, _) in next_ancestors.iter().rev() {
                if !prev.contains(&id)
                    && st.listens::<Interaction>(id, InteractionEvent::PointerEnter)
                {
                    enters.push(id);
                }
            }
            prev.clear();
            prev.extend(next_ancestors.iter().map(|&(id, _)| id));
            (leaves, enters)
        };
        if to_leave.is_empty() && to_enter.is_empty() {
            return;
        }
        let (x, y) = (pe.client_x() as f32, pe.client_y() as f32);
        let pid = pe.pointer_id().max(0) as u32;
        let btns = pe.buttons();
        let ptype = pointer_type_code(pe);
        let flags = pointer_flags(pe, 0, EventFlags::NO_BUBBLE);
        sinks.emit(|q, sid, ts| {
            for id in to_leave {
                q.push_pointer_ex(
                    sid,
                    id,
                    ts,
                    InteractionEvent::PointerLeave,
                    x,
                    y,
                    pid,
                    btns,
                    ptype,
                    flags,
                );
            }
            for id in to_enter {
                q.push_pointer_ex(
                    sid,
                    id,
                    ts,
                    InteractionEvent::PointerEnter,
                    x,
                    y,
                    pid,
                    btns,
                    ptype,
                    flags,
                );
            }
        });
    })
}

fn attach_pointerout(
    root: &web_sys::Element,
    sinks: &EventSinks,
    entered_chain: &Rc<RefCell<Vec<NodeId>>>,
) -> DomListenerGuard {
    let r = root.clone();
    let sinks = sinks.clone();
    let entered = Rc::clone(entered_chain);
    on_root(root, "pointerout", move |ev| {
        let Some(pe) = ev.dyn_ref::<web_sys::PointerEvent>() else { return };
        if let Some(rel_el) = target_to_element(pe.related_target())
            && r.contains(Some(&rel_el))
        {
            return;
        }
        let to_leave = {
            let mut prev = entered.borrow_mut();
            let st = sinks.state.borrow();
            let leaves: Vec<NodeId> = prev
                .iter()
                .copied()
                .filter(|&id| st.listens::<Interaction>(id, InteractionEvent::PointerLeave))
                .collect();
            prev.clear();
            leaves
        };
        if to_leave.is_empty() {
            return;
        }
        let (x, y) = (pe.client_x() as f32, pe.client_y() as f32);
        let pid = pe.pointer_id().max(0) as u32;
        let btns = pe.buttons();
        let ptype = pointer_type_code(pe);
        let flags = pointer_flags(pe, 0, EventFlags::NO_BUBBLE);
        sinks.emit(|q, sid, ts| {
            for id in to_leave {
                q.push_pointer_ex(
                    sid,
                    id,
                    ts,
                    InteractionEvent::PointerLeave,
                    x,
                    y,
                    pid,
                    btns,
                    ptype,
                    flags,
                );
            }
        });
    })
}
