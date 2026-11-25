//! Browser `wasm32` `NveWebHost` live binary WebSocket pairing client (`NVT1`/`NVD1`).
//!
//! Supports both:
//! 1. Direct Rust-managed WebSocket (`NveWebHost::connect_pair_ws(mount_id, url, token)`), and
//! 2. Browser JS `new WebSocket(url)` with `ws.binaryType = "arraybuffer"` driving
//!    `NveWebHost::create_paired` + `attach_pair_socket` + `build_pair_hello` +
//!    `on_pair_binary_message(&[u8]) -> Vec<u8>`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_engine::Viewport;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::pairing::PairedWebSession;
use crate::session::DEFAULT_SURFACE_ID;
use crate::wasm_api::{
    browser_viewport, install_panic_hook, install_resize_listener, nve_extract_binary_message,
    set_hud_status, WindowResizeListener,
};

struct PairSocketHandle {
    socket: web_sys::WebSocket,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl Drop for PairSocketHandle {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
    }
}

struct ActivePairedHost {
    generation: u64,
    /// Owns the session for the page lifetime (only read by the scene-view exports).
    #[cfg_attr(not(feature = "scene-view"), allow(dead_code))]
    session: Rc<RefCell<PairedWebSession>>,
    _owned_socket: Option<PairSocketHandle>,
    _resize_listener: Option<WindowResizeListener>,
}

thread_local! {
    static ACTIVE_PAIRED_HOST: RefCell<Option<ActivePairedHost>> = const { RefCell::new(None) };
    static NEXT_PAIRED_GENERATION: Cell<u64> = const { Cell::new(1) };
}

#[cfg(feature = "scene-view")]
pub(crate) fn active_paired_scene_view_ptr() -> Option<usize> {
    ACTIVE_PAIRED_HOST.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|active| active.session.borrow().scene().view_ptr() as usize)
    })
}

fn send_binary_if_open(socket: &web_sys::WebSocket, bytes: &[u8]) {
    if bytes.is_empty() || socket.ready_state() != web_sys::WebSocket::OPEN {
        return;
    }
    let _ = socket.send_with_u8_array(bytes);
}

fn flush_all_outbound(session: &mut PairedWebSession, socket: Option<&web_sys::WebSocket>) {
    let Some(ws) = socket else {
        return;
    };
    while let Some(extra) = session.take_outbound_message() {
        send_binary_if_open(ws, &extra);
    }
}

fn update_paired_hud(session: &PairedWebSession) {
    if session.is_connected() {
        set_hud_status(
            "connected",
            &format!(
                "Paired WS · seq #{} ({} frames)",
                session.last_applied_seq(),
                session.frames_received()
            ),
        );
    }
}

/// Live `NVT1`/`NVD1` binary WebSocket thin-client host for browser WASM.
///
/// Renders remote `NVD1` `CommandBuffer` deltas straight into `WebBackend::submit_frame()` and
/// streams local DOM clicks, typing, scrolling, focus/blur, resize, and `NK1` keydowns upstream
/// as zero-JSON `NVT1` `InputEvents = 11` binary frames.
#[wasm_bindgen]
pub struct NveWebHost {
    mount_id: String,
    generation: u64,
    session: Rc<RefCell<PairedWebSession>>,
    socket: Rc<RefCell<Option<web_sys::WebSocket>>>,
}

#[wasm_bindgen]
impl NveWebHost {
    /// Mounts a paired thin-client host into `#<mount_id>` ready to be driven by a browser
    /// `new WebSocket(url)` (`ws.binaryType = "arraybuffer"`).
    pub fn create_paired(mount_id: &str, token: &str) -> Result<NveWebHost, JsValue> {
        Self::mount_internal(mount_id, token, None)
    }

    /// Mounts a paired thin-client host into `#<mount_id>` and immediately opens a binary
    /// `NVT1`/`NVD1` WebSocket connection to `url` using `token`.
    pub fn connect_pair_ws(mount_id: &str, url: &str, token: &str) -> Result<NveWebHost, JsValue> {
        Self::mount_internal(mount_id, token, Some(url))
    }

    /// Attaches an externally created browser `WebSocket` (`ws.binaryType = "arraybuffer"`) so
    /// live DOM interactions (clicks, typing, scroll, focus/blur, resize, keydown) automatically
    /// transmit `NVT1` `InputEvents = 11` binary frames upstream.
    pub fn attach_pair_socket(&self, socket: web_sys::WebSocket) {
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
        *self.socket.borrow_mut() = Some(socket);
    }

    /// Builds the initial zero-JSON `NVT1` `Hello` binary envelope to send on WebSocket open.
    pub fn build_pair_hello(&self) -> Vec<u8> {
        self.session.borrow_mut().build_hello()
    }

    /// Feeds one incoming binary `NVT1` WebSocket frame (`Uint8Array` from `ArrayBuffer`),
    /// decodes any `NVD1` delta directly into `WebBackend::submit_frame()`, and returns the
    /// primary outbound binary `NVT1` response bytes (`Ack`, `Pong`, `KeyframeRequest`, `Layout`).
    pub fn on_pair_binary_message(&self, raw: &[u8]) -> Result<Vec<u8>, JsValue> {
        let mut guard = self.session.borrow_mut();
        let reply = guard
            .on_pair_binary_message(raw)
            .map_err(|e| JsValue::from_str(&e))?;
        update_paired_hud(&guard);
        Ok(reply)
    }

    /// Pops any additional queued outbound binary `NVT1` frames (`Nack`, `Layout`, etc.).
    pub fn take_outbound_message(&self) -> Option<Vec<u8>> {
        self.session.borrow_mut().take_outbound_message()
    }

    /// Drains pending DOM events from `WebBackend` and encodes them into one binary `NVT1`
    /// `InputEvents = 11` frame.
    pub fn flush_dom_events(&self) -> Result<Option<Vec<u8>>, JsValue> {
        self.session
            .borrow_mut()
            .drain_dom_events_as_nvt1()
            .map_err(|e| JsValue::from_str(&e))
    }

    /// Updates viewport dimensions and returns an outbound binary `NVT1` `InputEvents = 11`
    /// `Layout` frame (and transmits it immediately if a socket is attached).
    pub fn dispatch_viewport_resize(
        &self,
        width: f32,
        height: f32,
    ) -> Result<Option<Vec<u8>>, JsValue> {
        let scale_factor = web_sys::window()
            .map(|w| w.device_pixel_ratio() as f32)
            .filter(|d| d.is_finite() && *d > 0.0)
            .unwrap_or(1.0);
        let pkt = self
            .session
            .borrow_mut()
            .encode_viewport_resize(Viewport {
                width,
                height,
                scale_factor,
            })
            .map_err(|e| JsValue::from_str(&e))?;
        if let (Some(bytes), Some(ws)) = (pkt.as_deref(), self.socket.borrow().as_ref()) {
            send_binary_if_open(ws, bytes);
        }
        Ok(pkt)
    }

    /// Dispatches a synthetic `NK1` keydown event upstream as an `NVT1` `InputEvents = 11` frame.
    pub fn dispatch_keydown(
        &self,
        key: &str,
        meta: bool,
        ctrl: bool,
    ) -> Result<Option<Vec<u8>>, JsValue> {
        let pkt = self
            .session
            .borrow_mut()
            .encode_keydown(key, meta, ctrl)
            .map_err(|e| JsValue::from_str(&e))?;
        if let (Some(bytes), Some(ws)) = (pkt.as_deref(), self.socket.borrow().as_ref()) {
            send_binary_if_open(ws, bytes);
        }
        Ok(pkt)
    }

    pub fn is_connected(&self) -> bool {
        self.session.borrow().is_connected()
    }

    pub fn session_id(&self) -> u32 {
        self.session.borrow().session_id()
    }

    pub fn frames_received(&self) -> u64 {
        self.session.borrow().frames_received()
    }

    pub fn node_count(&self) -> usize {
        self.session.borrow().backend().node_count()
    }

    pub fn render_html(&self) -> String {
        self.session.borrow().backend().render_html()
    }

    #[cfg(feature = "scene-view")]
    pub fn scene_view_ptr(&self) -> usize {
        self.session.borrow().scene().view_ptr() as usize
    }

    /// Sends a graceful `Bye` frame, closes any attached WebSocket, and unmounts the surface.
    pub fn disconnect(&self) {
        if let (Some(bye), Some(ws)) = (
            self.session.borrow_mut().build_bye(),
            self.socket.borrow().as_ref(),
        ) {
            send_binary_if_open(ws, &bye);
            let _ = ws.close();
        }
        *self.socket.borrow_mut() = None;

        let removed = ACTIVE_PAIRED_HOST.with(|slot| {
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
            set_hud_status("offline", "Paired WS: disconnected");
        }
    }
}

impl NveWebHost {
    fn mount_internal(
        mount_id: &str,
        token: &str,
        ws_url: Option<&str>,
    ) -> Result<NveWebHost, JsValue> {
        install_panic_hook();
        if token.trim().is_empty() {
            return Err(JsValue::from_str("pairing token cannot be empty"));
        }

        let window =
            web_sys::window().ok_or_else(|| JsValue::from_str("missing global window"))?;
        let document = window
            .document()
            .ok_or_else(|| JsValue::from_str("missing window.document"))?;
        let container = document
            .get_element_by_id(mount_id)
            .ok_or_else(|| JsValue::from_str(&format!("root element '#{mount_id}' not found")))?;

        let viewport = browser_viewport(&container);
        let container_for_resize = container.clone();
        let backend = WebBackend::mount_into(container);
        let surface_id: SurfaceId = DEFAULT_SURFACE_ID;
        let session = Rc::new(RefCell::new(PairedWebSession::new(
            surface_id,
            backend,
            viewport,
            token.trim(),
        )));
        let socket_slot: Rc<RefCell<Option<web_sys::WebSocket>>> = Rc::new(RefCell::new(None));

        // Wire live browser DOM interactions (click/input/scroll/focus/blur/keydown/submit)
        // straight into binary NVT1 InputEvents = 11 frames over the active WebSocket.
        let session_for_events = Rc::clone(&session);
        let socket_for_events = Rc::clone(&socket_slot);
        session
            .borrow_mut()
            .backend_mut()
            .set_render_callback(move || {
                let Ok(mut guard) = session_for_events.try_borrow_mut() else {
                    return;
                };
                match guard.drain_dom_events_as_nvt1() {
                    Ok(Some(pkt)) => {
                        if let Some(ws) = socket_for_events.borrow().as_ref() {
                            send_binary_if_open(ws, &pkt);
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        web_sys::console::warn_1(&JsValue::from_str(&format!(
                            "[NVE Pair] Failed to encode DOM InputEvents: {err}"
                        )));
                    }
                }
            });

        // Forward window & container resizes upstream as Layout InputEvents.
        let session_for_resize = Rc::clone(&session);
        let socket_for_resize = Rc::clone(&socket_slot);
        let resize_listener =
            install_resize_listener(&window, container_for_resize, move |next_vp| {
                let Ok(mut guard) = session_for_resize.try_borrow_mut() else {
                    return;
                };
                if let Ok(Some(pkt)) = guard.encode_viewport_resize(next_vp)
                    && let Some(ws) = socket_for_resize.borrow().as_ref()
                {
                    send_binary_if_open(ws, &pkt);
                }
            });

        let owned_socket = if let Some(url) = ws_url {
            let handle =
                open_managed_pair_socket(url, Rc::clone(&session), Rc::clone(&socket_slot))?;
            Some(handle)
        } else {
            set_hud_status("connecting", "Pair: waiting for socket…");
            None
        };

        let generation = NEXT_PAIRED_GENERATION.with(|counter| {
            let next = counter.get();
            counter.set(next.wrapping_add(1));
            next
        });

        ACTIVE_PAIRED_HOST.with(|slot| {
            *slot.borrow_mut() = Some(ActivePairedHost {
                generation,
                session: Rc::clone(&session),
                _owned_socket: owned_socket,
                _resize_listener: resize_listener,
            });
        });

        Ok(NveWebHost {
            mount_id: mount_id.to_string(),
            generation,
            session,
            socket: socket_slot,
        })
    }
}

fn open_managed_pair_socket(
    ws_url: &str,
    session: Rc<RefCell<PairedWebSession>>,
    socket_slot: Rc<RefCell<Option<web_sys::WebSocket>>>,
) -> Result<PairSocketHandle, JsValue> {
    let socket = web_sys::WebSocket::new(ws_url)?;
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
    *socket_slot.borrow_mut() = Some(socket.clone());
    set_hud_status("connecting", &format!("Pair: connecting ({ws_url})"));

    let session_open = Rc::clone(&session);
    let ws_open = socket.clone();
    let url_open = ws_url.to_string();
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let hello = session_open.borrow_mut().build_hello();
        send_binary_if_open(&ws_open, &hello);
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[NVE Pair] Binary WebSocket opened ({url_open}); sent NVT1 Hello ({} bytes)",
            hello.len()
        )));
        set_hud_status("connecting", "Pair: authenticating…");
    });

    let session_msg = Rc::clone(&session);
    let ws_msg = socket.clone();
    let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
        move |ev: web_sys::MessageEvent| {
            let bytes = nve_extract_binary_message(&ev.data());
            if bytes.is_empty() {
                return;
            }
            let Ok(mut guard) = session_msg.try_borrow_mut() else {
                return;
            };
            match guard.on_pair_binary_message(&bytes) {
                Ok(reply) => {
                    send_binary_if_open(&ws_msg, &reply);
                    flush_all_outbound(&mut guard, Some(&ws_msg));
                    update_paired_hud(&guard);
                }
                Err(err) => {
                    web_sys::console::warn_1(&JsValue::from_str(&format!(
                        "[NVE Pair] Binary message error: {err}"
                    )));
                    set_hud_status("offline", &format!("Pair error: {err}"));
                }
            }
        },
    );

    let url_err = ws_url.to_string();
    let on_error = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |_| {
        web_sys::console::warn_1(&JsValue::from_str(&format!(
            "[NVE Pair] WebSocket error on {url_err}"
        )));
        set_hud_status("offline", "Pair: socket error");
    });

    let on_close = Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_| {
        web_sys::console::log_1(&JsValue::from_str("[NVE Pair] WebSocket closed"));
        set_hud_status("offline", "Pair: disconnected");
    });

    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    Ok(PairSocketHandle {
        socket,
        _on_open: on_open,
        _on_message: on_message,
        _on_error: on_error,
        _on_close: on_close,
    })
}
