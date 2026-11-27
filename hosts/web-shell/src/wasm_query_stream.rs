//! Option B mid-stream async query hydration + post-stream Light-DOM fast path for
//! `<script type="application/x-nve-query" data-nve-seq="N">` SSR payloads.

use std::cell::RefCell;
use std::rc::Rc;

use base64::Engine as _;
use nve_abi::SurfaceId;
use nve_query_dioxus::ErasedQueryClient;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

pub(crate) use crate::query_stream_state::StreamIngestState;

const QUERY_SCRIPT_SELECTOR: &str = "script[type=\"application/x-nve-query\"]";
const STREAM_END_SELECTOR: &str = "template[data-nve-stream-end]";
const DSD_HOST_SELECTOR: &str = "[data-nve-dsd-host]";

fn warn(msg: &str) {
    web_sys::console::warn_1(&JsValue::from_str(&format!("[NVE QueryStream] {msg}")));
}

fn owner_or_window_document(el: &web_sys::Element) -> Option<web_sys::Document> {
    el.owner_document()
        .or_else(|| web_sys::window().and_then(|w| w.document()))
}

fn is_element_complete(el: &web_sys::Element, document: Option<&web_sys::Document>) -> bool {
    el.next_sibling().is_some() || document.is_some_and(|doc| doc.ready_state() != "loading")
}

fn remove_script_and_ack(script: &web_sys::Element, seq: Option<u64>) {
    let parent = script.parent_element();
    script.remove();
    if let (Some(parent), Some(seq)) = (parent, seq)
        && let Ok(acks) = parent.query_selector_all(&format!("template[data-nve-q-ack=\"{seq}\"]"))
    {
        let len = acks.length();
        for i in 0..len {
            if let Some(node) = acks.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
}

fn remove_stream_end_templates(document: &web_sys::Document) {
    if let Ok(list) = document.query_selector_all(STREAM_END_SELECTOR) {
        let len = list.length();
        for i in 0..len {
            if let Some(node) = list.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
}

/// Hydrates one completed `<script type="application/x-nve-query" data-nve-seq="N">` element
/// into `client` while enforcing monotonic sequencing, chunk-count cap, and byte cap.
///
/// Leaves incomplete `<script>` nodes untouched until the HTML5 parser finishes them.
pub(crate) fn hydrate_script_element(
    script: &web_sys::Element,
    client: &ErasedQueryClient,
    state: &mut StreamIngestState,
) -> bool {
    let document = owner_or_window_document(script);
    if !is_element_complete(script, document.as_ref()) {
        return false;
    }
    hydrate_completed_script_element(script, client, state)
}

fn hydrate_completed_script_element(
    script: &web_sys::Element,
    client: &ErasedQueryClient,
    state: &mut StreamIngestState,
) -> bool {
    let Some(seq) = script
        .get_attribute("data-nve-seq")
        .and_then(|raw| raw.trim().parse::<u64>().ok())
    else {
        warn("ignoring application/x-nve-query script with missing or invalid data-nve-seq");
        remove_script_and_ack(script, None);
        return false;
    };

    if let Err(reason) = state.check_header(seq) {
        warn(&format!("rejecting chunk seq={seq}: {reason}"));
        remove_script_and_ack(script, Some(seq));
        return false;
    }

    let raw_text = script.text_content().unwrap_or_default();
    let prepared = match state.prepare_base64(&raw_text) {
        Ok(b64) => b64,
        Err(reason) => {
            warn(&format!("rejecting chunk seq={seq}: {reason}"));
            remove_script_and_ack(script, Some(seq));
            return false;
        }
    };

    let decoded = match base64::engine::general_purpose::STANDARD.decode(prepared.as_ref()) {
        Ok(bytes) => bytes,
        Err(err) => {
            warn(&format!("rejecting chunk seq={seq}: invalid Base64 ({err})"));
            remove_script_and_ack(script, Some(seq));
            return false;
        }
    };

    if let Err(reason) = state.check_decoded_len(decoded.len()) {
        warn(&format!("rejecting chunk seq={seq}: {reason}"));
        remove_script_and_ack(script, Some(seq));
        return false;
    }

    remove_script_and_ack(script, Some(seq));

    let res = if seq == 0 {
        client.hydrate_bytes(&decoded)
    } else {
        client.hydrate_stream_chunk_bytes(&decoded)
    };
    match res {
        Ok(_) => {
            state.record_ingested(seq, decoded.len());
            true
        }
        Err(err) => {
            warn(&format!("failed to hydrate NVEQ chunk seq={seq}: {err}"));
            false
        }
    }
}

fn collect_scripts_in_element(root: &web_sys::Element, out: &mut Vec<web_sys::Element>) {
    if let Ok(list) = root.query_selector_all(QUERY_SCRIPT_SELECTOR) {
        let len = list.length();
        for i in 0..len {
            if let Some(node) = list.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                out.push(el);
            }
        }
    }
}

fn collect_query_scripts_from_document(
    document: &web_sys::Document,
    exclude_host: Option<&web_sys::Element>,
) -> Vec<web_sys::Element> {
    let mut out = Vec::new();
    if let Ok(list) = document.query_selector_all(QUERY_SCRIPT_SELECTOR) {
        let len = list.length();
        for i in 0..len {
            if let Some(node) = list.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
                && !exclude_host.is_some_and(|host| host.contains(Some(el.as_ref())))
            {
                out.push(el);
            }
        }
    }
    out
}

fn ingest_completed_scripts(
    scripts: Vec<(web_sys::Element, bool)>,
    client: &ErasedQueryClient,
    state: &mut StreamIngestState,
) {
    let mut valid: Vec<(u64, web_sys::Element)> = Vec::new();
    for (script, complete) in scripts {
        if !complete {
            continue;
        }
        match script
            .get_attribute("data-nve-seq")
            .and_then(|raw| raw.trim().parse::<u64>().ok())
        {
            Some(seq) => valid.push((seq, script)),
            None => {
                let _ = hydrate_completed_script_element(&script, client, state);
            }
        }
    }
    valid.sort_by_key(|(seq, _)| *seq);
    for (_, script) in valid {
        let _ = hydrate_completed_script_element(&script, client, state);
    }
}

fn drain_dsd_host(
    host: &web_sys::Element,
    document: &web_sys::Document,
    surface_id: SurfaceId,
    client: &ErasedQueryClient,
    state: &mut StreamIngestState,
    final_drain: bool,
) -> bool {
    let ready_done = document.ready_state() != "loading";
    let mut children: Vec<(web_sys::Element, bool)> = Vec::new();
    let mut cur = host.first_element_child();
    while let Some(el) = cur {
        let complete = ready_done || el.next_sibling().is_some();
        children.push((el.clone(), complete));
        cur = el.next_element_sibling();
    }

    let head = document
        .query_selector("head")
        .ok()
        .flatten()
        .or_else(|| document.document_element());

    let mut scripts: Vec<(web_sys::Element, bool)> = Vec::new();
    let mut slots_to_remove: Vec<web_sys::Element> = Vec::new();
    let mut saw_stream_end = false;

    for (el, complete) in children {
        let tag = el.tag_name();
        if tag.eq_ignore_ascii_case("template") && el.has_attribute("data-nve-stream-end") {
            saw_stream_end = true;
        } else if tag.eq_ignore_ascii_case("style") && el.has_attribute("data-nve-ssr-chunk") {
            if complete || final_drain {
                if let Some(hashes) = el.get_attribute("data-nve-hashes")
                    && !hashes.trim().is_empty()
                {
                    crate::wasm_api::seed_surface_ssr_hashes(surface_id, &hashes);
                }
                if let Some(ref target_head) = head {
                    let _ = target_head.append_child(&el);
                }
            }
        } else if el.has_attribute("data-nve-slot")
            || (tag.eq_ignore_ascii_case("div") && el.has_attribute("slot"))
        {
            slots_to_remove.push(el);
        } else if tag.eq_ignore_ascii_case("script")
            && el
                .get_attribute("type")
                .is_some_and(|t| t.eq_ignore_ascii_case("application/x-nve-query"))
        {
            scripts.push((el, complete));
        }
    }

    for slot in slots_to_remove {
        slot.remove();
    }

    if !scripts.is_empty() {
        ingest_completed_scripts(scripts, client, state);
    }

    saw_stream_end
}

struct StreamRuntime {
    state: StreamIngestState,
    finished: bool,
    surface_id: SurfaceId,
    host: Option<web_sys::Element>,
    document: Option<web_sys::Document>,
    client: Option<ErasedQueryClient>,
    observer: Option<web_sys::MutationObserver>,
    dom_ready_js: Option<JsValue>,
}

impl StreamRuntime {
    fn detach_listeners(&mut self) {
        if let Some(observer) = self.observer.take() {
            observer.disconnect();
        }
        if let Some(cb_js) = self.dom_ready_js.take()
            && let Some(doc) = self.document.as_ref()
        {
            let _ = doc.remove_event_listener_with_callback(
                "DOMContentLoaded",
                cb_js.unchecked_ref(),
            );
        }
    }

    fn on_mutation(&mut self) {
        if self.finished {
            return;
        }
        let Some(host) = self.host.clone() else {
            return;
        };
        let Some(doc) = self.document.clone() else {
            return;
        };
        let Some(client) = self.client.clone() else {
            return;
        };

        let saw_end = drain_dsd_host(&host, &doc, self.surface_id, &client, &mut self.state, false);
        let ended = saw_end
            || doc.ready_state() != "loading"
            || doc
                .query_selector(STREAM_END_SELECTOR)
                .ok()
                .flatten()
                .is_some();
        if ended {
            self.finish_stream();
        }
    }

    fn finish_stream(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.detach_listeners();

        let host = self.host.take();
        let document = self.document.take();
        let client = self.client.take();

        if let (Some(doc), Some(client)) = (document.as_ref(), client.as_ref()) {
            if let Some(ref host_el) = host {
                let _ = drain_dsd_host(
                    host_el,
                    doc,
                    self.surface_id,
                    client,
                    &mut self.state,
                    true,
                );
            }
            let extra = collect_query_scripts_from_document(doc, host.as_ref());
            if !extra.is_empty() {
                let with_status: Vec<(web_sys::Element, bool)> = extra
                    .into_iter()
                    .map(|el| {
                        let complete = is_element_complete(&el, Some(doc));
                        (el, complete)
                    })
                    .collect();
                ingest_completed_scripts(with_status, client, &mut self.state);
            }
            client.finish_ssr_stream();
        }
        if let Some(host_el) = host {
            host_el.remove();
        }
        if let Some(doc) = document.as_ref() {
            remove_stream_end_templates(doc);
        }
    }

    fn teardown_on_drop(&mut self) {
        self.finished = true;
        self.detach_listeners();
        self.host = None;
        self.document = None;
        self.client = None;
    }
}

/// Lifecycle guard owning mid-stream `MutationObserver` and `DOMContentLoaded` listeners until
/// stream completion or surface `unmount()`.
pub(crate) struct QueryStreamGuard {
    runtime: Rc<RefCell<StreamRuntime>>,
    _on_mutation: Closure<dyn FnMut(JsValue)>,
    _on_dom_ready: Option<Closure<dyn FnMut(web_sys::Event)>>,
}

impl Drop for QueryStreamGuard {
    fn drop(&mut self) {
        if let Ok(mut rt) = self.runtime.try_borrow_mut() {
            rt.teardown_on_drop();
        }
    }
}

/// Phase 1 synchronous pre-Frame-0 hydration + optional Phase 2 mid-stream observer attachment.
///
/// Returns `None` on the post-stream / Light-DOM fast path (installing zero `MutationObserver`s)
/// or `Some(QueryStreamGuard)` when a streaming `[data-nve-dsd-host]` is still open.
pub(crate) fn attach_nve_query_stream(
    container: &web_sys::Element,
    surface_id: SurfaceId,
    client: ErasedQueryClient,
) -> Option<QueryStreamGuard> {
    let document = owner_or_window_document(container)?;
    let dsd_host = container.query_selector(DSD_HOST_SELECTOR).ok().flatten();
    let mut state = StreamIngestState::default();

    let mut scripts = Vec::new();
    if let Some(ref host) = dsd_host {
        collect_scripts_in_element(host, &mut scripts);
    }
    scripts.extend(collect_query_scripts_from_document(
        &document,
        dsd_host.as_ref(),
    ));

    if scripts.len() == 1 {
        let _ = hydrate_script_element(&scripts[0], &client, &mut state);
    } else if !scripts.is_empty() {
        let with_status: Vec<(web_sys::Element, bool)> = scripts
            .into_iter()
            .map(|el| {
                let complete = is_element_complete(&el, Some(&document));
                (el, complete)
            })
            .collect();
        ingest_completed_scripts(with_status, &client, &mut state);
    }

    let has_stream_end = document.ready_state() != "loading"
        || document
            .query_selector(STREAM_END_SELECTOR)
            .ok()
            .flatten()
            .is_some()
        || dsd_host.as_ref().is_some_and(|h| {
            h.query_selector(STREAM_END_SELECTOR)
                .ok()
                .flatten()
                .is_some()
        });

    if has_stream_end || dsd_host.is_none() {
        client.finish_ssr_stream();
        if let Some(host) = dsd_host {
            host.remove();
        }
        remove_stream_end_templates(&document);
        return None;
    }

    let host = dsd_host?;
    let runtime = Rc::new(RefCell::new(StreamRuntime {
        state,
        finished: false,
        surface_id,
        host: Some(host.clone()),
        document: Some(document.clone()),
        client: Some(client.clone()),
        observer: None,
        dom_ready_js: None,
    }));

    let rt_mut = Rc::clone(&runtime);
    let on_mutation = Closure::<dyn FnMut(JsValue)>::new(move |_records: JsValue| {
        if let Ok(mut rt) = rt_mut.try_borrow_mut() {
            rt.on_mutation();
        }
    });

    let observer = match web_sys::MutationObserver::new(on_mutation.as_ref().unchecked_ref()) {
        Ok(obs) => obs,
        Err(err) => {
            warn(&format!("MutationObserver::new failed: {err:?}"));
            client.finish_ssr_stream();
            host.remove();
            return None;
        }
    };

    let init = web_sys::MutationObserverInit::new();
    init.set_child_list(true);
    if let Err(err) = observer.observe_with_options(&host, &init) {
        warn(&format!("MutationObserver::observe failed: {err:?}"));
        observer.disconnect();
        client.finish_ssr_stream();
        host.remove();
        return None;
    }

    let rt_ready = Rc::clone(&runtime);
    let on_dom_ready = Closure::<dyn FnMut(web_sys::Event)>::new(move |_ev: web_sys::Event| {
        if let Ok(mut rt) = rt_ready.try_borrow_mut() {
            rt.finish_stream();
        }
    });
    let registered_dom_ready = document
        .add_event_listener_with_callback(
            "DOMContentLoaded",
            on_dom_ready.as_ref().unchecked_ref(),
        )
        .is_ok()
        .then_some(on_dom_ready);

    {
        let mut rt = runtime.borrow_mut();
        rt.observer = Some(observer);
        rt.dom_ready_js = registered_dom_ready
            .as_ref()
            .map(|cb| cb.as_ref().clone());
    }

    Some(QueryStreamGuard {
        runtime,
        _on_mutation: on_mutation,
        _on_dom_ready: registered_dom_ready,
    })
}
