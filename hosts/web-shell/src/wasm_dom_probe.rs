//! E2E-only (`issues-probes`) live-DOM probes for real Chrome: native `<dialog>` / `[popover]`
//! top-layer state (`:modal`, `:popover-open`, `closedby` backdrop fallback, modality flips,
//! moves, retags), plus re-exported hydration probes (`wasm_dom_hydration_probe`).

use nve_backend_web::WebBackend;
use nve_engine::StoreBackend;
use nve_store::{ChangeSet, NodeId, NodeStore, Transaction};
use nve_tables::tables::{Accessibility, TextContent, register_core_tables};
use nve_web_html::{
    ClosedBy, HtmlSemantics, PopoverMode, SemanticElement, register_web_html_tables,
};
use wasm_bindgen::prelude::*;

#[path = "wasm_dom_hydration_probe.rs"]
mod hydration;

pub use hydration::{nve_probe_container_root_hydration, nve_probe_select_hydration};

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
export function nve_probe_set_move_before(enabled) {
  const proto = Element.prototype;
  if (!enabled && Object.prototype.hasOwnProperty.call(proto, 'moveBefore')) {
    window.__nveMoveBefore = Object.getOwnPropertyDescriptor(proto, 'moveBefore');
    delete proto.moveBefore;
  } else if (enabled && window.__nveMoveBefore) {
    Object.defineProperty(proto, 'moveBefore', window.__nveMoveBefore);
    delete window.__nveMoveBefore;
  }
  return typeof proto.moveBefore === 'function';
}
export function nve_probe_reset_beforetoggle(el) {
  if (el.__nveBT === undefined) {
    el.addEventListener('beforetoggle', () => { el.__nveBT += 1; });
  }
  el.__nveBT = 0;
}
export function nve_probe_beforetoggle_count(el) {
  return el.__nveBT || 0;
}
")]
extern "C" {
    fn nve_probe_native_closedby() -> bool;
    fn nve_probe_click(el: &web_sys::Element, x: f64, y: f64);
    fn nve_probe_value(el: &web_sys::Element) -> String;
    /// Hides (`false`) or restores (`true`) `Element.prototype.moveBefore`; returns whether it
    /// is available afterwards.
    fn nve_probe_set_move_before(enabled: bool) -> bool;
    fn nve_probe_reset_beforetoggle(el: &web_sys::Element);
    fn nve_probe_beforetoggle_count(el: &web_sys::Element) -> u32;
}

pub(super) fn element_value(el: &web_sys::Element) -> String {
    nve_probe_value(el)
}

pub(crate) type ProbeResult = Result<String, String>;

pub(crate) fn check(ok: bool, what: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what.to_owned()) }
}

pub(crate) fn document() -> Result<web_sys::Document, String> {
    web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| "missing document".to_owned())
}

pub(crate) fn body() -> Result<web_sys::HtmlElement, String> {
    document()?.body().ok_or_else(|| "missing body".to_owned())
}

pub(crate) fn new_store() -> NodeStore {
    register_core_tables();
    register_web_html_tables();
    NodeStore::new()
}

pub(crate) fn commit(
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

pub(crate) fn query(root: &web_sys::Element, selector: &str) -> Result<web_sys::Element, String> {
    root.query_selector(selector)
        .ok()
        .flatten()
        .ok_or_else(|| format!("missing {selector}"))
}

pub(crate) fn matches(el: &web_sys::Element, selector: &str) -> bool {
    el.matches(selector).unwrap_or(false)
}

pub(crate) fn report(result: ProbeResult) -> String {
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
            // Manual: the fallback-move `showModal()` re-sync hides auto popovers (docs/UPSTREAM.md).
            .popover(Some(PopoverMode::Manual))
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
    let mut moves = String::new();
    for state_preserving in [true, false] {
        let available = nve_probe_set_move_before(state_preserving);
        let toggled = check(
            available == state_preserving,
            &format!("moveBefore toggle mismatch: requested={state_preserving}, available={available}"),
        );
        let result = toggled.and_then(|()| {
            drive_overlay_resync(&mut web, &mut store, host, &dialog, &popover, state_preserving)
        });
        nve_probe_set_move_before(true);
        result.map_err(|e| format!("{e} (moveBefore={available})"))?;
        moves.push_str(if available {
            "moveBefore "
        } else {
            "fallback "
        });
    }
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
        "dialog :modal/close/show, modality flips, moves + ancestor moves/retags ({}), retags, \
         popover open/hide, closedby fallback (native={native})",
        moves.trim_end()
    ))
}

/// Re-sync guards on the open modal `dialog` (node 2) and `popover` (node 3): (a) modal-only
/// edits while open, (b) reorders that really move the `<dialog>` and then the `[popover]`,
/// (d) moving and retagging an ancestor wrapper holding an open modal `<dialog>` + `[popover]`,
/// and (c) `<details open>` retagged to `<dialog>` (stale `open` dropped; modal enters
/// `:modal`). Runs with native `moveBefore()` and with it hidden (fallback subtree re-sync).
fn drive_overlay_resync(
    web: &mut WebBackend,
    store: &mut NodeStore,
    host: &web_sys::Element,
    dialog: &web_sys::Element,
    popover: &web_sys::Element,
    state_preserving: bool,
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

    let (sibling, auto_pop_n) = (NodeId::new(4, 0), NodeId::new(10, 0));
    commit(web, store, |tx| {
        tx.edit::<HtmlSemantics>(popover_n).data_nve_open(true);
        let _ = tx.create_at(sibling);
        tx.edit::<TextContent>(sibling).text("sibling");
        let _ = tx.create_at(auto_pop_n);
        tx.edit::<HtmlSemantics>(auto_pop_n)
            .popover(Some(PopoverMode::Auto))
            .data_nve_open(true);
        tx.set_children(NodeId::ROOT, &[sibling, dialog_n, popover_n, auto_pop_n]);
    })?;
    let auto_pop = query(host, &format!("[data-nve-id=\"{}\"]", auto_pop_n.raw()))?;
    check(
        matches(&auto_pop, ":popover-open"),
        "(b) canary auto popover must open before moveBefore checks",
    )?;
    nve_probe_reset_beforetoggle(popover);
    // The child-order cursor never moves the element it stands on: `dialog` must come first.
    for (order, label, check_popover_bt) in [
        ([dialog_n, sibling, popover_n, auto_pop_n], "(b) moved open modal <dialog>", false),
        ([popover_n, dialog_n, sibling, auto_pop_n], "(b) moved open [popover]", true),
    ] {
        commit(web, store, |tx| tx.set_children(NodeId::ROOT, &order))?;
        check(
            matches(dialog, ":modal") && matches(popover, ":popover-open"),
            &format!("{label}: both must stay in the top layer"),
        )?;
        if !check_popover_bt {
            let canary_open = matches(&auto_pop, ":popover-open");
            check(
                canary_open == state_preserving,
                &format!(
                    "{label}: canary auto popover expected open={state_preserving} (got {canary_open})"
                ),
            )?;
        } else {
            let bt = nve_probe_beforetoggle_count(popover);
            check(
                if state_preserving { bt == 0 } else { bt > 0 },
                &format!("{label}: expected beforetoggle {} (got {bt})", if state_preserving { "== 0" } else { "> 0" }),
            )?;
        }
    }
    drive_ancestor_resync(
        web,
        store,
        host,
        &[popover_n, dialog_n, sibling, auto_pop_n],
        &auto_pop,
        state_preserving,
    )?;
    commit(web, store, |tx| {
        tx.edit::<HtmlSemantics>(auto_pop_n).data_nve_open(false);
    })?;
    commit(web, store, |tx| {
        tx.remove(auto_pop_n);
        tx.remove(sibling);
        tx.set_children(NodeId::ROOT, &[dialog_n, popover_n]);
    })?;

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
        commit(web, store, |tx| tx.remove(n))?;
    }
    Ok(())
}

/// (d) A `<section>` wrapper holding an open modal `<dialog>` and an open `[popover]` (a
/// `Dialog`'s scrim `view` around its `<dialog>`): moving the wrapper and retagging it to
/// `<article>` keep both descendants in the top layer.
fn drive_ancestor_resync(
    web: &mut WebBackend,
    store: &mut NodeStore,
    host: &web_sys::Element,
    roots: &[NodeId],
    auto_pop: &web_sys::Element,
    state_preserving: bool,
) -> Result<(), String> {
    let (wrapper, inner_dialog, inner_popover) =
        (NodeId::new(7, 0), NodeId::new(8, 0), NodeId::new(9, 0));
    // Commit 1 builds the wrapper subtree unparented; Commit 2 attaches it with zero edits to the
    // overlays, proving attaching a disconnected subtree still runs Step-6 connected sync.
    let auto_pop_n = NodeId::new(10, 0);
    commit(web, store, |tx| {
        tx.edit::<HtmlSemantics>(auto_pop_n).data_nve_open(false);
        for n in [wrapper, inner_dialog, inner_popover] {
            let _ = tx.create_at(n);
        }
        tx.edit::<HtmlSemantics>(wrapper)
            .element(Some(SemanticElement::Section));
        tx.edit::<HtmlSemantics>(inner_dialog)
            .element(Some(SemanticElement::Dialog))
            .open(true);
        tx.edit::<Accessibility>(inner_dialog).modal(true);
        tx.edit::<HtmlSemantics>(inner_popover)
            .popover(Some(PopoverMode::Manual))
            .data_nve_open(true);
        tx.set_children(wrapper, &[inner_dialog, inner_popover]);
    })?;
    commit(web, store, |tx| tx.append_child(NodeId::ROOT, wrapper))?;
    let wrapper_sel = format!("[data-nve-id=\"{}\"]", wrapper.raw());
    let dialog_sel = format!("[data-nve-id=\"{}\"]", inner_dialog.raw());
    let popover_sel = format!("[data-nve-id=\"{}\"]", inner_popover.raw());
    let in_top_layer = |what: &str| -> Result<(), String> {
        let d = query(host, &dialog_sel)?;
        let p = query(host, &popover_sel)?;
        check(
            matches(&d, ":modal") && matches(&p, ":popover-open"),
            &format!("(d) {what}: nested <dialog> / [popover] must stay in the top layer"),
        )
    };
    in_top_layer("attached from unparented subtree")?;
    let inner_pop_el = query(host, &popover_sel)?;
    // Re-open the canary auto popover now that `inner_dialog` finished its initial `showModal()`,
    // calibrating both the `moveBefore` (stays open) and fallback `showModal()` (closes) paths.
    commit(web, store, |tx| {
        tx.edit::<HtmlSemantics>(auto_pop_n).data_nve_open(true);
    })?;
    check(matches(auto_pop, ":popover-open"), "(d) canary auto popover open")?;
    nve_probe_reset_beforetoggle(&inner_pop_el);
    let mut moved = vec![wrapper];
    moved.extend_from_slice(roots);
    commit(web, store, |tx| tx.set_children(NodeId::ROOT, &moved))?;
    in_top_layer("ancestor moved")?;
    let bt_move = nve_probe_beforetoggle_count(&inner_pop_el);
    let canary_after_move = matches(auto_pop, ":popover-open");
    check(
        (bt_move == 0) == state_preserving && canary_after_move == state_preserving,
        &format!(
            "(d) ancestor moved: state_preserving={state_preserving} beforetoggle={bt_move} \
             canary_open={canary_after_move}"
        ),
    )?;
    if !state_preserving {
        // Re-arm the canary after fallback move closed it so fallback retag proves it fires too.
        commit(web, store, |tx| {
            tx.edit::<HtmlSemantics>(auto_pop_n).data_nve_open(false);
        })?;
        commit(web, store, |tx| {
            tx.edit::<HtmlSemantics>(auto_pop_n).data_nve_open(true);
        })?;
        check(
            matches(auto_pop, ":popover-open"),
            "(d) canary auto popover re-armed before ancestor retag",
        )?;
    }
    nve_probe_reset_beforetoggle(&inner_pop_el);
    commit(web, store, |tx| {
        tx.edit::<HtmlSemantics>(wrapper)
            .element(Some(SemanticElement::Article));
    })?;
    check(
        query(host, &wrapper_sel)?
            .tag_name()
            .eq_ignore_ascii_case("article"),
        "(d) wrapper retagged to <article>",
    )?;
    in_top_layer("ancestor retagged")?;
    let bt_retag = nve_probe_beforetoggle_count(&inner_pop_el);
    let canary_after_retag = matches(auto_pop, ":popover-open");
    check(
        (bt_retag == 0) == state_preserving && canary_after_retag == state_preserving,
        &format!(
            "(d) ancestor retagged: state_preserving={state_preserving} beforetoggle={bt_retag} \
             canary_open={canary_after_retag}"
        ),
    )?;
    commit(web, store, |tx| {
        tx.edit::<HtmlSemantics>(inner_dialog).open(false);
        tx.edit::<HtmlSemantics>(inner_popover).data_nve_open(false);
    })?;
    commit(web, store, |tx| {
        tx.remove(wrapper);
        tx.set_children(NodeId::ROOT, roots);
    })
}
