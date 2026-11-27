//! Web TV D-Pad / remote / gamepad controller for `nve-backend-web` (`nve-tv-web`).
//!
//! [`WebTvController::install`] registers one generic key interceptor on a
//! [`WebBackend`]. Each TV key is normalized by [`nve_tv::map_web_tv_key`]; directional keys
//! collect `[data-nve-id]` focusable rects just in time (no per-frame DOM polling), move spatial
//! focus, focus + scroll the target into view, and `preventDefault()`. Other actions go to the
//! app's [`TvActionHandler`]; an unhandled Select activates the focused element.
//!
//! DOM attributes understood on `wasm32`:
//! - `data-nve-focus-group="<name>"` on a container: per-group focus memory (shelves, rails).
//! - `data-nve-next-{up,down,left,right}="<element id>"` on a focusable: explicit neighbor.
//! - Open modal `<dialog>` (anywhere in the document) or `[data-nve-focus-trap]`: navigation is
//!   confined to it.
//! - `data-nve-spatial-nav="off"` and arrow-owning ARIA widgets (`tablist`, `slider`, `listbox`,
//!   `grid`, native `<select>`, ...): their arrows reach the widget
//!   ([`nve_backend_web::ArrowAxes`]).
//!
//! Re-entrancy: no `RefCell` borrow is held while DOM focus / scroll / click run or while app
//! code (the action handler) runs, so `focusin` listeners and handlers may use the controller.
//!
//! Off `wasm32` the DOM steps are no-ops, so the decision logic is natively testable: register
//! nodes through [`WebTvController::with_engine`] and feed [`WebKeyEventInfo`]s.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

#[cfg(target_arch = "wasm32")]
mod dom;
#[cfg(target_arch = "wasm32")]
mod gamepad_poller;
pub mod platform;
pub mod rules;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nve_backend_web::{InterceptorId, WebBackend, WebKeyDisposition, WebKeyEventInfo};
use nve_tv::{
    DEFAULT_REPEAT_INTERVAL_US, FocusOutcome, NodeId, RemoteAction, RemoteInput,
    SpatialFocusEngine, TextInputContext, WebKeyInput, WebTvPlatform, map_web_tv_key,
};

/// App callback for non-directional actions; return `true` when handled (consumes the key).
/// Capture a [`WeakWebTvController`], never a [`WebTvController`], to avoid an `Rc` cycle.
pub type TvActionHandler = Box<dyn FnMut(RemoteAction, Option<NodeId>) -> bool>;

/// Controller configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebTvConfig {
    pub platform: WebTvPlatform,
    pub wrap_axis: bool,
    /// Minimum spacing of accepted auto-repeats per action (`0` disables rate limiting).
    pub repeat_interval_us: u64,
    /// Poll the W3C Gamepad API while a gamepad is connected.
    pub gamepad: bool,
    /// Register vendor remote keys (Tizen `tvinputdevice.registerKeyBatch`).
    pub register_platform_keys: bool,
}

impl WebTvConfig {
    pub const fn for_platform(platform: WebTvPlatform) -> Self {
        Self {
            platform,
            wrap_axis: false,
            repeat_interval_us: DEFAULT_REPEAT_INTERVAL_US,
            gamepad: false,
            register_platform_keys: true,
        }
    }

    /// Configuration for the running browser (`navigator.userAgent`; `Standard` off `wasm32`).
    pub fn detect() -> Self {
        Self::for_platform(platform::detect_platform())
    }
}

/// How a Select activates the focused element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectActivation {
    /// The browser activates it natively (Enter on a focused link / button): let the key through.
    Native,
    /// Dispatch a synthetic `click()` (remote OK keys, gamepad A, non-native focusables, and
    /// elements that do not hold DOM focus).
    SyntheticClick,
    /// Open a native `<select>` picker (`showPicker()`).
    ShowPicker,
}

impl SelectActivation {
    /// `tag_name` is the uppercase DOM tag; `input_type` the `<input type>` (else empty);
    /// `holds_dom_focus` whether the element is `document.activeElement` (only then does the
    /// browser's own Enter activation reach it).
    pub fn for_target(key: &str, tag_name: &str, input_type: &str, holds_dom_focus: bool) -> Self {
        let enter_activates = match tag_name {
            "SELECT" => return Self::ShowPicker,
            "A" | "BUTTON" | "SUMMARY" => true,
            "INPUT" => ["submit", "reset", "button", "image"]
                .iter()
                .any(|t| input_type.eq_ignore_ascii_case(t)),
            _ => false,
        };
        if key == "Enter" && enter_activates && holds_dom_focus {
            Self::Native
        } else {
            Self::SyntheticClick
        }
    }
}

struct TvState {
    engine: SpatialFocusEngine,
    #[cfg(target_arch = "wasm32")]
    dom: dom::DomFocusables,
}

#[derive(Default)]
struct HandlerSlot {
    /// `None` while the handler runs.
    handler: Option<TvActionHandler>,
    /// Bumped by every set / clear, so a handler replaced while running is not restored.
    generation: u64,
}

struct Inner {
    platform: WebTvPlatform,
    state: RefCell<TvState>,
    handler: RefCell<HandlerSlot>,
    /// Bit per [`RemoteAction::code`]: the last dispatched press of that action was consumed,
    /// so its dropped auto-repeats are consumed too (unhandled keys' repeats pass through).
    owned_actions: Cell<u64>,
    interceptor: Cell<Option<InterceptorId>>,
    #[cfg(target_arch = "wasm32")]
    gamepad: RefCell<Option<gamepad_poller::GamepadPoller>>,
}

/// Shared handle to one surface's TV focus controller (cheap to clone).
#[derive(Clone)]
pub struct WebTvController {
    inner: Rc<Inner>,
}

/// Non-owning controller handle for action handlers and other callbacks.
#[derive(Clone)]
pub struct WeakWebTvController(Weak<Inner>);

impl WeakWebTvController {
    pub fn upgrade(&self) -> Option<WebTvController> {
        self.0.upgrade().map(|inner| WebTvController { inner })
    }
}

impl WebTvController {
    pub fn new(config: WebTvConfig) -> Self {
        let mut engine = SpatialFocusEngine::new().with_wrap_axis(config.wrap_axis);
        engine
            .throttle_mut()
            .set_interval_us(config.repeat_interval_us);
        Self {
            inner: Rc::new(Inner {
                platform: config.platform,
                state: RefCell::new(TvState {
                    engine,
                    #[cfg(target_arch = "wasm32")]
                    dom: dom::DomFocusables::default(),
                }),
                handler: RefCell::default(),
                owned_actions: Cell::new(0),
                interceptor: Cell::new(None),
                #[cfg(target_arch = "wasm32")]
                gamepad: RefCell::new(None),
            }),
        }
    }

    /// Creates a controller, registers it as `backend`'s key interceptor, and (on `wasm32`)
    /// registers platform keys and the gamepad poller as configured. The backend keeps the
    /// controller alive until [`Self::uninstall`].
    pub fn install(backend: &mut WebBackend, config: WebTvConfig) -> Self {
        let controller = Self::new(config);
        #[cfg(target_arch = "wasm32")]
        if let (Some(root), Ok(mut st)) = (
            backend.mount_root(),
            controller.inner.state.try_borrow_mut(),
        ) {
            st.dom.set_root(root);
        }
        let hook = controller.clone();
        let id = backend.add_key_interceptor(move |info| hook.handle_key(info));
        controller.inner.interceptor.set(Some(id));
        if config.register_platform_keys {
            platform::register_platform_keys(config.platform);
        }
        if config.gamepad {
            controller.enable_gamepad();
        }
        controller
    }

    /// Removes the key interceptor from `backend` and releases the gamepad poller and action
    /// handler (breaking every reference the controller holds).
    pub fn uninstall(&self, backend: &mut WebBackend) {
        if let Some(id) = self.inner.interceptor.take() {
            backend.remove_key_interceptor(id);
        }
        self.disable_gamepad();
        self.clear_action_handler();
    }

    pub fn downgrade(&self) -> WeakWebTvController {
        WeakWebTvController(Rc::downgrade(&self.inner))
    }

    #[inline]
    pub fn platform(&self) -> WebTvPlatform {
        self.inner.platform
    }

    /// Sets the handler for non-directional actions (Back, media, digits, colors, Select).
    /// Callable from inside the running handler (the new one applies to the next action).
    pub fn set_action_handler<F>(&self, handler: F)
    where
        F: FnMut(RemoteAction, Option<NodeId>) -> bool + 'static,
    {
        self.replace_handler(Some(Box::new(handler)));
    }

    pub fn clear_action_handler(&self) {
        self.replace_handler(None);
    }

    fn replace_handler(&self, handler: Option<TvActionHandler>) {
        let previous = self.inner.handler.try_borrow_mut().ok().map(|mut slot| {
            slot.generation += 1;
            std::mem::replace(&mut slot.handler, handler)
        });
        // The old handler (and what it captured) drops outside the borrow.
        drop(previous);
    }

    /// Direct engine access (tests, native hosts, programmatic focus). `None` when called
    /// re-entrantly while the controller is dispatching.
    pub fn with_engine<R>(&self, f: impl FnOnce(&mut SpatialFocusEngine) -> R) -> Option<R> {
        let mut st = self.inner.state.try_borrow_mut().ok()?;
        Some(f(&mut st.engine))
    }

    /// Starts the W3C Gamepad poller (runs only while a standard-mapping gamepad is connected).
    /// On Xbox it also switches Edge's gamepad-as-keyboard emulation off, so presses are not
    /// delivered twice. No-op off `wasm32`.
    pub fn enable_gamepad(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let poller = gamepad_poller::GamepadPoller::attach(Rc::downgrade(&self.inner));
            if let Ok(mut slot) = self.inner.gamepad.try_borrow_mut() {
                let previous = std::mem::replace(&mut *slot, poller);
                drop(slot);
                drop(previous);
            }
            if self.inner.platform == WebTvPlatform::Xbox {
                platform::set_xbox_gamepad_emulation(false);
            }
        }
    }

    /// Stops the gamepad poller and removes its window listeners.
    pub fn disable_gamepad(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let poller = self
                .inner
                .gamepad
                .try_borrow_mut()
                .ok()
                .and_then(|mut p| p.take());
            if poller.is_some() && self.inner.platform == WebTvPlatform::Xbox {
                platform::set_xbox_gamepad_emulation(true);
            }
            drop(poller);
        }
    }

    /// The key interceptor: normalizes `info` and dispatches TV actions.
    pub fn handle_key(&self, info: &WebKeyEventInfo<'_>) -> WebKeyDisposition {
        let text_input = TextInputContext::from_flags(info.in_text_input, info.is_multiline_input);
        let input = WebKeyInput {
            key: info.key,
            code: info.code,
            key_code: info.key_code,
            is_composing: info.is_composing,
            command_modifier: info.has_command_modifier(),
            text_input,
        };
        let Some(action) = map_web_tv_key(self.inner.platform, &input) else {
            return WebKeyDisposition::PassThrough;
        };
        #[cfg(target_arch = "wasm32")]
        if let (Some(root), Ok(mut st)) = (info.surface_root, self.inner.state.try_borrow_mut()) {
            st.dom.set_root(root);
        }
        let inner = &self.inner;
        if let Some(direction) = action.direction() {
            let owned = if direction.is_horizontal() {
                info.owned_arrows.owns_horizontal()
            } else {
                info.owned_arrows.owns_vertical()
            };
            if owned {
                return inner.settle(action, WebKeyDisposition::PassThrough);
            }
        }
        if text_input.blurs_on(action) && !info.repeat {
            // Back / Escape first releases the text control (closing the on-screen keyboard).
            #[cfg(target_arch = "wasm32")]
            dom::blur_active();
            return inner.settle(action, WebKeyDisposition::ConsumedPreventDefault);
        }
        let input = RemoteInput {
            action,
            repeat: info.repeat,
            timestamp_us: info.timestamp_us,
        };
        inner.dispatch(input, info.target_node, info.key)
    }
}

impl Inner {
    /// Routes one normalized input. `hint` is the node holding DOM focus; `key` the originating
    /// `KeyboardEvent.key` (`""` for gamepads).
    fn dispatch(&self, input: RemoteInput, hint: Option<NodeId>, key: &str) -> WebKeyDisposition {
        let action = input.action;
        let Ok(mut guard) = self.state.try_borrow_mut() else {
            return WebKeyDisposition::PassThrough;
        };
        let st = &mut *guard;
        if !st.engine.would_admit(&input) {
            // A dropped repeat (checked before any layout read) mirrors its fresh press.
            return if self.owned_actions.get() & action_bit(action) != 0 {
                WebKeyDisposition::ConsumedPreventDefault
            } else {
                WebKeyDisposition::PassThrough
            };
        }
        if action.direction().is_some() {
            // `data-nve-next-*` are read for the node focus actually leaves (DOM focus first).
            #[cfg(target_arch = "wasm32")]
            {
                let leaving = hint.or(st.engine.focused());
                st.dom.collect(&mut st.engine, leaving);
            }
            if st.engine.is_empty() {
                drop(guard);
                return self.settle(action, WebKeyDisposition::PassThrough);
            }
        }
        if let Some(node) = hint
            && st.engine.focused() != Some(node)
        {
            st.engine.set_focus(node);
        }
        let previous = st.engine.focused();
        let focused = hint.or(previous);
        let outcome = st.engine.handle_input(input);
        #[cfg(target_arch = "wasm32")]
        let target = match outcome {
            FocusOutcome::Moved(node) => st.dom.element(node),
            FocusOutcome::Action(RemoteAction::Select) => {
                focused.and_then(|node| st.dom.activation_target(node))
            }
            _ => None,
        };
        #[cfg(not(target_arch = "wasm32"))]
        let target: Option<NoDom> = None;
        // Never hold the state borrow across DOM focus / click events or app code.
        drop(guard);
        let disposition = match outcome {
            FocusOutcome::Moved(_) => {
                if let Some(el) = target
                    && !el.focus_and_reveal()
                {
                    // Focus did not stick (element became inert / hidden): keep the old focus.
                    self.restore_focus(previous);
                }
                WebKeyDisposition::ConsumedPreventDefault
            }
            FocusOutcome::Blocked | FocusOutcome::Throttled => {
                WebKeyDisposition::ConsumedPreventDefault
            }
            FocusOutcome::Action(action) => self.dispatch_action(action, focused, key, target),
        };
        self.settle(action, disposition)
    }

    fn restore_focus(&self, previous: Option<NodeId>) {
        if let Ok(mut st) = self.state.try_borrow_mut() {
            match previous {
                Some(node) => {
                    st.engine.set_focus(node);
                }
                None => st.engine.clear_focus(),
            }
        }
    }

    /// Records whether `action` was consumed (its dropped repeats follow suit).
    fn settle(&self, action: RemoteAction, disposition: WebKeyDisposition) -> WebKeyDisposition {
        let bit = action_bit(action);
        let owned = self.owned_actions.get();
        self.owned_actions
            .set(if disposition == WebKeyDisposition::PassThrough {
                owned & !bit
            } else {
                owned | bit
            });
        disposition
    }

    fn dispatch_action<E: ActivationTarget>(
        &self,
        action: RemoteAction,
        focused: Option<NodeId>,
        key: &str,
        target: Option<E>,
    ) -> WebKeyDisposition {
        if self.call_handler(action, focused) {
            return WebKeyDisposition::ConsumedPreventDefault;
        }
        let (RemoteAction::Select, Some(el)) = (action, target) else {
            return WebKeyDisposition::PassThrough;
        };
        match el.activation(key) {
            SelectActivation::Native => WebKeyDisposition::PassThrough,
            SelectActivation::SyntheticClick => {
                el.click();
                WebKeyDisposition::ConsumedPreventDefault
            }
            SelectActivation::ShowPicker if el.show_picker() => {
                WebKeyDisposition::ConsumedPreventDefault
            }
            SelectActivation::ShowPicker => WebKeyDisposition::PassThrough,
        }
    }

    /// Runs the action handler with its slot emptied (re-entrant dispatches see no handler);
    /// it is put back only when nobody replaced or cleared it meanwhile.
    fn call_handler(&self, action: RemoteAction, focused: Option<NodeId>) -> bool {
        let taken = self
            .handler
            .try_borrow_mut()
            .ok()
            .and_then(|mut slot| Some((slot.handler.take()?, slot.generation)));
        let Some((mut handler, generation)) = taken else {
            return false;
        };
        let handled = handler(action, focused);
        let replaced = match self.handler.try_borrow_mut() {
            Ok(mut slot) if slot.generation == generation && slot.handler.is_none() => {
                slot.handler = Some(handler);
                None
            }
            _ => Some(handler),
        };
        drop(replaced);
        handled
    }
}

#[inline]
const fn action_bit(action: RemoteAction) -> u64 {
    1u64 << action.code()
}

const _: () = assert!(RemoteAction::CODE_COUNT <= u64::BITS as usize);

/// A DOM element a move focuses or a Select activates.
trait ActivationTarget {
    fn activation(&self, key: &str) -> SelectActivation;
    fn click(&self);
    /// Opens a native picker; `false` when unsupported or refused.
    fn show_picker(&self) -> bool;
    /// Focuses without the browser's jump-scroll, then reveals it; `false` when focus did not
    /// land on it.
    fn focus_and_reveal(&self) -> bool;
}

/// Native: there is no DOM (uninhabited).
#[cfg(not(target_arch = "wasm32"))]
enum NoDom {}

#[cfg(not(target_arch = "wasm32"))]
impl ActivationTarget for NoDom {
    fn activation(&self, _key: &str) -> SelectActivation {
        match *self {}
    }

    fn click(&self) {
        match *self {}
    }

    fn show_picker(&self) -> bool {
        match *self {}
    }

    fn focus_and_reveal(&self) -> bool {
        match *self {}
    }
}
