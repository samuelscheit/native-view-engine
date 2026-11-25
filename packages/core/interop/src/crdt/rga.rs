//! Replicated Growable Array (`RgaList<T>`) for convergent ordered sequences.
//!
//! Each element is uniquely identified by an [`OpId`] and anchored to `after_id: Option<OpId>`.
//! Concurrent insertions at the same anchor are ordered by descending [`OpId`]. Linear order
//! is computed by an iterative pre-order DFS over the anchor forest, guaranteeing strict
//! commutativity, associativity, idempotence, and resilience against out-of-order delivery
//! or malformed cycles.

use std::collections::{BTreeMap, BTreeSet};

use super::hlc::{OpId, VersionVector};

/// One element in an [`RgaList`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgaNode<T> {
    pub id: OpId,
    pub after_id: Option<OpId>,
    pub value: T,
    pub deleted: bool,
}

/// Causal delta extracted from an [`RgaList`] relative to a remote [`VersionVector`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgaDelta<T> {
    /// `(id, after_id, value)` insertions not yet observed by the remote peer.
    pub inserts: Vec<(OpId, Option<OpId>, T)>,
    /// `(target_id, remove_op)` tombstones not yet observed by the remote peer.
    pub deletes: Vec<(OpId, OpId)>,
}

impl<T> Default for RgaDelta<T> {
    fn default() -> Self {
        Self {
            inserts: Vec::new(),
            deletes: Vec::new(),
        }
    }
}

impl<T> RgaDelta<T> {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inserts.is_empty() && self.deletes.is_empty()
    }
}

/// Replicated Growable Array with tombstoned deletes and descending `OpId` tie-breaking.
#[derive(Debug, Clone)]
pub struct RgaList<T> {
    nodes: BTreeMap<OpId, RgaNode<T>>,
    tombstones: BTreeSet<OpId>,
    /// Earliest known `remove_op` per tombstoned node for minimal causal deltas.
    tombstone_ops: BTreeMap<OpId, OpId>,
}

impl<T> Default for RgaList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: PartialEq> PartialEq for RgaList<T> {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes && self.tombstones == other.tombstones
    }
}

impl<T: Eq> Eq for RgaList<T> {}

impl<T> RgaList<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            tombstones: BTreeSet::new(),
            tombstone_ops: BTreeMap::new(),
        }
    }

    /// Inserts `value` with unique `id` immediately after `after_id` (`None` = list head).
    /// Idempotent if `id` already exists; respects prior tombstones if delete arrived first.
    /// Rejects self-anchoring cycles by clamping `after_id == Some(id)` to `None`.
    pub fn insert_after(&mut self, after_id: Option<OpId>, id: OpId, value: T) -> bool {
        if self.nodes.contains_key(&id) {
            return false;
        }
        let sanitized_after = if after_id == Some(id) { None } else { after_id };
        let deleted = self.tombstones.contains(&id);
        self.nodes.insert(
            id,
            RgaNode {
                id,
                after_id: sanitized_after,
                value,
                deleted,
            },
        );
        true
    }

    /// Inserts `value` at visible position `index` (clamped to the current visible length).
    /// Returns the resolved `after_id` anchor used for the insertion.
    pub fn insert_at_index(&mut self, index: usize, id: OpId, value: T) -> Option<OpId> {
        let after_id = if index == 0 {
            None
        } else {
            let visible = self.ordered_ids(true);
            let anchor_idx = (index - 1).min(visible.len().saturating_sub(1));
            visible.get(anchor_idx).copied()
        };
        self.insert_after(after_id, id, value);
        after_id
    }

    /// Appends `value` after the last currently visible element (or head if empty).
    /// Returns the resolved `after_id` anchor.
    pub fn push_back(&mut self, id: OpId, value: T) -> Option<OpId> {
        let after_id = self.ordered_ids(true).last().copied();
        self.insert_after(after_id, id, value);
        after_id
    }

    /// Marks `target_id` as tombstoned (supports delete-before-insert arrival).
    /// Returns `true` if `target_id` was not already tombstoned.
    pub fn delete(&mut self, target_id: OpId) -> bool {
        self.delete_with_op(target_id, OpId::ZERO)
    }

    /// Marks `target_id` as tombstoned, associating `remove_op` for causal delta pruning.
    pub fn delete_with_op(&mut self, target_id: OpId, remove_op: OpId) -> bool {
        let newly = self.tombstones.insert(target_id);
        if remove_op.seq > 0 {
            self.tombstone_ops
                .entry(target_id)
                .and_modify(|existing| {
                    if existing.seq == 0 || remove_op < *existing {
                        *existing = remove_op;
                    }
                })
                .or_insert(remove_op);
        }
        if let Some(node) = self.nodes.get_mut(&target_id) {
            node.deleted = true;
        }
        newly
    }

    #[inline]
    pub fn get(&self, id: OpId) -> Option<&RgaNode<T>> {
        self.nodes.get(&id)
    }

    #[inline]
    pub fn is_deleted(&self, id: OpId) -> bool {
        self.tombstones.contains(&id)
    }

    #[inline]
    pub fn total_nodes(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn visible_len(&self) -> usize {
        self.nodes.values().filter(|n| !n.deleted).count()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.visible_len() == 0
    }

    /// Returns all visible nodes in deterministic RGA order.
    pub fn visible_nodes(&self) -> Vec<&RgaNode<T>> {
        self.ordered_ids(true)
            .into_iter()
            .filter_map(|id| self.nodes.get(&id))
            .collect()
    }

    /// Returns all nodes (including tombstones) in deterministic RGA order.
    pub fn all_nodes_ordered(&self) -> Vec<&RgaNode<T>> {
        self.ordered_ids(false)
            .into_iter()
            .filter_map(|id| self.nodes.get(&id))
            .collect()
    }

    /// Iterative pre-order DFS over the anchor forest with descending `OpId` sibling order.
    ///
    /// Guarantees termination even if out-of-order orphan nodes or malformed anchor cycles
    /// exist in `self.nodes`.
    fn ordered_ids(&self, visible_only: bool) -> Vec<OpId> {
        if self.nodes.is_empty() {
            return Vec::new();
        }

        // Group children by parent anchor. BTreeMap keys iterate in ascending OpId order,
        // so each child list ends up ascending; pushing onto the LIFO stack in ascending
        // order visits the largest OpId first (descending tie-break!).
        let mut roots: Vec<OpId> = Vec::new();
        let mut children: BTreeMap<OpId, Vec<OpId>> = BTreeMap::new();

        for (&id, node) in &self.nodes {
            match node.after_id {
                Some(parent) if self.nodes.contains_key(&parent) => {
                    children.entry(parent).or_default().push(id);
                }
                _ => {
                    roots.push(id);
                }
            }
        }

        let mut visited = BTreeSet::new();
        let mut stack: Vec<OpId> = Vec::with_capacity(self.nodes.len());
        let mut result: Vec<OpId> = Vec::with_capacity(self.nodes.len());

        // Roots are ascending in `self.nodes`, so pushing in order pops highest OpId first.
        stack.extend(roots);

        let drain_stack = |stack: &mut Vec<OpId>,
                           visited: &mut BTreeSet<OpId>,
                           result: &mut Vec<OpId>,
                           children: &BTreeMap<OpId, Vec<OpId>>,
                           nodes: &BTreeMap<OpId, RgaNode<T>>| {
            while let Some(id) = stack.pop() {
                if !visited.insert(id) {
                    continue;
                }
                if let Some(node) = nodes.get(&id)
                    && (!visible_only || !node.deleted)
                {
                    result.push(id);
                }
                if let Some(kids) = children.get(&id) {
                    stack.extend(kids.iter().copied());
                }
            }
        };

        drain_stack(&mut stack, &mut visited, &mut result, &children, &self.nodes);

        // Defensive fallback: if any cyclic component had no external root, visit remaining
        // nodes deterministically from highest OpId downward.
        if visited.len() < self.nodes.len() {
            let remaining: Vec<OpId> = self
                .nodes
                .keys()
                .copied()
                .filter(|id| !visited.contains(id))
                .collect();
            for id in remaining.into_iter().rev() {
                if visited.contains(&id) {
                    continue;
                }
                stack.push(id);
                drain_stack(&mut stack, &mut visited, &mut result, &children, &self.nodes);
            }
        }

        result
    }
}

impl<T: Clone> RgaList<T> {
    /// Materializes the current visible sequence in deterministic RGA order.
    pub fn to_vec(&self) -> Vec<T> {
        self.ordered_ids(true)
            .into_iter()
            .filter_map(|id| self.nodes.get(&id).map(|n| n.value.clone()))
            .collect()
    }

    /// Extracts all insertions and deletes not yet covered by `since`.
    pub fn delta_since(&self, since: &VersionVector) -> RgaDelta<T> {
        let mut delta = RgaDelta::default();
        for (&id, node) in &self.nodes {
            if !since.contains(id) {
                delta.inserts.push((id, node.after_id, node.value.clone()));
            }
        }
        for &target_id in &self.tombstones {
            let remove_op = self
                .tombstone_ops
                .get(&target_id)
                .copied()
                .unwrap_or(OpId::ZERO);
            if !since.contains(remove_op) {
                delta.deletes.push((target_id, remove_op));
            }
        }
        delta
    }

    /// Applies an [`RgaDelta`] idempotently. Returns `true` if logical state changed.
    pub fn merge_delta(&mut self, delta: &RgaDelta<T>) -> bool {
        let mut changed = false;
        for &(target_id, remove_op) in &delta.deletes {
            if self.delete_with_op(target_id, remove_op) {
                changed = true;
            }
        }
        for (id, after_id, value) in &delta.inserts {
            if self.insert_after(*after_id, *id, value.clone()) {
                changed = true;
            }
        }
        changed
    }

    /// Full join-semilattice merge with another [`RgaList`].
    /// Commutative, associative, and idempotent.
    pub fn merge(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for &target_id in &other.tombstones {
            let remove_op = other
                .tombstone_ops
                .get(&target_id)
                .copied()
                .unwrap_or(OpId::ZERO);
            if self.delete_with_op(target_id, remove_op) {
                changed = true;
            }
        }
        for (&id, node) in &other.nodes {
            if self.insert_after(node.after_id, id, node.value.clone()) {
                changed = true;
            }
        }
        changed
    }
}
