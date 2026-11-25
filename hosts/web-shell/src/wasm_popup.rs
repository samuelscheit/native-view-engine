//! Browser `wasm32` popup windows (`open_popup_window` / `close_popup_window`) and
//! zero-JSON `NVW1` `BroadcastChannel` window synchronization.

use std::cell::RefCell;
use std::collections::BTreeMap;

use nve_abi::SurfaceId;
use nve_interop::{
    decode_window_commands, encode_window_commands, SurfacePresentation, WindowCommand,
    WindowDescriptor, WindowHandleId, WindowManager,
};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::multi_window::{PopupWindowError, PopupWindowSpec, DEFAULT_WINDOW_SYNC_CHANNEL};
use crate::wasm_api::{install_panic_hook, primary_surface_id_if_mounted, remove_active_shell};
use crate::wasm_pair::nve_extract_binary_message;

#[wasm_bindgen(inline_js = "export function nve_slice_to_uint8array(bytes) {
  return bytes;
}")]
extern "C" {
    fn nve_slice_to_uint8array(bytes: &[u8]) -> JsValue;
}

struct ActivePopupWindow {
    handle: WindowHandleId,
    window: web_sys::Window,
}

struct WindowSyncChannel {
    channel: web_sys::BroadcastChannel,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
}

impl Drop for WindowSyncChannel {
    fn drop(&mut self) {
        self.channel.set_onmessage(None);
        self.channel.close();
    }
}

thread_local! {
    static POPUP_WINDOWS: RefCell<BTreeMap<SurfaceId, ActivePopupWindow>> = const { RefCell::new(BTreeMap::new()) };
    static WINDOW_MANAGER: RefCell<WindowManager> = RefCell::new(WindowManager::new());
    static SYNC_CHANNEL: RefCell<Option<WindowSyncChannel>> = const { RefCell::new(None) };
}

pub(crate) fn popup_window_for_surface(surface_id: SurfaceId) -> Option<web_sys::Window> {
    POPUP_WINDOWS.with(|popups| popups.borrow().get(&surface_id).map(|e| e.window.clone()))
}

pub(crate) fn register_mounted_surface_window(
    surface_id: SurfaceId,
    title: &str,
    width: f32,
    height: f32,
) -> Option<WindowHandleId> {
    WINDOW_MANAGER.with(|wm| {
        let mut mgr = wm.borrow_mut();
        if let Some(existing) = mgr.handle_for_surface(surface_id) {
            return Some(existing);
        }
        let desc = WindowDescriptor::new(title, width.max(1.0), height.max(1.0));
        let pres = if mgr.is_empty() {
            SurfacePresentation::PrimaryWindow(desc)
        } else {
            SurfacePresentation::SecondaryWindow { parent: None, modal: false, descriptor: desc }
        };
        mgr.open_window(surface_id, pres).ok()
    })
}

pub(crate) fn unregister_surface_window(handle: WindowHandleId) {
    let _ = WINDOW_MANAGER.with(|wm| wm.borrow_mut().close_window(handle));
}

fn ensure_sync_channel(channel_name: &str) -> Result<(), PopupWindowError> {
    SYNC_CHANNEL.with(|slot| {
        if slot.borrow().is_some() {
            return Ok(());
        }
        let channel = web_sys::BroadcastChannel::new(channel_name)
            .map_err(|err| PopupWindowError::BroadcastChannelUnavailable(format!("{err:?}")))?;
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |ev: web_sys::MessageEvent| {
                let bytes = nve_extract_binary_message(&ev.data());
                if bytes.is_empty() {
                    return;
                }
                if let Ok(cmds) = decode_window_commands(&bytes) {
                    WINDOW_MANAGER.with(|wm| {
                        let mut mgr = wm.borrow_mut();
                        for cmd in cmds {
                            let _ = mgr.apply_command(cmd);
                        }
                    });
                }
            },
        );
        channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        *slot.borrow_mut() = Some(WindowSyncChannel { channel, _on_message: on_message });
        Ok(())
    })
}

fn post_sync_bytes(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    SYNC_CHANNEL.with(|slot| {
        if let Some(sync) = slot.borrow().as_ref() {
            let js_payload = nve_slice_to_uint8array(bytes);
            let _ = sync.channel.post_message(&js_payload);
        }
    });
}

/// Broadcasts a binary `NVW1` command buffer across sibling browser windows via `BroadcastChannel`.
#[wasm_bindgen]
pub fn broadcast_window_commands(nvw1_bytes: &[u8]) -> Result<(), JsValue> {
    let cmds = decode_window_commands(nvw1_bytes)
        .map_err(|e| JsValue::from_str(&format!("invalid NVW1 command buffer: {e}")))?;
    ensure_sync_channel(DEFAULT_WINDOW_SYNC_CHANNEL)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    WINDOW_MANAGER.with(|wm| {
        let mut mgr = wm.borrow_mut();
        for cmd in cmds {
            let _ = mgr.apply_command(cmd);
        }
    });
    post_sync_bytes(nvw1_bytes);
    Ok(())
}

/// Handle to a secondary browser popup window opened via [`open_popup_window`].
#[wasm_bindgen]
pub struct WebPopupController {
    surface_id: SurfaceId,
    handle: WindowHandleId,
    mount_root_id: String,
    channel_name: String,
}

#[wasm_bindgen]
impl WebPopupController {
    #[wasm_bindgen(getter)]
    pub fn surface_id(&self) -> SurfaceId { self.surface_id }

    #[wasm_bindgen(getter)]
    pub fn window_handle(&self) -> WindowHandleId { self.handle }

    #[wasm_bindgen(getter)]
    pub fn mount_root_id(&self) -> String { self.mount_root_id.clone() }

    #[wasm_bindgen(getter)]
    pub fn channel_name(&self) -> String { self.channel_name.clone() }

    pub fn focus(&self) -> bool {
        let ok = WINDOW_MANAGER.with(|wm| wm.borrow_mut().focus_window(self.handle).is_ok());
        if !ok {
            return false;
        }
        POPUP_WINDOWS.with(|popups| {
            if let Some(entry) = popups.borrow().get(&self.surface_id) {
                let _ = entry.window.focus();
            }
        });
        post_sync_bytes(&encode_window_commands(&[WindowCommand::FocusWindow { handle: self.handle }]));
        true
    }

    pub fn set_title(&self, title: &str) {
        let _ = WINDOW_MANAGER.with(|wm| wm.borrow_mut().set_title(self.handle, title.to_owned()));
        POPUP_WINDOWS.with(|popups| {
            if let Some(entry) = popups.borrow().get(&self.surface_id)
                && let Some(doc) = entry.window.document()
            {
                doc.set_title(title);
            }
        });
        post_sync_bytes(&encode_window_commands(&[WindowCommand::SetTitle {
            handle: self.handle,
            title: title.to_owned(),
        }]));
    }

    pub fn close(&self) {
        close_popup_window(self.surface_id);
    }
}

/// Opens a secondary browser popup window for `surface_id` using `window.open(..., features)`
/// and synchronizes lifecycle via `BroadcastChannel` (`NVW1`), returning a clear error if blocked.
#[wasm_bindgen]
pub fn open_popup_window(
    surface_id: SurfaceId,
    title: &str,
    width: u32,
    height: u32,
) -> Result<WebPopupController, JsValue> {
    install_panic_hook();
    let spec = PopupWindowSpec::new(surface_id, title, width, height)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let host_window =
        web_sys::window().ok_or_else(|| JsValue::from_str("missing global window"))?;
    let popup = host_window
        .open_with_url_and_target_and_features("", &spec.target_name(), &spec.format_window_features())
        .ok()
        .flatten()
        .ok_or_else(|| {
            JsValue::from_str(
                &PopupWindowError::PopupBlocked { surface_id, title: spec.title.clone() }.to_string(),
            )
        })?;

    let mount_root_id = spec.mount_root_id();
    if let Some(doc) = popup.document() {
        doc.set_title(&spec.title);
        if doc.get_element_by_id(&mount_root_id).is_none()
            && let Some(body) = doc.body()
            && let Ok(root) = doc.create_element("div")
        {
            root.set_id(&mount_root_id);
            let _ = body.append_child(&root);
        }
    }

    let presentation = spec.to_presentation(primary_surface_id_if_mounted());
    let handle = WINDOW_MANAGER
        .with(|wm| wm.borrow_mut().open_window(surface_id, presentation.clone()))
        .map_err(|e| {
            let _ = popup.close();
            JsValue::from_str(&PopupWindowError::WindowManager(e).to_string())
        })?;

    let _ = ensure_sync_channel(&spec.channel_name);
    post_sync_bytes(&encode_window_commands(&[WindowCommand::OpenWindow {
        surface_id,
        presentation,
    }]));

    POPUP_WINDOWS.with(|popups| {
        popups.borrow_mut().insert(surface_id, ActivePopupWindow { handle, window: popup });
    });

    Ok(WebPopupController {
        surface_id,
        handle,
        mount_root_id,
        channel_name: spec.channel_name,
    })
}

/// Closes a previously opened popup window and broadcasts an `NVW1` `CloseWindow` frame.
#[wasm_bindgen]
pub fn close_popup_window(surface_id: SurfaceId) {
    if let Some(entry) = POPUP_WINDOWS.with(|popups| popups.borrow_mut().remove(&surface_id)) {
        let _ = entry.window.close();
        let _ = WINDOW_MANAGER.with(|wm| wm.borrow_mut().close_window(entry.handle));
        post_sync_bytes(&encode_window_commands(&[WindowCommand::CloseWindow {
            handle: entry.handle,
        }]));
    }
    remove_active_shell(surface_id);
}
