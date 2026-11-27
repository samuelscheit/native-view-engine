//! Coalesced async frame wakeup (`queueMicrotask` + `setTimeout(0)` macrotask yield on pump cap)
//! and `ResizeObserver` listener for `hosts/web-shell` (`wasm32`).

use std::rc::Rc;
#[cfg(feature = "app-issues")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "app-issues")]
use std::sync::Arc;
#[cfg(feature = "app-issues")]
use std::task::{Wake, Waker};

#[cfg(feature = "app-issues")]
use nve_abi::SurfaceId;
use nve_engine::Viewport;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[cfg(feature = "app-issues")]
#[wasm_bindgen(inline_js = "export function nve_queue_microtask(cb) { queueMicrotask(cb); }
export function nve_schedule_macrotask(cb) { setTimeout(cb, 0); }")]
extern "C" {
    fn nve_queue_microtask(cb: &JsValue);
    fn nve_schedule_macrotask(cb: &JsValue);
}

#[cfg(feature = "app-issues")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PumpNext {
    Done,
    Microtask,
    Macrotask,
}

#[cfg(feature = "app-issues")]
struct WebFrameWaker {
    surface_id: SurfaceId,
    generation: u64,
    scheduled: AtomicBool,
    alive: AtomicBool,
}

#[cfg(feature = "app-issues")]
impl WebFrameWaker {
    fn schedule(self: &Arc<Self>, macrotask: bool) {
        if !self.alive.load(Ordering::Acquire) {
            return;
        }
        if self.scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let waker_arc = Arc::clone(self);
        let cb = Closure::once_into_js(move || {
            waker_arc.scheduled.store(false, Ordering::Release);
            if !waker_arc.alive.load(Ordering::Acquire) {
                return;
            }
            let waker = Waker::from(Arc::clone(&waker_arc));
            match crate::wasm_api::pump_surface_async_work(
                waker_arc.surface_id,
                waker_arc.generation,
                &waker,
            ) {
                PumpNext::Done => {}
                PumpNext::Microtask => waker_arc.schedule(false),
                PumpNext::Macrotask => waker_arc.schedule(true),
            }
        });
        if macrotask {
            nve_schedule_macrotask(&cb);
        } else {
            nve_queue_microtask(&cb);
        }
    }
}

#[cfg(feature = "app-issues")]
impl Wake for WebFrameWaker {
    fn wake(self: Arc<Self>) {
        self.schedule(false);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.schedule(false);
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
            scheduled: AtomicBool::new(false),
            alive: AtomicBool::new(true),
        });
        let waker = Waker::from(Arc::clone(&inner));
        Self { inner, waker }
    }

    #[inline]
    pub(crate) fn waker(&self) -> &Waker {
        &self.waker
    }

    #[inline]
    pub(crate) fn schedule_macrotask(&self) {
        self.inner.schedule(true);
    }

    #[inline]
    pub(crate) fn macrotask_handle(&self) -> MacrotaskHandle {
        MacrotaskHandle(Arc::clone(&self.inner))
    }
}

#[cfg(feature = "app-issues")]
#[derive(Clone)]
pub(crate) struct MacrotaskHandle(Arc<WebFrameWaker>);

#[cfg(feature = "app-issues")]
impl MacrotaskHandle {
    #[inline]
    pub(crate) fn schedule_macrotask(&self) {
        self.0.schedule(true);
    }
}

#[cfg(feature = "app-issues")]
impl Drop for WebAsyncWakeGuard {
    fn drop(&mut self) {
        self.inner.alive.store(false, Ordering::Release);
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
