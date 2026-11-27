//! Zero-flicker SSR hydration (`hydrate_from_ssr`, `hydrate_into`), mismatch reporting,
//! pre-hydration event replay queue, and form control value preservation.

use nve_abi::NodeId;

use crate::event_replay::CapturedPreHydrationEvent;
use crate::ssr::SsrOutput;
use crate::WebBackend;

/// One detected divergence between SSR DOM and live client commit during hydration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationMismatch {
    pub node_id: NodeId,
    pub reason: String,
}

/// Telemetry report emitted during SSR hydration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HydrationReport {
    pub hydrated: bool,
    pub adopted_nodes: u32,
    pub replaced_nodes: u32,
    pub reused_css_rules: u32,
    pub replayed_events: u32,
    pub reconciled_dirty_controls: u32,
    pub expected_root_hash: Option<u64>,
    pub mismatches: Vec<HydrationMismatch>,
}

impl HydrationReport {
    #[inline]
    pub fn is_clean(&self) -> bool {
        self.replaced_nodes == 0 && self.mismatches.is_empty()
    }
}

impl WebBackend {
    #[inline]
    pub fn hydration_report(&self) -> &HydrationReport {
        &self.hydration_report
    }

    pub fn queue_pre_hydration_events(&mut self, events: Vec<CapturedPreHydrationEvent>) {
        self.pre_hydration_events.extend(events);
    }

    pub fn set_hydrated_dom_control_value(&mut self, node_id: NodeId, value: impl Into<String>) {
        self.hydrated_control_values.insert(node_id, value.into());
    }

    pub fn native_dom_control_value(&self, node_id: NodeId) -> Option<&str> {
        self.hydrated_control_values.get(&node_id).map(String::as_str)
    }

    /// Creates a headless hydrated [`WebBackend`] seeded with `ssr`'s deduplicated rule hashes,
    /// structural root hash, and parsed `(data-nve-id, tag)` map so the first `commit()` verifies
    /// every adopted node for tag parity.
    pub fn hydrate_from_ssr(ssr: &SsrOutput) -> Self {
        let mut backend = Self::new();
        backend
            .stylesheet
            .seed_ssr_hashes(ssr.rule_hashes.iter().map(String::as_str));
        backend.hydration_report.hydrated = true;
        backend.hydration_report.reused_css_rules = ssr.rule_hashes.len() as u32;
        backend.hydration_report.expected_root_hash = Some(ssr.root_hash);
        backend.pending_ssr_nodes = parse_ssr_node_tags(&ssr.html);
        backend
    }

    /// Hydrates into an existing SSR-populated browser container on `wasm32`, adopting matching
    /// `[data-nve-id]` DOM nodes in place with zero flicker.
    #[cfg(target_arch = "wasm32")]
    pub fn hydrate_into(container: web_sys::Element) -> Self {
        let mut backend = Self::attach_into(container.clone(), false);
        backend.hydration_report.hydrated = true;
        backend.adopt_existing_dom(&container);
        backend
            .pre_hydration_events
            .extend(crate::event_replay::drain_browser_pre_hydration_events());
        backend
    }
}

/// Parses `<tag ... data-nve-id="N" ...>` opening tags from NVE SSR HTML into `node_id -> tag`.
fn parse_ssr_node_tags(html: &str) -> nve_abi::hash::HashMap<NodeId, String> {
    let mut map = nve_abi::hash::HashMap::default();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_alphabetic() {
            let tag_start = i + 1;
            let mut tag_end = tag_start;
            while tag_end < bytes.len()
                && (bytes[tag_end].is_ascii_alphanumeric() || bytes[tag_end] == b'-')
            {
                tag_end += 1;
            }
            let tag = &html[tag_start..tag_end];
            if let Some(gt_rel) = html[tag_end..].find('>') {
                let attrs = &html[tag_end..tag_end + gt_rel];
                if let Some(pos) = attrs.find("data-nve-id=\"") {
                    let id_start = pos + "data-nve-id=\"".len();
                    if let Some(end_q) = attrs[id_start..].find('"')
                        && let Ok(id) = attrs[id_start..id_start + end_q].parse::<NodeId>()
                    {
                        map.insert(id, tag.to_owned());
                    }
                }
                i = tag_end + gt_rel + 1;
                continue;
            }
        }
        i += 1;
    }
    map
}
