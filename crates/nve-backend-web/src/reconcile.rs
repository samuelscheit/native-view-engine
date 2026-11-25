//! O(N) head/tail-trimmed + Longest Increasing Subsequence (LIS) keyed child reconciler.
//!
//! Preserves DOM element identity, focus, `<input>` cursor/IME state, and `ScrollView` scroll
//! position by emitting `Remove` and `InsertBefore` operations ONLY for removed, inserted, or
//! genuinely moved children.

use std::collections::{HashMap, HashSet};

use nve_abi::NodeId;

/// Minimal DOM child mutation produced by [`plan_keyed_child_ops`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildDiffOp {
    Remove(NodeId),
    InsertBefore {
        node_id: NodeId,
        before: Option<NodeId>,
    },
}

/// Computes the minimal sequence of `Remove` and `InsertBefore` operations to reconcile `old`
/// child IDs into `new` child IDs using O(N) head/tail trimming + LIS stability.
pub fn plan_keyed_child_ops(old: &[NodeId], new: &[NodeId]) -> Vec<ChildDiffOp> {
    if old == new {
        return Vec::new();
    }

    let mut ops = Vec::new();
    let new_set: HashSet<NodeId> = new.iter().copied().collect();

    // 1. Remove obsolete children that are absent in `new`.
    let mut surviving = Vec::with_capacity(old.len().min(new.len()));
    let mut seen_surviving = HashSet::with_capacity(old.len());
    for &id in old {
        if new_set.contains(&id) && seen_surviving.insert(id) {
            surviving.push(id);
        } else {
            ops.push(ChildDiffOp::Remove(id));
        }
    }

    // 2. Trim matching prefix and suffix in O(N).
    let mut start = 0usize;
    let mut end_s = surviving.len();
    let mut end_n = new.len();

    while start < end_s && start < end_n && surviving[start] == new[start] {
        start += 1;
    }
    while end_s > start && end_n > start && surviving[end_s - 1] == new[end_n - 1] {
        end_s -= 1;
        end_n -= 1;
    }

    if start == end_n {
        return ops;
    }

    // Pure insertion into trimmed window.
    if start == end_s {
        let anchor = new.get(end_n).copied();
        for &node_id in &new[start..end_n] {
            ops.push(ChildDiffOp::InsertBefore {
                node_id,
                before: anchor,
            });
        }
        return ops;
    }

    // 3. Compute LIS of surviving positions inside the trimmed middle window.
    let mid_len = end_n - start;
    let mut old_pos_map: HashMap<NodeId, usize> = HashMap::with_capacity(end_s - start);
    for (idx, &id) in surviving[start..end_s].iter().enumerate() {
        old_pos_map.insert(id, idx);
    }

    let mut mapped_pos: Vec<Option<usize>> = Vec::with_capacity(mid_len);
    for &id in &new[start..end_n] {
        mapped_pos.push(old_pos_map.get(&id).copied());
    }

    let in_lis = compute_lis_mask(&mapped_pos);

    // 4. Walk backwards so each `before` anchor is already settled in its final position.
    for i in (0..mid_len).rev() {
        if in_lis[i] {
            continue;
        }
        let node_id = new[start + i];
        let before = if i + 1 < mid_len {
            Some(new[start + i + 1])
        } else {
            new.get(end_n).copied()
        };
        ops.push(ChildDiffOp::InsertBefore { node_id, before });
    }

    ops
}

fn compute_lis_mask(positions: &[Option<usize>]) -> Vec<bool> {
    let n = positions.len();
    let mut mask = vec![false; n];
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; n];

    for (i, &opt_pos) in positions.iter().enumerate() {
        let Some(val) = opt_pos else {
            continue;
        };
        let mut lo = 0usize;
        let mut hi = tails.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_val = positions[tails[mid]].unwrap_or(0);
            if mid_val < val {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo > 0 {
            prev[i] = Some(tails[lo - 1]);
        }
        if lo == tails.len() {
            tails.push(i);
        } else {
            tails[lo] = i;
        }
    }

    if let Some(&last_idx) = tails.last() {
        let mut cur = Some(last_idx);
        while let Some(idx) = cur {
            mask[idx] = true;
            cur = prev[idx];
        }
    }
    mask
}
