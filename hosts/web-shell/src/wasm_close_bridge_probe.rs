//! E2E-only (`issues-probes`) live probe of the native `close` / `toggle` bridges in real
//! Chrome. `close` is dispatched from a queued task, so `run_e2e.mjs` lets a task run between
//! phases:
//!
//! 1. `arm`:
//!    - Raw `WebBackend`: an open modal `<dialog>` holds a nested open NVE `[popover]` and a
//!      non-NVE `<div popover="manual">`; two modality flips (`close()` + `show()` / `showModal()`)
//!      queue `close` events that find the dialog reopened.
//!    - Dioxus `nve-ui::Dialog`: an open modal `Dialog` holds an `onclose`-less nested NVE
//!      `[popover="manual"]`.
//! 2. `check`: zero bridged events from the modality flips; then hide the raw non-NVE popover,
//!    hide the NVE popovers, and close the raw `<dialog>`.
//! 3. `finish`:
//!    - Raw `WebBackend`: both NVE closes arrive target-only (`bubbles == false`,
//!      `NVE_EVENT_FLAG_NO_BUBBLE`), the non-NVE popover emits zero events, and the store stays
//!      open (the bridge never writes app state).
//!    - Dioxus `Dialog`: pumping the nested `onclose`-less popover's target-only `close` leaves
//!      the enclosing `Dialog` open and `:modal`.

use std::cell::RefCell;

use nve_abi::{EventBatchBuf, NodeId, SurfaceId};
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::{table_attr_fn, DioxusSurface};
use nve_engine::{StoreBackend, Viewport};
use nve_store::NodeStore;
use nve_tables::tables::Accessibility;
use nve_ui::prelude::*;
use nve_web_html::{HtmlSemantics, PopoverMode, SemanticElement};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::session::WebAppSession;
use crate::wasm_dom_probe::{
    ProbeResult, body, check, commit, document, matches, new_store, query, report,
};

#[wasm_bindgen(inline_js = "
export function nve_probe_show_popover(el) {
  el.showPopover();
}
export function nve_probe_hide_popover(el) {
  el.hidePopover();
}
")]
extern "C" {
    fn nve_probe_show_popover(el: &web_sys::Element);
    fn nve_probe_hide_popover(el: &web_sys::Element);
}

const DIALOG: NodeId = NodeId::new(2, 0);
const POPOVER: NodeId = NodeId::new(3, 0);

struct Armed {
    host: web_sys::Element,
    web: WebBackend,
    store: NodeStore,
    nested_host: web_sys::Element,
    nested_session: WebAppSession,
}

thread_local! {
    static ARMED: RefCell<Option<Armed>> = const { RefCell::new(None) };
}

#[component]
fn BareNestedPopoverDialogApp() -> Element {
    let mut open = use_signal(|| true);
    let bare_popover = table_attr_fn(move |tx, node| {
        tx.edit::<HtmlSemantics>(node)
            .popover(Some(PopoverMode::Manual))
            .data_nve_open(true);
    });
    rsx! {
        ThemeProvider {
            theme: Theme::DARK,
            view {
                text {
                    test_id: "cb-dialog-state",
                    if open() { "open" } else { "closed" }
                }
                Dialog {
                    open: open(),
                    keep_mounted: true,
                    test_id: "cb-dialog",
                    title: "Nested Close Probe",
                    on_open_change: move |next: bool| open.set(next),
                    // Nested overlay WITHOUT `onclose`: its target-only `toggle` close must never
                    // bubble into the enclosing `Dialog`'s `onclose`.
                    view {
                        test_id: "cb-bare-popover",
                        nve_table: bare_popover,
                        text { "bare" }
                    }
                }
            }
        }
    }
}

fn with_armed(f: impl FnOnce(&mut Armed) -> ProbeResult) -> ProbeResult {
    ARMED.with(|a| match a.borrow_mut().as_mut() {
        Some(armed) => f(armed),
        None => Err("close-bridge probe not armed".to_owned()),
    })
}

fn drain_custom_events(web: &mut WebBackend) -> Vec<(NodeId, String, bool)> {
    let mut batch = EventBatchBuf::new();
    web.drain_events(&mut batch);
    batch
        .as_validated()
        .iter()
        .filter(|ev| ev.is_custom())
        .map(|ev| (ev.node(), ev.custom_name().to_owned(), ev.bubbles()))
        .collect()
}

/// Phase 1: mount + modality flips while open + mount Dioxus Dialog with bare nested popover.
#[wasm_bindgen]
pub fn nve_probe_close_bridge_arm() -> String {
    report(arm())
}

fn arm() -> ProbeResult {
    let doc = document()?;
    let body_el = body()?;

    let host = doc.create_element("div").map_err(|_| "create host")?;
    body_el.append_child(&host).map_err(|_| "append host")?;
    let mut web = WebBackend::mount_into(host.clone());
    nve_example_issues_client::register_web_systems(web.table_registry_mut());
    let mut store = new_store();
    commit(&mut web, &mut store, |tx| {
        let _ = tx.create_at(NodeId::ROOT);
        let _ = tx.create_at(DIALOG);
        let _ = tx.create_at(POPOVER);
        tx.edit::<HtmlSemantics>(DIALOG)
            .element(Some(SemanticElement::Dialog))
            .open(true);
        tx.edit::<Accessibility>(DIALOG).modal(true);
        tx.edit::<HtmlSemantics>(POPOVER)
            .popover(Some(PopoverMode::Manual))
            .data_nve_open(true);
        tx.set_children(DIALOG, &[POPOVER]);
        tx.set_children(NodeId::ROOT, &[DIALOG]);
    })?;
    let dialog = query(&host, "dialog")?;
    check(matches(&dialog, ":modal"), "dialog opens modally")?;
    for modal in [false, true] {
        commit(&mut web, &mut store, |tx| {
            tx.edit::<Accessibility>(DIALOG).modal(modal);
        })?;
        check(
            dialog.has_attribute("open") && matches(&dialog, ":modal") == modal,
            &format!("modality flip to modal={modal} keeps the dialog open"),
        )?;
    }
    check(
        matches(&query(&host, "[popover]")?, ":popover-open"),
        "nested popover open",
    )?;
    // Non-NVE popover inside `dialog` (no `data-nve-id`): closing it must emit zero events.
    let raw_pop = doc.create_element("div").map_err(|_| "create raw pop")?;
    let _ = raw_pop.set_attribute("popover", "manual");
    let _ = raw_pop.set_attribute("data-probe-raw-popover", "1");
    dialog.append_child(&raw_pop).map_err(|_| "append raw pop")?;
    nve_probe_show_popover(&raw_pop);
    check(matches(&raw_pop, ":popover-open"), "raw non-NVE popover open")?;
    let _ = drain_custom_events(&mut web);

    // Dioxus `Dialog` holding an `onclose`-less nested `[popover="manual"]`.
    let nested_host = doc.create_element("div").map_err(|_| "create nested host")?;
    body_el
        .append_child(&nested_host)
        .map_err(|_| "append nested host")?;
    let nested_backend = WebBackend::mount_into(nested_host.clone());
    let viewport = Viewport {
        width: 1280.0,
        height: 840.0,
        scale_factor: 1.0,
    };
    let mut nested_session = WebAppSession::new(
        SurfaceId::ROOT,
        DioxusSurface::new(SurfaceId::ROOT, BareNestedPopoverDialogApp),
        crate::register_issues_web_systems,
        nested_backend,
        viewport,
    );
    nested_session
        .render_initial()
        .map_err(|e| format!("nested render: {e:?}"))?;
    check(
        matches(&query(&nested_host, "dialog")?, ":modal")
            && matches(
                &query(&nested_host, "[data-testid=\"cb-bare-popover\"]")?,
                ":popover-open",
            ),
        "Dioxus Dialog and nested bare popover must both open in the top layer",
    )?;

    ARMED.with(|a| {
        *a.borrow_mut() = Some(Armed {
            host,
            web,
            store,
            nested_host,
            nested_session,
        })
    });
    Ok("modal dialog + nested popovers open, modality flipped twice".to_owned())
}

/// Phase 2 (after the queued `close` tasks ran): flips bridged nothing; then close natively.
#[wasm_bindgen]
pub fn nve_probe_close_bridge_check() -> String {
    report(with_armed(|armed| {
        let customs = drain_custom_events(&mut armed.web);
        check(
            customs.is_empty(),
            &format!("modality flips must bridge zero close events, got {customs:?}"),
        )?;
        nve_probe_hide_popover(&query(&armed.host, "[data-probe-raw-popover]")?);
        nve_probe_hide_popover(&query(
            &armed.host,
            &format!("[data-nve-id=\"{}\"]", POPOVER.raw()),
        )?);
        let dialog = query(&armed.host, "dialog")?;
        dialog
            .dyn_ref::<web_sys::HtmlDialogElement>()
            .ok_or("not a <dialog>")?
            .close();
        // Hide the bare `onclose`-less popover inside the Dioxus `Dialog`.
        nve_probe_hide_popover(&query(
            &armed.nested_host,
            "[data-testid=\"cb-bare-popover\"]",
        )?);
        Ok("0 bridged events after 2 modality flips".to_owned())
    }))
}

/// Phase 3 (after native close / toggle tasks ran): target-only closes + Dioxus Dialog stays open.
#[wasm_bindgen]
pub fn nve_probe_close_bridge_finish() -> String {
    let result = with_armed(|armed| {
        let mut customs = drain_custom_events(&mut armed.web);
        customs.sort_by_key(|(id, _, _)| id.raw());
        let expected = vec![
            (DIALOG, "close".to_owned(), false),
            (POPOVER, "close".to_owned(), false),
        ];
        check(
            customs == expected,
            &format!(
                "expected exact target-only closes for dialog & NVE popover (0 for non-NVE \
                 popover), got {customs:?}"
            ),
        )?;
        // The store still says open: the app decides; the bridge only reports.
        check(
            armed
                .store
                .try_row::<HtmlSemantics>(DIALOG)
                .is_some_and(|s| s.open()),
            "the bridge never writes app state",
        )?;

        // Pumping the nested `onclose`-less popover's target-only `close` must NOT close Dialog.
        armed
            .nested_session
            .pump_events()
            .map_err(|e| format!("nested pump: {e:?}"))?;
        let state = query(&armed.nested_host, "[data-testid=\"cb-dialog-state\"]")?
            .text_content()
            .unwrap_or_default();
        check(
            state == "open" && matches(&query(&armed.nested_host, "dialog")?, ":modal"),
            &format!(
                "onclose-less nested popover toggle must not close enclosing Dialog (state={state:?})"
            ),
        )?;
        Ok("target-only closes verified; non-NVE ignored; enclosing Dialog stayed open".to_owned())
    });
    if let Some(armed) = ARMED.with(|a| a.borrow_mut().take()) {
        armed.host.remove();
        armed.nested_host.remove();
    }
    report(result)
}
