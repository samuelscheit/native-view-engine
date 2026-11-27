//! Coalesced `queueMicrotask` async frame wakeup for `hosts/web-shell` (`wasm32`).
//!
//! Bridges `std::task::Waker` from `DioxusSurface::poll_work` back into `WebAppSession` without
//! holding `RefMut<WebAppSession>` across `.await` points and without any `unsafe` waker vtables.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Wake, Waker};

use nve_abi::SurfaceId;
use nve_engine::Viewport;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen(inline_js = "export function nveQueueMicrotask(cb) { queueMicrotask(cb); }")]
extern "C" {
    fn nve_queue_microtask(cb: &JsValue);
}

struct WebFrameWaker {
    surface_id: SurfaceId,
    generation: u64,
    scheduled: AtomicBool,
    alive: AtomicBool,
}

impl WebFrameWaker {
    fn schedule(self: &Arc<Self>) {
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
            let retry = crate::wasm_api::pump_surface_async_work(
                waker_arc.surface_id,
                waker_arc.generation,
                &waker,
            );
            if retry {
                waker_arc.schedule();
            }
        });
        nve_queue_microtask(&cb);
    }
}

impl Wake for WebFrameWaker {
    fn wake(self: Arc<Self>) {
        self.schedule();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.schedule();
    }
}

/// Owned lifecycle guard keeping a surface's async `Waker` active until unmount.
pub(crate) struct WebAsyncWakeGuard {
    inner: Arc<WebFrameWaker>,
    waker: Waker,
}

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
}

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
        cb_win(crate::wasm_api::browser_viewport(&container_win));
    });
    let registered_win_resize = window
        .add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref())
        .is_ok()
        .then_some(on_resize);

    let cb_obs = callback;
    let container_obs = container.clone();
    let on_observe = Closure::<dyn FnMut(JsValue)>::new(move |_entries: JsValue| {
        cb_obs(crate::wasm_api::browser_viewport(&container_obs));
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
