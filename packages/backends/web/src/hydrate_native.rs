//! Native headless SSR & streaming DSD hydration simulation (`WebBackend::hydrate_from_ssr`,
//! `WebBackend::hydrate_from_streaming_html`, and top-down structural tree matching).

#[cfg(feature = "ssr")]
use nve_abi::hash::HashMap;
use nve_abi::NodeId;
#[cfg(feature = "ssr")]
use nve_abi::{RawPropValue, ROOT_NODE_ID};

#[cfg(feature = "ssr")]
use crate::hydrate::{
    is_form_control_tag, HydratedControlState, HydrationMismatch, HydrationReport,
};
#[cfg(feature = "ssr")]
use crate::hydrate_native_parse::parse_ssr_html_tree;
#[cfg(feature = "ssr")]
use crate::ssr::SsrOutput;
use crate::WebBackend;

/// Adopted or mounted native simulated DOM node keyed by live mirror `NodeId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeAdoptedNode {
    pub(crate) tag: String,
    pub(crate) instance_id: u64,
    pub(crate) control_value: Option<String>,
}

impl WebBackend {
    /// Simulates a user editing an SSR form control (`<input>`/`<textarea>`/`<select>`) before
    /// hydration completes so native tests can verify 3-way `StateReconciler` preservation even
    /// when client `NodeId`s shift relative to SSR `data-nve-id`s.
    pub fn set_hydrated_dom_control_value(&mut self, node_id: NodeId, value: impl Into<String>) {
        let value = value.into();
        #[cfg(feature = "ssr")]
        if self.hydrating
            && let Some(ref mut tree) = self.native_hydrate_tree
            && let Some(idx) = tree.find_by_ssr_id(node_id)
        {
            tree.arena[idx].live_control_value = Some(value);
            return;
        }
        if let Some(node) = self.native_adopted_nodes.get_mut(&node_id) {
            node.control_value = Some(value);
        }
    }

    /// Returns the simulated native DOM control `.value` for `node_id`.
    pub fn native_dom_control_value(&self, node_id: NodeId) -> Option<&str> {
        #[cfg(feature = "ssr")]
        if self.hydrating
            && let Some(ref tree) = self.native_hydrate_tree
            && let Some(idx) = tree.find_by_ssr_id(node_id)
        {
            return tree.arena[idx].live_control_value.as_deref();
        }
        self.native_adopted_nodes
            .get(&node_id)
            .and_then(|n| n.control_value.as_deref())
    }

    /// Returns the simulated native DOM object identity for `node_id` (before `submit_frame`, looks
    /// up by SSR `data-nve-id`; after hydration, looks up by live mirror `NodeId`).
    pub fn native_dom_instance_id(&self, node_id: NodeId) -> Option<u64> {
        #[cfg(feature = "ssr")]
        if self.hydrating
            && let Some(ref tree) = self.native_hydrate_tree
            && let Some(idx) = tree.find_by_ssr_id(node_id)
        {
            return Some(tree.arena[idx].instance_id);
        }
        self.native_adopted_nodes.get(&node_id).map(|n| n.instance_id)
    }

    /// Headless native hydration simulator from an [`SsrOutput`]: seeds `StyleSheetCache` from
    /// `ssr.rule_hashes` and parses the SSR HTML into a real structural element tree so the
    /// next `submit_frame()` executes exact top-down positional hydration.
    #[cfg(feature = "ssr")]
    pub fn hydrate_from_ssr(ssr: &SsrOutput) -> Self {
        let mut backend = Self::new();
        backend.stylesheet.mark_base_mounted();
        let hashes_csv = ssr.rule_hashes.join(",");
        let seeded = backend.stylesheet.seed_ssr_hashes(&hashes_csv);
        let tree = parse_ssr_html_tree(&ssr.html);
        backend.next_native_instance_id = tree.next_instance_id.max(1);
        let root_hash = tree.root_hash.clone();
        backend.native_hydrate_tree = Some(tree);
        backend.hydrating = true;
        backend.hydration_report = HydrationReport {
            hydrated: true,
            ssr_hash: root_hash,
            seeded_rule_count: seeded,
            ..Default::default()
        };
        backend
    }

    /// Headless native hydration simulator from a full or mid-stream partial DSD streaming HTML
    /// document: promotes Declarative Shadow DOM slots (last-wins Light-DOM chunks + fallback
    /// unwrapping for still-pending slots), seeds ALL `<style data-nve-base>`,
    /// `<style data-nve-ssr>`, and `<style data-nve-ssr-chunk>` hashes, and runs top-down
    /// positional structural hydration on the next `submit_frame()`.
    #[cfg(feature = "ssr")]
    pub fn hydrate_from_streaming_html(html: &str) -> Self {
        let mut backend = Self::new();
        let tree = parse_ssr_html_tree(html);
        if tree.has_base_style {
            backend.stylesheet.mark_base_mounted();
        }
        let mut seeded = 0usize;
        for hash in &tree.seeded_hashes {
            seeded += backend.stylesheet.seed_ssr_hashes(hash);
        }
        backend.next_native_instance_id = tree.next_instance_id.max(1);
        let root_hash = tree.root_hash.clone();
        backend.native_hydrate_tree = Some(tree);
        backend.hydrating = true;
        backend.hydration_report = HydrationReport {
            hydrated: true,
            ssr_hash: root_hash,
            seeded_rule_count: seeded,
            ..Default::default()
        };
        backend
    }

    #[cfg(feature = "ssr")]
    pub(crate) fn hydrate_walk_native(
        &mut self,
        ssr_id_remap: &mut HashMap<NodeId, NodeId>,
        captured_controls: &mut Vec<(NodeId, HydratedControlState)>,
        dirty_styles: &mut Vec<NodeId>,
    ) {
        let Some(tree) = self.native_hydrate_tree.take() else {
            return;
        };
        let Some(root_dom_idx) = tree.root_idx else {
            self.mount_native_subtree_from_mirror(ROOT_NODE_ID, dirty_styles);
            return;
        };

        let mut stack: Vec<(NodeId, usize)> = vec![(ROOT_NODE_ID, root_dom_idx)];
        while let Some((mirror_id, dom_idx)) = stack.pop() {
            let Some(el) = self.nodes.get(&mirror_id) else {
                continue;
            };
            let expected_eff_tag = self.effective_html_tag(mirror_id, el).to_owned();
            let expected_static_tag = el.tag;
            let wrap_code = el.wrap_code;
            let mirror_children = el.children.clone();
            let dom_node = &tree.arena[dom_idx];

            if dom_node.tag.eq_ignore_ascii_case(&expected_eff_tag) {
                self.hydration_report.adopted_nodes += 1;
                if let Some(old_ssr_id) = dom_node.ssr_id {
                    ssr_id_remap.insert(old_ssr_id, mirror_id);
                }
                self.native_adopted_nodes.insert(
                    mirror_id,
                    NativeAdoptedNode {
                        tag: dom_node.tag.clone(),
                        instance_id: dom_node.instance_id,
                        control_value: dom_node.live_control_value.clone(),
                    },
                );
                if is_form_control_tag(&dom_node.tag) {
                    let base = dom_node.base_control_value.clone().unwrap_or_default();
                    let local = dom_node
                        .live_control_value
                        .clone()
                        .unwrap_or_else(|| base.clone());
                    captured_controls.push((mirror_id, HydratedControlState { base, local }));
                }

                let dom_children = tree.ui_children_of(
                    dom_idx,
                    wrap_code,
                    &expected_eff_tag,
                    !mirror_children.is_empty(),
                );
                let paired = mirror_children.len().min(dom_children.len());
                for i in (0..paired).rev() {
                    stack.push((mirror_children[i], dom_children[i]));
                }
                for &missing_id in &mirror_children[paired..] {
                    let missing_tag = self.nodes.get(&missing_id).map_or("div", |n| n.tag);
                    self.hydration_report.mismatches.push(HydrationMismatch {
                        node_id: missing_id,
                        expected_tag: missing_tag,
                        actual_tag: "<missing>".into(),
                    });
                    self.hydration_report.replaced_nodes += 1;
                    self.mount_native_subtree_from_mirror(missing_id, dirty_styles);
                }
                for &extra_idx in &dom_children[paired..] {
                    self.hydration_report.mismatches.push(HydrationMismatch {
                        node_id: mirror_id,
                        expected_tag: "",
                        actual_tag: tree.arena[extra_idx].tag.clone(),
                    });
                }
            } else {
                self.hydration_report.mismatches.push(HydrationMismatch {
                    node_id: mirror_id,
                    expected_tag: expected_static_tag,
                    actual_tag: dom_node.tag.clone(),
                });
                self.hydration_report.replaced_nodes += 1;
                self.mount_native_subtree_from_mirror(mirror_id, dirty_styles);
            }
        }
    }

    #[cfg(feature = "ssr")]
    fn mount_native_subtree_from_mirror(
        &mut self,
        root_id: NodeId,
        dirty_styles: &mut Vec<NodeId>,
    ) {
        let mut stack = vec![root_id];
        while let Some(nid) = stack.pop() {
            let Some(el) = self.nodes.get(&nid) else {
                continue;
            };
            let eff_tag = self.effective_html_tag(nid, el).to_owned();
            let control_value = is_form_control_tag(&eff_tag).then(|| {
                match el.raw_props.get("value") {
                    Some(RawPropValue::String(v)) => v.clone(),
                    _ => el.text.clone().unwrap_or_default(),
                }
            });
            let children = el.children.clone();
            let new_id = self.next_native_instance_id;
            self.next_native_instance_id += 1;
            self.native_adopted_nodes.insert(
                nid,
                NativeAdoptedNode {
                    tag: eff_tag,
                    instance_id: new_id,
                    control_value,
                },
            );
            dirty_styles.push(nid);
            for cid in children.into_iter().rev() {
                stack.push(cid);
            }
        }
    }
}
