//! E2E-only (`issues-probes`) live-DOM probes for real Chrome: native `<dialog>` / `[popover]`
//! top-layer state (`:modal`, `:popover-open`, `closedby` backdrop fallback, modality flips,
//! moves, retags), hydration onto a mount container that IS the SSR root
//! (`<div id="app" data-nve-id="1">`), and `<select>` SSR base-value parity.

use nve_backend_web::hydrate::SELECT_BASE_CASES;
use nve_backend_web::{WebBackend, WebTableSystem};
use nve_tables::tables::{Accessibility, TextContent, TextInput, register_core_tables};
use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, TableSlot, Transaction};
use nve_web_html::{
    ClosedBy, HtmlDataAttrs, HtmlSemantics, PopoverMode, SemanticElement, register_web_html_tables,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = "
export function nve_probe_native_closedby() {
  return 'closedBy' in HTMLDialogElement.prototype;
}
export function nve_probe_click(el, x, y) {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, clientX: x, clientY: y }));
}
export function nve_probe_value(el) {
  return el.value;
}
")]
extern "C" {
    fn nve_probe_native_closedby() -> bool;
    fn nve_probe_click(el: &web_sys::Element, x: f64, y: f64);
    fn nve_probe_value(el: &web_sys::Element) -> String;
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
    let (dialog_n, popover_n) = (NodeId::new(2, 0), NodeId::new(3, 0));
    commit(&mut web, &mut store, |tx| {
        let _ = tx.create_at(NodeId::ROOT);
        let _ = tx.create_at(dialog_n);
        let _ = tx.create_at(popover_n);
        tx.edit::<HtmlSemantics>(dialog_n)
            .element(Some(SemanticElement::Dialog))
            .open(true)
            .closed_by(Some(ClosedBy::Any));
        tx.edit::<Accessibility>(dialog_n).modal(true);
        tx.edit::<TextContent>(dialog_n).text("Settings");
        tx.edit::<HtmlSemantics>(popover_n)
            .popover(Some(PopoverMode::Auto))
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
    drive_overlay_resync(&mut web, &mut store, host, &dialog, &popover)?;
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
        "dialog :modal/close/show, modality flips, moves, retags, popover open/hide, closedby \
         fallback (native={native})"
    ))
}

/// Re-sync guards on the open modal `dialog` (node 2) and `popover` (node 3): (a) modal-only
/// edits while open, (b) a same-commit sibling insert + reorder that moves both elements, and
/// (c) `<details open>` retagged to `<dialog>` (stale `open` dropped; modal enters `:modal`).
fn drive_overlay_resync(
    web: &mut WebBackend,
    store: &mut NodeStore,
    host: &web_sys::Element,
    dialog: &web_sys::Element,
    popover: &web_sys::Element,
) -> Result<(), String> {
    let (dialog_n, popover_n) = (NodeId::new(2, 0), NodeId::new(3, 0));
    commit(web, store, |tx| {
        tx.edit::<Accessibility>(dialog_n).modal(false);
    })?;
    check(
        dialog.has_attribute("open") && !matches(dialog, ":modal"),
        "(a) modal-only edit true->false while open must show() non-modally",
    )?;
    commit(web, store, |tx| {
        tx.edit::<Accessibility>(dialog_n).modal(true);
    })?;
    check(
        matches(dialog, ":modal"),
        "(a) modal-only edit false->true while open must re-enter :modal",
    )?;

    commit(web, store, |tx| {
        tx.edit::<HtmlSemantics>(popover_n).data_nve_open(true);
    })?;
    let sibling = NodeId::new(4, 0);
    commit(web, store, |tx| {
        let _ = tx.create_at(sibling);
        tx.edit::<TextContent>(sibling).text("sibling");
        tx.set_children(NodeId::ROOT, &[sibling, popover_n, dialog_n]);
    })?;
    check(
        matches(dialog, ":modal") && matches(popover, ":popover-open"),
        "(b) a moved open modal <dialog> / [popover] must stay in the top layer",
    )?;

    for (n, open, label) in [
        (NodeId::new(5, 0), false, "closed"),
        (NodeId::new(6, 0), true, "open modal"),
    ] {
        commit(web, store, |tx| {
            let _ = tx.create_at(n);
            tx.edit::<HtmlSemantics>(n)
                .element(Some(SemanticElement::Details))
                .open(true);
            tx.edit::<Accessibility>(n).modal(true);
            tx.append_child(NodeId::ROOT, n);
        })?;
        let selector = format!("[data-nve-id=\"{}\"]", n.raw());
        check(
            query(host, &selector)?
                .matches("details[open]")
                .unwrap_or(false),
            "(c) <details open> renders open",
        )?;
        commit(web, store, |tx| {
            tx.edit::<HtmlSemantics>(n)
                .element(Some(SemanticElement::Dialog))
                .open(open);
        })?;
        let el = query(host, &selector)?;
        let ok = el.tag_name().eq_ignore_ascii_case("dialog")
            && el.has_attribute("open") == open
            && matches(&el, ":modal") == open;
        check(
            ok,
            &format!("(c) <details open> -> <dialog> {label}: no stale open mirror"),
        )?;
        commit(web, store, |tx| {
            tx.edit::<HtmlSemantics>(n).open(true);
        })?;
        check(
            matches(&query(host, &selector)?, ":modal"),
            "(c) retagged <dialog> opens modally",
        )?;
        commit(web, store, |tx| {
            tx.edit::<HtmlSemantics>(n).open(false);
        })?;
    }
    Ok(())
}

/// Test plugin rendering any `HtmlDataAttrs` node as `<select>` (`<select>` is plugin-only).
struct SelectProbeSystem;

impl WebTableSystem for SelectProbeSystem {
    fn slot(&self) -> TableSlot {
        HtmlDataAttrs::slot()
    }

    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        Some("select")
    }
}

/// Hydrates every shared `SELECT_BASE_CASES` `<select>` (with and without `selected`,
/// `multiple`, `size`, disabled options / `<optgroup>`s) with the client value equal to the
/// browser's own `select.value`: the SSR base must match it (0 dirty reconciliations).
#[wasm_bindgen]
pub fn nve_probe_select_hydration() -> String {
    report(run_select_hydration())
}

fn run_select_hydration() -> ProbeResult {
    for case in SELECT_BASE_CASES {
        let mount = document()?
            .create_element("div")
            .map_err(|_| "create mount")?;
        let _ = mount.set_attribute("data-nve-id", "1");
        mount.set_inner_html(&format!(
            "<select data-nve-id=\"2\" {}>{}</select>",
            case.attrs, case.options
        ));
        body()?.append_child(&mount).map_err(|_| "append mount")?;
        let result = hydrate_select_case(&mount, case.expected);
        mount.remove();
        result.map_err(|e| format!("{e} for {case:?}"))?;
    }
    Ok(format!(
        "{} <select> base-value cases",
        SELECT_BASE_CASES.len()
    ))
}

fn hydrate_select_case(mount: &web_sys::Element, expected: &str) -> Result<(), String> {
    let select = query(mount, "select")?;
    let browser = nve_probe_value(&select);
    check(
        browser == expected,
        &format!("Chrome select.value {browser:?}"),
    )?;
    let mut client = WebBackend::hydrate_into(mount.clone());
    client.register_table_system(SelectProbeSystem);
    let mut store = new_store();
    let select_n = NodeId::new(2, 0);
    commit(&mut client, &mut store, |tx| {
        let _ = tx.create_at(NodeId::ROOT);
        let _ = tx.create_at(select_n);
        tx.edit::<HtmlDataAttrs>(select_n);
        tx.edit::<TextInput>(select_n).value(expected);
        tx.set_children(NodeId::ROOT, &[select_n]);
    })?;
    let report = client.hydration_report();
    check(report.mismatches.is_empty(), "select must adopt")?;
    check(
        report.reconciled_dirty_controls == 0,
        "SSR base value differs from the browser's select.value (false dirty control)",
    )
}

/// Hydrates onto mount containers that ARE the SSR root: a matching `<div id data-nve-id="1">`
/// is adopted in place (stays connected, keeps its `id`, never appended into itself), and a
/// mismatched `<main id data-nve-id="1">` stays connected while the client root rebuilds inside.
#[wasm_bindgen]
pub fn nve_probe_container_root_hydration() -> String {
    report(run_container_root_hydration())
}

fn build_text_frame(tx: &mut Transaction<'_>, label: &str) {
    let text_n = NodeId::new(2, 0);
    let _ = tx.create_at(NodeId::ROOT);
    let _ = tx.create_at(text_n);
    tx.edit::<TextContent>(text_n).text(label);
    tx.set_children(NodeId::ROOT, &[text_n]);
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
        if tag == "main" {
            // Stale SSR root content the demotion must clear: direct text, `class`, `style`.
            let _ = mount.insert_adjacent_text("afterbegin", "stale-ssr-text");
            let _ = mount.set_attribute("class", "ssr-root");
            let _ = mount.set_attribute("style", "color:red");
        }
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
        tx.edit::<TextContent>(NodeId::new(2, 0)).text("client-2");
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
        check(
            !mount.has_attribute("class") && !mount.has_attribute("style"),
            "demoted <main> drops its SSR class/style",
        )?;
        check(
            mount
                .first_child()
                .is_some_and(|c| c.node_type() == web_sys::Node::ELEMENT_NODE),
            "demoted <main> drops its direct SSR text",
        )?;
    }
    Ok(format!(
        "<{tag}> adopted={} mismatches={}",
        report.adopted_nodes,
        report.mismatches.len()
    ))
}
