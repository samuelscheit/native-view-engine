//! Dev-only browser WebSocket client for binary `NVP1` hot-reload patches (`feature = "dev"`).

use std::cell::RefCell;
use std::rc::Weak;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::session::{HotReloadAction, WebAppSession};
use crate::wasm_api::{nve_extract_binary_message, set_hud_status};

pub(crate) struct HotReloadSocket {
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

fn browser_now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

pub(crate) fn connect_hot_reload(
    ws_url: &str,
    session: Weak<RefCell<WebAppSession>>,
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
            let Some(session) = session.upgrade() else {
                return;
            };
            let result = session.borrow_mut().handle_hot_patch_frame(&bytes);
            match result {
                Ok(HotReloadAction::TemplatePatched { generation, .. }) => {
                    let mut ms = String::new();
                    nve_abi::num::write_f64(&mut ms, (browser_now_ms() - t0).max(0.0));
                    set_hud_status("patched", &format!("Patched gen {generation} ({ms}ms)"));
                }
                Ok(HotReloadAction::ReloadPage {
                    generation,
                    build_ms,
                    ..
                }) => {
                    set_hud_status(
                        "reloading",
                        &format!("Reloading gen {generation} ({build_ms}ms)..."),
                    );
                    if let Some(w) = web_sys::window() {
                        let _ = w.location().reload();
                    }
                }
                Ok(HotReloadAction::DevStatus {
                    message,
                    latency_ms,
                }) => {
                    set_hud_status("connected", &format!("{message} ({latency_ms}ms)"));
                }
                Err(err) => {
                    web_sys::console::warn_1(&JsValue::from_str(&format!(
                        "[NVE HotReload] {err}"
                    )));
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
