//! Targeted Tier-1 `HotPatchMsg::TemplatePatch` application for `WebBackend` DOM + mirror nodes.

use core::fmt::Write as _;
use nve_abi::hash::HashMap;

use nve_abi::{ComponentKind, NodeId, RawPropValue, ROOT_NODE_ID};

use crate::css::MirrorElement;
use crate::WebBackend;

impl WebBackend {
    /// Applies a Tier-1 `HotPatchMsg::TemplatePatch` directly to targeted live DOM + mirror nodes (<10ms).
    ///
    /// Respects `target_test_id`, `node_path`, and exact `text_replacements` so live WebSocket
    /// patches update ONLY the matching DOM node(s) without spraying unrelated elements.
    pub fn apply_template_patch(&mut self, patch: &nve_engine::HotPatchMsg) -> u32 {
        let nve_engine::HotPatchMsg::TemplatePatch {
            template_id,
            attr_updates,
            text_updates,
            target_test_id,
            node_path,
            text_replacements,
            ..
        } = patch
        else {
            return 0;
        };

        let mut sorted_ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        sorted_ids.sort_unstable();
        let mut mutations = 0u32;

        let matches_test_id = |el: &MirrorElement, tid: &str| -> bool {
            ["testID", "data-testid", "test_id"].iter().any(|k| {
                matches!(el.raw_props.get(*k), Some(RawPropValue::String(v)) if v == tid)
            })
        };

        let find_by_test_id = |nodes: &HashMap<NodeId, MirrorElement>, tid: &str| -> Vec<NodeId> {
            let mut ids: Vec<NodeId> = nodes
                .iter()
                .filter_map(|(&id, el)| {
                    (id != ROOT_NODE_ID && matches_test_id(el, tid)).then_some(id)
                })
                .collect();
            ids.sort_unstable();
            ids
        };

        let collect_subtree =
            |nodes: &HashMap<NodeId, MirrorElement>, roots: &[NodeId]| -> Vec<NodeId> {
                let mut out = Vec::new();
                let mut stack = roots.to_vec();
                while let Some(nid) = stack.pop() {
                    out.push(nid);
                    if let Some(el) = nodes.get(&nid) {
                        for &cid in &el.children {
                            stack.push(cid);
                        }
                    }
                }
                out.sort_unstable();
                out.dedup();
                out
            };

        let walk_path = |nodes: &HashMap<NodeId, MirrorElement>, path: &[usize]| -> Option<NodeId> {
            let mut cur = ROOT_NODE_ID;
            for &idx in path {
                cur = *nodes.get(&cur)?.children.get(idx)?;
            }
            (cur != ROOT_NODE_ID).then_some(cur)
        };

        let mut matched_text_nodes: Vec<NodeId> = Vec::new();

        // 1. Text updates: prefer exact `text_replacements` over positional `text_updates`.
        if !text_replacements.is_empty() {
            let scoped_ids = target_test_id
                .as_deref()
                .map(|tid| {
                    let roots = find_by_test_id(&self.nodes, tid);
                    collect_subtree(&self.nodes, &roots)
                })
                .unwrap_or_default();

            for (old_text, new_text) in text_replacements {
                let mut targets: Vec<NodeId> = Vec::new();
                if !scoped_ids.is_empty() {
                    for &nid in &scoped_ids {
                        if self
                            .nodes
                            .get(&nid)
                            .is_some_and(|el| el.text.as_deref() == Some(old_text.as_str()))
                        {
                            targets.push(nid);
                        }
                    }
                }
                if targets.is_empty() {
                    for &nid in &sorted_ids {
                        if nid != ROOT_NODE_ID
                            && self
                                .nodes
                                .get(&nid)
                                .is_some_and(|el| el.text.as_deref() == Some(old_text.as_str()))
                        {
                            targets.push(nid);
                        }
                    }
                }
                for nid in targets {
                    self.apply_text_mutation(nid, new_text);
                    matched_text_nodes.push(nid);
                    mutations += 1;
                }
            }
        } else if !text_updates.is_empty() {
            let is_targeted = target_test_id.is_some() || !node_path.is_empty();
            let candidate_pool: Vec<NodeId> = if let Some(tid) = target_test_id.as_deref() {
                let roots = find_by_test_id(&self.nodes, tid);
                collect_subtree(&self.nodes, &roots)
            } else if !node_path.is_empty() {
                walk_path(&self.nodes, node_path)
                    .map(|nid| collect_subtree(&self.nodes, &[nid]))
                    .unwrap_or_default()
            } else {
                sorted_ids.clone()
            };

            let text_node_ids: Vec<NodeId> = candidate_pool
                .iter()
                .copied()
                .filter(|id| {
                    self.nodes
                        .get(id)
                        .is_some_and(|n| n.kind == ComponentKind::Text && n.text.is_some())
                })
                .collect();

            for (idx, text) in text_updates {
                let target = text_node_ids.get(*idx).copied().or_else(|| {
                    if is_targeted {
                        None
                    } else {
                        sorted_ids.get(*idx).copied()
                    }
                });
                if let Some(nid) = target {
                    self.apply_text_mutation(nid, text);
                    mutations += 1;
                }
            }
        }

        // 2. Attribute / CSS style patches: target exact node(s), routed to Tier-1 inline lane.
        if !attr_updates.is_empty() {
            let mut css_fragment = String::with_capacity(attr_updates.len() * 32);
            for (k, v) in attr_updates {
                if k == "style" {
                    css_fragment.push_str(v.trim());
                    if !css_fragment.ends_with(';') {
                        css_fragment.push(';');
                    }
                } else {
                    let _ = write!(&mut css_fragment, "{k}:{v};");
                }
            }

            let attr_targets: Vec<NodeId> = if let Some(tid) = target_test_id.as_deref() {
                find_by_test_id(&self.nodes, tid)
            } else if !node_path.is_empty() {
                walk_path(&self.nodes, node_path).into_iter().collect()
            } else if !text_replacements.is_empty() {
                matched_text_nodes
            } else {
                let by_tpl = if !template_id.is_empty() {
                    find_by_test_id(&self.nodes, template_id)
                } else {
                    Vec::new()
                };
                if !by_tpl.is_empty() {
                    by_tpl
                } else {
                    sorted_ids
                        .iter()
                        .copied()
                        .filter(|&id| id != ROOT_NODE_ID)
                        .collect()
                }
            };

            let mut dirty_targets = Vec::with_capacity(attr_targets.len());
            for nid in attr_targets {
                if let Some(el) = self.nodes.get_mut(&nid) {
                    el.patch_css.push_str(&css_fragment);
                    dirty_targets.push(nid);
                }
                mutations += 1;
            }
            self.flush_dirty_styles(&dirty_targets);
        }

        mutations
    }
}
