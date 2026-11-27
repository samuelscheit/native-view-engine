//! E2E-only (`issues-probes`) live-DOM probes for real Chrome: native `<dialog>` / `[popover]`
//! top-layer state (`:modal`, `:popover-open`, `closedby` backdrop fallback) and hydration onto a
//! mount container that IS the SSR root (`<div id="app" data-nve-id="1">`).

use nve_backend_web::WebBackend;
use nve_components::tables::{Accessibility, TextContent, register_core_tables};
use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore, Transaction};
use nve_web_html::{
    ClosedBy, HtmlSemantics, PopoverMode, SemanticElement, register_web_html_tables,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = "
export function nve_probe_native_closedby() {
  return 'closedBy' in HTMLDialogElement.prototype;
}
export function nve_probe_click(el, x, y) {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, clientX: x, clientY: y }));
}
")]
extern "C" {
    fn nve_probe_native_closedby() -> bool;
    fn nve_probe_click(el: &web_sys::Element, x: f64, y: f64);
}

type ProbeResult = Result<String, String>;

fn check(ok: bool, what: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what.to_owned()) }
}

fn document() -> Result<web_sys::Document, String> {
    web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| "missing document".to_owned())
}

fn body() -> Result<web_sys::HtmlElement, String> {
    document()?.body().ok_or_else(|| "missing body".to_owned())
}

fn new_store() -> NodeStore {
    register_core_tables();
    register_web_html_tables();
    NodeStore::new()
}

fn commit(
    web: &mut WebBackend,
    store: &mut NodeStore,
    build: impl FnOnce(&mut Transaction<'_>),
) -> Result<(), String> {
    let mut tx = store.tx();
    build(&mut tx);
    let mut changes = ChangeSet::default();
    tx.commit_into(&mut changes);
    web.commit(store, &changes)
        .map(|_| ())
        .map_err(|e| format!("commit: {e:?}"))
}

fn query(root: &web_sys::Element, selector: &str) -> Result<web_sys::Element, String> {
    root.query_selector(selector)
        .ok()
        .flatten()
        .ok_or_else(|| format!("missing {selector}"))
}

fn matches(el: &web_sys::Element, selector: &str) -> bool {
    el.matches(selector).unwrap_or(false)
}

fn report(result: ProbeResult) -> String {
    match result {
        Ok(detail) => format!("OK: {detail}"),
        Err(err) => format!("FAIL: {err}"),
    }
}

/// Drives `HtmlSemantics` + `Accessibility::modal` through a live mount and asserts the native
/// top layer follows: `:modal` after `open=true`, closed after `open=false`, non-modal `show()`,
/// `:popover-open` toggling, and the `closedby="any"` backdrop fallback (passive when native).
#[wasm_bindgen]
pub fn nve_probe_native_overlays() -> String {
    report(run_native_overlays())
}

fn run_native_overlays() -> ProbeResult {
    let host = document()?
        .create_element("div")
        .map_err(|_| "create host")?;
    body()?.append_child(&host).map_err(|_| "append host")?;
    let result = drive_native_overlays(&host);
    host.remove();
    result
}

fn drive_native_overlays(host: &web_sys::Element) -> ProbeResult {
    let mut web = WebBackend::mount_into(host.clone());
    nve_example_issues_client::register_web_systems(web.table_registry_mut());
    let mut store = new_store();
    let (dialog_n, popover_n) = (NodeId(2), NodeId(3));
    commit(&mut web, &mut store, |tx| {
        let _ = tx.create_at(NodeId::ROOT);
        let _ = tx.create_at(dialog_n);
        let _ = tx.create_at(popover_n);
        tx.edit::<HtmlSemantics>(dialog_n)
            .element(SemanticElement::Dialog)
            .open(true)
            .closed_by(ClosedBy::Any);
        tx.edit::<Accessibility>(dialog_n).modal(true);
        tx.edit::<TextContent>(dialog_n).text("Settings");
        tx.edit::<HtmlSemantics>(popover_n)
            .popover(PopoverMode::Auto)
            .data_nve_open(true);
        tx.set_children(NodeId::ROOT, &[dialog_n, popover_n]);
    })?;
    let dialog = query(host, "dialog")?;
    let popover = query(host, "[popover]")?;
    check(
        matches(&dialog, ":modal"),
        "new modal <dialog open> must match :modal (showModal, not the open attribute)",
    )?;
    check(
        matches(&popover, ":popover-open"),
        "data_nve_open popover must match :popover-open",
    )?;

    commit(&mut web, &mut store, |tx| {
        tx.edit::<HtmlSemantics>(dialog_n).open(false);
        tx.edit::<HtmlSemantics>(popover_n).data_nve_open(false);
    })?;
    check(
        !dialog.has_attribute("open") && !matches(&dialog, ":modal"),
        "open=false must close()",
    )?;
    check(
        !matches(&popover, ":popover-open"),
        "data_nve_open=false must hidePopover()",
    )?;

    commit(&mut web, &mut store, |tx| {
        tx.edit::<Accessibility>(dialog_n).modal(false);
        tx.edit::<HtmlSemantics>(dialog_n).open(true);
    })?;
    check(
        dialog.has_attribute("open") && !matches(&dialog, ":modal"),
        "non-modal open must show()",
    )?;

    commit(&mut web, &mut store, |tx| {
        tx.edit::<Accessibility>(dialog_n).modal(true);
        tx.edit::<HtmlSemantics>(dialog_n).open(false);
    })?;
    commit(&mut web, &mut store, |tx| {
        tx.edit::<HtmlSemantics>(dialog_n).open(true);
    })?;
    check(
        matches(&dialog, ":modal"),
        "reopened modal must match :modal",
    )?;
    // A synthetic click at the viewport corner targets the dialog itself (backdrop hit).
    nve_probe_click(&dialog, 1.0, 1.0);
    let native = nve_probe_native_closedby();
    if native {
        check(
            dialog.has_attribute("open"),
            "native closedBy: the fallback must stay passive",
        )?;
    } else {
        check(
            !dialog.has_attribute("open"),
            "no native closedBy: backdrop click must close",
        )?;
    }
    Ok(format!(
        "dialog :modal/close/show, popover open/hide, closedby fallback (native={native})"
    ))
}

/// Hydrates onto mount containers that ARE the SSR root: a matching `<div id data-nve-id="1">`
/// is adopted in place (stays connected, keeps its `id`, never appended into itself), and a
/// mismatched `<main id data-nve-id="1">` stays connected while the client root rebuilds inside.
#[wasm_bindgen]
pub fn nve_probe_container_root_hydration() -> String {
    report(run_container_root_hydration())
}

fn build_text_frame(tx: &mut Transaction<'_>, label: &str) {
    let _ = tx.create_at(NodeId::ROOT);
    let _ = tx.create_at(NodeId(2));
    tx.edit::<TextContent>(NodeId(2)).text(label);
    tx.set_children(NodeId::ROOT, &[NodeId(2)]);
}

fn run_container_root_hydration() -> ProbeResult {
    let mut server_store = new_store();
    let mut server = WebBackend::new();
    commit(&mut server, &mut server_store, |tx| {
        build_text_frame(tx, "server")
    })?;
    let ssr = server.render_ssr(&server_store);
    let staging = document()?
        .create_element("div")
        .map_err(|_| "create staging")?;
    staging.set_inner_html(&ssr.html);
    let ssr_root = query(&staging, "[data-nve-id=\"1\"]")?;
    let mut details = Vec::new();
    for tag in ["div", "main"] {
        let mount = document()?
            .create_element(tag)
            .map_err(|_| "create mount")?;
        let mount_id = format!("nve-probe-root-{tag}");
        let _ = mount.set_attribute("id", &mount_id);
        let _ = mount.set_attribute("data-nve-id", "1");
        mount.set_inner_html(&ssr_root.inner_html());
        body()?.append_child(&mount).map_err(|_| "append mount")?;
        let result = hydrate_container_root(&mount, &mount_id, tag);
        mount.remove();
        details.push(result?);
    }
    Ok(details.join("; "))
}

fn hydrate_container_root(mount: &web_sys::Element, mount_id: &str, tag: &str) -> ProbeResult {
    let parent = mount.parent_element();
    let mut client = WebBackend::hydrate_into(mount.clone());
    let mut store = new_store();
    commit(&mut client, &mut store, |tx| build_text_frame(tx, "client"))?;
    commit(&mut client, &mut store, |tx| {
        tx.edit::<TextContent>(NodeId(2)).text("client-2");
    })?;
    check(
        mount.is_connected(),
        &format!("<{tag}> mount must stay connected"),
    )?;
    check(
        mount.parent_element() == parent,
        &format!("<{tag}> mount must keep its parent"),
    )?;
    check(
        mount.id() == mount_id,
        &format!("<{tag}> mount must keep id=\"{mount_id}\""),
    )?;
    check(
        mount.text_content().as_deref() == Some("client-2"),
        &format!("<{tag}> must render client updates"),
    )?;
    let report = client.hydration_report();
    if tag == "div" {
        check(
            report.mismatches.is_empty(),
            "matching <div> root must adopt with 0 mismatches",
        )?;
        check(
            mount.get_attribute("data-nve-id").as_deref() == Some("1"),
            "adopted mount stays ROOT",
        )?;
    } else {
        let rebuilt = query(mount, ":scope > div[data-nve-id=\"1\"]")?;
        check(
            rebuilt.parent_element().as_ref() == Some(mount),
            "rebuilt root lives inside <main>",
        )?;
        check(
            !mount.has_attribute("data-nve-id"),
            "demoted <main> drops its stale SSR id",
        )?;
    }
    Ok(format!(
        "<{tag}> adopted={} mismatches={}",
        report.adopted_nodes,
        report.mismatches.len()
    ))
}
