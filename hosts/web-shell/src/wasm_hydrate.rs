//! Browser `wasm32` bindings for zero-flicker SSR hydration (`hydrate_studio_workspace_app`,
//! `hydrate_counter_app`, `SsrRenderResult`), progressive island hydration
//! (`ProgressiveIslandHost`), and a live `RawStyleProps`/semantic HTML5 SSR probe.

use std::cell::{Cell, RefCell};
use nve_abi::hash::HashMap;
use std::rc::Rc;

use nve_abi::NodeId;
#[cfg(any(feature = "app-studio", feature = "app-counter"))]
use nve_abi::SurfaceId;
#[cfg(feature = "app-studio")]
use nve_backend_web::SsrDocumentOptions;
#[cfg(any(feature = "app-studio", feature = "app-counter"))]
use nve_backend_web::WebBackend;
#[cfg(any(feature = "app-studio", feature = "app-counter"))]
use nve_engine::Viewport;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::hydrate::{CapturedInteraction, HydrateTrigger, IslandHydrationScheduler};
#[cfg(any(feature = "app-studio", feature = "app-counter"))]
use crate::session::{WebAppSession, DEFAULT_SURFACE_ID};
#[cfg(any(feature = "app-studio", feature = "app-counter"))]
use crate::wasm_api::{start_surface_app_inner, WebAppController};

#[wasm_bindgen(inline_js = "
export function nve_intersecting_targets(entries) {
  const out = [];
  if (!entries) return out;
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    if (e && e.isIntersecting && e.target) out.push(e.target);
  }
  return out;
}

export function nve_schedule_idle(cb) {
  if (typeof requestIdleCallback === 'function') {
    requestIdleCallback(() => cb());
  } else {
    setTimeout(() => cb(), 1);
  }
}

export function nve_dispatch_hydrated_event(el, islandId, trigger) {
  if (!el) return;
  el.setAttribute('data-nve-hydrated', 'true');
  el.dispatchEvent(new CustomEvent('nve:island-hydrated', {
    bubbles: true,
    detail: { islandId, trigger },
  }));
}

export function nve_replay_interaction(target, eventType, clientX, clientY, key) {
  if (!target) return false;
  try {
    if (eventType === 'click') {
      target.dispatchEvent(new MouseEvent('click', {
        bubbles: true,
        cancelable: true,
        clientX,
        clientY,
      }));
      return true;
    }
    if (eventType === 'keydown') {
      target.dispatchEvent(new KeyboardEvent('keydown', {
        bubbles: true,
        cancelable: true,
        key: key || '',
      }));
      return true;
    }
    if (eventType === 'focusin') {
      if (typeof target.focus === 'function') target.focus();
      return true;
    }
    target.dispatchEvent(new Event(eventType, { bubbles: true, cancelable: true }));
    return true;
  } catch (_) {
    return false;
  }
}
")]
extern "C" {
    fn nve_intersecting_targets(entries: &JsValue) -> Vec<web_sys::Element>;
    fn nve_schedule_idle(cb: &JsValue);
    fn nve_dispatch_hydrated_event(el: &web_sys::Element, island_id: &str, trigger: &str);
    fn nve_replay_interaction(
        target: &web_sys::Element,
        event_type: &str,
        client_x: i32,
        client_y: i32,
        key: &str,
    ) -> bool;
}

/// Zero-JSON `wasm_bindgen` SSR bundle returned to browser JS / E2E harnesses.
#[wasm_bindgen]
pub struct SsrRenderResult {
    base_css: String,
    style_css: String,
    rule_hashes_csv: String,
    rule_count: usize,
    html: String,
    root_hash_hex: String,
}

impl SsrRenderResult {
    pub(crate) fn from_output(ssr: nve_backend_web::SsrOutput) -> Self {
        let rule_count = ssr.rule_hashes.len();
        let rule_hashes_csv = ssr.rule_hashes.join(",");
        Self {
            base_css: ssr.base_css.to_owned(),
            style_css: ssr.style_css,
            rule_hashes_csv,
            rule_count,
            html: ssr.html,
            root_hash_hex: format!("{:016x}", ssr.root_hash),
        }
    }
}

#[wasm_bindgen]
impl SsrRenderResult {
    #[wasm_bindgen(getter)]
    pub fn base_css(&self) -> String {
        self.base_css.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn style_css(&self) -> String {
        self.style_css.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn rule_hashes_csv(&self) -> String {
        self.rule_hashes_csv.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn rule_count(&self) -> usize {
        self.rule_count
    }

    #[wasm_bindgen(getter)]
    pub fn html(&self) -> String {
        self.html.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn root_hash_hex(&self) -> String {
        self.root_hash_hex.clone()
    }
}

/// Renders StudioWorkspace Frame 0 into a headless `WebBackend` and returns its Semantic HTML5 +
/// Option 5 Hybrid CSS SSR payload without touching the live DOM.
#[cfg(feature = "app-studio")]
#[wasm_bindgen]
pub fn render_studio_ssr_bundle() -> Result<SsrRenderResult, JsValue> {
    let vp = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 1.0,
    };
    let mut session = WebAppSession::new_studio_workspace(DEFAULT_SURFACE_ID, WebBackend::new(), vp);
    session
        .render_initial()
        .map_err(|e| JsValue::from_str(&format!("studio SSR render failed: {e}")))?;
    Ok(SsrRenderResult::from_output(session.backend().render_ssr()))
}

#[cfg(feature = "app-counter")]
#[wasm_bindgen]
pub fn render_counter_ssr_bundle() -> Result<SsrRenderResult, JsValue> {
    let mut session = WebAppSession::new_counter(DEFAULT_SURFACE_ID, WebBackend::new(), Viewport::default());
    session.render_initial().map_err(|e| JsValue::from_str(&format!("counter SSR render failed: {e}")))?;
    Ok(SsrRenderResult::from_output(session.backend().render_ssr()))
}

#[cfg(feature = "app-studio")]
#[wasm_bindgen]
pub fn render_studio_ssr_document(title: &str) -> Result<String, JsValue> {
    let vp = Viewport { width: 1280.0, height: 800.0, scale_factor: 1.0 };
    let mut session = WebAppSession::new_studio_workspace(DEFAULT_SURFACE_ID, WebBackend::new(), vp);
    session.render_initial().map_err(|e| JsValue::from_str(&format!("studio SSR document render failed: {e}")))?;
    Ok(session.backend().render_ssr_document(&SsrDocumentOptions { title: title.to_owned(), ..Default::default() }))
}

#[cfg(feature = "app-studio")]
#[wasm_bindgen]
pub fn hydrate_studio_workspace_app(mount_id: &str, ws_url: Option<String>) -> Result<WebAppController, JsValue> {
    hydrate_studio_workspace_app_on_surface(DEFAULT_SURFACE_ID, mount_id, ws_url)
}

#[cfg(feature = "app-studio")]
#[wasm_bindgen]
pub fn hydrate_studio_workspace_app_on_surface(
    surface_id: SurfaceId,
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_surface_app_inner("StudioWorkspaceApp", surface_id, mount_id, ws_url, true, WebAppSession::new_studio_workspace)
}

#[cfg(feature = "app-counter")]
#[wasm_bindgen]
pub fn hydrate_counter_app(mount_id: &str, ws_url: Option<String>) -> Result<WebAppController, JsValue> {
    hydrate_counter_app_on_surface(DEFAULT_SURFACE_ID, mount_id, ws_url)
}

#[cfg(feature = "app-counter")]
#[wasm_bindgen]
pub fn hydrate_counter_app_on_surface(
    surface_id: SurfaceId,
    mount_id: &str,
    ws_url: Option<String>,
) -> Result<WebAppController, JsValue> {
    start_surface_app_inner("CounterApp", surface_id, mount_id, ws_url, true, WebAppSession::new_counter)
}

struct InteractionCaptureHandle {
    el: web_sys::Element,
    on_click: Closure<dyn Fn(web_sys::MouseEvent)>,
    on_keydown: Closure<dyn Fn(web_sys::KeyboardEvent)>,
    on_focusin: Closure<dyn Fn(web_sys::FocusEvent)>,
}

impl Drop for InteractionCaptureHandle {
    fn drop(&mut self) {
        let _ = self.el.remove_event_listener_with_callback_and_bool(
            "click",
            self.on_click.as_ref().unchecked_ref(),
            true,
        );
        let _ = self.el.remove_event_listener_with_callback_and_bool(
            "keydown",
            self.on_keydown.as_ref().unchecked_ref(),
            true,
        );
        let _ = self.el.remove_event_listener_with_callback_and_bool(
            "focusin",
            self.on_focusin.as_ref().unchecked_ref(),
            true,
        );
    }
}

/// Browser `wasm32` controller driving progressive `[data-nve-island]` hydration triggers
/// (`load`, `visible` via `IntersectionObserver`, `interaction` with capture+replay, `idle`).
#[wasm_bindgen]
pub struct ProgressiveIslandHost {
    scheduler: Rc<RefCell<IslandHydrationScheduler>>,
    replayed_count: Rc<Cell<usize>>,
    observer: Option<web_sys::IntersectionObserver>,
    _on_intersect: Option<Closure<dyn FnMut(JsValue)>>,
    _idle_cb: Option<Closure<dyn FnMut()>>,
    _interaction_handles: Vec<InteractionCaptureHandle>,
}

impl Drop for ProgressiveIslandHost {
    fn drop(&mut self) {
        if let Some(obs) = self.observer.take() {
            obs.disconnect();
        }
    }
}

fn attach_interaction_trap(
    island_id: String,
    el: web_sys::Element,
    scheduler: Rc<RefCell<IslandHydrationScheduler>>,
    replayed_count: Rc<Cell<usize>>,
) -> InteractionCaptureHandle {
    let fire = |ev_type: &'static str,
                island_id: String,
                island_el: web_sys::Element,
                sched: Rc<RefCell<IslandHydrationScheduler>>,
                replayed: Rc<Cell<usize>>| {
        move |target_el: Option<web_sys::Element>,
              cx: i32,
              cy: i32,
              key: Option<String>,
              stop: &dyn Fn()| {
            if sched.borrow().is_hydrated(&island_id) {
                return;
            }
            let target_nid = target_el
                .as_ref()
                .and_then(|t| t.get_attribute("data-nve-id"))
                .and_then(|s| s.parse::<NodeId>().ok());
            let captured = CapturedInteraction {
                island_id: island_id.clone(),
                event_type: ev_type.to_owned(),
                target_node_id: target_nid,
                client_x: cx,
                client_y: cy,
                key: key.clone(),
            };
            let replays = sched.borrow_mut().on_interaction(captured);
            if let Some(replays) = replays {
                stop();
                nve_dispatch_hydrated_event(
                    &island_el,
                    &island_id,
                    HydrateTrigger::Interaction.as_str(),
                );
                if let Some(tgt) = target_el {
                    for rep in replays {
                        if nve_replay_interaction(
                            &tgt,
                            &rep.event_type,
                            rep.client_x,
                            rep.client_y,
                            rep.key.as_deref().unwrap_or(""),
                        ) {
                            replayed.set(replayed.get() + 1);
                        }
                    }
                }
            }
        }
    };

    let click_fn = fire(
        "click",
        island_id.clone(),
        el.clone(),
        Rc::clone(&scheduler),
        Rc::clone(&replayed_count),
    );
    let on_click = Closure::<dyn Fn(web_sys::MouseEvent)>::new(move |ev: web_sys::MouseEvent| {
        let tgt = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        click_fn(tgt, ev.client_x(), ev.client_y(), None, &|| {
            ev.stop_immediate_propagation();
            ev.prevent_default();
        });
    });

    let key_fn = fire(
        "keydown",
        island_id.clone(),
        el.clone(),
        Rc::clone(&scheduler),
        Rc::clone(&replayed_count),
    );
    let on_keydown =
        Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            let tgt = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
            key_fn(tgt, 0, 0, Some(ev.key()), &|| {
                ev.stop_immediate_propagation();
                ev.prevent_default();
            });
        });

    let focus_fn = fire("focusin", island_id, el.clone(), scheduler, replayed_count);
    let on_focusin =
        Closure::<dyn Fn(web_sys::FocusEvent)>::new(move |ev: web_sys::FocusEvent| {
            let tgt = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
            focus_fn(tgt, 0, 0, None, &|| {
                ev.stop_immediate_propagation();
            });
        });

    let _ = el.add_event_listener_with_callback_and_bool(
        "click",
        on_click.as_ref().unchecked_ref(),
        true,
    );
    let _ = el.add_event_listener_with_callback_and_bool(
        "keydown",
        on_keydown.as_ref().unchecked_ref(),
        true,
    );
    let _ = el.add_event_listener_with_callback_and_bool(
        "focusin",
        on_focusin.as_ref().unchecked_ref(),
        true,
    );

    InteractionCaptureHandle {
        el,
        on_click,
        on_keydown,
        on_focusin,
    }
}

#[wasm_bindgen]
impl ProgressiveIslandHost {
    pub fn attach(mount_id: &str) -> Result<ProgressiveIslandHost, JsValue> {
        let doc = web_sys::window()
            .and_then(|w| w.document())
            .ok_or_else(|| JsValue::from_str("missing document"))?;
        let container = doc
            .get_element_by_id(mount_id)
            .ok_or_else(|| JsValue::from_str(&format!("mount '#{mount_id}' not found")))?;

        let scheduler = Rc::new(RefCell::new(IslandHydrationScheduler::new()));
        let replayed_count = Rc::new(Cell::new(0usize));
        let island_elements: Rc<RefCell<HashMap<String, web_sys::Element>>> =
            Rc::new(RefCell::new(HashMap::default()));

        let mut visible_els = Vec::new();
        let mut interaction_els = Vec::new();
        let mut has_idle = false;

        let list = container
            .query_selector_all("[data-nve-island]")
            .map_err(|e| JsValue::from_str(&format!("query_selector_all failed: {e:?}")))?;
        for i in 0..list.length() {
            let Some(node) = list.item(i) else { continue };
            let Ok(el) = node.dyn_into::<web_sys::Element>() else {
                continue;
            };
            let island_id = el.get_attribute("data-nve-island").unwrap_or_default();
            if island_id.trim().is_empty() {
                continue;
            }
            let node_id = el
                .get_attribute("data-nve-id")
                .and_then(|s| s.parse::<NodeId>().ok())
                .unwrap_or(0);
            let trigger = el
                .get_attribute("data-nve-hydrate")
                .and_then(|s| HydrateTrigger::parse(&s))
                .unwrap_or(HydrateTrigger::Load);

            scheduler
                .borrow_mut()
                .register(island_id.clone(), node_id, trigger);
            island_elements
                .borrow_mut()
                .insert(island_id.clone(), el.clone());

            match trigger {
                HydrateTrigger::Load => {}
                HydrateTrigger::Visible => visible_els.push(el),
                HydrateTrigger::Interaction => interaction_els.push((island_id, el)),
                HydrateTrigger::Idle => has_idle = true,
            }
        }

        let loaded_ids = scheduler.borrow_mut().on_load();
        for id in loaded_ids {
            let el = island_elements.borrow().get(&id).cloned();
            if let Some(el) = el {
                nve_dispatch_hydrated_event(&el, &id, HydrateTrigger::Load.as_str());
            }
        }

        let (observer, on_intersect) = if !visible_els.is_empty() {
            let sched_vis = Rc::clone(&scheduler);
            let cb = Closure::<dyn FnMut(JsValue)>::new(move |entries: JsValue| {
                for target in nve_intersecting_targets(&entries) {
                    if let Some(id) = target.get_attribute("data-nve-island") {
                        let newly_hydrated = sched_vis.borrow_mut().on_visible(&id);
                        if newly_hydrated {
                            nve_dispatch_hydrated_event(
                                &target,
                                &id,
                                HydrateTrigger::Visible.as_str(),
                            );
                        }
                    }
                }
            });
            match web_sys::IntersectionObserver::new(cb.as_ref().unchecked_ref()) {
                Ok(obs) => {
                    for el in &visible_els {
                        obs.observe(el);
                    }
                    (Some(obs), Some(cb))
                }
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        };

        let interaction_handles = interaction_els
            .into_iter()
            .map(|(id, el)| {
                attach_interaction_trap(id, el, Rc::clone(&scheduler), Rc::clone(&replayed_count))
            })
            .collect();

        let idle_cb = if has_idle {
            let sched_idle = Rc::clone(&scheduler);
            let els_idle = Rc::clone(&island_elements);
            let cb = Closure::<dyn FnMut()>::new(move || {
                let idle_ids = sched_idle.borrow_mut().on_idle();
                for id in idle_ids {
                    let el = els_idle.borrow().get(&id).cloned();
                    if let Some(el) = el {
                        nve_dispatch_hydrated_event(&el, &id, HydrateTrigger::Idle.as_str());
                    }
                }
            });
            nve_schedule_idle(cb.as_ref());
            Some(cb)
        } else {
            None
        };

        Ok(ProgressiveIslandHost {
            scheduler,
            replayed_count,
            observer,
            _on_intersect: on_intersect,
            _idle_cb: idle_cb,
            _interaction_handles: interaction_handles,
        })
    }

    pub fn hydrated_count(&self) -> usize {
        self.scheduler.borrow().hydrated_count()
    }

    pub fn pending_count(&self) -> usize {
        self.scheduler.borrow().pending_count()
    }

    pub fn is_hydrated(&self, island_id: &str) -> bool {
        self.scheduler.borrow().is_hydrated(island_id)
    }

    pub fn replayed_event_count(&self) -> usize {
        self.replayed_count.get()
    }
}
