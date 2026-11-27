//! Opt-in W3C Gamepad poller: a `requestAnimationFrame` loop that runs only while a
//! standard-mapping gamepad is connected (zero idle CPU), feeding [`nve_tv::GamepadRepeater`]
//! edges into the controller. Zero Rust heap traffic per frame once warm.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nve_tv::{GamepadFrame, GamepadRepeater, RemoteInput};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::Inner;
use crate::rules::{MAX_GAMEPADS, absent_pad_slots};

type FrameCallback = Closure<dyn FnMut(f64)>;
type EventCallback = Closure<dyn FnMut(web_sys::Event)>;

/// Standard-mapping buttons read per frame.
const POLLED_BUTTONS: u32 = 16;

struct LoopState {
    controller: Weak<Inner>,
    running: Cell<bool>,
    /// `true` while [`tick`] runs (the rAF closure must not be freed under its own feet).
    in_tick: Cell<bool>,
    /// Pending `requestAnimationFrame` id (cancelled on drop).
    frame_id: Cell<Option<i32>>,
    repeaters: RefCell<Vec<GamepadRepeater>>,
    /// Reused per-frame input buffer.
    inputs: RefCell<Vec<RemoteInput>>,
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
            in_tick: Cell::new(false),
            frame_id: Cell::new(None),
            repeaters: RefCell::new(Vec::new()),
            inputs: RefCell::new(Vec::new()),
            frame: RefCell::new(None),
        });
        let on_connect = Rc::clone(&state);
        let connected = EventCallback::new(move |_: web_sys::Event| {
            if any_connected() {
                start(&on_connect);
            }
        });
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
        let frame = self.state.frame.borrow_mut().take();
        if let Some(frame) = frame
            && self.state.in_tick.get()
        {
            // Dropped from inside this very callback (e.g. an action handler disabled the
            // gamepad): hand the running closure to the JS GC instead of freeing it.
            let _ = frame.into_js_value();
        }
    }
}

/// Every connected standard-mapping pad with an index below [`MAX_GAMEPADS`], without
/// collecting `navigator.getGamepads()` into a Rust `Vec`.
fn for_each_standard_pad(mut f: impl FnMut(&web_sys::Gamepad)) {
    let Some(pads) = web_sys::window().and_then(|w| w.navigator().get_gamepads().ok()) else {
        return;
    };
    for i in 0..pads.length() {
        if let Ok(pad) = pads.get(i).dyn_into::<web_sys::Gamepad>()
            && pad.connected()
            && pad.mapping() == web_sys::GamepadMappingType::Standard
            && (pad.index() as usize) < MAX_GAMEPADS
        {
            f(&pad);
        }
    }
}

/// Only standard-mapping pads keep the loop alive (unmapped HID devices never spin rAF).
fn any_connected() -> bool {
    let mut any = false;
    for_each_standard_pad(|_| any = true);
    any
}

fn document_hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
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
    release_held(state);
}

fn release_held(state: &LoopState) {
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

/// Clears [`LoopState::in_tick`] however the tick ends.
struct TickGuard<'a>(&'a Cell<bool>);

impl Drop for TickGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
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
    state.in_tick.set(true);
    let _in_tick = TickGuard(&state.in_tick);
    if document_hidden() {
        // Background tab: held buttons must not fire a burst of repeats on return.
        release_held(state);
    } else {
        let now_us = (now_ms * 1000.0) as u64;
        let mut inputs = std::mem::take(&mut *state.inputs.borrow_mut());
        poll_into(state, now_us, &mut inputs);
        for input in inputs.drain(..) {
            // DOM focus moves with every input: re-read it so a second input in the same frame
            // never resyncs the engine back to the pre-move node.
            let _ = controller.dispatch(input, crate::dom::active_node(), "");
        }
        *state.inputs.borrow_mut() = inputs;
    }
    drop(controller);
    if state.running.get() {
        request_frame(state);
    }
}

fn poll_into(state: &LoopState, now_us: u64, inputs: &mut Vec<RemoteInput>) {
    let mut repeaters = state.repeaters.borrow_mut();
    let mut present = 0u64;
    for_each_standard_pad(|pad| {
        let slot = pad.index() as usize;
        present |= 1u64 << slot;
        if repeaters.len() <= slot {
            repeaters.resize_with(slot + 1, GamepadRepeater::new);
        }
        repeaters[slot].update(frame_of(pad), now_us, |input| inputs.push(input));
    });
    // A pad that disconnected while others stay connected must not keep held buttons.
    for slot in absent_pad_slots(present, repeaters.len()) {
        repeaters[slot].reset();
    }
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
