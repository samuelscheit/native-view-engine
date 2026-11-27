//! Browser `wasm32` zero-flicker SSR hydration entrypoint for `nve-docs-site`.
//!
//! Hydrates SSR-rendered HTML in-place via `WebBackend::hydrate_into`, integrates client-side
//! SPA routing via `nve_router::web::{BrowserHistoryBackend, WebHistoryAdapter}`, intercepts
//! internal `<a href>` clicks, synchronizes `history.pushState` + `popstate`, adapts responsively
//! across desktop/tablet/mobile viewports, and enables `⌘K` search, theme toggle, code copy,
//! and all 5 interactive playgrounds.

use std::cell::RefCell;
use std::rc::Rc;

use nve_abi::{
    EventPayload, FrameId, NativeEvent, NativeEventKind, NodeId, NVE_KEY_MOD_CTRL,
    NVE_KEY_MOD_META, ROOT_NODE_ID, SurfaceId,
};
use nve_backend_web::{HydrationReport, WebBackend};
use nve_canvas_web::CanvasWebSystem;
use nve_components::tables::DebugId;
use nve_dioxus::DioxusSurface;
use nve_docs::{DocsRoute, DocsViewport};
use nve_engine::{StoreBackend, Viewport};
use nve_router::web::{BrowserHistoryBackend, WebHistoryAdapter};
use nve_web_html::{DataAttrList, HtmlDataAttrs};
use nve_web_html_web::register_web_html_systems;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

use crate::create_nve_docs_surface_full;
use crate::site::{embedded_compiled_site, nve_docs_site_config};

const DOCS_SURFACE_ID: SurfaceId = SurfaceId::ROOT;

struct DocsWebSession {
    base_path: String,
    surface: DioxusSurface,
    backend: WebBackend,
    history: WebHistoryAdapter<DocsRoute, BrowserHistoryBackend>,
    frame_id: FrameId,
    viewport: DocsViewport,
    dark_mode: bool,
    last_route: String,
    hydration_report: HydrationReport,
}

impl DocsWebSession {
    fn render_next_frame(&mut self) -> Result<(), String> {
        self.frame_id = self.frame_id.next();
        self.surface
            .commit_to(self.frame_id, &mut self.backend)
            .map_err(|e| format!("WebBackend commit_to failed: {e}"))?;
        Ok(())
    }

    fn find_node_by_test_id(&self, tid: &str) -> Option<NodeId> {
        let store = self.surface.store();
        let mut stack = vec![NodeId::ROOT];
        while let Some(id) = stack.pop() {
            if id != NodeId::ROOT
                && let Some(dbg) = store.try_row::<DebugId>(id)
                && dbg.test_id() == tid
            {
                return Some(id);
            }
            stack.extend_from_slice(store.children(id));
        }
        None
    }

    fn data_attr_of(&self, id: NodeId, key: &str) -> Option<String> {
        let store = self.surface.store();
        let res = store.try_row::<HtmlDataAttrs>(id)?.entries();
        let list = store.resource::<DataAttrList>(res)?;
        let stripped = key.strip_prefix("data-").unwrap_or(key);
        list.iter()
            .find(|(k, _)| *k == key || *k == stripped)
            .map(|(_, v)| v.to_owned())
    }

    fn current_active_route(&self) -> Option<String> {
        let id = self
            .find_node_by_test_id("docs-root")
            .or_else(|| self.find_node_by_test_id("docs-article"))?;
        self.data_attr_of(id, "data-route")
    }

    fn current_dark_mode(&self) -> bool {
        let Some(id) = self.find_node_by_test_id("docs-root") else {
            return self.dark_mode;
        };
        match self.data_attr_of(id, "data-theme") {
            Some(s) => s != "light",
            None => self.dark_mode,
        }
    }

    fn sync_browser_chrome(&mut self, push_history: bool) {
        let dark = self.current_dark_mode();
        self.dark_mode = dark;
        sync_document_theme(dark);

        if let Some(route) = self.current_active_route()
            && route != self.last_route
        {
            self.last_route = route.clone();
            let parsed = DocsRoute::new(&route);
            if push_history {
                self.history.sync_push(&parsed);
                reset_scroll_containers();
            }
            sync_document_title(&route, &self.base_path);
        }
    }

    fn retarget_root_keydown(&self, events: &mut [NativeEvent]) {
        let search_input_id = self.find_node_by_test_id("docs-search-input");
        let docs_root_id = self.find_node_by_test_id("docs-root");

        for ev in events {
            if ev.node_id == ROOT_NODE_ID
                && ev.kind == NativeEventKind::KeyDown
                && let EventPayload::Key { key, modifiers, .. } = &ev.payload
            {
                let has_primary = (*modifiers & (NVE_KEY_MOD_META | NVE_KEY_MOD_CTRL)) != 0;
                let is_cmd_k = has_primary && key.eq_ignore_ascii_case("k");
                if is_cmd_k {
                    if let Some(root_id) = docs_root_id {
                        ev.node_id = root_id;
                    }
                } else if let Some(input_id) = search_input_id {
                    ev.node_id = input_id;
                } else if let Some(root_id) = docs_root_id {
                    ev.node_id = root_id;
                }
            }
        }
    }

    fn pump_events(&mut self) -> Result<(), String> {
        let mut drained = self.backend.drain_events();
        if drained.is_empty() {
            return Ok(());
        }
        self.retarget_root_keydown(&mut drained);
        if self.surface.dispatch_events(&drained) {
            self.render_next_frame()?;
            self.sync_browser_chrome(true);
        }
        Ok(())
    }

    fn rebuild_for_route_and_viewport(
        &mut self,
        route: &str,
        viewport: DocsViewport,
    ) -> Result<(), String> {
        let dark = self.current_dark_mode();
        self.viewport = viewport;
        self.dark_mode = dark;
        self.last_route = DocsRoute::new(route).as_str().to_string();
        self.backend.reset();
        self.backend.set_viewport(
            DOCS_SURFACE_ID,
            Viewport {
                width: viewport.width,
                height: viewport.height,
                scale_factor: device_pixel_ratio(),
            },
        );
        self.surface = create_nve_docs_surface_full(
            DOCS_SURFACE_ID,
            embedded_compiled_site(),
            route,
            &self.base_path,
            viewport,
            dark,
        );
        self.render_next_frame()?;
        sync_document_theme(dark);
        sync_document_title(&self.last_route, &self.base_path);
        Ok(())
    }
}

struct ActiveDocsApp {
    _session: Rc<RefCell<DocsWebSession>>,
    container: web_sys::Element,
    link_interceptor: Closure<dyn FnMut(web_sys::MouseEvent)>,
    resize_listener: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for ActiveDocsApp {
    fn drop(&mut self) {
        let _ = self.container.remove_event_listener_with_callback_and_bool(
            "click",
            self.link_interceptor.as_ref().unchecked_ref(),
            true,
        );
        if let Some(window) = web_sys::window() {
            let _ = window.remove_event_listener_with_callback(
                "resize",
                self.resize_listener.as_ref().unchecked_ref(),
            );
        }
    }
}

thread_local! {
    static ACTIVE_DOCS_APP: RefCell<Option<ActiveDocsApp>> = const { RefCell::new(None) };
}

/// Handle returned by [`hydrate_docs_app`] for E2E telemetry & programmatic inspection.
#[wasm_bindgen]
pub struct DocsWebController {
    session: Rc<RefCell<DocsWebSession>>,
}

#[wasm_bindgen]
impl DocsWebController {
    #[wasm_bindgen(getter)]
    pub fn hydrated(&self) -> bool {
        self.session.borrow().hydration_report.adopted_nodes > 0
    }

    #[wasm_bindgen(getter)]
    pub fn adopted_nodes(&self) -> usize {
        self.session.borrow().hydration_report.adopted_nodes as usize
    }

    #[wasm_bindgen(getter)]
    pub fn mismatches(&self) -> usize {
        self.session.borrow().hydration_report.mismatches.len()
    }

    #[wasm_bindgen(getter)]
    pub fn replaced_nodes(&self) -> usize {
        self.session.borrow().hydration_report.replaced_nodes as usize
    }

    #[wasm_bindgen(getter)]
    pub fn seeded_rules(&self) -> usize {
        self.session.borrow().hydration_report.reused_css_rules as usize
    }

    #[wasm_bindgen(getter)]
    pub fn replayed_events(&self) -> usize {
        self.session.borrow().hydration_report.replayed_events as usize
    }

    #[wasm_bindgen(getter)]
    pub fn reconciled_dirty_controls(&self) -> usize {
        self.session
            .borrow()
            .hydration_report
            .reconciled_dirty_controls as usize
    }

    #[wasm_bindgen(getter)]
    pub fn current_route(&self) -> String {
        self.session.borrow().last_route.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn dark_mode(&self) -> bool {
        self.session.borrow().dark_mode
    }

    #[wasm_bindgen(getter)]
    pub fn viewport_mode(&self) -> String {
        self.session.borrow().viewport.mode_label().to_string()
    }

    pub fn node_count(&self) -> usize {
        self.session.borrow().surface.store().stats().live_nodes
    }
}

fn install_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "[nve-docs-site panic] {info}"
            )));
        }));
    });
}

fn device_pixel_ratio() -> f32 {
    web_sys::window()
        .map(|w| w.device_pixel_ratio() as f32)
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(1.0)
}

fn measure_browser_viewport(window: &web_sys::Window, container: &web_sys::Element) -> DocsViewport {
    let win_w = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(1280.0);
    let win_h = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(800.0);
    let rect = container.get_bounding_client_rect();
    let w = if rect.width().is_finite() && rect.width() > 0.0 {
        rect.width().min(win_w) as f32
    } else {
        win_w as f32
    };
    let h = if rect.height().is_finite() && rect.height() > 0.0 {
        rect.height().max(win_h) as f32
    } else {
        win_h as f32
    };
    DocsViewport::new(w, h)
}

fn reset_scroll_containers() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        for sel in [
            "[data-testid=\"docs-custom-scroll\"]",
            "[data-testid=\"docs-main-scroll\"]",
        ] {
            if let Ok(Some(el)) = doc.query_selector(sel) {
                el.set_scroll_top(0);
            }
        }
    }
}

fn sync_document_theme(dark: bool) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(root) = doc.document_element() {
            let _ = root.set_attribute("data-theme", if dark { "dark" } else { "light" });
        }
        if let Some(body) = doc.body() {
            let bg = if dark { "#09090b" } else { "#ffffff" };
            let fg = if dark { "#fafafa" } else { "#18181b" };
            let _ = body.set_attribute(
                "style",
                &format!("margin:0;background-color:{bg};color:{fg};"),
            );
        }
    }
}

fn sync_document_title(route: &str, base_path: &str) {
    let site = embedded_compiled_site();
    let cfg = nve_docs_site_config(base_path);
    let title = match route {
        "/" => format!("{} — Native-speed UI in Rust", cfg.site_title),
        "/examples" => format!("Examples & Live Playgrounds — {}", cfg.site_title),
        other => match cfg
            .content_route_for(other)
            .and_then(|cr| site.page(&cr))
        {
            Some(page) => format!("{} — {}", page.title, cfg.site_title),
            None => cfg.site_title,
        },
    };
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        doc.set_title(&title);
    }
}

/// Progressively hydrate the SSR-rendered `nve-docs-site` DOM inside `#<container_id>`.
#[wasm_bindgen]
pub fn hydrate_docs_app(container_id: &str, base_path: &str) -> Result<DocsWebController, JsValue> {
    install_panic_hook();

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("missing window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("missing document"))?;
    let container = document
        .get_element_by_id(container_id)
        .ok_or_else(|| JsValue::from_str(&format!("container '#{container_id}' not found")))?;

    let actual_vp = measure_browser_viewport(&window, &container);

    let history_backend = BrowserHistoryBackend::new()
        .map_err(|e| JsValue::from_str(&format!("BrowserHistoryBackend failed: {e}")))?;
    let history = WebHistoryAdapter::<DocsRoute, BrowserHistoryBackend>::new(base_path, history_backend);
    let initial_route = history
        .initial_route()
        .unwrap_or_else(DocsRoute::root);

    let mut backend = WebBackend::hydrate_into(container.clone());
    register_web_html_systems(backend.table_registry_mut());
    backend.register_table_system(CanvasWebSystem);
    backend.set_viewport(
        DOCS_SURFACE_ID,
        Viewport {
            width: actual_vp.width,
            height: actual_vp.height,
            scale_factor: device_pixel_ratio(),
        },
    );

    let surface = create_nve_docs_surface_full(
        DOCS_SURFACE_ID,
        embedded_compiled_site(),
        initial_route.as_str(),
        base_path,
        actual_vp,
        true,
    );

    let session = Rc::new(RefCell::new(DocsWebSession {
        base_path: base_path.to_string(),
        surface,
        backend,
        history,
        frame_id: FrameId::ZERO,
        viewport: actual_vp,
        dark_mode: true,
        last_route: initial_route.as_str().to_string(),
        hydration_report: HydrationReport::default(),
    }));

    // Wire reactive event callback from WebBackend's RootEventDelegator into DioxusSurface
    // using a Weak reference so `session -> backend -> on_event -> session` never leaks.
    let session_events = Rc::downgrade(&session);
    session
        .borrow_mut()
        .backend
        .set_on_event(move || {
            if let Some(session_events) = session_events.upgrade()
                && let Ok(mut guard) = session_events.try_borrow_mut()
                && let Err(err) = guard.pump_events()
            {
                web_sys::console::error_1(&JsValue::from_str(&format!("[nve-docs-site] {err}")));
            }
        });

    // Execute Frame 0 zero-CLS, zero-rebuild hydration directly at `actual_vp`, then immediately
    // pump any pre-hydration events drained atomically from `window.__nveReplayDrain()`.
    {
        let mut guard = session.borrow_mut();
        guard
            .render_next_frame()
            .map_err(|e| JsValue::from_str(&e))?;
        guard.hydration_report = guard.backend.hydration_report().clone();
        guard.sync_browser_chrome(false);
        guard
            .pump_events()
            .map_err(|e| JsValue::from_str(&e))?;
    }

    // Intercept same-origin internal `<a href>` clicks so Dioxus navigates via pushState without full reload.
    let session_links = Rc::downgrade(&session);
    let link_interceptor = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(
        move |ev: web_sys::MouseEvent| {
            let Some(session_links) = session_links.upgrade() else {
                return;
            };
            let Some(target_el) = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                .or_else(|| {
                    ev.target()
                        .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                        .and_then(|n| n.parent_element())
                })
            else {
                return;
            };
            let Ok(Some(anchor)) = target_el.closest("a[href]") else {
                return;
            };
            let Some(href) = anchor.get_attribute("href") else {
                return;
            };
            // Standalone lazy-loaded WASM example apps (/examples/studio/, /examples/kitchen-sink/)
            // must perform a real browser navigation so their independent WASM bundles boot cleanly.
            if anchor.get_attribute("data-nve-hard-nav").as_deref() == Some("true") {
                return;
            }
            let link_target = anchor.get_attribute("target");
            let intercepted = session_links
                .borrow()
                .history
                .should_intercept_link_click(
                    &href,
                    ev.ctrl_key(),
                    ev.meta_key(),
                    ev.shift_key(),
                    ev.alt_key(),
                    ev.button(),
                    link_target.as_deref(),
                );
            if let Some(route) = intercepted
                && !route.as_str().starts_with("/examples/")
            {
                ev.prevent_default();
            }
        },
    );
    let _ = container.add_event_listener_with_callback_and_bool(
        "click",
        link_interceptor.as_ref().unchecked_ref(),
        true,
    );

    // Register browser Back/Forward `popstate` handler.
    let session_pop = Rc::downgrade(&session);
    {
        let mut guard = session.borrow_mut();
        let _ = guard.history.backend_mut().register_popstate(move || {
            if let Some(session_pop) = session_pop.upgrade()
                && let Ok(mut s) = session_pop.try_borrow_mut()
                && let Some(route) = s.history.initial_route()
                && route.as_str() != s.last_route
            {
                let vp = s.viewport;
                if let Err(err) = s.rebuild_for_route_and_viewport(route.as_str(), vp) {
                    web_sys::console::error_1(&JsValue::from_str(&format!(
                        "[nve-docs-site popstate] {err}"
                    )));
                }
            }
        });
    }

    // Register responsive viewport resize handler (CSS `@media` adapts layout with zero rebuilds).
    let session_resize = Rc::downgrade(&session);
    let container_resize = container.clone();
    let resize_listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
        if let Some(win) = web_sys::window()
            && let Some(session_resize) = session_resize.upgrade()
            && let Ok(mut s) = session_resize.try_borrow_mut()
        {
            let next_vp = measure_browser_viewport(&win, &container_resize);
            if next_vp != s.viewport {
                s.viewport = next_vp;
                s.backend.set_viewport(
                    DOCS_SURFACE_ID,
                    Viewport {
                        width: next_vp.width,
                        height: next_vp.height,
                        scale_factor: device_pixel_ratio(),
                    },
                );
            }
        }
    });
    let _ = window.add_event_listener_with_callback(
        "resize",
        resize_listener.as_ref().unchecked_ref(),
    );

    ACTIVE_DOCS_APP.with(|slot| {
        *slot.borrow_mut() = Some(ActiveDocsApp {
            _session: Rc::clone(&session),
            container,
            link_interceptor,
            resize_listener,
        });
    });

    Ok(DocsWebController { session })
}
