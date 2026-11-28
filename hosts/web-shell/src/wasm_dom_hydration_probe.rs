//! E2E-only (`issues-probes`) live-DOM hydration probes for real Chrome: `<select>` SSR
//! base-value parity (`SELECT_BASE_CASES`) and container-as-root hydration
//! (`<div id="app" data-nve-id="1">` adopted vs `<main data-nve-id="1">` demoted).

use nve_backend_web::select_parity::SELECT_BASE_CASES;
use nve_backend_web::{WebBackend, WebTableSystem};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot, Transaction};
use nve_tables::tables::{TextContent, TextInput};
use nve_web_html::HtmlDataAttrs;
use wasm_bindgen::prelude::*;

use super::{body, check, commit, document, element_value, new_store, query, report, ProbeResult};

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
/// `multiple`, `size`, disabled options / `<optgroup>`s) with a client value DIFFERENT from the
/// browser's own `select.value` (an equal one converges without consulting the base): 0 dirty
/// reconciliations then proves the SSR base value equals `select.value`.
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
    let browser = element_value(&select);
    check(
        browser == expected,
        &format!("Chrome select.value {browser:?}"),
    )?;
    let mut client = WebBackend::hydrate_into(mount.clone());
    client.register_table_system(SelectProbeSystem);
    let mut store = new_store();
    let select_n = NodeId::new(2, 0);
    let incoming = format!("{expected}__client");
    commit(&mut client, &mut store, |tx| {
        let _ = tx.create_at(NodeId::ROOT);
        let _ = tx.create_at(select_n);
        tx.edit::<HtmlDataAttrs>(select_n);
        tx.edit::<TextInput>(select_n).value(&incoming);
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
