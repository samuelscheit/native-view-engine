//! E2E-only (`issues-probes`) live probe of a real `nve-ui` [`Dialog`] mounted through Dioxus:
//! a TRUSTED backdrop click (CDP `Input.dispatchMouseEvent`, driven by `run_e2e.mjs`) closes the
//! native modal `<dialog>`, the bridged `close` event drives `on_open_change(false)` into app
//! state, and reopening re-enters `:modal`. The `closedby="any"` fallback is proven with
//! SYNTHETIC clicks (never a native light dismiss) after `run_e2e.mjs` deletes
//! `HTMLDialogElement.prototype.closedBy`, observed through the fallback's close counter.

use std::cell::RefCell;

use nve_abi::SurfaceId;
use nve_backend_web::WebBackend;
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_engine::Viewport;
use nve_ui::prelude::*;
use wasm_bindgen::prelude::*;

use crate::session::WebAppSession;
use crate::wasm_dom_probe::{check, query, report};

#[wasm_bindgen(inline_js = "
export function nve_probe_overlay_click(el, x, y) {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, clientX: x, clientY: y }));
}
export function nve_probe_supports_closedby() {
  return 'closedBy' in HTMLDialogElement.prototype;
}
")]
extern "C" {
    fn nve_probe_overlay_click(el: &web_sys::Element, x: f64, y: f64);
    fn nve_probe_supports_closedby() -> bool;
}

struct MountedProbe {
    host: web_sys::Element,
    session: WebAppSession,
}

thread_local! {
    static PROBE: RefCell<Option<MountedProbe>> = const { RefCell::new(None) };
}

#[component]
fn ProbeDialogApp() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        ThemeProvider {
            theme: Theme::DARK,
            view {
                pressable {
                    test_id: "probe-dialog-reopen",
                    onpress: move |_| open.set(true),
                    text { "Reopen" }
                }
                text { test_id: "probe-dialog-state", if open() { "open" } else { "closed" } }
                Dialog {
                    open: open(),
                    test_id: "probe-dialog",
                    title: "Probe",
                    on_open_change: move |next: bool| open.set(next),
                    text { "Dialog body" }
                }
            }
        }
    }
}

fn with_probe<T>(f: impl FnOnce(&mut MountedProbe) -> Result<T, String>) -> Result<T, String> {
    PROBE.with(|p| match p.borrow_mut().as_mut() {
        Some(probe) => f(probe),
        None => Err("probe dialog not mounted".to_owned()),
    })
}

/// Whether a `:modal` dialog is shown + app state text of the mounted probe (a closed
/// non-`keep_mounted` `Dialog` unmounts its `<dialog>`).
fn dialog_state(host: &web_sys::Element) -> Result<(bool, String), String> {
    let modal = host
        .query_selector("dialog")
        .ok()
        .flatten()
        .is_some_and(|d| d.matches(":modal").unwrap_or(false));
    let state = query(host, "[data-testid=\"probe-dialog-state\"]")?;
    Ok((modal, state.text_content().unwrap_or_default()))
}

/// Mounts `ProbeDialogApp` (modal `Dialog` open) into a fresh `<div>` under `<body>`.
#[wasm_bindgen]
pub fn nve_probe_dialog_mount() -> String {
    report(mount())
}

fn mount() -> Result<String, String> {
    let doc = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("missing document")?;
    let host = doc.create_element("div").map_err(|_| "create host")?;
    doc.body()
        .ok_or("missing body")?
        .append_child(&host)
        .map_err(|_| "append host")?;
    let backend = WebBackend::mount_into(host.clone());
    let viewport = Viewport {
        width: 1280.0,
        height: 840.0,
        scale_factor: 1.0,
    };
    let mut session = WebAppSession::new(
        SurfaceId::ROOT,
        DioxusSurface::new(SurfaceId::ROOT, ProbeDialogApp),
        crate::register_issues_web_systems,
        backend,
        viewport,
    );
    session
        .render_initial()
        .map_err(|e| format!("render: {e:?}"))?;
    let (modal, state) = dialog_state(&host)?;
    check(
        modal && state == "open",
        "mounted Dialog must open in the top layer (:modal)",
    )?;
    PROBE.with(|p| *p.borrow_mut() = Some(MountedProbe { host, session }));
    Ok("Dialog mounted :modal".to_owned())
}

/// After the trusted backdrop click (and the queued `close` task) ran: the app state reads
/// closed, then the in-app reopen button re-enters `:modal`.
#[wasm_bindgen]
pub fn nve_probe_dialog_after_backdrop_click() -> String {
    report(with_probe(|probe| {
        probe
            .session
            .pump_events()
            .map_err(|e| format!("pump: {e:?}"))?;
        let (modal, state) = dialog_state(&probe.host)?;
        let still_open = probe
            .host
            .query_selector("dialog[open]")
            .ok()
            .flatten()
            .is_some();
        check(!modal && !still_open, "backdrop click must close the native dialog")?;
        check(
            state == "closed",
            &format!("app state must follow the native close, got {state:?}"),
        )?;

        nve_probe_overlay_click(
            &query(&probe.host, "[data-testid=\"probe-dialog-reopen\"]")?,
            0.0,
            0.0,
        );
        probe
            .session
            .pump_events()
            .map_err(|e| format!("pump: {e:?}"))?;
        let (modal, state) = dialog_state(&probe.host)?;
        check(
            modal && state == "open",
            "reopened Dialog must match :modal",
        )?;
        Ok("backdrop close -> app closed -> reopen :modal".to_owned())
    }))
}

/// Synthetic (untrusted) clicks on the open `<dialog>`: an in-box click never closes it; a
/// backdrop-corner click closes it through the `closedby="any"` fallback exactly when native
/// `closedBy` is unsupported (fallback counter +1), else the fallback stays passive.
#[wasm_bindgen]
pub fn nve_probe_dialog_synthetic_backdrop() -> String {
    report(with_probe(|probe| {
        let dialog = query(&probe.host, "dialog")?;
        check(
            dialog.matches(":modal").unwrap_or(false),
            "Dialog must be open before the synthetic clicks",
        )?;
        check(
            dialog.get_attribute("closedby").as_deref() == Some("any"),
            "Dialog renders closedby=\"any\"",
        )?;
        let native = nve_probe_supports_closedby();
        let before = nve_web_html_web::backdrop_fallback_close_count();
        let r = dialog.get_bounding_client_rect();
        let (cx, cy) = (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
        nve_probe_overlay_click(&dialog, cx, cy);
        check(
            dialog.has_attribute("open")
                && nve_web_html_web::backdrop_fallback_close_count() == before,
            "an in-box click must keep the dialog open",
        )?;
        nve_probe_overlay_click(&dialog, 4.0, 4.0);
        let closes = nve_web_html_web::backdrop_fallback_close_count() - before;
        if native {
            check(
                dialog.has_attribute("open") && closes == 0,
                "native closedBy: the fallback must stay passive",
            )?;
        } else {
            check(
                !dialog.has_attribute("open") && closes == 1,
                &format!("no native closedBy: the fallback must close once (closes={closes})"),
            )?;
        }
        Ok(format!(
            "synthetic backdrop: native={native}, fallback closes={closes}"
        ))
    }))
}

/// Unmounts the probe app and removes its host element.
#[wasm_bindgen]
pub fn nve_probe_dialog_unmount() {
    if let Some(probe) = PROBE.with(|p| p.borrow_mut().take()) {
        probe.host.remove();
    }
}
