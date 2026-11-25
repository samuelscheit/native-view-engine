//! Browser `wasm32` bindings: `WebAppController`, `HotReloadSocket`, DOM bootstrap,
//! and zero-copy `nve_web_scene_view_ptr()` linear-memory export.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_engine::Viewport;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::session::{HotReloadAction, WebAppSession, PREINIT_SURFACE_ID};

struct HotReloadSocket {
    socket: web_sys::WebSocket,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl Drop for HotReloadSocket {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
    }
}

struct WindowResizeListener {
    on_resize: Option<Closure<dyn FnMut(web_sys::Event)>>,
    resize_observer: Option<web_sys::ResizeObserver>,
    _on_observe: Option<Closure<dyn FnMut(JsValue)>>,
}

impl Drop for WindowResizeListener {
    fn drop(&mut self) {
        if let Some(on_resize) = self.on_resize.take() {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "resize",
                    on_resize.as_ref().unchecked_ref(),
                );
            }
        }
        if let Some(observer) = self.resize_observer.take() {
            observer.disconnect();
        }
    }
}

struct ActiveShell {
    generation: u64,
    session: Rc<RefCell<WebAppSession>>,
    _hot_reload: Option<HotReloadSocket>,
    _resize_listener: Option<WindowResizeListener>,
}

thread_local! {
    static ACTIVE_SHELL: RefCell<Option<ActiveShell>> = const { RefCell::new(None) };
    static NEXT_SHELL_GENERATION: Cell<u64> = const { Cell::new(1) };
}

/// Returns the byte offset of the active shell's `NveSceneView` header inside WASM linear memory
/// (or `0` when no surface is mounted), ready for `NveSceneViewReader.fromWasm32Pointer(...)`.
#[wasm_bindgen]
pub fn nve_web_scene_view_ptr() -> usize {
    ACTIVE_SHELL.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|active| active.session.borrow().scene_view_ptr() as usize)
            .unwrap_or(0)
    })
}

/// Handle to a mounted NVE Web WASM application.
///
/// The active surface and hot-reload socket are owned by the shell slot so callers can safely
/// ignore this handle without unmounting the app; call `unmount()` to detach explicitly.
#[wasm_bindgen]
pub struct WebAppController {
    app_name: &'static str,
    mount_id: String,
    generation: u64,
    session: Rc<RefCell<WebAppSession>>,
}

#[wasm_bindgen]
impl WebAppController {
    /// Name of the mounted NVE application (e.g. `"StudioWorkspaceApp"` or `"CounterApp"`).
    #[wasm_bindgen(getter)]
    pub fn app_name(&self) -> String {
        self.app_name.to_string()
    }

    /// DOM element ID where the surface is mounted.
    #[wasm_bindgen(getter)]
    pub fn mount_id(&self) -> String {
        self.mount_id.clone()
    }

    /// Number of nodes currently tracked in the WebBackend DOM mirror.
    pub fn node_count(&self) -> usize {
        self.session.borrow().backend().node_count()
    }

    /// Serializes the current DOM mirror tree to HTML for inspection or diagnostics.
    pub fn render_html(&self) -> String {
        self.session.borrow().backend().render_html()
    }

    /// Byte offset of this app's `NveSceneView` header inside WASM linear memory for
    /// `NveSceneViewReader.fromWasm32Pointer(memory.buffer, ptr)`.
    pub fn scene_view_ptr(&self) -> usize {
        self.session.borrow().scene_view_ptr() as usize
    }

    /// Updates the mounted surface's viewport and dispatches `NativeEventKind::Layout` on
    /// `ROOT_NODE_ID`, returning `true` when the app re-rendered a frame.
    pub fn dispatch_viewport_resize(&self, width: f32, height: f32) -> Result<bool, JsValue> {
        let scale_factor = web_sys::window()
            .map(|w| w.device_pixel_ratio() as f32)
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(1.0);
        self.session
            .borrow_mut()
            .resize_viewport(Viewport {
                width,
                height,
                scale_factor,
            })
            .map(|timing| timing.is_some())
            .map_err(|e| JsValue::from_str(&format!("viewport resize failed: {e}")))
    }

    /// Dispatches a synthetic `keydown` custom event (`Cmd+K`/`Ctrl+K`, `ArrowUp`, `ArrowDown`,
    /// `Enter`, `Escape`) on `ROOT_NODE_ID`, returning `true` when the app re-rendered a frame.
    pub fn dispatch_keydown(&self, key: &str, meta: bool, ctrl: bool) -> Result<bool, JsValue> {
        self.session
            .borrow_mut()
            .dispatch_keydown(key, meta, ctrl)
            .map(|timing| timing.is_some())
            .map_err(|e| JsValue::from_str(&format!("keydown dispatch failed: {e}")))
    }

    /// Detaches this app if it is still active, clearing its mount root and closing any WS.
    pub fn unmount(&self) {
        let removed = ACTIVE_SHELL.with(|slot| {
            let mut guard = slot.borrow_mut();
            if guard
                .as_ref()
                .is_some_and(|active| active.generation == self.generation)
            {
                guard.take().is_some()
            } else {
                false
            }
        });
        if removed {
            if let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id(&self.mount_id))
            {
                el.set_text_content(None);
            }
            set_hud_status("offline", "Unmounted");
        }
    }
}

fn browser_now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

fn browser_viewport(container: &web_sys::Element) -> Viewport {
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
    Viewport {
        width,
        height,
        scale_factor: scale,
    }
}

fn set_hud_status(state: &str, text: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("nve-status"))
    {
        el.set_text_content(Some(text));
        let _ = el.set_attribute("data-state", state);
    }
}

fn connect_hot_reload(
    ws_url: &str,
    session: Rc<RefCell<WebAppSession>>,
) -> Result<HotReloadSocket, JsValue> {
    let socket = web_sys::WebSocket::new(ws_url)?;
    set_hud_status("connecting", &format!("Hot reload: connecting ({ws_url})"));

    let url_open = ws_url.to_string();
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[NVE Web] Connected to hot-reload server: {url_open}"
        )));
        set_hud_status("connected", "Hot reload: connected");
    });

    let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
        move |ev: web_sys::MessageEvent| {
            let Some(raw) = ev.data().as_string() else {
                return;
            };
            let t0 = browser_now_ms();
            let result = session.borrow_mut().handle_hot_patch_json(&raw);
            let elapsed_ms = (browser_now_ms() - t0).max(0.0);

            match result {
                Ok(HotReloadAction::TemplatePatched {
                    generation,
                    template_id,
                    mutations,
                }) => {
                    let msg = format!(
                        "[NVE HotReload] Tier-1 TemplatePatch gen={generation} ({template_id}, {mutations} mutations) applied in {elapsed_ms:.2}ms"
                    );
                    web_sys::console::log_1(&JsValue::from_str(&msg));
                    set_hud_status(
                        "patched",
                        &format!("Patched gen {generation} ({elapsed_ms:.1}ms)"),
                    );
                }
                Ok(HotReloadAction::ReloadPage {
                    generation,
                    dylib_path,
                    build_ms,
                }) => {
                    let msg = format!(
                        "[NVE HotReload] Tier-2 DylibReload gen={generation} ({build_ms}ms, {dylib_path}) — reloading page"
                    );
                    web_sys::console::log_1(&JsValue::from_str(&msg));
                    set_hud_status(
                        "reloading",
                        &format!("Reloading gen {generation} ({build_ms}ms)..."),
                    );
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().reload();
                    }
                }
                Ok(HotReloadAction::DevStatus {
                    message,
                    latency_ms,
                }) => {
                    web_sys::console::log_1(&JsValue::from_str(&format!(
                        "[NVE DevStatus] {message} ({latency_ms}ms)"
                    )));
                    set_hud_status("connected", &format!("{message} ({latency_ms}ms)"));
                }
                Err(err) => {
                    web_sys::console::warn_1(&JsValue::from_str(&format!(
                        "[NVE HotReload] Ignored message: {err}"
                    )));
                }
            }
        },
    );

    let url_err = ws_url.to_string();
    let on_error = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |_| {
        web_sys::console::warn_1(&JsValue::from_str(&format!(
            "[NVE Web] Hot-reload WebSocket unavailable at {url_err}"
        )));
        set_hud_status("offline", "Hot reload: offline");
    });

    let on_close = Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_| {
        web_sys::console::log_1(&JsValue::from_str("[NVE Web] Hot-reload WebSocket closed"));
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

fn install_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web Panic] {info}")));
        }));
    });
}

fn start_surface_app<F>(
    app_name: &'static str,
    mount_id: &str,
    ws_url: Option<String>,
    build_session: F,
) -> Result<WebAppController, JsValue>
where
    F: FnOnce(SurfaceId, WebBackend, Viewport) -> WebAppSession,
{
    install_panic_hook();
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("missing global window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("missing window.document"))?;
    let container = document
        .get_element_by_id(mount_id)
        .ok_or_else(|| JsValue::from_str(&format!("root element '#{mount_id}' not found")))?;

    let viewport = browser_viewport(&container);
    let container_for_resize = container.clone();
    let backend = WebBackend::mount_into(container);
    let surface_id: SurfaceId = PREINIT_SURFACE_ID;
    let session_val = build_session(surface_id, backend, viewport);
    let used_preinit = session_val.has_preinit_frame0();
    let session = Rc::new(RefCell::new(session_val));

    // Wire live DOM events -> app dispatch -> incremental CommandBuffer -> live DOM patch.
    let session_for_events = Rc::clone(&session);
    session
        .borrow_mut()
        .backend_mut()
        .set_render_callback(move || {
            let Ok(mut guard) = session_for_events.try_borrow_mut() else {
                return;
            };
            if let Err(err) = guard.pump_events() {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "[NVE Web] Failed to render incremental frame: {err}"
                )));
            }
        });

    let initial_timing = session
        .borrow_mut()
        .render_initial()
        .map_err(|e| JsValue::from_str(&format!("initial frame failed: {e}")))?;

    web_sys::console::log_1(&JsValue::from_str(&format!(
        "[NVE Web] Mounted {app_name} into '#{mount_id}' (preinit={used_preinit}, {} ops, {} mutations, dec={}us mount={}us)",
        initial_timing.op_count,
        initial_timing.mutation_count,
        initial_timing.decode_us,
        initial_timing.mount_us
    )));

    // Forward browser `window` resize events AND container `ResizeObserver` notifications into
    // `WebBackend` + `ROOT_NODE_ID` Layout events so responsive layouts (e.g. Studio Workspace
    // 620pt breakpoint) adapt live across desktop resizes, split panes, and mobile emulation.
    let session_for_resize = Rc::clone(&session);
    let container_for_win_resize = container_for_resize.clone();
    let on_resize = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
        let next_vp = browser_viewport(&container_for_win_resize);
        let Ok(mut guard) = session_for_resize.try_borrow_mut() else {
            return;
        };
        if let Err(err) = guard.resize_viewport(next_vp) {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "[NVE Web] Failed to handle viewport resize: {err}"
            )));
        }
    });
    let registered_win_resize = window
        .add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref())
        .is_ok()
        .then_some(on_resize);

    let session_for_observer = Rc::clone(&session);
    let container_for_observer = container_for_resize.clone();
    let on_observe = Closure::<dyn FnMut(JsValue)>::new(move |_entries: JsValue| {
        let next_vp = browser_viewport(&container_for_observer);
        let Ok(mut guard) = session_for_observer.try_borrow_mut() else {
            return;
        };
        if let Err(err) = guard.resize_viewport(next_vp) {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "[NVE Web] Failed to handle container resize: {err}"
            )));
        }
    });
    let (resize_observer, registered_observe) =
        match web_sys::ResizeObserver::new(on_observe.as_ref().unchecked_ref()) {
            Ok(obs) => {
                obs.observe(&container_for_resize);
                (Some(obs), Some(on_observe))
            }
            Err(_) => (None, None),
        };

    let resize_listener = if registered_win_resize.is_some() || resize_observer.is_some() {
        Some(WindowResizeListener {
            on_resize: registered_win_resize,
            resize_observer,
            _on_observe: registered_observe,
        })
    } else {
        None
    };

    let hot_reload = match ws_url.filter(|u| !u.trim().is_empty()) {
        Some(url) => match connect_hot_reload(&url, Rc::clone(&session)) {
            Ok(sock) => Some(sock),
            Err(err) => {
                web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "[NVE Web] Could not open hot-reload WebSocket '{url}': {err:?}"
                )));
                set_hud_status("offline", "Hot reload: offline");
                None
            }
        },
        None => {
            set_hud_status("disabled", "Hot reload: disabled");
            None
        }
    };

    let generation = NEXT_SHELL_GENERATION.with(|counter| {
        let next = counter.get();
        counter.set(next.wrapping_add(1));
        next
    });

    // Replace any previously mounted shell cleanly (dropping prior WS & closures).
    ACTIVE_SHELL.with(|slot| {
        *slot.borrow_mut() = Some(ActiveShell {
            generation,
            session: Rc::clone(&session),
            _hot_reload: hot_reload,
            _resize_listener: resize_listener,
        });
    });

    Ok(WebAppController {
        app_name,
        mount_id: mount_id.to_string(),
        generation,
        session,
    })
}

/// Mounts the flagship Aether Workspace + NVE Studio app into `#<mount_id>` and optionally
/// connects to the `nve dev` WebSocket.
#[wasm_bindgen]
pub fn start_studio_workspace_app(
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_surface_app(
        "StudioWorkspaceApp",
        mount_id,
        ws_url,
        WebAppSession::new_studio_workspace,
    )
}

/// Mounts the NVE `CounterApp` into `#<mount_id>` and optionally connects to `nve dev` WebSocket.
#[wasm_bindgen]
pub fn start_counter_app(
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_surface_app("CounterApp", mount_id, ws_url, WebAppSession::new_counter)
}
