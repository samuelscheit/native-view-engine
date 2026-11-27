//! Generic `keydown` interception hook (TV remotes, game pads, app-level key routing) and the
//! surface-scoped `wasm32` keydown listener that feeds it before the default `KeyDown` event.
//!
//! The backend knows nothing about TV semantics: capability modules (e.g. `nve-tv-web`) register
//! interceptors via [`crate::WebBackend::add_key_interceptor`].

use std::cell::RefCell;

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
    /// Arrow axes the focused widget handles itself (see [`ArrowAxes::of_widget`]).
    pub owned_arrows: ArrowAxes,
    /// Nearest `data-nve-id` ancestor of the event target (never [`NodeId::NULL`]).
    pub target_node: Option<NodeId>,
    /// `performance.now()` in microseconds.
    pub timestamp_us: u64,
    /// Mount root of the surface this key is scoped to.
    #[cfg(target_arch = "wasm32")]
    pub surface_root: Option<&'a web_sys::Element>,
}

impl<'a> WebKeyEventInfo<'a> {
    /// A plain press with no modifiers, target, or text context on every target (headless replay
    /// and tests; set the remaining fields with struct-update syntax).
    pub const fn new(key: &'a str, code: &'a str, key_code: u32) -> Self {
        Self {
            key,
            code,
            key_code,
            modifiers: 0,
            repeat: false,
            is_composing: false,
            in_text_input: false,
            is_multiline_input: false,
            owned_arrows: ArrowAxes::NONE,
            target_node: None,
            timestamp_us: 0,
            #[cfg(target_arch = "wasm32")]
            surface_root: None,
        }
    }

    /// Ctrl, Meta, or Alt held: the key is an application / OS shortcut.
    #[inline]
    pub const fn has_command_modifier(&self) -> bool {
        self.modifiers
            & (nve_abi::NVE_KEY_MOD_META | nve_abi::NVE_KEY_MOD_CTRL | nve_abi::NVE_KEY_MOD_ALT)
            != 0
    }
}

/// Arrow-key axes a focused composite widget consumes itself (WAI-ARIA keyboard models), so
/// spatial navigation must let them through.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ArrowAxes(u8);

impl ArrowAxes {
    pub const NONE: Self = Self(0);
    /// Left / Right.
    pub const HORIZONTAL: Self = Self(1);
    /// Up / Down.
    pub const VERTICAL: Self = Self(2);
    pub const BOTH: Self = Self(3);

    #[inline]
    pub const fn owns_horizontal(self) -> bool {
        self.0 & Self::HORIZONTAL.0 != 0
    }

    #[inline]
    pub const fn owns_vertical(self) -> bool {
        self.0 & Self::VERTICAL.0 != 0
    }

    /// Axes owned by the nearest element matching [`ARROW_OWNER_SELECTOR`]:
    /// - `data-nve-spatial-nav="off"` owns both; any other value opts a widget back into spatial
    ///   navigation (owns none).
    /// - Native `<select>` owns Up / Down (Left / Right still leave it).
    /// - Roles: `slider`, `tablist`, `menubar` horizontal; `listbox`, `menu`, `combobox`,
    ///   `spinbutton` vertical; `grid`, `treegrid`, `tree` both. `aria-orientation` overrides
    ///   the axis of `slider` / `tablist` / `menubar` / `listbox` / `menu`.
    ///
    /// `tag_name` is the uppercase DOM tag; absent attributes are `None`.
    pub fn of_widget(
        tag_name: &str,
        role: Option<&str>,
        orientation: Option<&str>,
        spatial_nav: Option<&str>,
    ) -> Self {
        if let Some(mode) = spatial_nav {
            return if mode.eq_ignore_ascii_case("off") {
                Self::BOTH
            } else {
                Self::NONE
            };
        }
        if tag_name.eq_ignore_ascii_case("SELECT") {
            return Self::VERTICAL;
        }
        let (axes, oriented) = match role.unwrap_or_default() {
            "slider" | "tablist" | "menubar" => (Self::HORIZONTAL, true),
            "listbox" | "menu" => (Self::VERTICAL, true),
            "combobox" | "spinbutton" => (Self::VERTICAL, false),
            "grid" | "treegrid" | "tree" => (Self::BOTH, false),
            _ => return Self::NONE,
        };
        match orientation.filter(|_| oriented) {
            Some("horizontal") => Self::HORIZONTAL,
            Some("vertical") => Self::VERTICAL,
            _ => axes,
        }
    }
}

/// `Element.closest` selector of the elements [`ArrowAxes::of_widget`] classifies.
pub const ARROW_OWNER_SELECTOR: &str = "[data-nve-spatial-nav],select,[role=slider],\
    [role=tablist],[role=menubar],[role=listbox],[role=menu],[role=combobox],[role=spinbutton],\
    [role=grid],[role=treegrid],[role=tree]";

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

/// Handle of a registered key interceptor (for [`crate::WebBackend::remove_key_interceptor`]).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterceptorId(u64);

pub(crate) type KeyInterceptor = Box<dyn FnMut(&WebKeyEventInfo<'_>) -> WebKeyDisposition>;

/// Interceptors in registration order (ascending id).
#[derive(Default)]
pub(crate) struct KeyInterceptors {
    next_id: u64,
    /// `None` while that interceptor runs.
    hooks: Vec<(InterceptorId, Option<KeyInterceptor>)>,
}

impl KeyInterceptors {
    pub(crate) fn push(&mut self, hook: KeyInterceptor) -> InterceptorId {
        let id = InterceptorId(self.next_id);
        self.next_id += 1;
        self.hooks.push((id, Some(hook)));
        id
    }

    /// Unregisters `id`; returns the interceptor (`None` when unknown, or while it runs: it is
    /// then dropped once it returns).
    pub(crate) fn remove(&mut self, id: InterceptorId) -> Option<Option<KeyInterceptor>> {
        let pos = self.hooks.binary_search_by_key(&id, |(i, _)| *i).ok()?;
        Some(self.hooks.remove(pos).1)
    }

    /// Runs interceptors in order until one consumes the key. No borrow of `cell` is held while
    /// an interceptor runs, so interceptors may add / remove interceptors or dispatch again; a
    /// nested dispatch skips the interceptors already running, and interceptors added meanwhile
    /// run after the current one.
    pub(crate) fn try_dispatch(
        cell: &RefCell<Self>,
        info: &WebKeyEventInfo<'_>,
    ) -> WebKeyDisposition {
        let mut after = None;
        loop {
            let next = cell
                .try_borrow_mut()
                .ok()
                .and_then(|mut s| s.take_next(after));
            let Some((id, mut hook)) = next else {
                return WebKeyDisposition::PassThrough;
            };
            let disposition = hook(info);
            let removed = match cell.try_borrow_mut() {
                Ok(mut s) => s.restore(id, hook),
                Err(_) => Some(hook),
            };
            // A self-removed interceptor drops here, outside any borrow.
            drop(removed);
            if disposition != WebKeyDisposition::PassThrough {
                return disposition;
            }
            after = Some(id);
        }
    }

    fn take_next(
        &mut self,
        after: Option<InterceptorId>,
    ) -> Option<(InterceptorId, KeyInterceptor)> {
        let start = after.map_or(0, |a| self.hooks.partition_point(|(id, _)| *id <= a));
        self.hooks[start..]
            .iter_mut()
            .find_map(|(id, slot)| Some((*id, slot.take()?)))
    }

    /// Puts a ran interceptor back; hands it back when it was removed meanwhile.
    fn restore(&mut self, id: InterceptorId, hook: KeyInterceptor) -> Option<KeyInterceptor> {
        match self.hooks.binary_search_by_key(&id, |(i, _)| *i) {
            Ok(pos) if self.hooks[pos].1.is_none() => {
                self.hooks[pos].1 = Some(hook);
                None
            }
            _ => Some(hook),
        }
    }
}

/// Returns `true` for `<input type>` values that edit text with a caret (or, for `range`, consume
/// horizontal arrows natively).
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn is_text_like_input_type(input_type: &str) -> bool {
    ![
        "checkbox", "radio", "button", "submit", "reset", "image", "file", "color", "hidden",
    ]
    .iter()
    .any(|t| input_type.eq_ignore_ascii_case(t))
}

/// `NodeId` from a raw `data-nve-id` / wire value; `0` (the null node) yields `None`.
#[inline]
pub(crate) const fn node_id_from_raw(raw: u32) -> Option<NodeId> {
    if raw == 0 {
        None
    } else {
        Some(NodeId::from_raw(raw))
    }
}

/// Parses a `data-nve-id` attribute value; `"0"` (the null node) and garbage yield `None`.
pub fn parse_dom_node_id(attr: &str) -> Option<NodeId> {
    attr.parse::<u32>().ok().and_then(node_id_from_raw)
}

/// Routes keys aimed at `<body>` / `<html>` (nothing focused) to one mounted surface: the last
/// one focus entered, else the first one mounted. An unmounted (dropped) or detached owner is
/// replaced by the next surface that asks.
#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) struct BodyKeyRoute<T>(Option<std::rc::Weak<T>>);

#[cfg(any(test, target_arch = "wasm32"))]
impl<T> BodyKeyRoute<T> {
    pub(crate) const fn new() -> Self {
        Self(None)
    }

    pub(crate) fn focus_entered(&mut self, surface: &std::rc::Rc<T>) {
        self.0 = Some(std::rc::Rc::downgrade(surface));
    }

    pub(crate) fn accepts(
        &mut self,
        surface: &std::rc::Rc<T>,
        attached: impl Fn(&T) -> bool,
    ) -> bool {
        if let Some(owner) = self.0.as_ref().and_then(std::rc::Weak::upgrade)
            && attached(&owner)
        {
            return std::rc::Rc::ptr_eq(&owner, surface);
        }
        self.focus_entered(surface);
        true
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use wasm::attach_keydown;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;
    use std::rc::Rc;

    use nve_abi::{EventMask, EventPayload, NativeEvent, NativeEventKind, pack_key_modifiers};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    use super::{
        ARROW_OWNER_SELECTOR, ArrowAxes, BodyKeyRoute, KeyInterceptors, WebKeyDisposition,
        WebKeyEventInfo, is_text_like_input_type, node_id_from_raw,
    };
    use crate::events::{
        DomEventListener, collect_ancestors, notify_event, now_us, read_dom_input_value,
        register_listener, target_to_element,
    };

    thread_local! {
        static BODY_KEYS: RefCell<BodyKeyRoute<web_sys::Element>> =
            const { RefCell::new(BodyKeyRoute::new()) };
    }

    fn body_keys_accept(surface: &Rc<web_sys::Element>) -> bool {
        BODY_KEYS.with(|route| {
            route
                .borrow_mut()
                .accepts(surface, |root: &web_sys::Element| root.is_connected())
        })
    }

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

    fn owned_arrows(el: &web_sys::Element) -> ArrowAxes {
        el.closest(ARROW_OWNER_SELECTOR)
            .ok()
            .flatten()
            .map_or(ArrowAxes::NONE, |w| {
                ArrowAxes::of_widget(
                    &w.tag_name(),
                    w.get_attribute("role").as_deref(),
                    w.get_attribute("aria-orientation").as_deref(),
                    w.get_attribute("data-nve-spatial-nav").as_deref(),
                )
            })
    }

    fn is_document_level(el: &web_sys::Element) -> bool {
        let tag = el.tag_name();
        tag.eq_ignore_ascii_case("BODY") || tag.eq_ignore_ascii_case("HTML")
    }

    /// Keydown on `window` (global shortcuts like Cmd+K / Escape reach ROOT even with nothing
    /// focused), scoped to this surface, then key interceptors, then Enter-submit / default
    /// `KeyDown`. Keys aimed at `<body>` go to one surface only ([`BodyKeyRoute`]).
    pub(crate) fn attach_keydown(
        listeners: &mut Vec<DomEventListener>,
        root: &web_sys::Element,
        backend: &crate::WebBackend,
    ) {
        // Owned by this surface's listeners: dropping them (unmount) releases body-key routing.
        let surface = Rc::new(root.clone());
        body_keys_accept(&surface);
        let focus_surface = Rc::clone(&surface);
        let on_focus = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            BODY_KEYS.with(|route| route.borrow_mut().focus_entered(&focus_surface));
        });
        register_listener(listeners, &root.clone().into(), "focusin", false, on_focus);

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
            let r: &web_sys::Element = &surface;
            let el = target_to_element(ev.target());
            let inside = el.as_ref().is_some_and(|e| r.contains(Some(e)));
            // Ignore events targeting another mounted NVE surface's subtree.
            if let Some(ref e) = el
                && !inside
                && e.closest("[data-nve-live],[data-nve-id]")
                    .ok()
                    .flatten()
                    .is_some()
            {
                return;
            }
            let chain = el
                .as_ref()
                .filter(|_| inside)
                .map(|e| collect_ancestors(e, r, &m.borrow()))
                .unwrap_or_default();
            let key = ke.key();
            let in_this_surface =
                inside || el.as_ref().is_none_or(is_document_level) && body_keys_accept(&surface);
            let code = ke.code();
            let modifiers = pack_key_modifiers(
                ke.meta_key(),
                ke.ctrl_key(),
                ke.shift_key(),
                ke.alt_key(),
                ke.repeat(),
            );
            let timestamp_us = now_us();

            if in_this_surface {
                let (in_text_input, is_multiline_input) =
                    el.as_ref().map_or((false, false), text_input_kind);
                let info = WebKeyEventInfo {
                    modifiers,
                    repeat: ke.repeat(),
                    is_composing: ke.is_composing(),
                    in_text_input,
                    is_multiline_input,
                    owned_arrows: el.as_ref().map_or(ArrowAxes::NONE, owned_arrows),
                    target_node: chain.first().and_then(|(id, _, _)| node_id_from_raw(*id)),
                    timestamp_us,
                    surface_root: Some(r),
                    ..WebKeyEventInfo::new(&key, &code, ke.key_code())
                };
                match KeyInterceptors::try_dispatch(&interceptors, &info) {
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

