//! Zero-JSON `NK1` binary keyboard encoding, CJK IME composition guards, and `wasm32` DOM
//! event listener lifecycle management.

use std::cell::Cell;
use std::rc::Rc;

use nve_abi::events::encode_keydown_bytes;

#[cfg(target_arch = "wasm32")]
use nve_abi::{ComponentKind, EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId, ROOT_NODE_ID};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::WebBackend;

/// Virtual key code reported by browsers (`KeyboardEvent.keyCode == 229`) during active IME
/// composition, even when `isComposing` lags on WebKit/Safari.
pub const IME_PROCESS_KEY_CODE: u32 = 229;

/// Encodes a keyboard event into the zero-JSON compact `NK1` binary payload consumed by
/// `NativeEventKind::Custom` (`name: "keydown"`).
pub fn encode_keydown_payload(
    key: &str,
    meta: bool,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Vec<u8> {
    encode_keydown_bytes(key, meta, ctrl, shift, alt)
}

/// Shared CJK IME composition state guard for `<input>` elements.
///
/// Prevents intermediate pre-edit `input` events and candidate-commit `Enter` keydowns from
/// firing premature `change` or `submit` events while an IME composition session is active.
#[derive(Debug, Clone, Default)]
pub struct ImeCompositionGuard {
    composing: Rc<Cell<bool>>,
}

impl ImeCompositionGuard {
    pub fn new() -> Self {
        Self {
            composing: Rc::new(Cell::new(false)),
        }
    }

    #[inline]
    pub fn on_composition_start(&self) {
        self.composing.set(true);
    }

    #[inline]
    pub fn on_composition_end(&self) {
        self.composing.set(false);
    }

    #[inline]
    pub fn is_composing(&self) -> bool {
        self.composing.get()
    }

    /// Returns `true` when an `input` event should be suppressed until `compositionend`.
    #[inline]
    pub fn should_suppress_input(&self) -> bool {
        self.composing.get()
    }

    /// Returns `true` when a `keydown` `Enter` (or navigation key) is part of an active IME
    /// candidate commit and must NOT trigger form `submit` or global shortcuts.
    #[inline]
    pub fn should_suppress_keydown(&self, event_is_composing: bool, key_code: u32) -> bool {
        self.composing.get() || event_is_composing || key_code == IME_PROCESS_KEY_CODE
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct NodeListeners {
    pub(crate) click: Option<Closure<dyn FnMut(web_sys::MouseEvent)>>,
    pub(crate) input: Option<Closure<dyn FnMut(web_sys::Event)>>,
    pub(crate) scroll: Option<Closure<dyn FnMut(web_sys::Event)>>,
    pub(crate) focus: Option<Closure<dyn FnMut(web_sys::FocusEvent)>>,
    pub(crate) blur: Option<Closure<dyn FnMut(web_sys::FocusEvent)>>,
    pub(crate) keydown: Option<Closure<dyn FnMut(web_sys::KeyboardEvent)>>,
    pub(crate) compositionstart: Option<Closure<dyn FnMut(web_sys::Event)>>,
    pub(crate) compositionend: Option<Closure<dyn FnMut(web_sys::Event)>>,
    pub(crate) ime_guard: Option<ImeCompositionGuard>,
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_us() -> u64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| (p.now() * 1000.0) as u64)
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
impl WebBackend {
    pub(crate) fn attach_window_keydown_listener(&mut self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let events = Rc::clone(&self.events);
        let on_event = Rc::clone(&self.on_event);
        let surface_id = Rc::clone(&self.surface_id);
        let closure = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |ev: web_sys::KeyboardEvent| {
                if ev.is_composing() || ev.key_code() == IME_PROCESS_KEY_CODE {
                    return;
                }
                let key = ev.key();
                let meta = ev.meta_key();
                let ctrl = ev.ctrl_key();
                let shift = ev.shift_key();
                let alt = ev.alt_key();
                let is_cmd_k = (meta || ctrl) && key.eq_ignore_ascii_case("k");
                let is_nav = matches!(key.as_str(), "ArrowUp" | "ArrowDown" | "Enter" | "Escape");
                if !is_cmd_k && !is_nav {
                    return;
                }
                if is_cmd_k {
                    ev.prevent_default();
                }
                let data = encode_keydown_payload(&key, meta, ctrl, shift, alt);
                events.borrow_mut().push(NativeEvent {
                    surface_id: surface_id.get(),
                    node_id: ROOT_NODE_ID,
                    timestamp_us: now_us(),
                    kind: NativeEventKind::Custom,
                    payload: EventPayload::Custom {
                        name: "keydown".into(),
                        data,
                    },
                });
                let cb = on_event.borrow().clone();
                if let Some(cb) = cb {
                    cb();
                }
            },
        );
        let _ =
            window.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
        self.window_keydown = Some(closure);
    }

    pub(crate) fn sync_event_listeners(&mut self, node_id: NodeId, mask: u64) {
        let Some(dom_el) = self.dom_nodes.get(&node_id).cloned() else {
            return;
        };
        let kind = self
            .nodes
            .get(&node_id)
            .map(|n| n.kind)
            .unwrap_or(ComponentKind::View);

        let want_click = (mask & EventMask::PRESS) != 0;
        let want_input = (mask & EventMask::TEXT_CHANGE) != 0;
        let want_scroll = kind == ComponentKind::ScrollView || (mask & EventMask::SCROLL) != 0;
        let want_focus = kind == ComponentKind::TextInput || (mask & EventMask::FOCUS) != 0;
        let want_blur = kind == ComponentKind::TextInput || (mask & EventMask::BLUR) != 0;
        let want_keydown = kind == ComponentKind::TextInput;
        let want_ime = kind == ComponentKind::TextInput;

        let entry = self.listeners.entry(node_id).or_default();
        let ime_guard = if want_ime {
            Some(entry.ime_guard.get_or_insert_with(ImeCompositionGuard::new).clone())
        } else {
            entry.ime_guard = None;
            None
        };

        if want_ime && entry.compositionstart.is_none() {
            let guard = ime_guard.clone().unwrap();
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
                guard.on_composition_start();
            });
            let _ = dom_el.add_event_listener_with_callback(
                "compositionstart",
                closure.as_ref().unchecked_ref(),
            );
            entry.compositionstart = Some(closure);
        } else if !want_ime
            && let Some(old) = entry.compositionstart.take()
        {
            let _ = dom_el.remove_event_listener_with_callback(
                "compositionstart",
                old.as_ref().unchecked_ref(),
            );
        }

        if want_ime && entry.compositionend.is_none() {
            let guard = ime_guard.clone().unwrap();
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                guard.on_composition_end();
                let text = ev
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                    .map(|inp| inp.value())
                    .unwrap_or_default();
                events.borrow_mut().push(NativeEvent {
                    surface_id: surface_id.get(),
                    node_id,
                    timestamp_us: now_us(),
                    kind: NativeEventKind::TextChange,
                    payload: EventPayload::Text(text),
                });
                let cb = on_event.borrow().clone();
                if let Some(cb) = cb {
                    cb();
                }
            });
            let _ = dom_el.add_event_listener_with_callback(
                "compositionend",
                closure.as_ref().unchecked_ref(),
            );
            entry.compositionend = Some(closure);
        } else if !want_ime
            && let Some(old) = entry.compositionend.take()
        {
            let _ = dom_el.remove_event_listener_with_callback(
                "compositionend",
                old.as_ref().unchecked_ref(),
            );
        }

        if want_click && entry.click.is_none() {
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let closure = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(
                move |ev: web_sys::MouseEvent| {
                    events.borrow_mut().push(NativeEvent {
                        surface_id: surface_id.get(),
                        node_id,
                        timestamp_us: now_us(),
                        kind: NativeEventKind::Press,
                        payload: EventPayload::Point {
                            x: ev.client_x() as f32,
                            y: ev.client_y() as f32,
                        },
                    });
                    let cb = on_event.borrow().clone();
                    if let Some(cb) = cb {
                        cb();
                    }
                },
            );
            let _ = dom_el
                .add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
            entry.click = Some(closure);
        } else if !want_click
            && let Some(old) = entry.click.take()
        {
            let _ =
                dom_el.remove_event_listener_with_callback("click", old.as_ref().unchecked_ref());
        }

        if want_input && entry.input.is_none() {
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let guard = ime_guard.clone();
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                if guard.as_ref().is_some_and(ImeCompositionGuard::should_suppress_input) {
                    return;
                }
                let text = ev
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                    .map(|inp| inp.value())
                    .unwrap_or_default();
                events.borrow_mut().push(NativeEvent {
                    surface_id: surface_id.get(),
                    node_id,
                    timestamp_us: now_us(),
                    kind: NativeEventKind::TextChange,
                    payload: EventPayload::Text(text),
                });
                let cb = on_event.borrow().clone();
                if let Some(cb) = cb {
                    cb();
                }
            });
            let _ = dom_el
                .add_event_listener_with_callback("input", closure.as_ref().unchecked_ref());
            entry.input = Some(closure);
        } else if !want_input
            && let Some(old) = entry.input.take()
        {
            let _ =
                dom_el.remove_event_listener_with_callback("input", old.as_ref().unchecked_ref());
        }

        if want_scroll && entry.scroll.is_none() {
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let scroll_el = dom_el.clone();
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
                events.borrow_mut().push(NativeEvent {
                    surface_id: surface_id.get(),
                    node_id,
                    timestamp_us: now_us(),
                    kind: NativeEventKind::Scroll,
                    payload: EventPayload::Scroll {
                        x: scroll_el.scroll_left() as f32,
                        y: scroll_el.scroll_top() as f32,
                        content_width: scroll_el.scroll_width() as f32,
                        content_height: scroll_el.scroll_height() as f32,
                    },
                });
                let cb = on_event.borrow().clone();
                if let Some(cb) = cb {
                    cb();
                }
            });
            let _ = dom_el
                .add_event_listener_with_callback("scroll", closure.as_ref().unchecked_ref());
            entry.scroll = Some(closure);
        } else if !want_scroll
            && let Some(old) = entry.scroll.take()
        {
            let _ =
                dom_el.remove_event_listener_with_callback("scroll", old.as_ref().unchecked_ref());
        }

        if want_focus && entry.focus.is_none() {
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let closure = Closure::<dyn FnMut(web_sys::FocusEvent)>::new(
                move |_ev: web_sys::FocusEvent| {
                    events.borrow_mut().push(NativeEvent {
                        surface_id: surface_id.get(),
                        node_id,
                        timestamp_us: now_us(),
                        kind: NativeEventKind::Focus,
                        payload: EventPayload::Empty,
                    });
                    let cb = on_event.borrow().clone();
                    if let Some(cb) = cb {
                        cb();
                    }
                },
            );
            let _ = dom_el
                .add_event_listener_with_callback("focus", closure.as_ref().unchecked_ref());
            entry.focus = Some(closure);
        } else if !want_focus
            && let Some(old) = entry.focus.take()
        {
            let _ =
                dom_el.remove_event_listener_with_callback("focus", old.as_ref().unchecked_ref());
        }

        if want_blur && entry.blur.is_none() {
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let closure = Closure::<dyn FnMut(web_sys::FocusEvent)>::new(
                move |_ev: web_sys::FocusEvent| {
                    events.borrow_mut().push(NativeEvent {
                        surface_id: surface_id.get(),
                        node_id,
                        timestamp_us: now_us(),
                        kind: NativeEventKind::Blur,
                        payload: EventPayload::Empty,
                    });
                    let cb = on_event.borrow().clone();
                    if let Some(cb) = cb {
                        cb();
                    }
                },
            );
            let _ = dom_el
                .add_event_listener_with_callback("blur", closure.as_ref().unchecked_ref());
            entry.blur = Some(closure);
        } else if !want_blur
            && let Some(old) = entry.blur.take()
        {
            let _ =
                dom_el.remove_event_listener_with_callback("blur", old.as_ref().unchecked_ref());
        }

        if want_keydown && entry.keydown.is_none() {
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let guard = ime_guard;
            let closure = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
                move |ev: web_sys::KeyboardEvent| {
                    if guard.as_ref().is_some_and(|g| {
                        g.should_suppress_keydown(ev.is_composing(), ev.key_code())
                    }) || ev.is_composing()
                        || ev.key_code() == IME_PROCESS_KEY_CODE
                    {
                        return;
                    }
                    if ev.key() == "Enter"
                        && !ev.shift_key()
                        && !ev.meta_key()
                        && !ev.ctrl_key()
                        && !ev.alt_key()
                    {
                        ev.prevent_default();
                        ev.stop_propagation();
                        let value = ev
                            .target()
                            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                            .map(|inp| inp.value())
                            .unwrap_or_default();
                        events.borrow_mut().push(NativeEvent {
                            surface_id: surface_id.get(),
                            node_id: ROOT_NODE_ID,
                            timestamp_us: now_us(),
                            kind: NativeEventKind::Custom,
                            payload: EventPayload::Custom {
                                name: "submit".into(),
                                data: value.into_bytes(),
                            },
                        });
                        let cb = on_event.borrow().clone();
                        if let Some(cb) = cb {
                            cb();
                        }
                    }
                },
            );
            let _ = dom_el
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
            entry.keydown = Some(closure);
        } else if !want_keydown
            && let Some(old) = entry.keydown.take()
        {
            let _ = dom_el
                .remove_event_listener_with_callback("keydown", old.as_ref().unchecked_ref());
        }

        if entry.click.is_none()
            && entry.input.is_none()
            && entry.scroll.is_none()
            && entry.focus.is_none()
            && entry.blur.is_none()
            && entry.keydown.is_none()
            && entry.compositionstart.is_none()
            && entry.compositionend.is_none()
        {
            self.listeners.remove(&node_id);
        }
    }

    pub(crate) fn detach_listeners(&mut self, node_id: NodeId, dom_el: &web_sys::Element) {
        if let Some(listeners) = self.listeners.remove(&node_id) {
            if let Some(click) = listeners.click {
                let _ = dom_el
                    .remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
            }
            if let Some(input) = listeners.input {
                let _ = dom_el
                    .remove_event_listener_with_callback("input", input.as_ref().unchecked_ref());
            }
            if let Some(scroll) = listeners.scroll {
                let _ = dom_el
                    .remove_event_listener_with_callback("scroll", scroll.as_ref().unchecked_ref());
            }
            if let Some(focus) = listeners.focus {
                let _ = dom_el
                    .remove_event_listener_with_callback("focus", focus.as_ref().unchecked_ref());
            }
            if let Some(blur) = listeners.blur {
                let _ = dom_el
                    .remove_event_listener_with_callback("blur", blur.as_ref().unchecked_ref());
            }
            if let Some(keydown) = listeners.keydown {
                let _ = dom_el.remove_event_listener_with_callback(
                    "keydown",
                    keydown.as_ref().unchecked_ref(),
                );
            }
            if let Some(comp_start) = listeners.compositionstart {
                let _ = dom_el.remove_event_listener_with_callback(
                    "compositionstart",
                    comp_start.as_ref().unchecked_ref(),
                );
            }
            if let Some(comp_end) = listeners.compositionend {
                let _ = dom_el.remove_event_listener_with_callback(
                    "compositionend",
                    comp_end.as_ref().unchecked_ref(),
                );
            }
        }
    }
}
