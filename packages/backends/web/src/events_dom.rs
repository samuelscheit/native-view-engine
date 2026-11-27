//! Lazy refcounted `wasm32` DOM root event delegation driven by [`SurfaceEventTracker`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nve_abi::{
    EventBatchBuf, EventFlags, InteractionEvent, ScrollEvent, SurfaceId, TextInputEvent,
};
use nve_components::tables::{Interaction, Scroll, TextInput};
use nve_store::NveTable;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::events::{
    collect_ancestors, is_hard_nav_anchor, notify_event, now_us, read_dom_input_value,
    should_intercept_anchor_click, target_to_element, DomListenerGuard, WebEventState,
};
use crate::input_attrs::{
    web_composition_cancel_on_blur, web_composition_end, web_composition_start,
    web_composition_update, ImeCompositionGuard, InputSyncTracker,
};
use crate::RenderCallback;

/// Owns lazy refcounted delegated DOM event listeners on the single mount root (`wasm32`).
///
/// Each DOM event category attaches strictly on `0 -> 1` active subscribers and detaches on
/// `1 -> 0` via [`Self::sync_listeners`], driven directly by [`nve_store::SurfaceEventTracker`].
pub(crate) struct RootEventDelegator {
    root: web_sys::Element,
    root_target: web_sys::EventTarget,
    surface_id: Rc<Cell<SurfaceId>>,
    events: Rc<RefCell<EventBatchBuf>>,
    state: Rc<RefCell<WebEventState>>,
    input_sync: Rc<RefCell<InputSyncTracker>>,
    ime: Rc<ImeCompositionGuard>,
    on_event: Rc<RefCell<Option<RenderCallback>>>,
    click: Option<DomListenerGuard>,
    input: Option<DomListenerGuard>,
    comp_start: Option<DomListenerGuard>,
    comp_update: Option<DomListenerGuard>,
    comp_end: Option<DomListenerGuard>,
    focusin: Option<DomListenerGuard>,
    focusout: Option<DomListenerGuard>,
    scroll: Option<DomListenerGuard>,
    keydown: Option<DomListenerGuard>,
    submit: Option<DomListenerGuard>,
}

impl RootEventDelegator {
    pub(crate) fn new(
        root: web_sys::Element,
        surface_id: Rc<Cell<SurfaceId>>,
        events: Rc<RefCell<EventBatchBuf>>,
        state: Rc<RefCell<WebEventState>>,
        input_sync: Rc<RefCell<InputSyncTracker>>,
        ime: Rc<ImeCompositionGuard>,
        on_event: Rc<RefCell<Option<RenderCallback>>>,
    ) -> Self {
        let root_target: web_sys::EventTarget = root.clone().into();
        Self {
            root,
            root_target,
            surface_id,
            events,
            state,
            input_sync,
            ime,
            on_event,
            click: None,
            input: None,
            comp_start: None,
            comp_update: None,
            comp_end: None,
            focusin: None,
            focusout: None,
            scroll: None,
            keydown: None,
            submit: None,
        }
    }

    /// Synchronizes DOM listener attachment against the tracker's refcounts (`0 -> 1` attach,
    /// `1 -> 0` detach).
    pub(crate) fn sync_listeners(&mut self, st: &WebEventState) {
        let want_click = st.tracker.has_active_of::<Interaction>(InteractionEvent::Press);
        let want_input = st.input_count > 0 || st.tracker.active_mask(TextInput::slot()) != 0;
        let want_focusin = st.tracker.has_active_of::<Interaction>(InteractionEvent::Focus);
        let want_focusout =
            st.tracker.has_active_of::<Interaction>(InteractionEvent::Blur) || want_input;
        let want_scroll = st.tracker.has_active_of::<Scroll>(ScrollEvent::Scroll);
        let want_submit =
            st.tracker.has_active_of::<TextInput>(TextInputEvent::Submit) || st.input_count > 0;
        let want_keydown =
            st.tracker.has_active_of::<Interaction>(InteractionEvent::KeyDown) || want_submit;

        if want_click && self.click.is_none() {
            self.click = Some(self.attach_click());
        } else if !want_click {
            self.click = None;
        }
        if want_input && self.input.is_none() {
            self.input = Some(self.attach_input());
            self.comp_start = Some(self.attach_comp_start());
            self.comp_update = Some(self.attach_comp_update());
            self.comp_end = Some(self.attach_comp_end());
        } else if !want_input {
            self.input = None;
            self.comp_start = None;
            self.comp_update = None;
            self.comp_end = None;
        }
        if want_focusin && self.focusin.is_none() {
            self.focusin = Some(self.attach_focusin());
        } else if !want_focusin {
            self.focusin = None;
        }
        if want_focusout && self.focusout.is_none() {
            self.focusout = Some(self.attach_focusout());
        } else if !want_focusout {
            self.focusout = None;
        }
        if want_scroll && self.scroll.is_none() {
            self.scroll = Some(self.attach_scroll());
        } else if !want_scroll {
            self.scroll = None;
        }
        if want_keydown && self.keydown.is_none() {
            self.keydown = Some(self.attach_keydown());
        } else if !want_keydown {
            self.keydown = None;
        }
        if want_submit && self.submit.is_none() {
            self.submit = Some(self.attach_submit());
        } else if !want_submit {
            self.submit = None;
        }
    }

    fn attach_click(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let st = Rc::clone(&self.state);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let st_ref = st.borrow();
            let Some((id, _)) = chain
                .iter()
                .find(|(id, _)| st_ref.listens::<Interaction>(*id, InteractionEvent::Press))
            else {
                return;
            };
            let Some(me) = ev.dyn_ref::<web_sys::MouseEvent>() else { return };
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
            evs.borrow_mut().push_press(
                sid.get(),
                *id,
                now_us(),
                me.client_x() as f32,
                me.client_y() as f32,
            );
            notify_event(&notify);
        });
        DomListenerGuard::attach(self.root_target.clone(), "click", false, cb)
    }

    fn attach_input(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let st = Rc::clone(&self.state);
        let sync = Rc::clone(&self.input_sync);
        let ime_g = Rc::clone(&self.ime);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            if ime_g.should_suppress_input() {
                return;
            }
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let st_ref = st.borrow();
            let found = chain
                .iter()
                .find(|(id, _)| {
                    st_ref.is_input(*id)
                        || st_ref.listens::<TextInput>(*id, TextInputEvent::Change)
                })
                .map(|(id, target_el)| (*id, read_dom_input_value(target_el)));
            drop(st_ref);
            if let Some((id, val)) = found
                && let Some(changed) = sync.borrow_mut().on_input_event(id, val, false)
            {
                evs.borrow_mut()
                    .push_text_change(sid.get(), id, now_us(), &changed);
                notify_event(&notify);
            }
        });
        DomListenerGuard::attach(self.root_target.clone(), "input", false, cb)
    }

    fn attach_comp_start(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let ime_g = Rc::clone(&self.ime);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            if let Some((id, _)) = chain.first() {
                web_composition_start(&ime_g, sid.get(), *id, now_us(), &mut evs.borrow_mut());
                notify_event(&notify);
            }
        });
        DomListenerGuard::attach(self.root_target.clone(), "compositionstart", false, cb)
    }

    fn attach_comp_update(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let ime_g = Rc::clone(&self.ime);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            if let Some((id, _)) = chain.first() {
                let data = ev
                    .dyn_ref::<web_sys::CompositionEvent>()
                    .and_then(|c| c.data())
                    .unwrap_or_default();
                web_composition_update(
                    &ime_g,
                    sid.get(),
                    *id,
                    now_us(),
                    &data,
                    &mut evs.borrow_mut(),
                );
                notify_event(&notify);
            }
        });
        DomListenerGuard::attach(self.root_target.clone(), "compositionupdate", false, cb)
    }

    fn attach_comp_end(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let sync = Rc::clone(&self.input_sync);
        let ime_g = Rc::clone(&self.ime);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            if let Some((id, target_el)) = chain.first() {
                let committed = ev
                    .dyn_ref::<web_sys::CompositionEvent>()
                    .and_then(|c| c.data())
                    .unwrap_or_default();
                let full = read_dom_input_value(target_el);
                let mut guard = sync.borrow_mut();
                web_composition_end(
                    &ime_g,
                    &mut guard,
                    sid.get(),
                    *id,
                    now_us(),
                    &committed,
                    full,
                    &mut evs.borrow_mut(),
                );
                notify_event(&notify);
            }
        });
        DomListenerGuard::attach(self.root_target.clone(), "compositionend", false, cb)
    }

    fn attach_focusin(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let st = Rc::clone(&self.state);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let st_ref = st.borrow();
            let focus_id = chain
                .iter()
                .find(|(id, _)| st_ref.listens::<Interaction>(*id, InteractionEvent::Focus))
                .map(|(id, _)| *id);
            drop(st_ref);
            if let Some(id) = focus_id {
                evs.borrow_mut().push_focus(sid.get(), id, now_us());
                notify_event(&notify);
            }
        });
        DomListenerGuard::attach(self.root_target.clone(), "focusin", false, cb)
    }

    fn attach_focusout(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let st = Rc::clone(&self.state);
        let ime_g = Rc::clone(&self.ime);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let st_ref = st.borrow();
            let blur_id = chain
                .iter()
                .find(|(id, _)| st_ref.listens::<Interaction>(*id, InteractionEvent::Blur))
                .map(|(id, _)| *id);
            drop(st_ref);
            let mut pushed = false;
            if let Some((id, _)) = chain.first()
                && web_composition_cancel_on_blur(&ime_g, sid.get(), *id, now_us(), &mut evs.borrow_mut())
            {
                pushed = true;
            }
            if let Some(id) = blur_id {
                evs.borrow_mut().push_blur(sid.get(), id, now_us());
                pushed = true;
            }
            if pushed {
                notify_event(&notify);
            }
        });
        DomListenerGuard::attach(self.root_target.clone(), "focusout", false, cb)
    }

    fn attach_scroll(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let st = Rc::clone(&self.state);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let st_ref = st.borrow();
            let scroll_target = chain
                .iter()
                .find(|(id, _)| st_ref.listens::<Scroll>(*id, ScrollEvent::Scroll))
                .map(|(id, target_el)| {
                    (
                        *id,
                        target_el.scroll_left() as f32,
                        target_el.scroll_top() as f32,
                        target_el.scroll_width() as f32,
                        target_el.scroll_height() as f32,
                    )
                });
            drop(st_ref);
            if let Some((id, x, y, content_width, content_height)) = scroll_target {
                evs.borrow_mut().push_scroll(
                    sid.get(),
                    id,
                    now_us(),
                    x,
                    y,
                    content_width,
                    content_height,
                );
                notify_event(&notify);
            }
        });
        DomListenerGuard::attach(self.root_target.clone(), "scroll", true, cb)
    }

    fn attach_keydown(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let st = Rc::clone(&self.state);
        let ime_g = Rc::clone(&self.ime);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let key_target: web_sys::EventTarget = web_sys::window()
            .map(Into::into)
            .unwrap_or_else(|| self.root_target.clone());
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(ke) = ev.dyn_ref::<web_sys::KeyboardEvent>() else { return };
            if ime_g.should_suppress_keydown(ke.is_composing(), ke.key_code()) {
                return;
            }
            let el = target_to_element(ev.target());
            if let Some(ref e) = el
                && !r.contains(Some(e))
                && e.closest("[data-nve-live],[data-nve-id]").ok().flatten().is_some()
            {
                return;
            }
            let chain = el
                .as_ref()
                .filter(|e| r.contains(Some(e)))
                .map(|e| collect_ancestors(e, &r))
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

            let st_ref = st.borrow();
            let submit_target = if key == "Enter"
                && let Some(ref target_el) = el
                && target_el.dyn_ref::<web_sys::HtmlInputElement>().is_some()
                && let Some((submit_id, _)) = chain
                    .iter()
                    .find(|(id, _)| st_ref.listens::<TextInput>(*id, TextInputEvent::Submit))
            {
                ev.prevent_default();
                Some((*submit_id, read_dom_input_value(target_el)))
            } else {
                None
            };
            if let Some((submit_id, val)) = submit_target {
                drop(st_ref);
                evs.borrow_mut().push_submit(sid.get(), submit_id, now_us(), &val);
                notify_event(&notify);
                return;
            }
            if !in_this_surface {
                return;
            }
            let target_id = chain
                .iter()
                .find(|(id, _)| st_ref.listens::<Interaction>(*id, InteractionEvent::KeyDown))
                .map(|(id, _)| *id)
                .or_else(|| chain.first().map(|(id, _)| *id))
                .unwrap_or(nve_abi::ROOT_NODE_ID);
            drop(st_ref);

            let mut flags = EventFlags::from_modifiers(
                ke.meta_key(),
                ke.ctrl_key(),
                ke.shift_key(),
                ke.alt_key(),
                ke.repeat(),
            );
            if ke.is_composing() {
                flags = flags.union(EventFlags::COMPOSING);
            }
            let code = ke.code();
            evs.borrow_mut().push_key_down(
                sid.get(),
                target_id,
                now_us(),
                &key,
                &code,
                flags,
                ke.key_code(),
            );
            notify_event(&notify);
        });
        DomListenerGuard::attach(key_target, "keydown", false, cb)
    }

    fn attach_submit(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let evs = Rc::clone(&self.events);
        let st = Rc::clone(&self.state);
        let sid = Rc::clone(&self.surface_id);
        let notify = Rc::clone(&self.on_event);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let st_ref = st.borrow();
            let submit_id = chain
                .iter()
                .find(|(id, _)| st_ref.listens::<TextInput>(*id, TextInputEvent::Submit))
                .map(|(id, _)| *id);
            drop(st_ref);
            if let Some(id) = submit_id {
                ev.prevent_default();
                let val = el
                    .query_selector("input,textarea,select")
                    .ok()
                    .flatten()
                    .map(|ctrl| read_dom_input_value(&ctrl))
                    .unwrap_or_else(|| read_dom_input_value(&el));
                evs.borrow_mut().push_submit(sid.get(), id, now_us(), &val);
                notify_event(&notify);
            }
        });
        DomListenerGuard::attach(self.root_target.clone(), "submit", false, cb)
    }
}
