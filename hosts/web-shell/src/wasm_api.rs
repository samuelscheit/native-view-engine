//! Browser `wasm32` bindings: multi-surface `ACTIVE_SHELLS`, `WebAppController`,
//! `HotReloadSocket`, DOM bootstrap, and zero-copy `nve_web_scene_view_ptr()` exports.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_engine::Viewport;
#[cfg(feature = "multi-window")]
use nve_interop::WindowHandleId;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::wasm_hydrate::SsrRenderResult;
#[cfg(feature = "dev")]
use crate::session::HotReloadAction;
use crate::session::{WebAppSession, DEFAULT_SURFACE_ID};
#[cfg(all(feature = "dev", feature = "scene-view"))]
use crate::wasm_pair::active_paired_scene_view_ptr;
#[cfg(feature = "multi-window")]
use crate::wasm_popup::{
    popup_window_for_surface, register_mounted_surface_window, unregister_surface_window,
};

#[cfg(any(feature = "dev", feature = "multi-window"))]
#[wasm_bindgen(inline_js = "export function nve_extract_binary_message(data) {
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  return new Uint8Array(0);
}")]
extern "C" {
    pub(crate) fn nve_extract_binary_message(data: &JsValue) -> Vec<u8>;
}

#[cfg(feature = "dev")]
struct HotReloadSocket {
    socket: web_sys::WebSocket,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

#[cfg(feature = "dev")]
impl Drop for HotReloadSocket {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
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

struct ActiveShell {
    generation: u64,
    #[cfg(feature = "multi-window")]
    window_handle: Option<WindowHandleId>,
    /// Owns the session for the page lifetime (only read by the scene-view exports).
    #[cfg_attr(not(feature = "scene-view"), allow(dead_code))]
    session: Rc<RefCell<WebAppSession>>,
    #[cfg(feature = "dev")]
    _hot_reload: Option<HotReloadSocket>,
    _resize_listener: Option<WindowResizeListener>,
}

thread_local! {
    static ACTIVE_SHELLS: RefCell<BTreeMap<SurfaceId, ActiveShell>> = const { RefCell::new(BTreeMap::new()) };
    static PRIMARY_SURFACE_ID: Cell<SurfaceId> = const { Cell::new(DEFAULT_SURFACE_ID) };
    static NEXT_SHELL_GENERATION: Cell<u64> = const { Cell::new(1) };
}

#[cfg(feature = "multi-window")]
pub(crate) fn primary_surface_id_if_mounted() -> Option<SurfaceId> {
    let primary = PRIMARY_SURFACE_ID.with(Cell::get);
    ACTIVE_SHELLS.with(|shells| shells.borrow().contains_key(&primary).then_some(primary))
}

#[cfg(feature = "multi-window")]
pub(crate) fn remove_active_shell(surface_id: SurfaceId) {
    ACTIVE_SHELLS.with(|shells| {
        shells.borrow_mut().remove(&surface_id);
    });
}

#[cfg(feature = "scene-view")]
#[wasm_bindgen]
pub fn nve_web_scene_view_ptr() -> usize {
    #[cfg(feature = "dev")]
    if let Some(ptr) = active_paired_scene_view_ptr() {
        return ptr;
    }
    let primary = PRIMARY_SURFACE_ID.with(Cell::get);
    ACTIVE_SHELLS.with(|shells| {
        let map = shells.borrow();
        map.get(&primary)
            .or_else(|| map.values().next())
            .map(|active| active.session.borrow().scene().view_ptr() as usize)
            .unwrap_or(0)
    })
}

#[cfg(feature = "scene-view")]
#[wasm_bindgen]
pub fn nve_web_scene_view_ptr_for_surface(surface_id: SurfaceId) -> usize {
    ACTIVE_SHELLS.with(|shells| {
        shells
            .borrow()
            .get(&surface_id)
            .map(|active| active.session.borrow().scene().view_ptr() as usize)
            .unwrap_or(0)
    })
}

#[wasm_bindgen]
pub fn active_surface_ids() -> Vec<SurfaceId> {
    ACTIVE_SHELLS.with(|shells| shells.borrow().keys().copied().collect())
}

#[wasm_bindgen]
pub fn active_shell_count() -> usize {
    ACTIVE_SHELLS.with(|shells| shells.borrow().len())
}

#[wasm_bindgen]
pub struct WebAppController {
    app_name: &'static str,
    mount_id: String,
    surface_id: SurfaceId,
    generation: u64,
    session: Rc<RefCell<WebAppSession>>,
}

#[wasm_bindgen]
impl WebAppController {
    #[wasm_bindgen(getter)]
    pub fn app_name(&self) -> String { self.app_name.to_string() }

    #[wasm_bindgen(getter)]
    pub fn mount_id(&self) -> String { self.mount_id.clone() }

    #[wasm_bindgen(getter)]
    pub fn surface_id(&self) -> SurfaceId { self.surface_id }

    pub fn node_count(&self) -> usize { self.session.borrow().backend().node_count() }

    pub fn render_html(&self) -> String { self.session.borrow().backend().render_html() }

    pub fn render_ssr_bundle(&self) -> SsrRenderResult {
        SsrRenderResult::from_output(self.session.borrow().backend().render_ssr())
    }

    pub fn is_hydrated(&self) -> bool {
        self.session.borrow().backend().hydration_report().hydrated
    }

    pub fn hydration_mismatch_count(&self) -> usize {
        self.session.borrow().backend().hydration_report().mismatches.len()
    }

    pub fn hydration_adopted_count(&self) -> usize {
        self.session.borrow().backend().hydration_report().adopted_nodes
    }

    pub fn hydration_replaced_count(&self) -> usize {
        self.session.borrow().backend().hydration_report().replaced_nodes
    }

    pub fn hydration_seeded_rules(&self) -> usize {
        self.session.borrow().backend().hydration_report().seeded_rule_count
    }

    pub fn stylesheet_rule_count(&self) -> usize {
        self.session.borrow().backend().stylesheet().rule_count()
    }

    #[cfg(feature = "scene-view")]
    pub fn scene_view_ptr(&self) -> usize { self.session.borrow().scene().view_ptr() as usize }

    pub fn dispatch_viewport_resize(&self, width: f32, height: f32) -> Result<bool, JsValue> {
        let scale_factor = web_sys::window()
            .map(|w| w.device_pixel_ratio() as f32)
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(1.0);
        self.session
            .borrow_mut()
            .resize_viewport(Viewport { width, height, scale_factor })
            .map(|timing| timing.is_some())
            .map_err(|e| JsValue::from_str(&format!("viewport resize failed: {e}")))
    }

    pub fn dispatch_keydown(&self, key: &str, meta: bool, ctrl: bool) -> Result<bool, JsValue> {
        self.session
            .borrow_mut()
            .dispatch_keydown(key, meta, ctrl)
            .map(|timing| timing.is_some())
            .map_err(|e| JsValue::from_str(&format!("keydown dispatch failed: {e}")))
    }

    pub fn unmount(&self) {
        let removed = ACTIVE_SHELLS.with(|shells| {
            let mut map = shells.borrow_mut();
            if map.get(&self.surface_id).is_some_and(|a| a.generation == self.generation) {
                map.remove(&self.surface_id)
            } else {
                None
            }
        });
        if let Some(_shell) = removed {
            #[cfg(feature = "multi-window")]
            if let Some(handle) = _shell.window_handle {
                unregister_surface_window(handle);
            }
            if let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id(&self.mount_id))
            {
                el.set_text_content(None);
            }
            if active_shell_count() == 0 {
                set_hud_status("offline", "Unmounted");
            }
        }
    }
}

#[cfg(feature = "dev")]
fn browser_now_ms() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now()).unwrap_or(0.0)
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

pub(crate) fn set_hud_status(state: &str, text: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("nve-status"))
    {
        el.set_text_content(Some(text));
        let _ = el.set_attribute("data-state", state);
    }
}

#[cfg(feature = "dev")]
fn connect_hot_reload(
    ws_url: &str,
    session: Rc<RefCell<WebAppSession>>,
) -> Result<HotReloadSocket, JsValue> {
    let socket = web_sys::WebSocket::new(ws_url)?;
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
    set_hud_status("connecting", &format!("Hot reload: connecting ({ws_url})"));

    let url_open = ws_url.to_string();
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        web_sys::console::log_1(&JsValue::from_str(&format!("[NVE Web] Connected: {url_open}")));
        set_hud_status("connected", "Hot reload: connected");
    });

    let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
        move |ev: web_sys::MessageEvent| {
            let t0 = browser_now_ms();
            let bytes = nve_extract_binary_message(&ev.data());
            if bytes.is_empty() {
                return;
            }
            let result = session.borrow_mut().handle_hot_patch_frame(&bytes);
            match result {
                Ok(HotReloadAction::TemplatePatched { generation, .. }) => {
                    let mut ms = String::new();
                    nve_abi::num::write_f64(&mut ms, (browser_now_ms() - t0).max(0.0));
                    set_hud_status("patched", &format!("Patched gen {generation} ({ms}ms)"));
                }
                Ok(HotReloadAction::ReloadPage { generation, build_ms, .. }) => {
                    set_hud_status("reloading", &format!("Reloading gen {generation} ({build_ms}ms)..."));
                    if let Some(w) = web_sys::window() { let _ = w.location().reload(); }
                }
                Ok(HotReloadAction::DevStatus { message, latency_ms }) => {
                    set_hud_status("connected", &format!("{message} ({latency_ms}ms)"));
                }
                Err(err) => {
                    web_sys::console::warn_1(&JsValue::from_str(&format!("[NVE HotReload] {err}")));
                }
            }
        },
    );

    let on_error = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |_| {
        set_hud_status("offline", "Hot reload: offline");
    });
    let on_close = Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_| {
        set_hud_status("offline", "Hot reload: disconnected");
    });

    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    Ok(HotReloadSocket {
        socket,
        _on_open: on_open,
        _on_message: on_message,
        _on_error: on_error,
        _on_close: on_close,
    })
}

/// Runs once when the module is instantiated, in every bundle (studio, counter, dev): a Rust
/// panic prints its message and `file:line` to the console before the `panic = "abort"` trap.
#[wasm_bindgen(start)]
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web Panic] {info}")));
    }));
}

#[cfg_attr(not(feature = "multi-window"), allow(unused_variables))]
fn find_mount_container(
    surface_id: SurfaceId,
    mount_id: &str,
) -> Result<(web_sys::Window, web_sys::Element), JsValue> {
    #[cfg(feature = "multi-window")]
    if let Some(popup_win) = popup_window_for_surface(surface_id)
        && let Some(doc) = popup_win.document()
        && let Some(el) = doc.get_element_by_id(mount_id)
    {
        return Ok((popup_win, el));
    }
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("missing global window"))?;
    let document = window.document().ok_or_else(|| JsValue::from_str("missing window.document"))?;
    let container = document
        .get_element_by_id(mount_id)
        .ok_or_else(|| JsValue::from_str(&format!("root element '#{mount_id}' not found")))?;
    Ok((window, container))
}

pub(crate) fn start_surface_app_inner<F>(
    app_name: &'static str,
    surface_id: SurfaceId,
    mount_id: &str,
    ws_url: Option<String>,
    hydrate: bool,
    build_session: F,
) -> Result<WebAppController, JsValue>
where
    F: FnOnce(SurfaceId, WebBackend, Viewport) -> WebAppSession,
{
    if surface_id == 0 {
        return Err(JsValue::from_str("surface_id must be non-zero"));
    }
    let (window, container) = find_mount_container(surface_id, mount_id)?;
    let viewport = browser_viewport(&container);
    let container_for_resize = container.clone();
    let backend = if hydrate {
        WebBackend::hydrate_into(container)
    } else {
        WebBackend::mount_into(container)
    };
    let session = Rc::new(RefCell::new(build_session(surface_id, backend, viewport)));

    let session_for_events = Rc::clone(&session);
    session.borrow_mut().backend_mut().set_render_callback(move || {
        if let Ok(mut guard) = session_for_events.try_borrow_mut()
            && let Err(err) = guard.pump_events()
        {
            web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] {err}")));
        }
    });

    session
        .borrow_mut()
        .render_initial()
        .map_err(|e| JsValue::from_str(&format!("initial frame failed: {e}")))?;

    let session_for_resize = Rc::clone(&session);
    let resize_listener = install_resize_listener(&window, container_for_resize, move |next_vp| {
        if let Ok(mut guard) = session_for_resize.try_borrow_mut()
            && let Err(err) = guard.resize_viewport(next_vp)
        {
            web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] {err}")));
        }
    });

    #[cfg(feature = "dev")]
    let hot_reload = match ws_url.filter(|u| !u.trim().is_empty()) {
        Some(url) => connect_hot_reload(&url, Rc::clone(&session)).ok(),
        None => {
            if surface_id == DEFAULT_SURFACE_ID {
                set_hud_status("disabled", "Hot reload: disabled");
            }
            None
        }
    };
    #[cfg(not(feature = "dev"))]
    {
        let _ = ws_url;
        if surface_id == DEFAULT_SURFACE_ID {
            set_hud_status("ready", "Live");
        }
    }

    let generation = NEXT_SHELL_GENERATION.with(|c| {
        let next = c.get();
        c.set(next.wrapping_add(1));
        next
    });
    #[cfg(feature = "multi-window")]
    let window_handle =
        register_mounted_surface_window(surface_id, app_name, viewport.width, viewport.height);

    ACTIVE_SHELLS.with(|shells| {
        let mut map = shells.borrow_mut();
        if map.is_empty() || surface_id == DEFAULT_SURFACE_ID {
            PRIMARY_SURFACE_ID.with(|p| p.set(surface_id));
        }
        map.insert(
            surface_id,
            ActiveShell {
                generation,
                #[cfg(feature = "multi-window")]
                window_handle,
                session: Rc::clone(&session),
                #[cfg(feature = "dev")]
                _hot_reload: hot_reload,
                _resize_listener: resize_listener,
            },
        );
    });

    Ok(WebAppController {
        app_name,
        mount_id: mount_id.to_string(),
        surface_id,
        generation,
        session,
    })
}

#[cfg(feature = "app-studio")]
#[wasm_bindgen]
pub fn start_studio_workspace_app(
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_studio_workspace_app_on_surface(DEFAULT_SURFACE_ID, mount_id, ws_url)
}

#[cfg(feature = "app-studio")]
#[wasm_bindgen]
pub fn start_studio_workspace_app_on_surface(
    surface_id: SurfaceId,
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_surface_app_inner("StudioWorkspaceApp", surface_id, mount_id, ws_url, false, WebAppSession::new_studio_workspace)
}

#[cfg(feature = "app-counter")]
#[wasm_bindgen]
pub fn start_counter_app(
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_counter_app_on_surface(DEFAULT_SURFACE_ID, mount_id, ws_url)
}

#[cfg(feature = "app-counter")]
#[wasm_bindgen]
pub fn start_counter_app_on_surface(
    surface_id: SurfaceId,
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_surface_app_inner("CounterApp", surface_id, mount_id, ws_url, false, WebAppSession::new_counter)
}
