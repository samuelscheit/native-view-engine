//! Browser `wasm32` 0ms snapshot persistence lifecycle: `localStorage` state restore on boot,
//! debounced commit writes (200ms), and immediate flush on `visibilitychange` (hidden) and
//! `pagehide`.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nve_backend_web::{query_has_reset_param, WebSnapshotStore};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::session::WebAppSession;

const SNAPSHOT_DEBOUNCE_MS: i32 = 200;

type DebounceCallbackSlot = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

pub(crate) fn window_search(window: &web_sys::Window) -> String {
    window.location().search().unwrap_or_default()
}

pub(crate) fn query_has_reset(window: &web_sys::Window) -> bool {
    query_has_reset_param(&window_search(window))
}

pub(crate) fn query_has_ssr_demo(window: &web_sys::Window) -> bool {
    let search = window_search(window);
    let trimmed = search.trim_start_matches('?');
    trimmed.split('&').any(|pair| {
        pair.split_once('=')
            .is_some_and(|(k, v)| k == "ssr" && (v == "1" || v == "true"))
    })
}

pub(crate) fn load_persisted_state(
    app_slug: &str,
    schema_version: u32,
    is_reset: bool,
) -> Option<Vec<u8>> {
    let mut store = WebSnapshotStore::browser(app_slug, schema_version)?;
    if is_reset {
        store.clear();
        return None;
    }
    store.load_state()
}

pub(crate) fn clear_persisted_snapshot(app_slug: &str, schema_version: u32) {
    if let Some(mut store) = WebSnapshotStore::browser(app_slug, schema_version) {
        store.clear();
    }
}

pub(crate) fn collect_scroll_offsets(container: &web_sys::Element) -> Vec<(u32, u32)> {
    let mut offsets = Vec::new();
    let Ok(list) = container.query_selector_all(".nve-s") else {
        return offsets;
    };
    let len = list.length();
    for i in 0..len {
        if let Some(node) = list.item(i)
            && let Ok(el) = node.dyn_into::<web_sys::Element>()
        {
            let top = el.scroll_top();
            if top > 0 {
                offsets.push((i, top as u32));
            }
        }
    }
    offsets
}

fn persist_session_snapshot_inner(
    session: &mut WebAppSession,
    container: &web_sys::Element,
    last_hash: &Cell<u64>,
    force: bool,
) -> bool {
    let Some(slug) = session.app_slug() else {
        return false;
    };
    let schema_version = session.schema_version();
    let scroll_offsets = collect_scroll_offsets(container);
    let Some((state_bytes, fp)) = session.snapshot_fingerprint(&scroll_offsets) else {
        return false;
    };
    if !force && last_hash.get() == fp {
        return false;
    }
    let Some(payload) = session.render_canonical_snapshot_for_state(&state_bytes, scroll_offsets)
    else {
        return false;
    };
    let Some(mut store) = WebSnapshotStore::browser(slug, schema_version) else {
        return false;
    };
    match store.save_all(&payload, &state_bytes) {
        Ok(_) => {
            last_hash.set(fp);
            true
        }
        Err(_) => false,
    }
}

/// Shared handle used by event callbacks and `WebAppController` to schedule or flush snapshots.
#[derive(Clone)]
pub(crate) struct SnapshotTrigger {
    session: Weak<RefCell<WebAppSession>>,
    container: web_sys::Element,
    app_slug: &'static str,
    schema_version: u32,
    dirty: Rc<Cell<bool>>,
    last_hash: Rc<Cell<u64>>,
    timer_id: Rc<Cell<Option<i32>>>,
    timer_cb: DebounceCallbackSlot,
}

impl SnapshotTrigger {
    /// Marks state dirty and schedules a debounced snapshot save (`200ms` after the latest commit).
    pub(crate) fn schedule_debounced(&self) {
        self.dirty.set(true);
        let Some(window) = web_sys::window() else {
            return;
        };
        if let Some(id) = self.timer_id.take() {
            window.clear_timeout_with_handle(id);
        }
        let session = Weak::clone(&self.session);
        let container = self.container.clone();
        let last_hash = Rc::clone(&self.last_hash);
        let timer_id = Rc::clone(&self.timer_id);
        let cb = Closure::<dyn FnMut()>::new(move || {
            timer_id.set(None);
            if let Some(session) = session.upgrade()
                && let Ok(mut guard) = session.try_borrow_mut()
            {
                let _ = persist_session_snapshot_inner(&mut guard, &container, &last_hash, false);
            }
        });
        if let Ok(id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            SNAPSHOT_DEBOUNCE_MS,
        ) {
            self.timer_id.set(Some(id));
            *self.timer_cb.borrow_mut() = Some(cb);
        }
    }

    /// Immediately persists the latest canonical snapshot + binary state if dirty (or `force`).
    pub(crate) fn flush_now(&self, force: bool) -> bool {
        if let Some(id) = self.timer_id.take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(id);
        }
        *self.timer_cb.borrow_mut() = None;
        if !force && !self.dirty.get() {
            return false;
        }
        let Some(session) = self.session.upgrade() else {
            return false;
        };
        let Ok(mut guard) = session.try_borrow_mut() else {
            return false;
        };
        let saved = persist_session_snapshot_inner(&mut guard, &self.container, &self.last_hash, force);
        if saved {
            self.dirty.set(true);
        }
        saved
    }

    /// Cancels any pending write and clears all stored snapshot/state entries for this app.
    pub(crate) fn clear(&self) {
        if let Some(id) = self.timer_id.take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(id);
        }
        *self.timer_cb.borrow_mut() = None;
        self.dirty.set(false);
        self.last_hash.set(0);
        clear_persisted_snapshot(self.app_slug, self.schema_version);
    }
}

/// Owns `visibilitychange` + `pagehide` browser listeners and cleans them up on drop/unmount.
pub(crate) struct SnapshotPersistenceGuard {
    trigger: SnapshotTrigger,
    on_visibility: Option<Closure<dyn FnMut(web_sys::Event)>>,
    on_pagehide: Option<Closure<dyn FnMut(web_sys::Event)>>,
}

impl SnapshotPersistenceGuard {
    pub(crate) fn install(
        window: &web_sys::Window,
        session: &Rc<RefCell<WebAppSession>>,
        container: web_sys::Element,
        initial_dirty: bool,
    ) -> Option<(Self, SnapshotTrigger)> {
        let (app_slug, schema_version) = {
            let guard = session.borrow();
            (guard.app_slug()?, guard.schema_version())
        };
        let trigger = SnapshotTrigger {
            session: Rc::downgrade(session),
            container,
            app_slug,
            schema_version,
            dirty: Rc::new(Cell::new(initial_dirty)),
            last_hash: Rc::new(Cell::new(0)),
            timer_id: Rc::new(Cell::new(None)),
            timer_cb: Rc::new(RefCell::new(None)),
        };

        let trig_vis = trigger.clone();
        let on_visibility = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
            if let Some(doc) = web_sys::window().and_then(|w| w.document())
                && doc.visibility_state() == web_sys::VisibilityState::Hidden
            {
                let _ = trig_vis.flush_now(false);
            }
        });
        let registered_vis = window
            .document()
            .and_then(|doc| {
                doc.add_event_listener_with_callback(
                    "visibilitychange",
                    on_visibility.as_ref().unchecked_ref(),
                )
                .ok()
            })
            .is_some()
            .then_some(on_visibility);

        let trig_hide = trigger.clone();
        let on_pagehide = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
            let _ = trig_hide.flush_now(false);
        });
        let registered_hide = window
            .add_event_listener_with_callback("pagehide", on_pagehide.as_ref().unchecked_ref())
            .is_ok()
            .then_some(on_pagehide);

        let guard = Self {
            trigger: trigger.clone(),
            on_visibility: registered_vis,
            on_pagehide: registered_hide,
        };
        Some((guard, trigger))
    }
}

impl Drop for SnapshotPersistenceGuard {
    fn drop(&mut self) {
        if let Some(id) = self.trigger.timer_id.take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(id);
        }
        *self.trigger.timer_cb.borrow_mut() = None;
        if let Some(window) = web_sys::window() {
            if let Some(on_vis) = self.on_visibility.take()
                && let Some(doc) = window.document()
            {
                let _ = doc.remove_event_listener_with_callback(
                    "visibilitychange",
                    on_vis.as_ref().unchecked_ref(),
                );
            }
            if let Some(on_hide) = self.on_pagehide.take() {
                let _ = window.remove_event_listener_with_callback(
                    "pagehide",
                    on_hide.as_ref().unchecked_ref(),
                );
            }
        }
    }
}
