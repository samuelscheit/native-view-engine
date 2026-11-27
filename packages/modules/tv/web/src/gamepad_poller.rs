//! Opt-in W3C Gamepad poller: a `requestAnimationFrame` loop that runs only between
//! `gamepadconnected` and the last `gamepaddisconnected` (zero idle CPU), feeding
//! [`nve_tv::GamepadRepeater`] edges into the controller.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nve_tv::{GamepadFrame, GamepadRepeater};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::Inner;

type FrameCallback = Closure<dyn FnMut(f64)>;
type EventCallback = Closure<dyn FnMut(web_sys::Event)>;

/// Standard-mapping buttons read per frame.
const POLLED_BUTTONS: u32 = 16;

struct LoopState {
    controller: Weak<Inner>,
    running: Cell<bool>,
    /// Pending `requestAnimationFrame` id (cancelled on drop).
    frame_id: Cell<Option<i32>>,
    repeaters: RefCell<Vec<GamepadRepeater>>,
    /// rAF callback, created once and reused across connect / disconnect cycles.
    frame: RefCell<Option<FrameCallback>>,
}

pub(crate) struct GamepadPoller {
    window: web_sys::Window,
    state: Rc<LoopState>,
    connected: EventCallback,
    disconnected: EventCallback,
}

impl GamepadPoller {
    pub(crate) fn attach(controller: Weak<Inner>) -> Option<Self> {
        let window = web_sys::window()?;
        let state = Rc::new(LoopState {
            controller,
            running: Cell::new(false),
            frame_id: Cell::new(None),
            repeaters: RefCell::new(Vec::new()),
            frame: RefCell::new(None),
        });
        let on_connect = Rc::clone(&state);
        let connected = EventCallback::new(move |_: web_sys::Event| start(&on_connect));
        let on_disconnect = Rc::clone(&state);
        let disconnected = EventCallback::new(move |_: web_sys::Event| {
            if !any_connected() {
                stop(&on_disconnect);
            }
        });
        let _ = window.add_event_listener_with_callback(
            "gamepadconnected",
            connected.as_ref().unchecked_ref(),
        );
        let _ = window.add_event_listener_with_callback(
            "gamepaddisconnected",
            disconnected.as_ref().unchecked_ref(),
        );
        // A pad connected before attach (e.g. after a reload) never fires `gamepadconnected`.
        if any_connected() {
            start(&state);
        }
        Some(Self {
            window,
            state,
            connected,
            disconnected,
        })
    }
}

impl Drop for GamepadPoller {
    fn drop(&mut self) {
        let _ = self.window.remove_event_listener_with_callback(
            "gamepadconnected",
            self.connected.as_ref().unchecked_ref(),
        );
        let _ = self.window.remove_event_listener_with_callback(
            "gamepaddisconnected",
            self.disconnected.as_ref().unchecked_ref(),
        );
        stop(&self.state);
        if let Some(id) = self.state.frame_id.take() {
            let _ = self.window.cancel_animation_frame(id);
        }
        // Drop may run inside this very callback (last controller handle released mid-tick):
        // hand the closure to the JS GC instead of freeing it under its own feet.
        if let Some(frame) = self.state.frame.borrow_mut().take() {
            let _ = frame.into_js_value();
        }
    }
}

fn gamepads() -> impl Iterator<Item = web_sys::Gamepad> {
    web_sys::window()
        .and_then(|w| w.navigator().get_gamepads().ok())
        .into_iter()
        .flat_map(|pads| pads.to_vec())
        .filter_map(|pad| pad.dyn_into::<web_sys::Gamepad>().ok())
        .filter(web_sys::Gamepad::connected)
}

fn any_connected() -> bool {
    gamepads().next().is_some()
}

fn start(state: &Rc<LoopState>) {
    // Still running, or stopped but its last frame is pending (that tick resumes the loop).
    if state.running.replace(true) || state.frame_id.get().is_some() {
        return;
    }
    let weak = Rc::downgrade(state);
    state.frame.borrow_mut().get_or_insert_with(|| {
        FrameCallback::new(move |now_ms: f64| {
            if let Some(state) = weak.upgrade() {
                tick(&state, now_ms);
            }
        })
    });
    request_frame(state);
}

fn stop(state: &LoopState) {
    state.running.set(false);
    state
        .repeaters
        .borrow_mut()
        .iter_mut()
        .for_each(GamepadRepeater::reset);
}

fn request_frame(state: &LoopState) {
    if let (Some(window), Some(cb)) = (web_sys::window(), state.frame.borrow().as_ref()) {
        state.frame_id.set(
            window
                .request_animation_frame(cb.as_ref().unchecked_ref())
                .ok(),
        );
    }
}

fn tick(state: &LoopState, now_ms: f64) {
    state.frame_id.set(None);
    if !state.running.get() {
        return;
    }
    let Some(controller) = state.controller.upgrade() else {
        stop(state);
        return;
    };
    let now_us = (now_ms * 1000.0) as u64;
    let mut inputs = Vec::new();
    {
        let mut repeaters = state.repeaters.borrow_mut();
        for pad in gamepads().filter(|p| p.mapping() == web_sys::GamepadMappingType::Standard) {
            let slot = pad.index() as usize;
            if repeaters.len() <= slot {
                repeaters.resize_with(slot + 1, GamepadRepeater::new);
            }
            repeaters[slot].update(frame_of(&pad), now_us, |input| inputs.push(input));
        }
    }
    let hint = crate::dom::active_node();
    for input in inputs {
        let _ = controller.dispatch(input, hint, "");
    }
    request_frame(state);
}

fn frame_of(pad: &web_sys::Gamepad) -> GamepadFrame {
    let pressed = pad
        .buttons()
        .iter()
        .take(POLLED_BUTTONS as usize)
        .enumerate()
        .filter(|(_, b)| {
            b.dyn_ref::<web_sys::GamepadButton>()
                .is_some_and(|b| b.pressed())
        })
        .fold(0u32, |mask, (i, _)| mask | 1 << i);
    let axes = pad.axes();
    let axis = |i: u32| axes.get(i).as_f64().unwrap_or(0.0) as f32;
    GamepadFrame {
        pressed,
        axis_x: axis(0),
        axis_y: axis(1),
    }
}
