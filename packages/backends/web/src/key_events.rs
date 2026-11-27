//! Generic `keydown` interception hook (TV remotes, game pads, app-level key routing) and the
//! surface-scoped `wasm32` keydown listener that feeds it before the default `KeyDown` event.
//!
//! The backend knows nothing about TV semantics: capability modules (e.g. `nve-tv-web`) register
//! interceptors via [`crate::WebBackend::add_key_interceptor`].

use nve_store::NodeId;

/// Borrowed view of one surface-scoped `keydown`, after IME suppression.
#[derive(Debug, Clone, Copy)]
pub struct WebKeyEventInfo<'a> {
    pub key: &'a str,
    pub code: &'a str,
    pub key_code: u32,
    /// `nve_abi::NVE_KEY_MOD_*` bits.
    pub modifiers: u8,
    pub repeat: bool,
    pub is_composing: bool,
    /// Target is a text-editing control (`<input>` text-like, `<textarea>`, contenteditable).
    pub in_text_input: bool,
    /// Target is a multi-line text control (`<textarea>`, contenteditable).
    pub is_multiline_input: bool,
    /// Nearest `data-nve-id` ancestor of the event target.
    pub target_node: Option<NodeId>,
    /// `performance.now()` in microseconds.
    pub timestamp_us: u64,
    /// Mount root of the surface this key is scoped to.
    #[cfg(target_arch = "wasm32")]
    pub surface_root: &'a web_sys::Element,
}

impl WebKeyEventInfo<'_> {
    /// Ctrl, Meta, or Alt held: the key is an application / OS shortcut.
    #[inline]
    pub const fn has_command_modifier(&self) -> bool {
        self.modifiers
            & (nve_abi::NVE_KEY_MOD_META | nve_abi::NVE_KEY_MOD_CTRL | nve_abi::NVE_KEY_MOD_ALT)
            != 0
    }
}

/// What an interceptor did with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebKeyDisposition {
    /// Not handled: later interceptors and the default `KeyDown` event run.
    #[default]
    PassThrough,
    /// Handled: stop dispatch and call `preventDefault()`.
    ConsumedPreventDefault,
    /// Handled: stop dispatch but keep the browser default action.
    ConsumedAllowDefault,
}

/// A registered key interceptor.
pub type KeyInterceptor = Box<dyn FnMut(&WebKeyEventInfo<'_>) -> WebKeyDisposition>;

/// Interceptors in registration order.
#[derive(Default)]
pub(crate) struct KeyInterceptors {
    hooks: Vec<KeyInterceptor>,
}

impl KeyInterceptors {
    pub(crate) fn push(&mut self, hook: KeyInterceptor) {
        self.hooks.push(hook);
    }

    /// Runs interceptors in order until one consumes the key.
    pub(crate) fn dispatch(&mut self, info: &WebKeyEventInfo<'_>) -> WebKeyDisposition {
        self.hooks
            .iter_mut()
            .map(|hook| hook(info))
            .find(|d| *d != WebKeyDisposition::PassThrough)
            .unwrap_or_default()
    }
}

/// Returns `true` for `<input type>` values that edit text with a caret (or, for `range`, consume
/// horizontal arrows natively).
pub fn is_text_like_input_type(input_type: &str) -> bool {
    ![
        "checkbox", "radio", "button", "submit", "reset", "image", "file", "color", "hidden",
    ]
    .iter()
    .any(|t| input_type.eq_ignore_ascii_case(t))
}

/// `NodeId` from the raw `data-nve-id` / wire value.
#[inline]
pub(crate) const fn store_node_id(raw: u32) -> NodeId {
    NodeId::new(raw & NodeId::MAX_INDEX, (raw >> NodeId::INDEX_BITS) as u8)
}

/// Parses a `data-nve-id` attribute value; `"0"` (the null node) and garbage yield `None`.
pub fn parse_dom_node_id(attr: &str) -> Option<NodeId> {
    attr.parse::<u32>()
        .ok()
        .filter(|&raw| raw != 0)
        .map(store_node_id)
}

#[cfg(target_arch = "wasm32")]
pub(crate) use wasm::attach_keydown;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::rc::Rc;

    use nve_abi::{EventMask, EventPayload, NativeEvent, NativeEventKind, pack_key_modifiers};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    use super::{WebKeyDisposition, WebKeyEventInfo, is_text_like_input_type, store_node_id};
    use crate::events::{
        DomEventListener, collect_ancestors, notify_event, now_us, read_dom_input_value,
        register_listener, target_to_element,
    };

    /// `(in_text_input, is_multiline_input)` for a keydown target.
    fn text_input_kind(el: &web_sys::Element) -> (bool, bool) {
        if el.dyn_ref::<web_sys::HtmlTextAreaElement>().is_some() {
            return (true, true);
        }
        if let Some(input) = el.dyn_ref::<web_sys::HtmlInputElement>() {
            return (is_text_like_input_type(&input.type_()), false);
        }
        let editable = el
            .dyn_ref::<web_sys::HtmlElement>()
            .is_some_and(|h| h.is_content_editable());
        (editable, editable)
    }

    /// Keydown on `window` (global shortcuts like Cmd+K / Escape always reach ROOT), scoped to
    /// this surface, then key interceptors, then Enter-submit / default `KeyDown`.
    pub(crate) fn attach_keydown(
        listeners: &mut Vec<DomEventListener>,
        root: &web_sys::Element,
        backend: &crate::WebBackend,
    ) {
        let r = root.clone();
        let evs = Rc::clone(&backend.pending_events);
        let m = Rc::clone(&backend.node_event_meta);
        let ime_g = Rc::clone(&backend.ime_guard);
        let sid = Rc::clone(&backend.surface_id);
        let notify = Rc::clone(&backend.on_event);
        let interceptors = Rc::clone(&backend.key_interceptors);
        let key_target: web_sys::EventTarget =
            web_sys::window().map_or_else(|| root.clone().into(), Into::into);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
            let Some(ke) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
                return;
            };
            if ime_g.should_suppress_keydown(ke.is_composing(), ke.key_code()) {
                return;
            }
            let el = target_to_element(ev.target());
            // Ignore events targeting another mounted NVE surface's subtree.
            if let Some(ref e) = el
                && !r.contains(Some(e))
                && e.closest("[data-nve-live],[data-nve-id]")
                    .ok()
                    .flatten()
                    .is_some()
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
            let code = ke.code();
            let modifiers = pack_key_modifiers(
                ke.meta_key(),
                ke.ctrl_key(),
                ke.shift_key(),
                ke.alt_key(),
                ke.repeat(),
            );
            let timestamp_us = now_us();

            if in_this_surface && let Ok(mut hooks) = interceptors.try_borrow_mut() {
                let (in_text_input, is_multiline_input) =
                    el.as_ref().map_or((false, false), text_input_kind);
                let info = WebKeyEventInfo {
                    key: &key,
                    code: &code,
                    key_code: ke.key_code(),
                    modifiers,
                    repeat: ke.repeat(),
                    is_composing: ke.is_composing(),
                    in_text_input,
                    is_multiline_input,
                    target_node: chain.first().map(|(id, _, _)| store_node_id(*id)),
                    timestamp_us,
                    surface_root: &r,
                };
                match hooks.dispatch(&info) {
                    WebKeyDisposition::PassThrough => {}
                    WebKeyDisposition::ConsumedPreventDefault => {
                        ev.prevent_default();
                        return;
                    }
                    WebKeyDisposition::ConsumedAllowDefault => return,
                }
            }

            if in_this_surface && (ke.meta_key() || ke.ctrl_key()) && key.eq_ignore_ascii_case("k")
            {
                ev.prevent_default();
            }

            if key == "Enter"
                && let Some(ref target_el) = el
                && target_el.dyn_ref::<web_sys::HtmlInputElement>().is_some()
                && let Some((submit_id, _, _)) = chain
                    .iter()
                    .find(|(_, meta, _)| (meta.mask & EventMask::SUBMIT) != 0)
            {
                ev.prevent_default();
                evs.borrow_mut().push(NativeEvent {
                    kind: NativeEventKind::Submit,
                    surface_id: sid.get(),
                    node_id: *submit_id,
                    timestamp_us,
                    payload: EventPayload::Text(read_dom_input_value(target_el)),
                });
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
            evs.borrow_mut().push(NativeEvent {
                kind: NativeEventKind::KeyDown,
                surface_id: sid.get(),
                node_id: target_id,
                timestamp_us,
                payload: EventPayload::Key {
                    key,
                    code,
                    modifiers,
                    key_code: ke.key_code(),
                    is_composing: ke.is_composing(),
                },
            });
            notify_event(&notify);
        });
        register_listener(listeners, &key_target, "keydown", false, cb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(key: &str) -> WebKeyEventInfo<'_> {
        WebKeyEventInfo {
            key,
            code: "",
            key_code: 0,
            modifiers: 0,
            repeat: false,
            is_composing: false,
            in_text_input: false,
            is_multiline_input: false,
            target_node: None,
            timestamp_us: 0,
        }
    }

    #[test]
    fn interceptors_run_in_order_until_one_consumes() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut hooks = KeyInterceptors::default();
        for (name, consumes) in [("a", None), ("b", Some("Enter")), ("c", Some("x"))] {
            let seen = Rc::clone(&seen);
            hooks.push(Box::new(move |i| {
                seen.borrow_mut().push(name);
                match consumes {
                    Some(k) if k == i.key => WebKeyDisposition::ConsumedPreventDefault,
                    _ => WebKeyDisposition::PassThrough,
                }
            }));
        }
        assert_eq!(
            hooks.dispatch(&info("Enter")),
            WebKeyDisposition::ConsumedPreventDefault
        );
        assert_eq!(*seen.borrow(), ["a", "b"]);
        seen.borrow_mut().clear();
        assert_eq!(hooks.dispatch(&info("q")), WebKeyDisposition::PassThrough);
        assert_eq!(*seen.borrow(), ["a", "b", "c"]);
        assert_eq!(
            KeyInterceptors::default().dispatch(&info("q")),
            WebKeyDisposition::PassThrough
        );
    }

    #[test]
    fn text_like_input_types_and_raw_node_ids() {
        for t in [
            "text", "search", "EMAIL", "password", "number", "range", "date", "",
        ] {
            assert!(is_text_like_input_type(t), "{t}");
        }
        for t in [
            "checkbox", "Radio", "submit", "button", "file", "color", "hidden",
        ] {
            assert!(!is_text_like_input_type(t), "{t}");
        }
        let id = NodeId::new(0x12_3456, 7);
        assert_eq!(store_node_id(id.raw()), id);
        assert_eq!(parse_dom_node_id(&id.raw().to_string()), Some(id));
        assert_eq!(parse_dom_node_id("1"), Some(NodeId::ROOT));
        for bad in ["0", "", "-3", "x1", "4294967296"] {
            assert_eq!(parse_dom_node_id(bad), None, "{bad}");
        }
        let mut shortcut = info("k");
        assert!(!shortcut.has_command_modifier());
        shortcut.modifiers = nve_abi::NVE_KEY_MOD_SHIFT | nve_abi::NVE_KEY_MOD_REPEAT;
        assert!(!shortcut.has_command_modifier());
        for bit in [
            nve_abi::NVE_KEY_MOD_META,
            nve_abi::NVE_KEY_MOD_CTRL,
            nve_abi::NVE_KEY_MOD_ALT,
        ] {
            shortcut.modifiers = bit;
            assert!(shortcut.has_command_modifier());
        }
    }
}
