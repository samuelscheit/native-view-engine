//! The mirror's tree model, shared by the native mirror, SSR and the live DOM path: DOM
//! `SetChildren` semantics (one parent per node, no cycles, no repeats, the root container never
//! a child) and a non-recursive depth-first walk for the serializers.

use nve_abi::hash::HashSet;
use nve_abi::{NodeId, ROOT_NODE_ID};

use crate::WebBackend;

/// Step of [`WebBackend::walk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Visit {
    Enter,
    Exit,
}

impl WebBackend {
    /// Applies DOM `SetChildren` semantics and returns the effective child list. Unknown ids,
    /// the root, repeated ids (first occurrence wins) and the parent itself or any of its
    /// ancestors (a cycle the DOM rejects with `HierarchyRequestError`) are skipped; a child
    /// attached elsewhere moves here (one parent per node, like `insertBefore`). `None` when the
    /// parent does not exist. Updates `parent_of` and the previous parents' child lists; the
    /// caller stores the returned list on the parent.
    pub(crate) fn adopt_children(&mut self, parent_id: NodeId, requested: &[NodeId]) -> Option<Vec<NodeId>> {
        let old = self.nodes.get(&parent_id)?.children.clone();
        // Only a node with children can be a proper ancestor of the parent, so the O(depth) walk
        // (the DOM's own hierarchy check) is skipped when every requested child is a leaf: building
        // a deep tree top-down stays linear.
        let mut ancestors = HashSet::default();
        ancestors.insert(parent_id);
        if requested
            .iter()
            .any(|c| *c != parent_id && self.nodes.get(c).is_some_and(|n| !n.children.is_empty()))
        {
            let mut cur = self.parent_of.get(&parent_id).copied();
            while let Some(id) = cur.filter(|&id| ancestors.insert(id)) {
                cur = self.parent_of.get(&id).copied();
            }
        }
        let mut seen = HashSet::with_capacity_and_hasher(requested.len(), Default::default());
        let children: Vec<NodeId> = requested
            .iter()
            .copied()
            .filter(|&c| {
                c != ROOT_NODE_ID && self.nodes.contains_key(&c) && !ancestors.contains(&c) && seen.insert(c)
            })
            .collect();
        for cid in old {
            if !seen.contains(&cid) && self.parent_of.get(&cid) == Some(&parent_id) {
                self.parent_of.remove(&cid);
            }
        }
        for &cid in &children {
            if let Some(prev) = self.parent_of.insert(cid, parent_id)
                && prev != parent_id
                && let Some(p) = self.nodes.get_mut(&prev)
            {
                p.children.retain(|&x| x != cid);
            }
        }
        Some(children)
    }

    /// Removes `node_id` from its parent's child list in O(children of that parent).
    pub(crate) fn detach_from_parent(&mut self, node_id: NodeId) {
        if let Some(parent) = self.parent_of.remove(&node_id)
            && let Some(p) = self.nodes.get_mut(&parent)
        {
            p.children.retain(|&cid| cid != node_id);
        }
    }

    /// Depth-first walk of the subtree at `root` with an explicit stack, so a deep tree cannot
    /// overflow the native stack. `visit(id, parent, Visit::Enter)` returns whether to descend;
    /// every node entered that way gets one `Visit::Exit` after its children (the return value
    /// is ignored there). Unknown ids are skipped. Terminates because [`Self::adopt_children`]
    /// keeps the mirror acyclic.
    pub(crate) fn walk(&self, root: NodeId, mut visit: impl FnMut(NodeId, Option<NodeId>, Visit) -> bool) {
        let mut stack = vec![(root, None, Visit::Enter)];
        while let Some((id, parent, step)) = stack.pop() {
            if step == Visit::Exit {
                visit(id, parent, Visit::Exit);
                continue;
            }
            let Some(el) = self.nodes.get(&id) else { continue };
            if visit(id, parent, Visit::Enter) {
                stack.push((id, parent, Visit::Exit));
                stack.extend(el.children.iter().rev().map(|&cid| (cid, Some(id), Visit::Enter)));
            }
        }
    }
}
