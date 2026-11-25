//! Zero-flicker SSR DOM hydration (`WebBackend::hydrate_into` on `wasm32`,
//! `WebBackend::hydrate_from_ssr` headless hydration simulator on native targets),
//! structured [`HydrationMismatch`] reporting, 3-way `StateReconciler` dirty-control preservation,
//! and atomic pre-hydration event replay.

#[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
use nve_abi::hash::HashMap;
use nve_abi::hash::HashSet;
use nve_abi::{EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId, RawPropValue, ROOT_NODE_ID};
use nve_components::reconcile::{reconcile_three_way, ConflictPolicy, ThreeWayEntry};

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as _;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

use crate::dom::is_truthy_raw_prop;
use crate::event_replay::{
    drain_valid_pre_hydration_events, pre_hydration_event_to_native, CapturedPreHydrationEvent,
};
#[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
use crate::ssr::SsrOutput;
use crate::WebBackend;

/// Structured mismatch recorded when an SSR DOM element's tag diverges from the hydrated tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationMismatch {
    pub node_id: NodeId,
    pub expected_tag: &'static str,
    pub actual_tag: String,
}

/// Telemetry & diagnostics produced by zero-flicker hydration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HydrationReport {
    pub hydrated: bool,
    pub ssr_hash: Option<String>,
    pub seeded_rule_count: usize,
    pub adopted_nodes: usize,
    pub replaced_nodes: usize,
    pub mismatches: Vec<HydrationMismatch>,
    pub replayed_events: usize,
    pub reconciled_dirty_controls: usize,
}

/// Captured SSR baseline (`base`) and live DOM value (`local`) of an adopted form control
/// (`<input>`, `<textarea>`, `<select>`) awaiting 3-way reconciliation in `finalize_hydration`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HydratedControlState {
    pub(crate) base: String,
    pub(crate) local: String,
}

/// Native-side simulated SSR DOM node used to test exact DOM identity preservation,
/// selective mismatch replacement, and 3-way dirty-control reconciliation without a browser.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeSsrNode {
    pub(crate) tag: String,
    pub(crate) instance_id: u64,
    pub(crate) has_inline_style: bool,
    pub(crate) control_value: Option<String>,
}

#[inline]
fn is_form_control_tag(tag: &str) -> bool {
    matches!(tag, "input" | "textarea" | "select")
}

#[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
fn unescape_html_attr(val: &str) -> String {
    if !val.contains('&') {
        return val.to_owned();
    }
    let mut out = String::with_capacity(val.len());
    let mut rest = val;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        if let Some(after) = rest.strip_prefix("&quot;") {
            out.push('"');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("&lt;") {
            out.push('<');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("&gt;") {
            out.push('>');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("&amp;") {
            out.push('&');
            rest = after;
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Parses opening HTML tags containing `data-nve-id="..."` from an SSR HTML fragment for native
/// hydration simulation tests.
#[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
fn parse_ssr_html_nodes(html: &str) -> (Option<String>, HashMap<NodeId, NativeSsrNode>) {
    let mut nodes = HashMap::default();
    let mut root_hash = None;
    let mut next_instance_id = 1u64;
    let bytes = html.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() && (bytes[i + 1] == b'/' || bytes[i + 1] == b'!') {
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        let mut in_quote = false;
        while end < bytes.len() {
            let b = bytes[end];
            if b == b'"' {
                in_quote = !in_quote;
            } else if b == b'>' && !in_quote {
                break;
            }
            end += 1;
        }
        if end >= bytes.len() {
            break;
        }
        let tag_slice = &html[start..end];
        let mut parts = tag_slice.splitn(2, char::is_whitespace);
        let tag_name = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let attrs = parts.next().unwrap_or("");

        if let Some(id_str) = extract_html_attr(attrs, "data-nve-id")
            && let Ok(node_id) = id_str.parse::<NodeId>()
        {
            if node_id == ROOT_NODE_ID
                && let Some(h) = extract_html_attr(attrs, "data-nve-ssr-hash")
            {
                root_hash = Some(h.to_owned());
            }
            let has_inline_style = extract_html_attr(attrs, "style")
                .is_some_and(|s| !s.trim().is_empty());
            let control_value = is_form_control_tag(&tag_name).then(|| {
                extract_html_attr(attrs, "value")
                    .map(unescape_html_attr)
                    .unwrap_or_default()
            });
            nodes.insert(
                node_id,
                NativeSsrNode {
                    tag: tag_name,
                    instance_id: next_instance_id,
                    has_inline_style,
                    control_value,
                },
            );
            next_instance_id += 1;
        }
        i = end + 1;
    }

    (root_hash, nodes)
}

#[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
fn extract_html_attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let pos = attrs.find(&needle)?;
    let rest = &attrs[pos + needle.len()..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

#[cfg(target_arch = "wasm32")]
fn read_dom_control_base(el: &web_sys::Element, tag: &str) -> String {
    match tag {
        "input" => el.get_attribute("value").unwrap_or_default(),
        "textarea" => el.text_content().unwrap_or_default(),
        "select" => el
            .query_selector("option[selected]")
            .ok()
            .flatten()
            .and_then(|o| o.get_attribute("value").or_else(|| o.text_content()))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

#[cfg(target_arch = "wasm32")]
fn read_dom_control_value(el: &web_sys::Element) -> Option<String> {
    if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        return Some(inp.value());
    }
    if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        return Some(ta.value());
    }
    if let Some(sel) = el.dyn_ref::<web_sys::HtmlSelectElement>() {
        return Some(sel.value());
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn write_dom_control_value(el: &web_sys::Element, val: &str) {
    if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        if inp.value() != val {
            inp.set_value(val);
        }
    } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        if ta.value() != val {
            ta.set_value(val);
        }
    } else if let Some(sel) = el.dyn_ref::<web_sys::HtmlSelectElement>()
        && sel.value() != val
    {
        sel.set_value(val);
    }
}

impl WebBackend {
    /// Returns the structured hydration report populated by [`Self::hydrate_into`] or
    /// [`Self::hydrate_from_ssr`].
    #[inline]
    pub fn hydration_report(&self) -> &HydrationReport {
        &self.hydration_report
    }

    /// Returns `true` when `node_id` currently has a pre-existing SSR DOM element awaiting
    /// adoption during the active hydration frame.
    #[inline]
    pub(crate) fn has_ssr_dom_node(&self, node_id: NodeId) -> bool {
        self.hydrated_dom_tags.contains_key(&node_id)
    }

    /// Queues captured pre-hydration events to be filtered and replayed at the end of Frame 0
    /// hydration (used directly by native tests; on `wasm32`, `hydrate_into` drains
    /// `window.__nveReplayDrain()` automatically).
    pub fn queue_pre_hydration_events(&mut self, events: Vec<CapturedPreHydrationEvent>) {
        self.pending_pre_hydration_events.extend(events);
    }

    /// Simulates a user editing an SSR form control (`<input>`/`<textarea>`/`<select>`) before
    /// hydration completes so native tests can verify 3-way `StateReconciler` preservation.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_hydrated_dom_control_value(&mut self, node_id: NodeId, value: impl Into<String>) {
        let value = value.into();
        if let Some(ctrl) = self.hydrated_controls.get_mut(&node_id) {
            ctrl.local = value.clone();
        } else {
            self.hydrated_controls.insert(
                node_id,
                HydratedControlState {
                    base: String::new(),
                    local: value.clone(),
                },
            );
        }
        if let Some(ssr_node) = self.native_ssr_nodes.get_mut(&node_id) {
            ssr_node.control_value = Some(value);
        }
    }

    /// Returns the simulated native DOM control `.value` for `node_id` after hydration.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn native_dom_control_value(&self, node_id: NodeId) -> Option<&str> {
        self.native_ssr_nodes
            .get(&node_id)
            .and_then(|n| n.control_value.as_deref())
    }

    /// Headless native hydration simulator: seeds `StyleSheetCache` from `ssr.rule_hashes` and
    /// parses existing SSR DOM nodes so the next `submit_frame()` adopts matching nodes in-place
    /// (preserving `native_dom_instance_id`), reconciles dirty form controls via 3-way
    /// `StateReconciler`, replays valid pre-hydration events, and replaces mismatched tags.
    #[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
    pub fn hydrate_from_ssr(ssr: &SsrOutput) -> Self {
        let mut backend = Self::new();
        backend.stylesheet.mark_base_mounted();
        let hashes_csv = ssr.rule_hashes.join(",");
        let seeded = backend.stylesheet.seed_ssr_hashes(&hashes_csv);
        let (root_hash, parsed_nodes) = parse_ssr_html_nodes(&ssr.html);

        for (&node_id, node) in &parsed_nodes {
            backend.hydrated_dom_tags.insert(node_id, node.tag.clone());
            if node.has_inline_style {
                backend.hydrated_inline_nodes.insert(node_id);
            }
            if let Some(base) = &node.control_value {
                backend.hydrated_controls.insert(
                    node_id,
                    HydratedControlState {
                        base: base.clone(),
                        local: base.clone(),
                    },
                );
            }
        }
        backend.native_ssr_nodes = parsed_nodes;
        backend.next_native_instance_id = backend.native_ssr_nodes.len() as u64 + 1;
        backend.hydrating = true;
        backend.hydration_report = HydrationReport {
            hydrated: true,
            ssr_hash: root_hash,
            seeded_rule_count: seeded,
            adopted_nodes: 0,
            replaced_nodes: 0,
            mismatches: Vec::new(),
            replayed_events: 0,
            reconciled_dirty_controls: 0,
        };
        backend
    }

    /// Returns the simulated native DOM object identity for `node_id` (used to verify
    /// `instance_before == instance_after` on native hydration tests).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn native_dom_instance_id(&self, node_id: NodeId) -> Option<u64> {
        self.native_ssr_nodes.get(&node_id).map(|n| n.instance_id)
    }

    /// Zero-flicker browser DOM hydration: adopts existing `[data-nve-id]` elements inside
    /// `container` WITHOUT wiping `container`, pre-populates [`StyleSheetCache`] from
    /// `<style data-nve-ssr data-nve-hashes="...">`, captures SSR/live control baselines for 3-way
    /// reconciliation, attaches root delegated listeners, and atomically drains
    /// `window.__nveReplayDrain()`.
    #[cfg(target_arch = "wasm32")]
    pub fn hydrate_into(container: web_sys::Element) -> Self {
        let mut backend = Self::new();

        // 1. Pre-seed stylesheet cache from SSR `<style>` tags to guarantee 0 duplicate rules & 0 FOUC.
        let mut seeded_rule_count = 0usize;
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if doc.query_selector("style[data-nve-base]").ok().flatten().is_some() {
                backend.stylesheet.mark_base_mounted();
            }
            if let Ok(Some(ssr_style)) = doc.query_selector("style[data-nve-ssr]")
                && let Some(hashes) = ssr_style.get_attribute("data-nve-hashes")
            {
                seeded_rule_count = backend.stylesheet.seed_ssr_hashes(&hashes);
            }
        }

        // 2. Walk existing `[data-nve-id]` elements without clearing `container`.
        let mut register_el = |el: web_sys::Element| {
            if let Some(raw_id) = el.get_attribute("data-nve-id")
                && let Ok(nid) = raw_id.parse::<NodeId>()
            {
                let tag = el.tag_name().to_ascii_lowercase();
                if nid == ROOT_NODE_ID
                    && let Some(hash) = el.get_attribute("data-nve-ssr-hash")
                {
                    backend.hydration_report.ssr_hash = Some(hash);
                }
                if el
                    .get_attribute("style")
                    .is_some_and(|s| !s.trim().is_empty())
                {
                    backend.hydrated_inline_nodes.insert(nid);
                }
                if is_form_control_tag(&tag) {
                    let base = read_dom_control_base(&el, &tag);
                    let local = read_dom_control_value(&el).unwrap_or_else(|| base.clone());
                    backend
                        .hydrated_controls
                        .insert(nid, HydratedControlState { base, local });
                }
                backend.hydrated_dom_tags.insert(nid, tag);
                backend.dom_nodes.insert(nid, el);
            }
        };

        if container.has_attribute("data-nve-id") {
            register_el(container.clone());
        }
        if let Ok(list) = container.query_selector_all("[data-nve-id]") {
            let len = list.length();
            for i in 0..len {
                if let Some(node) = list.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    register_el(el);
                }
            }
        }

        // Ensure ROOT_NODE_ID has a DOM anchor even if hydrating into an empty/unwrapped container.
        if !backend.dom_nodes.contains_key(&ROOT_NODE_ID) {
            backend.attr_scratch.clear();
            let _ = write!(&mut backend.attr_scratch, "{ROOT_NODE_ID}");
            let _ = container.set_attribute("data-nve-id", &backend.attr_scratch);
            backend.dom_nodes.insert(ROOT_NODE_ID, container.clone());
        }

        backend.hydrating = true;
        backend.hydration_report.hydrated = true;
        backend.hydration_report.seeded_rule_count = seeded_rule_count;

        backend.attach_root_event_delegator(container);
        backend.attach_window_keydown_listener();
        backend.attach_visual_viewport_listener();
        // Atomically detach pre-hydration capture listeners and buffer events for Frame 0 replay.
        backend.pending_pre_hydration_events =
            crate::event_replay::drain_browser_pre_hydration_events();
        backend
    }

    fn resolve_press_replay_target(&self, start_id: NodeId) -> Option<NodeId> {
        let mut cur = Some(start_id);
        let mut depth = 0usize;
        while let Some(id) = cur {
            if depth > 256 {
                break;
            }
            depth += 1;
            if let Some(el) = self.nodes.get(&id)
                && (el.event_mask & EventMask::PRESS) != 0
            {
                if el.raw_props.get("disabled").is_some_and(is_truthy_raw_prop) {
                    return None;
                }
                return Some(id);
            }
            cur = self.parent_of.get(&id).copied();
        }
        let el = self.nodes.get(&start_id)?;
        if el.raw_props.get("disabled").is_some_and(is_truthy_raw_prop) {
            return None;
        }
        Some(start_id)
    }

    /// Completes the hydration frame: verifies every adopted element's final semantic tag,
    /// replaces ONLY mismatched subtrees, reconciles dirty form controls via 3-way
    /// `StateReconciler`, and replays valid pre-hydration events atomically.
    pub(crate) fn finalize_hydration(
        &mut self,
        seen_ids: &[NodeId],
        dirty_styles: &mut Vec<NodeId>,
    ) {
        let mut checked: HashSet<NodeId> =
            HashSet::with_capacity_and_hasher(seen_ids.len() + 1, Default::default());
        let mut to_check: Vec<NodeId> = Vec::with_capacity(seen_ids.len() + 1);
        if self.hydrated_dom_tags.contains_key(&ROOT_NODE_ID) && self.nodes.contains_key(&ROOT_NODE_ID)
        {
            to_check.push(ROOT_NODE_ID);
            checked.insert(ROOT_NODE_ID);
        }
        for &nid in seen_ids {
            if checked.insert(nid) && self.nodes.contains_key(&nid) {
                to_check.push(nid);
            }
        }

        let mut mismatched: Vec<NodeId> = Vec::new();
        for nid in to_check {
            let Some(actual_tag) = self.hydrated_dom_tags.get(&nid).cloned() else {
                continue;
            };
            let Some(expected_tag) = self.nodes.get(&nid).map(|n| n.tag) else {
                continue;
            };
            if actual_tag.eq_ignore_ascii_case(expected_tag) {
                self.hydration_report.adopted_nodes += 1;
            } else {
                self.hydration_report.mismatches.push(HydrationMismatch {
                    node_id: nid,
                    expected_tag,
                    actual_tag,
                });
                self.hydration_report.replaced_nodes += 1;
                mismatched.push(nid);
            }
        }

        let mismatched_set: HashSet<NodeId> = mismatched.iter().copied().collect();
        for nid in mismatched {
            self.replace_mismatched_hydrated_node(nid);
            dirty_styles.push(nid);
        }

        // Remove any SSR DOM nodes that did not appear in the hydrated frame.
        let orphan_ssr_ids: Vec<NodeId> = self
            .hydrated_dom_tags
            .keys()
            .copied()
            .filter(|id| !self.nodes.contains_key(id))
            .collect();
        for orphan in orphan_ssr_ids {
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = self.dom_nodes.remove(&orphan) {
                el.remove();
            }
            #[cfg(not(target_arch = "wasm32"))]
            self.native_ssr_nodes.remove(&orphan);
        }

        #[cfg(target_arch = "wasm32")]
        let ts_us = crate::events::now_us();
        #[cfg(not(target_arch = "wasm32"))]
        let ts_us = 0u64;
        let surface_id = self.surface_id.get();
        let mut hydrated_events: Vec<NativeEvent> = Vec::new();

        // 3-way StateReconciler for adopted form controls (enqueued BEFORE replayed actions so
        // typed input syncs into Rust state before a pre-hydration Submit/Press runs).
        let controls = std::mem::take(&mut self.hydrated_controls);
        let mut control_entries: Vec<(NodeId, HydratedControlState)> = controls.into_iter().collect();
        control_entries.sort_unstable_by_key(|(nid, _)| *nid);
        for (nid, ctrl) in control_entries {
            if mismatched_set.contains(&nid) {
                continue;
            }
            let Some(incoming) = self.nodes.get(&nid).map(|el| match el.raw_props.get("value") {
                Some(RawPropValue::String(v)) => v.clone(),
                _ => el.text.clone().unwrap_or_default(),
            }) else {
                continue;
            };
            let decision = reconcile_three_way(
                ThreeWayEntry {
                    key: nid,
                    base: ctrl.base,
                    local: ctrl.local,
                    incoming,
                    local_clock: None,
                    incoming_clock: None,
                },
                ConflictPolicy::PreferLocal,
            );
            if decision.emit_upstream {
                if let Some(el) = self.nodes.get_mut(&nid) {
                    el.text = Some(decision.resolved.clone());
                    if el.raw_props.contains_key("value") {
                        el.raw_props
                            .insert("value".into(), RawPropValue::String(decision.resolved.clone()));
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(ssr_node) = self.native_ssr_nodes.get_mut(&nid) {
                    ssr_node.control_value = Some(decision.resolved.clone());
                }
                self.hydration_report.reconciled_dirty_controls += 1;
                hydrated_events.push(NativeEvent {
                    surface_id,
                    node_id: nid,
                    timestamp_us: ts_us,
                    kind: NativeEventKind::TextChange,
                    payload: EventPayload::Text(decision.resolved),
                });
            } else if decision.apply_local {
                #[cfg(target_arch = "wasm32")]
                if let Some(dom_el) = self.dom_nodes.get(&nid) {
                    write_dom_control_value(dom_el, &decision.resolved);
                }
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(ssr_node) = self.native_ssr_nodes.get_mut(&nid) {
                    ssr_node.control_value = Some(decision.resolved);
                }
            } else {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(ssr_node) = self.native_ssr_nodes.get_mut(&nid) {
                    ssr_node.control_value = Some(decision.resolved);
                }
            }
        }

        // Atomic replay of valid pre-hydration user interactions.
        let raw_replay = std::mem::take(&mut self.pending_pre_hydration_events);
        if !raw_replay.is_empty() {
            let valid = drain_valid_pre_hydration_events(raw_replay, |id| self.nodes.contains_key(&id));
            for ev in valid {
                let target_id = match &ev {
                    CapturedPreHydrationEvent::Press { node_id, .. } => {
                        let Some(resolved) = self.resolve_press_replay_target(*node_id) else {
                            continue;
                        };
                        resolved
                    }
                    _ => ev.node_id(),
                };
                hydrated_events.push(pre_hydration_event_to_native(ev, surface_id, ts_us, target_id));
                self.hydration_report.replayed_events += 1;
            }
        }

        self.hydrated_dom_tags.clear();
        self.hydrated_inline_nodes.clear();
        self.hydrating = false;

        if !hydrated_events.is_empty() {
            self.events.borrow_mut().extend(hydrated_events);
            let cb = self.on_event.borrow().clone();
            if let Some(cb) = cb {
                cb();
            }
        }
    }

    fn replace_mismatched_hydrated_node(&mut self, node_id: NodeId) {
        #[cfg(target_arch = "wasm32")]
        {
            self.upgrade_dom_element_tag(node_id);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(expected_tag) = self.nodes.get(&node_id).map(|n| n.tag) {
            let new_id = self.next_native_instance_id;
            self.next_native_instance_id += 1;
            self.native_ssr_nodes.insert(
                node_id,
                NativeSsrNode {
                    tag: expected_tag.to_owned(),
                    instance_id: new_id,
                    has_inline_style: false,
                    control_value: None,
                },
            );
        }
    }
}
