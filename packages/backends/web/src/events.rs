//! Zero-JSON `NK1` binary keyboard encoding, CJK IME composition guards, and single-root
//! `wasm32` DOM event delegation (`closest('[data-nve-id]')` -> `NodeEventMeta` lookup).

use std::cell::Cell;
use std::rc::Rc;

use nve_abi::events::encode_keydown_bytes;
use nve_abi::{ComponentKind, EventMask, NodeId};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use nve_abi::hash::HashMap;
#[cfg(target_arch = "wasm32")]
use nve_abi::{EventPayload, NativeEvent, NativeEventKind, ROOT_NODE_ID};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::WebBackend;

/// Virtual key code reported by browsers (`KeyboardEvent.keyCode == 229`) during active IME
/// composition, even when `isComposing` lags on WebKit/Safari.
pub const IME_PROCESS_KEY_CODE: u32 = 229;

/// Per-node event routing metadata consulted by the root event delegator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeEventMeta {
    pub kind: ComponentKind,
    pub mask: u64,
}

impl Default for NodeEventMeta {
    fn default() -> Self {
        Self {
            kind: ComponentKind::View,
            mask: EventMask::NONE,
        }
    }
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

/// Returns `true` when a hydrated click on an `<a href="...">` inside an interactive
/// `EventMask::PRESS` node should call `preventDefault()` and emit `NativeEventKind::Press` for
/// zero-reload SPA navigation, or `false` to let the browser handle it natively (modifier clicks,
/// non-primary mouse buttons, `target="_blank"`, or `download`).
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

    #[inline]
    pub fn should_suppress_input(&self) -> bool {
        self.composing.get()
    }

    #[inline]
    pub fn should_suppress_keydown(&self, event_is_composing: bool, key_code: u32) -> bool {
        self.composing.get() || event_is_composing || key_code == IME_PROCESS_KEY_CODE
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_us() -> u64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| (p.now() * 1000.0) as u64)
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
fn target_to_element(target: Option<web_sys::EventTarget>) -> Option<web_sys::Element> {
    let t = target?;
    if let Ok(el) = t.clone().dyn_into::<web_sys::Element>() {
        return Some(el);
    }
    t.dyn_into::<web_sys::Node>()
        .ok()
        .and_then(|node| node.parent_element())
}

#[cfg(target_arch = "wasm32")]
fn parse_nve_id(el: &web_sys::Element) -> Option<NodeId> {
    el.get_attribute("data-nve-id")?.parse::<NodeId>().ok()
}

#[cfg(target_arch = "wasm32")]
fn find_delegated_ancestor<F>(
    container: &web_sys::Element,
    target: Option<web_sys::EventTarget>,
    meta_map: &HashMap<NodeId, NodeEventMeta>,
    predicate: F,
) -> Option<(NodeId, web_sys::Element)>
where
    F: Fn(NodeEventMeta) -> bool,
{
    let start = target_to_element(target)?;
    let mut cur = start.closest("[data-nve-id]").ok().flatten();
    while let Some(el) = cur {
        if let Some(node_id) = parse_nve_id(&el)
            && let Some(&meta) = meta_map.get(&node_id)
            && predicate(meta)
        {
            return Some((node_id, el));
        }
        if el.is_same_node(Some(container)) {
            break;
        }
        cur = el
            .parent_element()
            .and_then(|p| p.closest("[data-nve-id]").ok().flatten());
    }
    None
}

/// Single root-container delegated listener set replacing per-node `Closure` fan-out.
#[cfg(target_arch = "wasm32")]
pub(crate) struct RootEventDelegator {
    container: web_sys::Element,
    click: Closure<dyn FnMut(web_sys::MouseEvent)>,
    input: Closure<dyn FnMut(web_sys::Event)>,
    keydown: Closure<dyn FnMut(web_sys::KeyboardEvent)>,
    focusin: Closure<dyn FnMut(web_sys::FocusEvent)>,
    focusout: Closure<dyn FnMut(web_sys::FocusEvent)>,
    scroll: Closure<dyn FnMut(web_sys::Event)>,
    compositionstart: Closure<dyn FnMut(web_sys::Event)>,
    compositionend: Closure<dyn FnMut(web_sys::Event)>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for RootEventDelegator {
    fn drop(&mut self) {
        let c = &self.container;
        let _ = c.remove_event_listener_with_callback("click", self.click.as_ref().unchecked_ref());
        let _ = c.remove_event_listener_with_callback("input", self.input.as_ref().unchecked_ref());
        let _ =
            c.remove_event_listener_with_callback("keydown", self.keydown.as_ref().unchecked_ref());
        let _ =
            c.remove_event_listener_with_callback("focusin", self.focusin.as_ref().unchecked_ref());
        let _ = c.remove_event_listener_with_callback(
            "focusout",
            self.focusout.as_ref().unchecked_ref(),
        );
        let _ = c.remove_event_listener_with_callback_and_bool(
            "scroll",
            self.scroll.as_ref().unchecked_ref(),
            true,
        );
        let _ = c.remove_event_listener_with_callback(
            "compositionstart",
            self.compositionstart.as_ref().unchecked_ref(),
        );
        let _ = c.remove_event_listener_with_callback(
            "compositionend",
            self.compositionend.as_ref().unchecked_ref(),
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn emit_event(
    events: &Rc<RefCell<Vec<NativeEvent>>>,
    on_event: &Rc<RefCell<Option<crate::RenderCallback>>>,
    event: NativeEvent,
) {
    events.borrow_mut().push(event);
    let cb = on_event.borrow().clone();
    if let Some(cb) = cb {
        cb();
    }
}

#[cfg(target_arch = "wasm32")]
impl WebBackend {
    pub(crate) fn attach_root_event_delegator(&mut self, container: web_sys::Element) {
        // 1. click (bubbled, walks ancestors to find nearest EventMask::PRESS node).
        let click = {
            let root = container.clone();
            let meta = Rc::clone(&self.node_event_meta);
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |ev: web_sys::MouseEvent| {
                let found = {
                    let map = meta.borrow();
                    find_delegated_ancestor(&root, ev.target(), &map, |m| {
                        (m.mask & EventMask::PRESS) != 0
                    })
                };
                if let Some((node_id, el)) = found {
                    if el.has_attribute("disabled") {
                        return;
                    }
                    // Post-hydration `<a href>` SPA interception (Option B): plain left-clicks on
                    // internal links call `preventDefault()` and emit `Press`; modifier/middle/
                    // `_blank`/`download` clicks pass through to native browser navigation.
                    if let Some(anchor) = target_to_element(ev.target())
                        .and_then(|t| t.closest("a[href]").ok().flatten())
                        .filter(|a| a.get_attribute("href").is_some_and(|h| !h.trim().is_empty()))
                    {
                        let target_attr = anchor.get_attribute("target");
                        if !should_intercept_anchor_click(
                            ev.button(),
                            ev.meta_key(),
                            ev.ctrl_key(),
                            ev.shift_key(),
                            ev.alt_key(),
                            target_attr.as_deref(),
                            anchor.has_attribute("download"),
                        ) {
                            return;
                        }
                        ev.prevent_default();
                    }
                    emit_event(
                        &events,
                        &on_event,
                        NativeEvent {
                            surface_id: surface_id.get(),
                            node_id,
                            timestamp_us: now_us(),
                            kind: NativeEventKind::Press,
                            payload: EventPayload::Point {
                                x: ev.client_x() as f32,
                                y: ev.client_y() as f32,
                            },
                        },
                    );
                }
            })
        };

        // 2. input (bubbled, suppressed during active IME composition).
        let input = {
            let root = container.clone();
            let meta = Rc::clone(&self.node_event_meta);
            let ime = Rc::clone(&self.ime_guards);
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let found = {
                    let map = meta.borrow();
                    find_delegated_ancestor(&root, ev.target(), &map, |m| {
                        (m.mask & EventMask::TEXT_CHANGE) != 0
                    })
                };
                if let Some((node_id, el)) = found {
                    if ime
                        .borrow()
                        .get(&node_id)
                        .is_some_and(ImeCompositionGuard::should_suppress_input)
                    {
                        return;
                    }
                    let text = el
                        .dyn_ref::<web_sys::HtmlInputElement>()
                        .map(|inp| inp.value())
                        .unwrap_or_default();
                    emit_event(
                        &events,
                        &on_event,
                        NativeEvent {
                            surface_id: surface_id.get(),
                            node_id,
                            timestamp_us: now_us(),
                            kind: NativeEventKind::TextChange,
                            payload: EventPayload::Text(text),
                        },
                    );
                }
            })
        };

        // 3. keydown (bubbled on container, handles TextInput Enter submit with IME guard).
        let keydown = {
            let root = container.clone();
            let meta = Rc::clone(&self.node_event_meta);
            let ime = Rc::clone(&self.ime_guards);
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
                move |ev: web_sys::KeyboardEvent| {
                    let found = {
                        let map = meta.borrow();
                        find_delegated_ancestor(&root, ev.target(), &map, |m| {
                            m.kind == ComponentKind::TextInput
                        })
                    };
                    if let Some((node_id, el)) = found {
                        let suppressed = ime.borrow().get(&node_id).is_some_and(|g| {
                            g.should_suppress_keydown(ev.is_composing(), ev.key_code())
                        }) || ev.is_composing()
                            || ev.key_code() == IME_PROCESS_KEY_CODE;
                        if suppressed {
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
                            let value = el
                                .dyn_ref::<web_sys::HtmlInputElement>()
                                .map(|inp| inp.value())
                                .unwrap_or_default();
                            emit_event(
                                &events,
                                &on_event,
                                NativeEvent {
                                    surface_id: surface_id.get(),
                                    node_id: ROOT_NODE_ID,
                                    timestamp_us: now_us(),
                                    kind: NativeEventKind::Custom,
                                    payload: EventPayload::Custom {
                                        name: "submit".into(),
                                        data: value.into_bytes(),
                                    },
                                },
                            );
                        }
                    }
                },
            )
        };

        // 4. focusin (bubbled replacement for non-bubbling focus).
        let focusin = {
            let root = container.clone();
            let meta = Rc::clone(&self.node_event_meta);
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            Closure::<dyn FnMut(web_sys::FocusEvent)>::new(move |ev: web_sys::FocusEvent| {
                let found = {
                    let map = meta.borrow();
                    find_delegated_ancestor(&root, ev.target(), &map, |m| {
                        m.kind == ComponentKind::TextInput || (m.mask & EventMask::FOCUS) != 0
                    })
                };
                if let Some((node_id, _)) = found {
                    emit_event(
                        &events,
                        &on_event,
                        NativeEvent {
                            surface_id: surface_id.get(),
                            node_id,
                            timestamp_us: now_us(),
                            kind: NativeEventKind::Focus,
                            payload: EventPayload::Empty,
                        },
                    );
                }
            })
        };

        // 5. focusout (bubbled replacement for non-bubbling blur).
        let focusout = {
            let root = container.clone();
            let meta = Rc::clone(&self.node_event_meta);
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            Closure::<dyn FnMut(web_sys::FocusEvent)>::new(move |ev: web_sys::FocusEvent| {
                let found = {
                    let map = meta.borrow();
                    find_delegated_ancestor(&root, ev.target(), &map, |m| {
                        m.kind == ComponentKind::TextInput || (m.mask & EventMask::BLUR) != 0
                    })
                };
                if let Some((node_id, _)) = found {
                    emit_event(
                        &events,
                        &on_event,
                        NativeEvent {
                            surface_id: surface_id.get(),
                            node_id,
                            timestamp_us: now_us(),
                            kind: NativeEventKind::Blur,
                            payload: EventPayload::Empty,
                        },
                    );
                }
            })
        };

        // 6. scroll (CAPTURE phase because scroll does not bubble).
        let scroll = {
            let meta = Rc::clone(&self.node_event_meta);
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let Some(el) = target_to_element(ev.target()) else {
                    return;
                };
                let Some(node_id) = parse_nve_id(&el) else {
                    return;
                };
                let wants_scroll = meta.borrow().get(&node_id).is_some_and(|m| {
                    m.kind == ComponentKind::ScrollView || (m.mask & EventMask::SCROLL) != 0
                });
                if wants_scroll {
                    emit_event(
                        &events,
                        &on_event,
                        NativeEvent {
                            surface_id: surface_id.get(),
                            node_id,
                            timestamp_us: now_us(),
                            kind: NativeEventKind::Scroll,
                            payload: EventPayload::Scroll {
                                x: el.scroll_left() as f32,
                                y: el.scroll_top() as f32,
                                content_width: el.scroll_width() as f32,
                                content_height: el.scroll_height() as f32,
                            },
                        },
                    );
                }
            })
        };

        // 7. compositionstart (bubbled).
        let compositionstart = {
            let root = container.clone();
            let meta = Rc::clone(&self.node_event_meta);
            let ime = Rc::clone(&self.ime_guards);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let found = {
                    let map = meta.borrow();
                    find_delegated_ancestor(&root, ev.target(), &map, |m| {
                        m.kind == ComponentKind::TextInput
                    })
                };
                if let Some((node_id, _)) = found {
                    ime.borrow_mut()
                        .entry(node_id)
                        .or_default()
                        .on_composition_start();
                }
            })
        };

        // 8. compositionend (bubbled, commits final IME string as TextChange).
        let compositionend = {
            let root = container.clone();
            let meta = Rc::clone(&self.node_event_meta);
            let ime = Rc::clone(&self.ime_guards);
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let found = {
                    let map = meta.borrow();
                    find_delegated_ancestor(&root, ev.target(), &map, |m| {
                        m.kind == ComponentKind::TextInput
                    })
                };
                if let Some((node_id, el)) = found {
                    ime.borrow_mut()
                        .entry(node_id)
                        .or_default()
                        .on_composition_end();
                    let text = el
                        .dyn_ref::<web_sys::HtmlInputElement>()
                        .map(|inp| inp.value())
                        .unwrap_or_default();
                    emit_event(
                        &events,
                        &on_event,
                        NativeEvent {
                            surface_id: surface_id.get(),
                            node_id,
                            timestamp_us: now_us(),
                            kind: NativeEventKind::TextChange,
                            payload: EventPayload::Text(text),
                        },
                    );
                }
            })
        };

        let _ =
            container.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        let _ =
            container.add_event_listener_with_callback("input", input.as_ref().unchecked_ref());
        let _ =
            container.add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
        let _ =
            container.add_event_listener_with_callback("focusin", focusin.as_ref().unchecked_ref());
        let _ = container
            .add_event_listener_with_callback("focusout", focusout.as_ref().unchecked_ref());
        let _ = container.add_event_listener_with_callback_and_bool(
            "scroll",
            scroll.as_ref().unchecked_ref(),
            true,
        );
        let _ = container.add_event_listener_with_callback(
            "compositionstart",
            compositionstart.as_ref().unchecked_ref(),
        );
        let _ = container.add_event_listener_with_callback(
            "compositionend",
            compositionend.as_ref().unchecked_ref(),
        );

        self.root_delegator = Some(RootEventDelegator {
            container,
            click,
            input,
            keydown,
            focusin,
            focusout,
            scroll,
            compositionstart,
            compositionend,
        });
    }

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
                emit_event(
                    &events,
                    &on_event,
                    NativeEvent {
                        surface_id: surface_id.get(),
                        node_id: ROOT_NODE_ID,
                        timestamp_us: now_us(),
                        kind: NativeEventKind::Custom,
                        payload: EventPayload::Custom {
                            name: "keydown".into(),
                            data,
                        },
                    },
                );
            },
        );
        let _ =
            window.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
        self.window_keydown = Some(closure);
    }
}
