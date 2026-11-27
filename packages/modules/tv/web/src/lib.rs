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
//! - Open modal `<dialog>` or `[data-nve-focus-trap]`: navigation is confined to it.
//!
//! Off `wasm32` the DOM steps are no-ops, so the decision logic is natively testable: register
//! nodes through [`WebTvController::with_engine`] and feed [`WebKeyEventInfo`]s.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

#[cfg(target_arch = "wasm32")]
mod dom;
#[cfg(target_arch = "wasm32")]
mod gamepad_poller;
pub mod platform;

use std::cell::RefCell;
use std::rc::Rc;

use nve_backend_web::{WebBackend, WebKeyDisposition, WebKeyEventInfo};
use nve_tv::{
    DEFAULT_REPEAT_INTERVAL_US, FocusOutcome, NodeId, RemoteAction, RemoteInput,
    SpatialFocusEngine, TextInputContext, WebKeyInput, WebTvPlatform, map_web_tv_key,
};

/// App callback for non-directional actions; return `true` when handled (consumes the key).
pub type TvActionHandler = Box<dyn FnMut(RemoteAction, Option<NodeId>) -> bool>;

/// Controller configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebTvConfig {
    pub platform: WebTvPlatform,
    pub wrap_axis: bool,
    /// Minimum spacing of accepted auto-repeats per action (`0` disables throttling).
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
    /// The browser activates it natively (Enter on a link / button): let the key through.
    Native,
    /// Dispatch a synthetic `click()` (remote OK keys, gamepad A, non-native focusables).
    SyntheticClick,
}

impl SelectActivation {
    /// `tag_name` is the uppercase DOM tag; `input_type` the `<input type>` (else empty).
    pub fn for_target(key: &str, tag_name: &str, input_type: &str) -> Self {
        let enter_activates = match tag_name {
            "A" | "BUTTON" | "SUMMARY" => true,
            "INPUT" => ["submit", "reset", "button", "image"]
                .iter()
                .any(|t| input_type.eq_ignore_ascii_case(t)),
            _ => false,
        };
        if key == "Enter" && enter_activates {
            Self::Native
        } else {
            Self::SyntheticClick
        }
    }
}

struct TvState {
    platform: WebTvPlatform,
    engine: SpatialFocusEngine,
    #[cfg(target_arch = "wasm32")]
    dom: dom::DomFocusables,
}

struct Inner {
    state: RefCell<TvState>,
    handler: RefCell<Option<TvActionHandler>>,
    #[cfg(target_arch = "wasm32")]
    gamepad: RefCell<Option<gamepad_poller::GamepadPoller>>,
}

/// Shared handle to one surface's TV focus controller (cheap to clone).
#[derive(Clone)]
pub struct WebTvController {
    inner: Rc<Inner>,
}

impl WebTvController {
    pub fn new(config: WebTvConfig) -> Self {
        let mut engine = SpatialFocusEngine::new().with_wrap_axis(config.wrap_axis);
        engine
            .throttle_mut()
            .set_interval_us(config.repeat_interval_us);
        Self {
            inner: Rc::new(Inner {
                state: RefCell::new(TvState {
                    platform: config.platform,
                    engine,
                    #[cfg(target_arch = "wasm32")]
                    dom: dom::DomFocusables::default(),
                }),
                handler: RefCell::new(None),
                #[cfg(target_arch = "wasm32")]
                gamepad: RefCell::new(None),
            }),
        }
    }

    /// Creates a controller, registers it as `backend`'s key interceptor, and (on `wasm32`)
    /// registers platform keys and the gamepad poller as configured.
    pub fn install(backend: &mut WebBackend, config: WebTvConfig) -> Self {
        let controller = Self::new(config);
        let hook = controller.clone();
        backend.add_key_interceptor(move |info| hook.handle_key(info));
        if config.register_platform_keys {
            platform::register_platform_keys(config.platform);
        }
        if config.gamepad {
            controller.enable_gamepad();
        }
        controller
    }

    pub fn platform(&self) -> WebTvPlatform {
        self.inner.state.borrow().platform
    }

    /// Sets the handler for non-directional actions (Back, media, digits, colors, Select).
    pub fn set_action_handler<F>(&self, handler: F)
    where
        F: FnMut(RemoteAction, Option<NodeId>) -> bool + 'static,
    {
        *self.inner.handler.borrow_mut() = Some(Box::new(handler));
    }

    /// Direct engine access (tests, native hosts, programmatic focus).
    pub fn with_engine<R>(&self, f: impl FnOnce(&mut SpatialFocusEngine) -> R) -> R {
        f(&mut self.inner.state.borrow_mut().engine)
    }

    /// Starts the W3C Gamepad poller (runs only while a gamepad is connected). No-op off `wasm32`.
    pub fn enable_gamepad(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let poller = gamepad_poller::GamepadPoller::attach(Rc::downgrade(&self.inner));
            *self.inner.gamepad.borrow_mut() = poller;
        }
    }

    /// The key interceptor: normalizes `info` and dispatches TV actions.
    pub fn handle_key(&self, info: &WebKeyEventInfo<'_>) -> WebKeyDisposition {
        let input = WebKeyInput {
            key: info.key,
            code: info.code,
            key_code: info.key_code,
            is_composing: info.is_composing,
            command_modifier: info.has_command_modifier(),
            text_input: TextInputContext::from_flags(info.in_text_input, info.is_multiline_input),
        };
        let Some(action) = map_web_tv_key(self.platform(), &input) else {
            return WebKeyDisposition::PassThrough;
        };
        #[cfg(target_arch = "wasm32")]
        if let Ok(mut st) = self.inner.state.try_borrow_mut() {
            st.dom.set_root(info.surface_root);
        }
        let input = RemoteInput {
            action,
            repeat: info.repeat,
            timestamp_us: info.timestamp_us,
        };
        self.inner.dispatch(input, info.target_node, info.key)
    }
}

impl Inner {
    /// Routes one normalized input. `hint` is the node holding DOM focus; `key` the originating
    /// `KeyboardEvent.key` (`""` for gamepads).
    fn dispatch(&self, input: RemoteInput, hint: Option<NodeId>, key: &str) -> WebKeyDisposition {
        let Ok(mut guard) = self.state.try_borrow_mut() else {
            return WebKeyDisposition::PassThrough;
        };
        let st = &mut *guard;
        if input.action.direction().is_some() {
            #[cfg(target_arch = "wasm32")]
            st.dom.collect(&mut st.engine);
            if st.engine.is_empty() {
                return WebKeyDisposition::PassThrough;
            }
        }
        if let Some(node) = hint
            && st.engine.focused() != Some(node)
        {
            st.engine.set_focus(node);
        }
        let focused = hint.or(st.engine.focused());
        match st.engine.handle_input(input) {
            FocusOutcome::Moved(_node) => {
                #[cfg(target_arch = "wasm32")]
                st.dom.focus(_node);
                WebKeyDisposition::ConsumedPreventDefault
            }
            FocusOutcome::Blocked | FocusOutcome::Throttled => {
                WebKeyDisposition::ConsumedPreventDefault
            }
            FocusOutcome::Action(action) => {
                #[cfg(target_arch = "wasm32")]
                let target = focused.and_then(|node| st.dom.element(node));
                #[cfg(not(target_arch = "wasm32"))]
                let target: Option<()> = None;
                // Never hold the state borrow across app code or synthetic DOM events.
                drop(guard);
                self.dispatch_action(action, focused, key, target)
            }
        }
    }

    fn dispatch_action<E: ActivationTarget>(
        &self,
        action: RemoteAction,
        focused: Option<NodeId>,
        key: &str,
        target: Option<E>,
    ) -> WebKeyDisposition {
        let handled = self
            .handler
            .try_borrow_mut()
            .ok()
            .and_then(|mut h| h.as_mut().map(|f| f(action, focused)))
            .unwrap_or(false);
        if handled {
            return WebKeyDisposition::ConsumedPreventDefault;
        }
        match (action, target) {
            (RemoteAction::Select, Some(el)) => match el.activation(key) {
                SelectActivation::Native => WebKeyDisposition::PassThrough,
                SelectActivation::SyntheticClick => {
                    el.click();
                    WebKeyDisposition::ConsumedPreventDefault
                }
            },
            _ => WebKeyDisposition::PassThrough,
        }
    }
}

/// A focused element a Select can activate.
trait ActivationTarget {
    fn activation(&self, key: &str) -> SelectActivation;
    fn click(&self);
}

/// Native placeholder: there is no DOM to activate.
#[cfg(not(target_arch = "wasm32"))]
impl ActivationTarget for () {
    fn activation(&self, _key: &str) -> SelectActivation {
        SelectActivation::Native
    }

    fn click(&self) {}
}
