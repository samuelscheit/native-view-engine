//! Lazy refcounted `wasm32` DOM root event delegation driven by [`SurfaceEventTracker`]
//! (`nve_store::SurfaceEventTracker`), plus the shared [`EventSinks`] every live listener pushes into.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nve_abi::{
    EventBatchBuf, InteractionEvent, NodeId, ScrollEvent, SurfaceId, TextInputEvent, TimestampUs,
};
use nve_tables::tables::{Interaction, Scroll, TextInput};
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
use crate::key_events::KeyInterceptors;
use crate::table_system::ClickInterceptors;
use crate::RenderCallback;

/// Shared event-queue handles a live DOM listener pushes into.
#[derive(Clone)]
pub(crate) struct EventSinks {
    pub(crate) surface_id: Rc<Cell<SurfaceId>>,
    pub(crate) events: Rc<RefCell<EventBatchBuf>>,
    pub(crate) state: Rc<RefCell<WebEventState>>,
    pub(crate) on_event: Rc<RefCell<Option<RenderCallback>>>,
}

impl EventSinks {
    /// Pushes events through `push(queue, surface, now)`, then notifies the host. The queue
    /// borrow ends before the (possibly re-entrant, commit-pumping) host callback runs.
    pub(crate) fn emit(&self, push: impl FnOnce(&mut EventBatchBuf, SurfaceId, TimestampUs)) {
        push(&mut self.events.borrow_mut(), self.surface_id.get(), now_us());
        notify_event(&self.on_event);
    }

    /// First node of `chain` (innermost first) satisfying `pred`; the state borrow ends on return.
    pub(crate) fn find<'c>(
        &self,
        chain: &'c [(NodeId, web_sys::Element)],
        pred: impl Fn(&WebEventState, NodeId) -> bool,
    ) -> Option<&'c (NodeId, web_sys::Element)> {
        let state = self.state.borrow();
        chain.iter().find(|(id, _)| pred(&state, *id))
    }
}

/// Owns lazy refcounted delegated DOM event listeners on the single mount root (`wasm32`).
///
/// Each DOM event category attaches strictly on `0 -> 1` active subscribers (or registered
/// click / key interceptors) and detaches on `1 -> 0` via [`Self::sync_listeners`].
pub(crate) struct RootEventDelegator {
    root: web_sys::Element,
    sinks: EventSinks,
    input_sync: Rc<RefCell<InputSyncTracker>>,
    ime: Rc<ImeCompositionGuard>,
    key_interceptors: Rc<RefCell<KeyInterceptors>>,
    click_interceptors: Rc<RefCell<ClickInterceptors>>,
    programmatic_scroll_guard: Rc<RefCell<nve_abi::hash::HashMap<NodeId, (i32, i32)>>>,
    active_capture: Rc<RefCell<Option<(i32, NodeId)>>>,
    entered_chain: Rc<RefCell<Vec<NodeId>>>,
    click: Option<DomListenerGuard>,
    /// `input` + `compositionstart` / `compositionupdate` / `compositionend`.
    input: Option<[DomListenerGuard; 4]>,
    focusin: Option<DomListenerGuard>,
    focusout: Option<DomListenerGuard>,
    scroll: Option<DomListenerGuard>,
    /// Window `keydown` + the root `focusin` that routes body-targeted keys to this surface.
    keydown: Option<[DomListenerGuard; 2]>,
    submit: Option<DomListenerGuard>,
    /// `pointerdown`, `pointermove`, `pointerup`, `pointercancel`, `lostpointercapture`.
    pointer_core: Option<[DomListenerGuard; 5]>,
    /// `pointerover` + `pointerout` synthesizing non-bubbling `PointerLeave`/`PointerEnter`.
    pointer_hover: Option<[DomListenerGuard; 2]>,
}

/// Attaches `$make` on `false -> true` of `$want`, drops (detaches) it on `true -> false`.
macro_rules! toggle_slot {
    ($slot:expr, $want:expr, $make:expr) => {
        if !$want {
            $slot = None;
        } else if $slot.is_none() {
            $slot = Some($make);
        }
    };
}

impl RootEventDelegator {
    pub(crate) fn attach(root: web_sys::Element, backend: &crate::WebBackend) -> Self {
        Self {
            root,
            sinks: backend.event_sinks(),
            input_sync: Rc::clone(&backend.input_sync),
            ime: Rc::clone(&backend.ime_guard),
            key_interceptors: Rc::clone(&backend.key_interceptors),
            click_interceptors: Rc::clone(&backend.click_interceptors),
            programmatic_scroll_guard: Rc::clone(&backend.programmatic_scroll_guard),
            active_capture: Rc::new(RefCell::new(None)),
            entered_chain: Rc::new(RefCell::new(Vec::new())),
            click: None,
            input: None,
            focusin: None,
            focusout: None,
            scroll: None,
            keydown: None,
            submit: None,
            pointer_core: None,
            pointer_hover: None,
        }
    }

    pub(crate) fn clear_pointer_state(&mut self) {
        *self.active_capture.borrow_mut() = None;
        self.entered_chain.borrow_mut().clear();
    }

    /// Synchronizes DOM listener attachment against the tracker's refcounts and the registered
    /// interceptors (`0 -> 1` attach, `1 -> 0` detach).
    pub(crate) fn sync_listeners(&mut self, st: &WebEventState) {
        // A running interceptor holds the borrow: it is registered, so keep listening.
        let has_hooks = |hooks: Result<bool, _>| hooks.map_or(true, |empty| !empty);
        let want_click = st.tracker.has_active_of::<Interaction>(InteractionEvent::Press)
            || has_hooks(self.click_interceptors.try_borrow().map(|h| h.is_empty()));
        let want_input = st.input_count > 0 || st.tracker.active_mask(TextInput::slot()) != 0;
        let want_focusin = st.tracker.has_active_of::<Interaction>(InteractionEvent::Focus);
        let want_focusout =
            st.tracker.has_active_of::<Interaction>(InteractionEvent::Blur) || want_input;
        let want_scroll = st.tracker.has_active_of::<Scroll>(ScrollEvent::Scroll);
        let want_submit =
            st.tracker.has_active_of::<TextInput>(TextInputEvent::Submit) || st.input_count > 0;
        let want_keydown = st.tracker.has_active_of::<Interaction>(InteractionEvent::KeyDown)
            || want_submit
            || has_hooks(self.key_interceptors.try_borrow().map(|h| h.is_empty()));
        let want_pointer_core = st.capture_count > 0
            || st.tracker.has_active_of::<Interaction>(InteractionEvent::PointerDown)
            || st.tracker.has_active_of::<Interaction>(InteractionEvent::PointerMove)
            || st.tracker.has_active_of::<Interaction>(InteractionEvent::PointerUp)
            || st.tracker.has_active_of::<Interaction>(InteractionEvent::PointerCancel);
        let want_pointer_hover =
            st.tracker.has_active_of::<Interaction>(InteractionEvent::PointerEnter)
                || st.tracker.has_active_of::<Interaction>(InteractionEvent::PointerLeave);

        toggle_slot!(self.click, want_click, self.attach_click());
        toggle_slot!(
            self.input,
            want_input,
            [
                self.attach_input(),
                self.attach_comp_start(),
                self.attach_comp_update(),
                self.attach_comp_end(),
            ]
        );
        toggle_slot!(self.focusin, want_focusin, self.attach_focusin());
        toggle_slot!(self.focusout, want_focusout, self.attach_focusout());
        toggle_slot!(self.scroll, want_scroll, self.attach_scroll());
        toggle_slot!(
            self.keydown,
            want_keydown,
            crate::key_events::attach_keydown(
                &self.root,
                &self.sinks,
                &self.ime,
                &self.key_interceptors,
            )
        );
        toggle_slot!(self.submit, want_submit, self.attach_submit());
        if !want_pointer_core {
            *self.active_capture.borrow_mut() = None;
        }
        toggle_slot!(
            self.pointer_core,
            want_pointer_core,
            crate::pointer_dom::attach_pointer_core(&self.root, &self.sinks, &self.active_capture)
        );
        if !want_pointer_hover {
            self.entered_chain.borrow_mut().clear();
        }
        toggle_slot!(
            self.pointer_hover,
            want_pointer_hover,
            crate::pointer_dom::attach_pointer_hover(&self.root, &self.sinks, &self.entered_chain)
        );
    }

    /// Attaches `cb` on the mount root. Uses `Fn` (not `FnMut`) so re-entry from a synchronous
    /// synthetic event dispatched by app code never hits wasm-bindgen's recursive-borrow abort.
    fn on_root(
        &self,
        name: &'static str,
        capture: bool,
        cb: impl Fn(web_sys::Event) + 'static,
    ) -> DomListenerGuard {
        let cb = Closure::<dyn Fn(web_sys::Event)>::new(cb);
        DomListenerGuard::attach(self.root.clone().into(), name, capture, cb)
    }

    /// Click -> plugin click hooks (e.g. `<dialog closedby>` backdrop fallback), else `Press`.
    fn attach_click(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        let hooks = Rc::clone(&self.click_interceptors);
        self.on_root("click", false, move |ev| {
            let Some(me) = ev.dyn_ref::<web_sys::MouseEvent>() else { return };
            if ClickInterceptors::try_consume(&hooks, me) {
                return;
            }
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let Some(&(id, _)) =
                sinks.find(&chain, |st, id| st.listens::<Interaction>(id, InteractionEvent::Press))
            else {
                return;
            };
            if let Ok(Some(anchor)) = el.closest("a[href]")
                && r.contains(Some(&anchor))
                && !is_hard_nav_anchor(anchor.get_attribute("data-nve-hard-nav").as_deref())
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
            let (x, y) = (me.client_x() as f32, me.client_y() as f32);
            sinks.emit(|q, sid, ts| q.push_press(sid, id, ts, x, y));
        })
    }

    fn attach_input(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        let sync = Rc::clone(&self.input_sync);
        let ime = Rc::clone(&self.ime);
        self.on_root("input", false, move |ev| {
            if ime.should_suppress_input() {
                return;
            }
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let Some((id, target_el)) = sinks.find(&chain, |st, id| {
                st.is_input(id) || st.listens::<TextInput>(id, TextInputEvent::Change)
            }) else {
                return;
            };
            let value = read_dom_input_value(target_el);
            // Bind first: the tracker borrow must end before the host callback runs.
            let changed = sync.borrow_mut().on_input_event(*id, value, false);
            if let Some(changed) = changed {
                sinks.emit(|q, sid, ts| q.push_text_change(sid, *id, ts, &changed));
            }
        })
    }

    fn attach_comp_start(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        let ime = Rc::clone(&self.ime);
        self.on_root("compositionstart", false, move |ev| {
            let Some(el) = target_to_element(ev.target()) else { return };
            if let Some(&(id, _)) = collect_ancestors(&el, &r).first() {
                sinks.emit(|q, sid, ts| web_composition_start(&ime, sid, id, ts, q));
            }
        })
    }

    fn attach_comp_update(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        let ime = Rc::clone(&self.ime);
        self.on_root("compositionupdate", false, move |ev| {
            let Some(el) = target_to_element(ev.target()) else { return };
            if let Some(&(id, _)) = collect_ancestors(&el, &r).first() {
                let data = composition_data(&ev);
                sinks.emit(|q, sid, ts| web_composition_update(&ime, sid, id, ts, &data, q));
            }
        })
    }

    fn attach_comp_end(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        let sync = Rc::clone(&self.input_sync);
        let ime = Rc::clone(&self.ime);
        self.on_root("compositionend", false, move |ev| {
            let Some(el) = target_to_element(ev.target()) else { return };
            if let Some((id, target_el)) = collect_ancestors(&el, &r).first() {
                let committed = composition_data(&ev);
                let full = read_dom_input_value(target_el);
                sinks.emit(|q, sid, ts| {
                    web_composition_end(
                        &ime,
                        &mut sync.borrow_mut(),
                        sid,
                        *id,
                        ts,
                        &committed,
                        full,
                        q,
                    );
                });
            }
        })
    }

    fn attach_focusin(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        self.on_root("focusin", false, move |ev| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            if let Some(&(id, _)) =
                sinks.find(&chain, |st, id| st.listens::<Interaction>(id, InteractionEvent::Focus))
            {
                sinks.emit(|q, sid, ts| q.push_focus(sid, id, ts));
            }
        })
    }

    fn attach_focusout(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        let ime = Rc::clone(&self.ime);
        self.on_root("focusout", false, move |ev| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let blur_id = sinks
                .find(&chain, |st, id| st.listens::<Interaction>(id, InteractionEvent::Blur))
                .map(|&(id, _)| id);
            let composing = chain.first().map(|&(id, _)| id).filter(|_| ime.is_composing());
            if composing.is_none() && blur_id.is_none() {
                return;
            }
            sinks.emit(|q, sid, ts| {
                if let Some(id) = composing {
                    web_composition_cancel_on_blur(&ime, sid, id, ts, q);
                }
                if let Some(id) = blur_id {
                    q.push_blur(sid, id, ts);
                }
            });
        })
    }

    /// Scroll (capture phase: `scroll` does not bubble in the DOM), suppressing echo feedback
    /// loops from programmatic `requested_scroll_seq` writes.
    fn attach_scroll(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        let prog_scroll = Rc::clone(&self.programmatic_scroll_guard);
        self.on_root("scroll", true, move |ev| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            if let Some((id, t)) =
                sinks.find(&chain, |st, id| st.listens::<Scroll>(id, ScrollEvent::Scroll))
            {
                let (lx, ly) = (t.scroll_left(), t.scroll_top());
                if let Ok(mut guard) = prog_scroll.try_borrow_mut() {
                    if guard.get(id).copied() == Some((lx, ly)) {
                        guard.remove(id);
                        return;
                    }
                    guard.remove(id);
                }
                let (x, y) = (lx as f32, ly as f32);
                let (w, h) = (t.scroll_width() as f32, t.scroll_height() as f32);
                sinks.emit(|q, sid, ts| q.push_scroll(sid, *id, ts, x, y, w, h));
            }
        })
    }

    fn attach_submit(&self) -> DomListenerGuard {
        let r = self.root.clone();
        let sinks = self.sinks.clone();
        self.on_root("submit", false, move |ev| {
            let Some(el) = target_to_element(ev.target()) else { return };
            let chain = collect_ancestors(&el, &r);
            let Some(&(id, _)) = sinks
                .find(&chain, |st, id| st.listens::<TextInput>(id, TextInputEvent::Submit))
            else {
                return;
            };
            ev.prevent_default();
            let val = el
                .query_selector("input,textarea,select")
                .ok()
                .flatten()
                .map_or_else(|| read_dom_input_value(&el), |ctrl| read_dom_input_value(&ctrl));
            sinks.emit(|q, sid, ts| q.push_submit(sid, id, ts, &val));
        })
    }
}

fn composition_data(ev: &web_sys::Event) -> String {
    ev.dyn_ref::<web_sys::CompositionEvent>()
        .and_then(web_sys::CompositionEvent::data)
        .unwrap_or_default()
}
