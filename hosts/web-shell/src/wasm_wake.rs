//! Coalesced async frame wakeup (`queueMicrotask` + `setTimeout(0)` macrotask yield on pump cap)
//! and `ResizeObserver` listener for `hosts/web-shell` (`wasm32`).

use std::rc::Rc;
#[cfg(feature = "app-issues")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "app-issues")]
use std::task::{Wake, Waker};

#[cfg(feature = "app-issues")]
use nve_abi::SurfaceId;
use nve_engine::Viewport;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[cfg(feature = "app-issues")]
pub(crate) use crate::wake_sched::{PumpNext, ScheduleKind, WakeScheduler};

#[cfg(feature = "app-issues")]
#[wasm_bindgen(inline_js = "export function nve_queue_microtask(cb) { queueMicrotask(cb); }
export function nve_schedule_macrotask(cb) { setTimeout(cb, 0); }")]
extern "C" {
    fn nve_queue_microtask(cb: &JsValue);
    fn nve_schedule_macrotask(cb: &JsValue);
}

#[cfg(feature = "app-issues")]
struct WebFrameWaker {
    surface_id: SurfaceId,
    generation: u64,
    sched: Mutex<WakeScheduler>,
}

#[cfg(feature = "app-issues")]
impl WebFrameWaker {
    #[inline]
    fn clear_mid_pump_wake(&self) {
        self.sched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear_mid_pump_wake();
    }

    fn dispatch(self: &Arc<Self>, item: Option<(ScheduleKind, u64)>) {
        let Some((kind, token)) = item else {
            return;
        };
        let waker_arc = Arc::clone(self);
        let cb = Closure::once_into_js(move || {
            let entered = waker_arc
                .sched
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .on_scheduled_tick_enter(token);
            if !entered {
                return;
            }
            let waker = Waker::from(Arc::clone(&waker_arc));
            let next = crate::wasm_api::pump_surface_async_work(
                waker_arc.surface_id,
                waker_arc.generation,
                &waker,
                || waker_arc.clear_mid_pump_wake(),
            );
            let follow = waker_arc
                .sched
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .on_pump_exit(next);
            waker_arc.dispatch(follow);
        });
        match kind {
            ScheduleKind::Microtask => nve_queue_microtask(&cb),
            ScheduleKind::Macrotask => nve_schedule_macrotask(&cb),
        }
    }

    fn run_direct_pump<R>(
        self: &Arc<Self>,
        waker: &Waker,
        f: impl FnOnce(&Waker) -> (PumpNext, R),
    ) -> Option<R> {
        if !self
            .sched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .on_direct_pump_enter()
        {
            return None;
        }
        let (next, res) = f(waker);
        let follow = self
            .sched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .on_pump_exit(next);
        self.dispatch(follow);
        Some(res)
    }
}

#[cfg(feature = "app-issues")]
impl Wake for WebFrameWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let item = self.sched.lock().unwrap_or_else(|e| e.into_inner()).on_wake();
        self.dispatch(item);
    }
}

/// Owned lifecycle guard keeping a surface's async `Waker` active until unmount.
#[cfg(feature = "app-issues")]
pub(crate) struct WebAsyncWakeGuard {
    inner: Arc<WebFrameWaker>,
    waker: Waker,
}

#[cfg(feature = "app-issues")]
impl WebAsyncWakeGuard {
    pub(crate) fn new(surface_id: SurfaceId, generation: u64) -> Self {
        let inner = Arc::new(WebFrameWaker {
            surface_id,
            generation,
            sched: Mutex::new(WakeScheduler::new()),
        });
        let waker = Waker::from(Arc::clone(&inner));
        Self { inner, waker }
    }

    /// Runs Frame-0 render, pre-hydration event replay, and initial async pump inside a single
    /// direct-pump window so mount-time wakes coalesce cleanly without spawning a redundant tick.
    pub(crate) fn run_boot_pump(
        &self,
        guard: &mut crate::session::WebAppSession,
    ) -> Result<bool, JsValue> {
        self.inner
            .run_direct_pump(&self.waker, |waker| {
                let replayed = match crate::wasm_api::run_initial_frame_and_events(guard) {
                    Ok(v) => v,
                    Err(err) => return (PumpNext::Done, Err(err)),
                };
                match guard.pump_async_work_with(waker, || self.inner.clear_mid_pump_wake()) {
                    Ok(out) => (
                        PumpNext::from_hit_cap(out.hit_cap),
                        Ok(replayed || out.timing.is_some()),
                    ),
                    Err(e) => (
                        PumpNext::Done,
                        Err(JsValue::from_str(&format!("initial async pump failed: {e}"))),
                    ),
                }
            })
            .ok_or_else(|| JsValue::from_str("async wake scheduler unavailable at boot"))?
    }

    #[inline]
    pub(crate) fn handle(&self) -> AsyncWakeHandle {
        AsyncWakeHandle {
            inner: Arc::clone(&self.inner),
            waker: self.waker.clone(),
        }
    }
}

#[cfg(feature = "app-issues")]
#[derive(Clone)]
pub(crate) struct AsyncWakeHandle {
    inner: Arc<WebFrameWaker>,
    waker: Waker,
}

#[cfg(feature = "app-issues")]
impl AsyncWakeHandle {
    /// Pumps DOM events and any spawned async work inside one direct-pump window so task-spawning
    /// clicks never allocate a wasted microtask closure before synchronous draining.
    pub(crate) fn run_event_pump(&self, guard: &mut crate::session::WebAppSession) -> bool {
        self.inner
            .run_direct_pump(&self.waker, |waker| {
                let ev = match guard.pump_events() {
                    Ok(f) => f.is_some(),
                    Err(err) => {
                        web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] {err}")));
                        return (PumpNext::Done, false);
                    }
                };
                match guard.pump_async_work_with(waker, || self.inner.clear_mid_pump_wake()) {
                    Ok(out) => (
                        PumpNext::from_hit_cap(out.hit_cap),
                        ev || out.timing.is_some(),
                    ),
                    Err(err) => {
                        web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] {err}")));
                        (PumpNext::Done, ev)
                    }
                }
            })
            .unwrap_or_else(|| match guard.pump_events() {
                Ok(f) => f.is_some(),
                Err(err) => {
                    web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] {err}")));
                    false
                }
            })
    }
}

#[cfg(feature = "app-issues")]
impl Drop for WebAsyncWakeGuard {
    fn drop(&mut self) {
        self.inner
            .sched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shutdown();
    }
}

pub(crate) struct WindowResizeListener {
    on_resize: Option<Closure<dyn FnMut(web_sys::Event)>>,
    resize_observer: Option<web_sys::ResizeObserver>,
    _on_observe: Option<Closure<dyn FnMut(JsValue)>>,
}

pub(crate) fn install_resize_listener<F>(
    window: &web_sys::Window,
    container: web_sys::Element,
    on_viewport: F,
) -> Option<WindowResizeListener>
where
    F: Fn(Viewport) + 'static,
{
    let callback = Rc::new(on_viewport);
    let cb_win = Rc::clone(&callback);
    let container_win = container.clone();
    let on_resize = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
        cb_win(browser_viewport(&container_win));
    });
    let registered_win_resize = window
        .add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref())
        .is_ok()
        .then_some(on_resize);

    let cb_obs = callback;
    let container_obs = container.clone();
    let on_observe = Closure::<dyn FnMut(JsValue)>::new(move |_entries: JsValue| {
        cb_obs(browser_viewport(&container_obs));
    });
    let (resize_observer, registered_observe) =
        match web_sys::ResizeObserver::new(on_observe.as_ref().unchecked_ref()) {
            Ok(obs) => {
                obs.observe(&container);
                (Some(obs), Some(on_observe))
            }
            Err(_) => (None, None),
        };

    if registered_win_resize.is_some() || resize_observer.is_some() {
        Some(WindowResizeListener {
            on_resize: registered_win_resize,
            resize_observer,
            _on_observe: registered_observe,
        })
    } else {
        None
    }
}

impl Drop for WindowResizeListener {
    fn drop(&mut self) {
        if let Some(on_resize) = self.on_resize.take()
            && let Some(window) = web_sys::window()
        {
            let _ = window.remove_event_listener_with_callback(
                "resize",
                on_resize.as_ref().unchecked_ref(),
            );
        }
        if let Some(observer) = self.resize_observer.take() {
            observer.disconnect();
        }
    }
}

pub(crate) fn browser_viewport(container: &web_sys::Element) -> Viewport {
    let window = web_sys::window();
    let scale = window
        .as_ref()
        .map(|w| w.device_pixel_ratio() as f32)
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(1.0);
    let rect = container.get_bounding_client_rect();
    let win_w = window
        .as_ref()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0);
    let rect_w = (rect.width().is_finite() && rect.width() > 0.0).then_some(rect.width());
    let width = match (win_w, rect_w) {
        (Some(w), Some(r)) => w.min(r) as f32,
        (Some(w), None) => w as f32,
        (None, Some(r)) => r as f32,
        (None, None) => 1280.0,
    };
    let win_h = window
        .as_ref()
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0);
    let rect_h = (rect.height().is_finite() && rect.height() > 0.0).then_some(rect.height());
    let height = match (rect_h, win_h) {
        (Some(r), _) => r as f32,
        (None, Some(h)) => h as f32,
        (None, None) => 720.0,
    };
    Viewport { width, height, scale_factor: scale }
}
