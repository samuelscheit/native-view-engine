//! Zero-flicker SSR DOM hydration (`WebBackend::hydrate_into` on `wasm32`,
//! `WebBackend::hydrate_from_ssr` headless hydration simulator on native targets),
//! structured [`HydrationMismatch`] reporting, and selective mismatched-subtree replacement.

#[cfg(not(target_arch = "wasm32"))]
use nve_abi::hash::HashMap;
use nve_abi::hash::HashSet;

use nve_abi::{NodeId, ROOT_NODE_ID};

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as _;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(not(target_arch = "wasm32"))]
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
}

/// Native-side simulated SSR DOM node used to test exact DOM identity preservation and
/// selective mismatch replacement without a browser.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeSsrNode {
    pub(crate) tag: String,
    pub(crate) instance_id: u64,
    pub(crate) has_inline_style: bool,
}

/// Parses opening HTML tags containing `data-nve-id="..."` from an SSR HTML fragment for native
/// hydration simulation tests.
#[cfg(not(target_arch = "wasm32"))]
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
            nodes.insert(
                node_id,
                NativeSsrNode {
                    tag: tag_name,
                    instance_id: next_instance_id,
                    has_inline_style,
                },
            );
            next_instance_id += 1;
        }
        i = end + 1;
    }

    (root_hash, nodes)
}

#[cfg(not(target_arch = "wasm32"))]
fn extract_html_attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let pos = attrs.find(&needle)?;
    let rest = &attrs[pos + needle.len()..];
    let end = rest.find('"')?;
    Some(&rest[..end])
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

    /// Headless native hydration simulator: seeds `StyleSheetCache` from `ssr.rule_hashes` and
    /// parses existing SSR DOM nodes so the next `submit_frame()` adopts matching nodes in-place
    /// (preserving `native_dom_instance_id`) and detects/replaces any mismatched tags.
    #[cfg(not(target_arch = "wasm32"))]
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
    /// `<style data-nve-ssr data-nve-hashes="...">`, and attaches root delegated listeners.
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
        backend
    }

    /// Completes the hydration frame: verifies every adopted element's final semantic tag,
    /// records structured [`HydrationMismatch`] entries, replaces ONLY mismatched subtrees,
    /// and removes any orphaned SSR nodes absent from Frame 0.
    pub(crate) fn finalize_hydration(
        &mut self,
        seen_ids: &[NodeId],
        dirty_styles: &mut Vec<NodeId>,
    ) {
        let mut checked: HashSet<NodeId> = HashSet::with_capacity_and_hasher(seen_ids.len() + 1, Default::default());
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

        self.hydrated_dom_tags.clear();
        self.hydrated_inline_nodes.clear();
        self.hydrating = false;
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
                },
            );
        }
    }
}
