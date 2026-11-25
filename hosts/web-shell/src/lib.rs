//! Web WASM shell mounting NVE apps onto browser DOM with live interactive events
//! and 2-tier WebSocket hot-reload (`HotPatchMsg::TemplatePatch` + `DylibReload` + `DevStatus`).

use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_dioxus::DioxusSurface;
use nve_engine::{BackendError, FrameTiming, HotPatchMsg, RenderBackend, SurfaceRunner, Viewport};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

/// Outcome of handling one `nve dev` WebSocket `HotPatchMsg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotReloadAction {
    /// Tier-1 template/style patch applied live without reloading the page.
    TemplatePatched {
        generation: u64,
        template_id: String,
        mutations: u32,
    },
    /// Tier-2 dylib/WASM rebuild finished; browser shell should log and reload cleanly.
    ReloadPage {
        generation: u64,
        dylib_path: String,
        build_ms: u64,
    },
    /// Status heartbeat from `nve dev`.
    DevStatus { message: String, latency_ms: u64 },
}

/// Cross-target session binding a `DioxusSurface` to a `SurfaceRunner<WebBackend>`.
///
/// Runs identically in browser WASM (`web-dom-wasm`) and in host unit tests (`web-dom-mirror`).
pub struct WebAppSession {
    surface: DioxusSurface,
    runner: SurfaceRunner<WebBackend>,
}

impl WebAppSession {
    pub fn new(
        surface_id: SurfaceId,
        surface: DioxusSurface,
        backend: WebBackend,
        viewport: Viewport,
    ) -> Self {
        Self {
            surface,
            runner: SurfaceRunner::new(surface_id, viewport, backend),
        }
    }

    /// Boots the `nve-example-counter` Dioxus surface on `WebBackend`.
    pub fn new_counter(surface_id: SurfaceId, backend: WebBackend, viewport: Viewport) -> Self {
        Self::new(
            surface_id,
            nve_example_counter::create_surface(surface_id),
            backend,
            viewport,
        )
    }

    pub fn backend(&self) -> &WebBackend {
        self.runner.backend()
    }

    pub fn backend_mut(&mut self) -> &mut WebBackend {
        self.runner.backend_mut()
    }

    /// Renders and mounts the initial frame.
    pub fn render_initial(&mut self) -> Result<FrameTiming, BackendError> {
        self.render_and_submit()
    }

    /// Drains pending DOM events, dispatches them to Dioxus, and commits an incremental frame if handled.
    pub fn pump_events(&mut self) -> Result<Option<FrameTiming>, BackendError> {
        let events = self.runner.backend_mut().drain_events();
        if events.is_empty() {
            return Ok(None);
        }
        if !self.surface.dispatch_events(&events) {
            return Ok(None);
        }
        self.render_and_submit().map(Some)
    }

    /// Parses and applies one JSON-serialized `HotPatchMsg` from `nve dev`.
    pub fn handle_hot_patch_json(&mut self, raw: &str) -> Result<HotReloadAction, String> {
        let msg = HotPatchMsg::from_json(raw).map_err(|e| format!("invalid HotPatchMsg: {e}"))?;
        self.handle_hot_patch(msg)
    }

    /// Applies one decoded `HotPatchMsg` from `nve dev`.
    pub fn handle_hot_patch(&mut self, msg: HotPatchMsg) -> Result<HotReloadAction, String> {
        match msg {
            HotPatchMsg::TemplatePatch {
                generation,
                template_id,
                attr_updates,
                text_updates,
            } => {
                let patch_msg = HotPatchMsg::TemplatePatch {
                    generation,
                    template_id: template_id.clone(),
                    attr_updates: attr_updates.clone(),
                    text_updates: text_updates.clone(),
                };
                let _ = self.surface.apply_hot_patch(&patch_msg);
                let mutations = self
                    .runner
                    .backend_mut()
                    .apply_template_patch(&attr_updates, &text_updates);
                Ok(HotReloadAction::TemplatePatched {
                    generation,
                    template_id,
                    mutations,
                })
            }
            HotPatchMsg::DylibReload {
                generation,
                dylib_path,
                build_ms,
            } => Ok(HotReloadAction::ReloadPage {
                generation,
                dylib_path,
                build_ms,
            }),
            HotPatchMsg::DevStatus {
                message,
                latency_ms,
            } => Ok(HotReloadAction::DevStatus {
                message,
                latency_ms,
            }),
        }
    }

    fn render_and_submit(&mut self) -> Result<FrameTiming, BackendError> {
        let frame_id = self.runner.next_frame_id();
        let (reconcile_us, encode_us, bytes) = self.surface.render_frame(frame_id);
        self.runner
            .submit_recorded_frame(frame_id, reconcile_us, encode_us, &bytes)
    }
}

#[cfg(target_arch = "wasm32")]
struct HotReloadSocket {
    socket: web_sys::WebSocket,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for HotReloadSocket {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
    }
}

#[cfg(target_arch = "wasm32")]
struct ActiveShell {
    _session: Rc<RefCell<WebAppSession>>,
    _hot_reload: Option<HotReloadSocket>,
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static ACTIVE_SHELL: RefCell<Option<ActiveShell>> = const { RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
fn browser_now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

#[cfg(target_arch = "wasm32")]
fn browser_viewport(container: &web_sys::Element) -> Viewport {
    let window = web_sys::window();
    let scale = window
        .as_ref()
        .map(|w| w.device_pixel_ratio() as f32)
        .unwrap_or(1.0);
    let rect = container.get_bounding_client_rect();
    let width = if rect.width() > 0.0 {
        rect.width() as f32
    } else {
        window
            .as_ref()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(1280.0) as f32
    };
    let height = if rect.height() > 0.0 {
        rect.height() as f32
    } else {
        window
            .as_ref()
            .and_then(|w| w.inner_height().ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(720.0) as f32
    };
    Viewport {
        width,
        height,
        scale_factor: scale,
    }
}

#[cfg(target_arch = "wasm32")]
fn set_hud_status(state: &str, text: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("nve-status"))
    {
        el.set_text_content(Some(text));
        let _ = el.set_attribute("data-state", state);
    }
}

#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
fn install_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web Panic] {info}")));
        }));
    });
}

/// Mounts the NVE `CounterApp` into `#<root_element_id>` and optionally connects to `nve dev` WebSocket.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_counter_app(
    root_element_id: &str,
    hot_reload_ws_url: Option<String>,
) -> Result<(), JsValue> {
    install_panic_hook();
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("missing global window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("missing window.document"))?;
    let container = document
        .get_element_by_id(root_element_id)
        .ok_or_else(|| JsValue::from_str(&format!("root element '#{root_element_id}' not found")))?;

    let viewport = browser_viewport(&container);
    let backend = WebBackend::mount_into(container);
    let session = Rc::new(RefCell::new(WebAppSession::new_counter(1, backend, viewport)));

    // Wire live DOM events -> DioxusSurface::dispatch_events -> incremental CommandBuffer -> live DOM patch.
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
        "[NVE Web] Mounted CounterApp into '#{root_element_id}' ({} ops, {} mutations, dec={}us mount={}us)",
        initial_timing.op_count,
        initial_timing.mutation_count,
        initial_timing.decode_us,
        initial_timing.mount_us
    )));

    let hot_reload = match hot_reload_ws_url.filter(|u| !u.trim().is_empty()) {
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

    // Replace any previously mounted shell cleanly (dropping prior WS & closures).
    ACTIVE_SHELL.with(|slot| {
        *slot.borrow_mut() = Some(ActiveShell {
            _session: session,
            _hot_reload: hot_reload,
        });
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId};

    fn find_pressable_nodes(backend: &WebBackend) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = (1..=64)
            .filter(|&id| (backend.event_mask_of(id) & EventMask::PRESS) != 0)
            .collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn counter_app_mounts_clicks_incrementally_and_resets() {
        let mut session = WebAppSession::new_counter(1, WebBackend::new(), Viewport::default());
        let init = session.render_initial().unwrap();
        assert!(init.op_count > 0);
        assert!(init.mutation_count > 0);

        let html0 = session.backend().render_html();
        assert!(html0.contains("NATIVE VIEW ENGINE"));
        assert!(html0.contains("Count: 0"));
        assert!(html0.contains("Increment +"));
        assert!(html0.contains("Reset"));

        let buttons = find_pressable_nodes(session.backend());
        assert_eq!(buttons.len(), 2, "expected Increment and Reset buttons");
        let increment_id = buttons[0];
        let reset_id = buttons[1];

        // Click Increment twice.
        for expected in 1..=2 {
            session.backend().push_event(NativeEvent {
                surface_id: 1,
                node_id: increment_id,
                timestamp_us: expected as u64 * 1000,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point { x: 80.0, y: 24.0 },
            });
            let timing = session.pump_events().unwrap().expect("expected frame");
            assert!(
                timing.mutation_count <= 2,
                "incremental click should only patch changed text, got {} mutations",
                timing.mutation_count
            );
            let html = session.backend().render_html();
            assert!(html.contains(&format!("Count: {expected}")));
        }

        // Click Reset -> returns to Count: 0.
        session.backend().push_event(NativeEvent {
            surface_id: 1,
            node_id: reset_id,
            timestamp_us: 3000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 60.0, y: 24.0 },
        });
        session.pump_events().unwrap().expect("expected reset frame");
        assert!(session.backend().render_html().contains("Count: 0"));
    }

    #[test]
    fn handles_hot_patch_messages_for_tier1_tier2_and_status() {
        let mut session = WebAppSession::new_counter(1, WebBackend::new(), Viewport::default());
        session.render_initial().unwrap();

        // Tier-1 TemplatePatch applies live to DOM mirror.
        let tpl = HotPatchMsg::TemplatePatch {
            generation: 3,
            template_id: "CounterApp".into(),
            attr_updates: vec![("border-color".into(), "#38bdf8".into())],
            text_updates: vec![(0, "NVE LIVE PATCHED".into())],
        };
        let act1 = session.handle_hot_patch_json(&tpl.to_json()).unwrap();
        match act1 {
            HotReloadAction::TemplatePatched {
                generation,
                template_id,
                mutations,
            } => {
                assert_eq!(generation, 3);
                assert_eq!(template_id, "CounterApp");
                assert!(mutations > 0);
            }
            other => panic!("unexpected action: {other:?}"),
        }
        assert!(session.backend().render_html().contains("NVE LIVE PATCHED"));

        // Tier-2 DylibReload returns clean ReloadPage directive.
        let reload = HotPatchMsg::DylibReload {
            generation: 4,
            dylib_path: "target/debug/libnve_example_counter.dylib".into(),
            build_ms: 215,
        };
        let act2 = session.handle_hot_patch_json(&reload.to_json()).unwrap();
        assert_eq!(
            act2,
            HotReloadAction::ReloadPage {
                generation: 4,
                dylib_path: "target/debug/libnve_example_counter.dylib".into(),
                build_ms: 215,
            }
        );

        // DevStatus heartbeat.
        let status = HotPatchMsg::DevStatus {
            message: "watching examples/counter/src".into(),
            latency_ms: 12,
        };
        let act3 = session.handle_hot_patch_json(&status.to_json()).unwrap();
        assert_eq!(
            act3,
            HotReloadAction::DevStatus {
                message: "watching examples/counter/src".into(),
                latency_ms: 12,
            }
        );

        // Malformed JSON returns descriptive error without panicking.
        assert!(session.handle_hot_patch_json("{not valid json").is_err());
    }
}
