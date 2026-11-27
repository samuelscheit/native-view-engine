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
use wasm_bindgen::prelude::*;
use crate::wasm_hydrate::SsrRenderResult;
#[cfg(feature = "dev")]
use crate::wasm_hotreload::{connect_hot_reload, HotReloadSocket};
use crate::session::{WebAppSession, DEFAULT_SURFACE_ID};
#[cfg(all(feature = "dev", feature = "scene-view"))]
use crate::wasm_pair::active_paired_scene_view_ptr;
#[cfg(feature = "multi-window")]
use crate::wasm_popup::{
    popup_window_for_surface, register_mounted_surface_window, unregister_surface_window,
};
use crate::wasm_snapshot::{
    clear_persisted_snapshot, load_persisted_state, query_has_reset, query_has_ssr_demo,
    SnapshotPersistenceGuard, SnapshotTrigger,
};
#[cfg(feature = "app-issues")]
use crate::wasm_wake::{PumpNext, WebAsyncWakeGuard};
pub(crate) use crate::wasm_wake::{browser_viewport, install_resize_listener, WindowResizeListener};

#[cfg(any(feature = "dev", feature = "multi-window"))]
#[wasm_bindgen(inline_js = "export function nve_extract_binary_message(data) {
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  return new Uint8Array(0);
}")]
extern "C" {
    pub(crate) fn nve_extract_binary_message(data: &JsValue) -> Vec<u8>;
}

struct ActiveShell {
    generation: u64,
    #[cfg(feature = "multi-window")]
    window_handle: Option<WindowHandleId>,
    #[cfg_attr(not(any(feature = "scene-view", feature = "app-issues")), allow(dead_code))]
    session: Rc<RefCell<WebAppSession>>,
    #[cfg(feature = "app-issues")]
    snapshot_trigger: Option<SnapshotTrigger>,
    #[cfg(feature = "dev")]
    _hot_reload: Option<HotReloadSocket>,
    _resize_listener: Option<WindowResizeListener>,
    _snapshot_guard: Option<SnapshotPersistenceGuard>,
    #[cfg(feature = "app-issues")]
    _async_wake: WebAsyncWakeGuard,
}

/// Pumps async Dioxus tasks/effects for `(surface_id, generation)`. Returns [`PumpNext::Microtask`]
/// if the session is momentarily borrowed (re-entrancy), [`PumpNext::Macrotask`] if `MAX_PUMPS`
/// was hit so the browser paints before continuing, or [`PumpNext::Done`].
#[cfg(feature = "app-issues")]
pub(crate) fn pump_surface_async_work(
    surface_id: SurfaceId,
    generation: u64,
    waker: &std::task::Waker,
    on_before_poll: impl FnMut(),
) -> PumpNext {
    let target = ACTIVE_SHELLS.with(|shells| {
        let map = shells.borrow();
        let active = map.get(&surface_id)?;
        if active.generation != generation {
            return None;
        }
        Some((Rc::clone(&active.session), active.snapshot_trigger.clone()))
    });
    let Some((session, snapshot_trigger)) = target else {
        return PumpNext::Done;
    };
    match session.try_borrow_mut() {
        Ok(mut guard) => match guard.pump_async_work_with(waker, on_before_poll) {
            Ok(outcome) => {
                if outcome.timing.is_some()
                    && let Some(t) = snapshot_trigger.as_ref()
                {
                    t.schedule_debounced();
                }
                PumpNext::from_hit_cap(outcome.hit_cap)
            }
            Err(err) => {
                web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] {err}")));
                PumpNext::Done
            }
        },
        Err(_) => PumpNext::Microtask,
    }
}

pub(crate) fn run_initial_frame_and_events(guard: &mut WebAppSession) -> Result<bool, JsValue> {
    guard
        .render_initial()
        .map_err(|e| JsValue::from_str(&format!("initial frame failed: {e}")))?;
    guard
        .pump_events()
        .map_err(|e| JsValue::from_str(&format!("post-hydration event pump failed: {e}")))
        .map(|f| f.is_some())
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
pub fn nve_web_scene_view_ptr_for_surface(surface_id: u32) -> usize {
    let Some(surface_id) = SurfaceId::from_raw(surface_id) else {
        return 0;
    };
    ACTIVE_SHELLS.with(|shells| {
        shells
            .borrow()
            .get(&surface_id)
            .map(|active| active.session.borrow().scene().view_ptr() as usize)
            .unwrap_or(0)
    })
}

#[wasm_bindgen]
pub fn active_surface_ids() -> Vec<u32> {
    ACTIVE_SHELLS.with(|shells| shells.borrow().keys().map(|id| id.raw()).collect())
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
    restored_from_snapshot: bool,
    session: Rc<RefCell<WebAppSession>>,
    snapshot_trigger: Option<SnapshotTrigger>,
}

#[wasm_bindgen]
impl WebAppController {
    #[wasm_bindgen(getter)]
    pub fn app_name(&self) -> String { self.app_name.to_string() }

    #[wasm_bindgen(getter)]
    pub fn mount_id(&self) -> String { self.mount_id.clone() }

    #[wasm_bindgen(getter)]
    pub fn surface_id(&self) -> u32 { self.surface_id.raw() }

    pub fn node_count(&self) -> usize { self.session.borrow().node_count() }

    pub fn render_html(&self) -> String { self.session.borrow().render_html() }

    pub fn render_ssr_bundle(&self) -> SsrRenderResult {
        SsrRenderResult::from_output(self.session.borrow().render_ssr())
    }

    pub fn is_hydrated(&self) -> bool {
        self.session.borrow().backend().hydration_report().hydrated
    }

    pub fn hydration_mismatch_count(&self) -> usize {
        self.session.borrow().backend().hydration_report().mismatches.len()
    }

    pub fn hydration_adopted_count(&self) -> usize {
        self.session.borrow().backend().hydration_report().adopted_nodes as usize
    }

    pub fn hydration_replaced_count(&self) -> usize {
        self.session.borrow().backend().hydration_report().replaced_nodes as usize
    }

    pub fn hydration_seeded_rules(&self) -> usize {
        self.session.borrow().backend().hydration_report().reused_css_rules as usize
    }

    pub fn stylesheet_rule_count(&self) -> usize {
        self.session.borrow().backend().stylesheet().rule_count()
    }

    pub fn hydration_replayed_events(&self) -> usize {
        self.session.borrow().backend().hydration_report().replayed_events as usize
    }

    pub fn hydration_reconciled_dirty_controls(&self) -> usize {
        self.session
            .borrow()
            .backend()
            .hydration_report()
            .reconciled_dirty_controls as usize
    }

    pub fn restored_from_snapshot(&self) -> bool {
        self.restored_from_snapshot
    }

    pub fn save_snapshot_now(&self) -> bool {
        self.snapshot_trigger
            .as_ref()
            .is_some_and(|t| t.flush_now(true))
    }

    pub fn clear_snapshot(&self) {
        if let Some(t) = self.snapshot_trigger.as_ref() {
            t.clear();
        }
    }

    pub fn keyboard_inset_height(&self) -> f32 {
        self.session.borrow().backend().keyboard_insets().height
    }

    pub fn keyboard_visible(&self) -> bool {
        self.session.borrow().backend().keyboard_insets().visible
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
        let changed = self
            .session
            .borrow_mut()
            .dispatch_keydown(key, meta, ctrl)
            .map(|timing| timing.is_some())
            .map_err(|e| JsValue::from_str(&format!("keydown dispatch failed: {e}")))?;
        if changed && let Some(t) = self.snapshot_trigger.as_ref() {
            t.schedule_debounced();
        }
        Ok(changed)
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

pub(crate) fn set_hud_status(state: &str, text: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("nve-status"))
    {
        el.set_text_content(Some(text));
        let _ = el.set_attribute("data-state", state);
    }
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
    if surface_id.is_null() {
        return Err(JsValue::from_str("surface_id must be non-zero"));
    }
    let (window, container) = find_mount_container(surface_id, mount_id)?;
    let viewport = browser_viewport(&container);
    let container_for_resize = container.clone();
    let container_for_snapshot = container.clone();

    let is_reset = query_has_reset(&window);
    let is_ssr_demo = query_has_ssr_demo(&window);
    let enable_persistence = !is_ssr_demo && surface_id == DEFAULT_SURFACE_ID;
    let has_restored_dom = container.get_attribute("data-nve-restored").as_deref() == Some("1");

    let deferred_vp = Viewport {
        width: 0.0,
        height: 0.0,
        scale_factor: 1.0,
    };
    let mut session_val = build_session(surface_id, WebBackend::new(), deferred_vp);
    let restored_from_snapshot = if let Some(slug) = session_val.app_slug() {
        let schema_version = session_val.schema_version();
        let restored = if is_reset {
            clear_persisted_snapshot(slug, schema_version);
            false
        } else if enable_persistence {
            load_persisted_state(slug, schema_version, false)
                .as_deref()
                .is_some_and(|bytes| session_val.try_restore_state(bytes))
        } else {
            false
        };
        if has_restored_dom && !restored {
            // Cached HTML existed without matching valid binary state: purge corrupt storage and fall back cleanly.
            clear_persisted_snapshot(slug, schema_version);
        }
        restored
    } else {
        false
    };

    // Hydrate only when pre-rendered DOM matches the state we are booting into:
    // - Layer 1 restored snapshot + valid binary state -> zero-mismatch hydration
    // - Cold build-time SSR / `?ssr=1` + clean default state -> zero-mismatch hydration
    // - Corrupt/partial state mismatch -> clean full mount fallback with zero panic.
    let should_hydrate = if has_restored_dom {
        restored_from_snapshot
    } else {
        !restored_from_snapshot && hydrate
    };

    let backend = if should_hydrate {
        WebBackend::hydrate_into(container.clone())
    } else {
        WebBackend::mount_into(container.clone())
    };
    session_val.replace_backend_with_viewport(backend, viewport);
    session_val.apply_initial_viewport();
    let session = Rc::new(RefCell::new(session_val));

    let (snapshot_guard, snapshot_trigger) = if enable_persistence {
        match SnapshotPersistenceGuard::install(
            &window,
            &session,
            container_for_snapshot,
            !is_reset,
        ) {
            Some((guard, trigger)) => (Some(guard), Some(trigger)),
            None => (None, None),
        }
    } else {
        (None, None)
    };

    let generation = NEXT_SHELL_GENERATION.with(|c| {
        let next = c.get();
        c.set(next.wrapping_add(1));
        next
    });
    #[cfg(feature = "app-issues")]
    let async_wake = WebAsyncWakeGuard::new(surface_id, generation);
    #[cfg(feature = "app-issues")]
    let wake_for_events = async_wake.handle();

    let session_for_events = Rc::downgrade(&session);
    let trigger_for_events = snapshot_trigger.clone();
    session.borrow_mut().backend_mut().set_on_event(move || {
        if let Some(session_for_events) = session_for_events.upgrade()
            && let Ok(mut guard) = session_for_events.try_borrow_mut()
        {
            #[cfg(feature = "app-issues")]
            let rendered = wake_for_events.run_event_pump(&mut guard);
            #[cfg(not(feature = "app-issues"))]
            let rendered = match guard.pump_events() {
                Ok(f) => f.is_some(),
                Err(err) => {
                    web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] {err}")));
                    false
                }
            };
            if rendered && let Some(t) = trigger_for_events.as_ref() {
                t.schedule_debounced();
            }
        }
    });

    {
        let mut guard = session.borrow_mut();
        #[cfg(feature = "app-issues")]
        let boot_rendered = async_wake.run_boot_pump(&mut guard)?;
        #[cfg(not(feature = "app-issues"))]
        let boot_rendered = run_initial_frame_and_events(&mut guard)?;
        if boot_rendered && let Some(t) = snapshot_trigger.as_ref() {
            t.schedule_debounced();
        }
    }
    let _ = container.set_attribute("data-nve-live", "1");

    let session_for_resize = Rc::downgrade(&session);
    let resize_listener = install_resize_listener(&window, container_for_resize, move |next_vp| {
        if let Some(session_for_resize) = session_for_resize.upgrade()
            && let Ok(mut guard) = session_for_resize.try_borrow_mut()
            && let Err(err) = guard.resize_viewport(next_vp)
        {
            web_sys::console::error_1(&JsValue::from_str(&format!("[NVE Web] {err}")));
        }
    });

    #[cfg(feature = "dev")]
    let hot_reload = match ws_url.filter(|u| !u.trim().is_empty()) {
        Some(url) => connect_hot_reload(&url, Rc::downgrade(&session)).ok(),
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
                #[cfg(feature = "app-issues")]
                snapshot_trigger: snapshot_trigger.clone(),
                #[cfg(feature = "dev")]
                _hot_reload: hot_reload,
                _resize_listener: resize_listener,
                _snapshot_guard: snapshot_guard,
                #[cfg(feature = "app-issues")]
                _async_wake: async_wake,
            },
        );
    });

    Ok(WebAppController {
        app_name,
        mount_id: mount_id.to_string(),
        surface_id,
        generation,
        restored_from_snapshot,
        session,
        snapshot_trigger,
    })
}

#[cfg(feature = "app-studio")]
#[wasm_bindgen]
pub fn start_studio_workspace_app(
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_studio_workspace_app_on_surface(DEFAULT_SURFACE_ID.raw(), mount_id, ws_url)
}

#[cfg(feature = "app-studio")]
#[wasm_bindgen]
pub fn start_studio_workspace_app_on_surface(
    surface_id: u32,
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    let surface_id = SurfaceId::from_raw(surface_id)
        .ok_or_else(|| JsValue::from_str("surface_id must have a non-zero 24-bit index"))?;
    start_surface_app_inner(
        "StudioWorkspaceApp",
        surface_id,
        mount_id,
        ws_url,
        false,
        WebAppSession::new_studio_workspace,
    )
}

#[cfg(feature = "app-kitchen-sink")]
#[wasm_bindgen]
pub fn start_kitchen_sink_app(
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_kitchen_sink_app_on_surface(DEFAULT_SURFACE_ID.raw(), mount_id, ws_url)
}

#[cfg(feature = "app-kitchen-sink")]
#[wasm_bindgen]
pub fn start_kitchen_sink_app_on_surface(
    surface_id: u32,
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    let surface_id = SurfaceId::from_raw(surface_id)
        .ok_or_else(|| JsValue::from_str("surface_id must have a non-zero 24-bit index"))?;
    start_surface_app_inner(
        "KitchenSinkApp",
        surface_id,
        mount_id,
        ws_url,
        false,
        WebAppSession::new_kitchen_sink,
    )
}
