//! Targeted Tier-1 `HotPatchMsg::TemplatePatch` indexing and surgical node patching
//! for `NveMutationWriter`.
//!
//! Ensures live hot-patches update ONLY the exact matching node(s) addressed by
//! `target_test_id`, `node_path`, or exact `text_replacements`, never spraying
//! `attr_updates` or text overwrites onto unrelated sibling nodes.

use nve_abi::{ComponentKind, NodeId, ROOT_NODE_ID};
use nve_engine::{HotPatchMsg, RenderSink};

use crate::NveMutationWriter;

#[inline]
pub(crate) fn is_test_id_attr_name(name: &str) -> bool {
    matches!(name, "testID" | "data-testid" | "test_id")
}

impl NveMutationWriter {
    #[inline]
    pub(crate) fn record_test_id(&mut self, node_id: NodeId, test_id: &str) {
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            rec.extras_mut().test_id = Some(test_id.to_owned());
        }
    }

    #[inline]
    pub(crate) fn clear_test_id(&mut self, node_id: NodeId) {
        if let Some(rec) = self.nodes.get_mut(&node_id)
            && let Some(ext) = rec.extras.as_deref_mut()
        {
            ext.test_id = None;
        }
    }

    #[inline]
    pub(crate) fn record_text(&mut self, node_id: NodeId, text: &str) {
        if let Some(rec) = self.nodes.get_mut(&node_id) {
            match &mut rec.extras_mut().text {
                Some(existing) => {
                    existing.clear();
                    existing.push_str(text);
                }
                None => rec.extras_mut().text = Some(text.to_owned()),
            }
        }
    }

    /// Collects all live nodes whose `test_id` (`testID` / `data-testid` / `test_id`) equals `tid`,
    /// sorted deterministically by `NodeId`.
    fn collect_nodes_by_test_id(&self, tid: &str, out: &mut Vec<NodeId>) {
        out.clear();
        for (&nid, rec) in &self.nodes {
            if nid != ROOT_NODE_ID && rec.extras().and_then(|e| e.test_id.as_deref()) == Some(tid) {
                out.push(nid);
            }
        }
        out.sort_unstable();
    }

    /// Walks a structural child path starting from `ROOT_NODE_ID`.
    fn walk_usize_path(&self, path: &[usize]) -> Option<NodeId> {
        let mut cur = ROOT_NODE_ID;
        for &idx in path {
            let rec = self.nodes.get(&cur)?;
            cur = *rec.children.get(idx)?;
        }
        (cur != ROOT_NODE_ID).then_some(cur)
    }

    /// Collects all nodes in the subtrees rooted at `roots` (inclusive), sorted and deduped.
    fn collect_subtree_nodes(&self, roots: &[NodeId], out: &mut Vec<NodeId>) {
        out.clear();
        let mut stack: Vec<NodeId> = roots.to_vec();
        while let Some(nid) = stack.pop() {
            out.push(nid);
            if let Some(rec) = self.nodes.get(&nid) {
                for &cid in &rec.children {
                    stack.push(cid);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }

    /// Applies a Tier-1 live template/style/text hot patch surgically to matching nodes (<10ms).
    pub fn apply_hot_patch<S: RenderSink>(&mut self, sink: &mut S, patch: &HotPatchMsg) -> bool {
        let HotPatchMsg::TemplatePatch {
            template_id,
            attr_updates,
            text_updates,
            target_test_id,
            node_path,
            text_replacements,
            ..
        } = patch
        else {
            return false;
        };

        let mut changed = false;
        let mut matched_text_nodes: Vec<NodeId> = Vec::new();

        // 1. Targeted text patching: prefer exact `text_replacements` over positional `text_updates`.
        if !text_replacements.is_empty() {
            let mut scoped_ids: Vec<NodeId> = Vec::new();
            if let Some(tid) = target_test_id.as_deref() {
                let mut tid_roots = Vec::new();
                self.collect_nodes_by_test_id(tid, &mut tid_roots);
                if !tid_roots.is_empty() {
                    self.collect_subtree_nodes(&tid_roots, &mut scoped_ids);
                }
            }

            self.patch_ids.clear();
            self.patch_ids
                .extend(self.nodes.keys().copied().filter(|&id| id != ROOT_NODE_ID));
            self.patch_ids.sort_unstable();

            for (old_text, new_text) in text_replacements {
                let mut targets: Vec<NodeId> = Vec::new();
                // Prefer exact matches inside `target_test_id`'s subtree first.
                if !scoped_ids.is_empty() {
                    for &nid in &scoped_ids {
                        if self
                            .nodes
                            .get(&nid)
                            .and_then(|n| n.extras())
                            .is_some_and(|e| e.text.as_deref() == Some(old_text.as_str()))
                        {
                            targets.push(nid);
                        }
                    }
                }
                // Fall back to exact old-text match across the full tree (supports multi-edit
                // saves where a style targets one element and text lives on another).
                if targets.is_empty() {
                    for &nid in &self.patch_ids {
                        if self
                            .nodes
                            .get(&nid)
                            .and_then(|n| n.extras())
                            .is_some_and(|e| e.text.as_deref() == Some(old_text.as_str()))
                        {
                            targets.push(nid);
                        }
                    }
                }

                for nid in targets {
                    self.record_text(nid, new_text);
                    sink.set_text(nid, new_text);
                    matched_text_nodes.push(nid);
                    changed = true;
                }
            }
        } else if !text_updates.is_empty() {
            self.patch_leaf_text_ids.clear();
            if let Some(tid) = target_test_id.as_deref() {
                let mut tid_roots = Vec::new();
                self.collect_nodes_by_test_id(tid, &mut tid_roots);
                let mut subtree = Vec::new();
                self.collect_subtree_nodes(&tid_roots, &mut subtree);
                for nid in subtree {
                    if self
                        .nodes
                        .get(&nid)
                        .is_some_and(|n| n.kind == ComponentKind::Text && n.children.is_empty())
                    {
                        self.patch_leaf_text_ids.push(nid);
                    }
                }
            } else if !node_path.is_empty() {
                if let Some(root_nid) = self.walk_usize_path(node_path) {
                    let mut subtree = Vec::new();
                    self.collect_subtree_nodes(&[root_nid], &mut subtree);
                    for nid in subtree {
                        if self
                            .nodes
                            .get(&nid)
                            .is_some_and(|n| n.kind == ComponentKind::Text && n.children.is_empty())
                        {
                            self.patch_leaf_text_ids.push(nid);
                        }
                    }
                }
            } else {
                self.patch_ids.clear();
                self.patch_ids
                    .extend(self.nodes.keys().copied().filter(|&id| id != ROOT_NODE_ID));
                self.patch_ids.sort_unstable();
                for &id in &self.patch_ids {
                    if self
                        .nodes
                        .get(&id)
                        .is_some_and(|n| n.kind == ComponentKind::Text && n.children.is_empty())
                    {
                        self.patch_leaf_text_ids.push(id);
                    }
                }
            }

            let is_targeted = target_test_id.is_some() || !node_path.is_empty();
            for (idx, text) in text_updates {
                let target = self.patch_leaf_text_ids.get(*idx).copied().or_else(|| {
                    if is_targeted {
                        None
                    } else {
                        self.patch_ids.get(*idx).copied()
                    }
                });
                if let Some(nid) = target {
                    self.record_text(nid, text);
                    sink.set_text(nid, text);
                    changed = true;
                }
            }
        }

        // 2. Targeted attribute/style patching — never spray onto unrelated nodes!
        if !attr_updates.is_empty() {
            let mut attr_targets: Vec<NodeId> = Vec::new();
            if let Some(tid) = target_test_id.as_deref() {
                self.collect_nodes_by_test_id(tid, &mut attr_targets);
            } else if !node_path.is_empty() {
                if let Some(nid) = self.walk_usize_path(node_path) {
                    attr_targets.push(nid);
                }
            } else if !text_replacements.is_empty() {
                // Target only the node(s) matched by `text_replacements` (and their parent Text
                // element when Dioxus wraps a leaf text node inside `text { ... }`).
                for &nid in &matched_text_nodes {
                    attr_targets.push(nid);
                    if let Some(parent) = self.nodes.get(&nid).and_then(crate::writer::NodeRecord::parent)
                        && self
                            .nodes
                            .get(&parent)
                            .is_some_and(|p| p.kind == ComponentKind::Text)
                    {
                        attr_targets.push(parent);
                    }
                }
                attr_targets.sort_unstable();
                attr_targets.dedup();
            } else {
                // Legacy untargeted patch: prefer matching `template_id` as a `test_id` if present,
                // otherwise preserve legacy broadcast for non-root nodes.
                if !template_id.is_empty() {
                    self.collect_nodes_by_test_id(template_id, &mut attr_targets);
                }
                if attr_targets.is_empty() {
                    attr_targets.extend(self.nodes.keys().copied().filter(|&id| id != ROOT_NODE_ID));
                    attr_targets.sort_unstable();
                }
            }

            for (k, v) in attr_updates {
                for &nid in &attr_targets {
                    self.apply_str_attr(sink, nid, k, v);
                    changed = true;
                }
            }
        }

        changed
    }
}

#[cfg(test)]
mod tests {
    use nve_abi::hash::HashMap;

    use nve_abi::{ColorRgba, Command, CommandDecoder, NodeId, RawPropValue, StyleProps};
    use nve_engine::HotPatchMsg;

    use crate::prelude::*;
    use crate::DioxusSurface;

    #[component]
    fn MultiNodeSurface() -> Element {
        rsx! {
            view {
                test_id: "root-panel",
                style: "width: 400px; height: 300px; background-color: #0f172a;",
                view {
                    test_id: "card-alpha",
                    style: "padding: 8px; background-color: #1e293b; border-radius: 6px;",
                    text {
                        test_id: "alpha-label",
                        style: "color: #ffffff; font-size: 14px;",
                        "# engineering"
                    }
                }
                view {
                    test_id: "card-beta",
                    style: "padding: 8px; background-color: #334155; border-radius: 6px;",
                    text {
                        test_id: "beta-label",
                        style: "color: #94a3b8; font-size: 14px;",
                        "# design-sync"
                    }
                }
            }
        }
    }

    #[derive(Default)]
    struct MirrorNode {
        style: StyleProps,
        text: Option<String>,
        test_id: Option<String>,
    }

    fn apply_frame(bytes: &[u8], nodes: &mut HashMap<NodeId, MirrorNode>) {
        let (_, cmds) = CommandDecoder::new(bytes).unwrap().decode_all().unwrap();
        for cmd in cmds {
            match cmd {
                Command::CreateNode { node_id, .. } => {
                    nodes.entry(node_id).or_default();
                }
                Command::SetStyle { node_id, style } => {
                    nodes.entry(node_id).or_default().style = style;
                }
                Command::SetText { node_id, text } => {
                    nodes.entry(node_id).or_default().text = Some(text);
                }
                Command::SetRawProp {
                    node_id,
                    key,
                    value: RawPropValue::String(s),
                } if key == "testID" || key == "data-testid" || key == "test_id" => {
                    nodes.entry(node_id).or_default().test_id = Some(s);
                }
                _ => {}
            }
        }
    }

    fn node_id_by_test_id(nodes: &HashMap<NodeId, MirrorNode>, tid: &str) -> NodeId {
        nodes
            .iter()
            .find_map(|(&id, n)| (n.test_id.as_deref() == Some(tid)).then_some(id))
            .unwrap_or_else(|| panic!("missing test_id {tid}"))
    }

    #[test]
    fn targeted_template_patch_only_mutates_matching_node_and_preserves_siblings() {
        let mut surface = DioxusSurface::new(1, MultiNodeSurface);
        let mut nodes = HashMap::default();

        let (_, _, f1) = surface.render_frame(1);
        apply_frame(&f1, &mut nodes);

        let alpha_id = node_id_by_test_id(&nodes, "card-alpha");
        let beta_id = node_id_by_test_id(&nodes, "card-beta");
        let beta_bg_before = nodes[&beta_id].style.background;
        let beta_radius_before = nodes[&beta_id].style.border_radius;

        // 1. Targeted attr patch by `target_test_id: Some("card-alpha")` must NOT touch `card-beta`!
        assert!(surface.apply_hot_patch(&HotPatchMsg::TemplatePatch {
            generation: 2,
            template_id: "ui.rs".into(),
            attr_updates: vec![(
                "style".into(),
                "padding: 20px; background-color: #2563eb; border-radius: 16px;".into(),
            )],
            text_updates: vec![],
            target_test_id: Some("card-alpha".into()),
            node_path: vec![0, 0],
            text_replacements: vec![],
        }));
        let (_, _, f2) = surface.render_frame(2);
        apply_frame(&f2, &mut nodes);

        assert_eq!(nodes[&alpha_id].style.background, ColorRgba::rgb(37, 99, 235));
        assert_eq!(nodes[&alpha_id].style.border_radius, 16.0);
        // Sibling `card-beta` MUST remain completely untouched!
        assert_eq!(nodes[&beta_id].style.background, beta_bg_before);
        assert_eq!(nodes[&beta_id].style.border_radius, beta_radius_before);

        // 2. Targeted `text_replacements` patches ONLY "# engineering" -> "# ergoiherg" while
        // leaving "# design-sync" completely untouched.
        assert!(surface.apply_hot_patch(&HotPatchMsg::TemplatePatch {
            generation: 3,
            template_id: "ui.rs".into(),
            attr_updates: vec![],
            text_updates: vec![(0, "# ergoiherg".into())],
            target_test_id: None,
            node_path: vec![],
            text_replacements: vec![("# engineering".into(), "# ergoiherg".into())],
        }));
        let (_, _, f3) = surface.render_frame(3);
        apply_frame(&f3, &mut nodes);

        assert!(nodes.values().any(|n| n.text.as_deref() == Some("# ergoiherg")));
        assert!(!nodes.values().any(|n| n.text.as_deref() == Some("# engineering")));
        assert!(nodes.values().any(|n| n.text.as_deref() == Some("# design-sync")));

        // 3. Consecutive Tier-1 replacement chains cleanly from "# ergoiherg" -> "# runtime"!
        assert!(surface.apply_hot_patch(&HotPatchMsg::TemplatePatch {
            generation: 4,
            template_id: "ui.rs".into(),
            attr_updates: vec![],
            text_updates: vec![(0, "# runtime".into())],
            target_test_id: None,
            node_path: vec![],
            text_replacements: vec![("# ergoiherg".into(), "# runtime".into())],
        }));
        let (_, _, f4) = surface.render_frame(4);
        apply_frame(&f4, &mut nodes);

        assert!(nodes.values().any(|n| n.text.as_deref() == Some("# runtime")));
        assert!(nodes.values().any(|n| n.text.as_deref() == Some("# design-sync")));

        // 4. Unknown `target_test_id` must NOT spray attributes onto any node!
        assert!(surface.apply_hot_patch(&HotPatchMsg::TemplatePatch {
            generation: 5,
            template_id: "ui.rs".into(),
            attr_updates: vec![("opacity".into(), "0.1".into())],
            text_updates: vec![],
            target_test_id: Some("non-existent-id".into()),
            node_path: vec![],
            text_replacements: vec![],
        }));
        let (_, _, f5) = surface.render_frame(5);
        apply_frame(&f5, &mut nodes);
        assert_eq!(nodes[&alpha_id].style.opacity, 1.0);
        assert_eq!(nodes[&beta_id].style.opacity, 1.0);

        // 5. `node_path` targeting updates ONLY the addressed child (`[0, 1]` == `card-beta`).
        assert!(surface.apply_hot_patch(&HotPatchMsg::TemplatePatch {
            generation: 6,
            template_id: "ui.rs".into(),
            attr_updates: vec![("opacity".into(), "0.75".into())],
            text_updates: vec![],
            target_test_id: None,
            node_path: vec![0, 1],
            text_replacements: vec![],
        }));
        let (_, _, f6) = surface.render_frame(6);
        apply_frame(&f6, &mut nodes);
        assert_eq!(nodes[&beta_id].style.opacity, 0.75);
        assert_eq!(nodes[&alpha_id].style.opacity, 1.0);
    }
}
