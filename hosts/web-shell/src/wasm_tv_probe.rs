//! E2E-only (`issues-probes`) live probe of a real `nve_tv_web::WebTvController` on a mounted
//! `WebBackend`: `run_e2e.mjs` sends TRUSTED CDP keys, so the backend's window `keydown`
//! listener (surface scoping, `owned_arrows`, `target_node`), the controller, and its DOM layer
//! (`dom.rs`: just-in-time rects, traps, focus, scroll, Select) all run for real.
//!
//! The fixture is plain HTML with `data-nve-id` attributes: the controller only reads the DOM.
//! The probe also checks re-entrancy (a `focusin` listener and the action handler use the
//! controller while it dispatches) and that `uninstall` releases the controller (no `Rc` cycle).

use std::cell::{Cell, RefCell};

use nve_backend_web::WebBackend;
use nve_tv_web::{WeakWebTvController, WebTvConfig, WebTvController};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

use crate::wasm_dom_probe::report;

/// Rows (two focus groups, one explicit `data-nve-next-down`, a disabled and an invisible
/// button), an ARIA tablist, a native `<select>` beside a button, a modal `<dialog>` with
/// `tabindex=-1` (never Select-clicked), and a hidden focus trap that must not shadow the page.
const FIXTURE: &str = r#"<div data-nve-id="100" style="padding:8px">
<style>#nve-tv-probe button,#nve-tv-probe select{width:90px;height:36px;margin:4px}</style>
<div data-nve-focus-group="row1" style="display:flex">
  <button data-nve-id="101" id="tv-a">A</button>
  <button data-nve-id="102" id="tv-b" data-nve-next-down="tv-f">B</button>
  <button data-nve-id="103" id="tv-c">C</button>
  <button data-nve-id="104" id="tv-disabled" disabled>X</button>
  <button data-nve-id="105" id="tv-invisible" style="visibility:hidden">Y</button>
</div>
<div data-nve-focus-group="row2" style="display:flex">
  <button data-nve-id="111" id="tv-d">D</button>
  <button data-nve-id="112" id="tv-e">E</button>
  <button data-nve-id="113" id="tv-f">F</button>
</div>
<div role="tablist" style="display:flex">
  <button data-nve-id="121" id="tv-tab1" role="tab">T1</button>
  <button data-nve-id="122" id="tv-tab2" role="tab">T2</button>
</div>
<div style="display:flex">
  <select data-nve-id="131" id="tv-select"><option>1</option><option>2</option><option>3</option></select>
  <button data-nve-id="141" id="tv-open">Open</button>
</div>
<dialog data-nve-id="150" id="tv-dialog" tabindex="-1">
  <button data-nve-id="151" id="tv-ok">OK</button>
  <button data-nve-id="152" id="tv-cancel">Cancel</button>
</dialog>
<div data-nve-focus-trap hidden><button data-nve-id="160">hidden trap</button></div>
</div>"#;

#[derive(Default)]
struct Reentry {
    /// `focusin` listener calls that found the controller free / busy.
    focusin_free: Cell<u32>,
    focusin_busy: Cell<u32>,
    /// Actions seen by the handler, each with whether `with_engine` was free inside it.
    actions: RefCell<Vec<(String, bool)>>,
}

struct MountedTv {
    host: web_sys::Element,
    backend: WebBackend,
    tv: WebTvController,
    on_focus: Closure<dyn FnMut(web_sys::Event)>,
}

thread_local! {
    static TV: RefCell<Option<MountedTv>> = const { RefCell::new(None) };
    static REENTRY: Reentry = Reentry::default();
}

/// Mounts the fixture under `<body>` in a fresh `WebBackend` with a `WebTvController` installed.
#[wasm_bindgen]
pub fn nve_probe_tv_mount() -> String {
    report(mount())
}

fn mount() -> Result<String, String> {
    let doc = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("missing document")?;
    let host = doc.create_element("div").map_err(|_| "create host")?;
    host.set_id("nve-tv-probe");
    doc.body()
        .ok_or("missing body")?
        .append_child(&host)
        .map_err(|_| "append host")?;
    let mut backend = WebBackend::mount_into(host.clone());
    host.set_inner_html(FIXTURE);
    let tv = WebTvController::install(&mut backend, WebTvConfig::detect());
    let weak = tv.downgrade();
    tv.set_action_handler(move |action, _node| {
        let engine_free = weak
            .upgrade()
            .and_then(|tv| tv.with_engine(|e| e.len()))
            .is_some();
        REENTRY.with(|r| {
            r.actions
                .borrow_mut()
                .push((format!("{action:?}"), engine_free))
        });
        false
    });
    let focus_probe: WeakWebTvController = tv.downgrade();
    let on_focus = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        let free = focus_probe
            .upgrade()
            .and_then(|tv| tv.with_engine(|e| e.focused()))
            .is_some();
        REENTRY.with(|r| {
            let counter = if free {
                &r.focusin_free
            } else {
                &r.focusin_busy
            };
            counter.set(counter.get() + 1);
        });
    });
    doc.add_event_listener_with_callback("focusin", on_focus.as_ref().unchecked_ref())
        .map_err(|_| "focusin listener")?;
    TV.with(|t| {
        *t.borrow_mut() = Some(MountedTv {
            host,
            backend,
            tv,
            on_focus,
        })
    });
    Ok(format!(
        "WebTvController mounted ({:?})",
        nve_tv_web::platform::detect_platform()
    ))
}

/// Re-entrancy summary: `focusin` listeners fired by the controller's own `focus()` and the
/// action handler both found the controller's state free.
#[wasm_bindgen]
pub fn nve_probe_tv_reentrancy() -> String {
    report(REENTRY.with(|r| {
        let (free, busy) = (r.focusin_free.get(), r.focusin_busy.get());
        let actions = r.actions.borrow();
        if busy != 0 {
            return Err(format!(
                "{busy} focusin listener call(s) found the controller borrowed"
            ));
        }
        if free == 0 {
            return Err("no focusin observed".to_owned());
        }
        if let Some((action, _)) = actions.iter().find(|(_, free)| !free) {
            return Err(format!("handler for {action} found the engine borrowed"));
        }
        let names: Vec<&str> = actions.iter().map(|(a, _)| a.as_str()).collect();
        Ok(format!(
            "{free} re-entrant focusin calls, handler saw {names:?}"
        ))
    }))
}

/// Uninstalls the controller, removes the fixture, and checks nothing keeps it alive.
#[wasm_bindgen]
pub fn nve_probe_tv_unmount() -> String {
    report(TV.with(|t| t.borrow_mut().take()).map_or_else(
        || Err("TV probe not mounted".to_owned()),
        |mut mounted| {
            let probe = mounted.tv.downgrade();
            mounted.tv.uninstall(&mut mounted.backend);
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                let _ = doc.remove_event_listener_with_callback(
                    "focusin",
                    mounted.on_focus.as_ref().unchecked_ref(),
                );
            }
            mounted.host.remove();
            drop(mounted);
            if probe.upgrade().is_some() {
                return Err("controller leaked after uninstall".to_owned());
            }
            Ok("uninstalled, controller freed".to_owned())
        },
    ))
}
